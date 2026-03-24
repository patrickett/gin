use crate::codegen::{prelude::*, ty_to_mlir};
use crate::diagnostic::codegen::CodegenSymptom;
use crate::prelude::*;

/// Exhaustive conditional expression.
///
/// Boolean condition form:
/// ```gin
/// when n % 15 = 0 then print("FizzBuzz")
///      n % 05 = 0 then print("Fizz")
///      n % 03 = 0 then print("Buzz")
///      else print(n)
/// ```
///
/// Pattern matching form:
/// ```gin
/// when value
///     is Some(x) then x
///     is None    then 0
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhenExpr {
    /// Subject expression for pattern matching (e.g., `when self`)
    /// None for condition-based when
    pub subject: Option<Box<Expr>>,
    pub arms: Vec<WhenArm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WhenArm {
    /// Boolean condition: `<condition> then <body>`
    Cond {
        condition: Box<Expr>,
        body: Box<Expr>,
    },
    /// Pattern match: `is <tag> then <body>`
    Is { pattern: Tag, body: Box<Expr> },
    /// Fallthrough: `else <body>`
    Else(Box<Expr>),
}

/// Internal enum for disambiguating the two when forms during parsing.
enum WhenTail {
    /// Boolean: the initial expr was a condition, here's the result + more arms
    Boolean {
        first_result: Expr,
        rest: Vec<WhenArm>,
    },
    /// Pattern: the initial expr was the subject, here are the is/else arms
    Pattern(Vec<WhenArm>),
}

pub fn when_expr<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, WhenExpr, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let tag_parser = tag(expr.clone());

    // is <Tag> then <expr> (inline, then on same line)
    let is_arm = just(Is)
        .ignore_then(tag_parser.clone())
        .then_ignore(just(Then))
        .then(expr.clone())
        .map(|(pattern, body)| WhenArm::Is {
            pattern,
            body: Box::new(body),
        });

    // <expr> then <expr>
    let cond_arm = expr
        .clone()
        .then_ignore(just(Then))
        .then(expr.clone())
        .map(|(condition, body)| WhenArm::Cond {
            condition: Box::new(condition),
            body: Box::new(body),
        });

    // else <expr>
    let else_arm = just(Else)
        .ignore_then(expr.clone())
        .map(|body| WhenArm::Else(Box::new(body)));

    // Pattern form starting with `Is <tag>`, handling both:
    //   - Inline: `is Some(x) then result [else ...]`
    //   - Indented then/else: `is Some(x)\n    then result\n    else ...`
    let pattern_form = just(Is)
        .ignore_then(tag_parser)
        .then(choice((
            // Inline: Then immediately follows the tag
            just(Then)
                .ignore_then(expr.clone())
                .then(
                    choice((is_arm.clone(), else_arm.clone()))
                        .repeated()
                        .collect::<Vec<_>>(),
                )
                .boxed(),
            // Indented: Then/else on next line(s) in an indented block
            just(Newline)
                .repeated()
                .at_least(1)
                .ignore_then(just(Indent))
                .ignore_then(
                    just(Then)
                        .ignore_then(expr.clone())
                        .then(
                            choice((is_arm.clone(), else_arm.clone()))
                                .repeated()
                                .collect::<Vec<_>>(),
                        ),
                )
                .then_ignore(just(Dedent).or_not())
                .boxed(),
        )))
        .map(|(pattern, (first_result, rest))| {
            let mut arms = vec![WhenArm::Is {
                pattern,
                body: Box::new(first_result),
            }];
            arms.extend(rest);
            WhenTail::Pattern(arms)
        })
        .boxed();

    // After `when <expr>`, the next token disambiguates:
    //   Then   → boolean form (the expr was a condition)
    //   Is     → pattern form (the expr was the subject)
    //   Indent → block pattern form (the expr was the subject, arms indented)
    just(When)
        .ignore_then(expr.clone())
        .then(choice((
            // Boolean form: Then <result>, optionally followed by more arms
            just(Then)
                .ignore_then(expr.clone())
                .then(
                    choice((
                        // Inline else
                        else_arm.clone().map(|arm| vec![arm]),
                        // Indented block of additional arms
                        just(Indent)
                            .ignore_then(
                                choice((cond_arm.clone(), else_arm.clone()))
                                    .repeated()
                                    .collect::<Vec<_>>(),
                            )
                            .then_ignore(just(Dedent).or_not()),
                    ))
                    .or_not(),
                )
                .map(|(first_result, rest)| WhenTail::Boolean {
                    first_result,
                    rest: rest.unwrap_or_default(),
                })
                .boxed(),
            // Pattern form: Is <tag> ...
            pattern_form,
            // Block pattern form: Indent (is|else arms)+ Dedent
            just(Indent)
                .ignore_then(
                    choice((is_arm, else_arm))
                        .repeated()
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .then_ignore(just(Dedent).or_not())
                .map(WhenTail::Pattern)
                .boxed(),
        )))
        .map(|(initial_expr, tail)| match tail {
            WhenTail::Boolean { first_result, rest } => {
                let mut arms = vec![WhenArm::Cond {
                    condition: Box::new(initial_expr),
                    body: Box::new(first_result),
                }];
                arms.extend(rest);
                WhenExpr {
                    subject: None,
                    arms,
                }
            }
            WhenTail::Pattern(arms) => WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
            },
        })
}

impl<'c> Lower<'c> for WhenExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        // Infer the result type from the else arm, falling back to the first arm.
        let result_ty = {
            let body = self
                .arms
                .iter()
                .find_map(|a| if let WhenArm::Else(b) = a { Some(b.as_ref()) } else { None })
                .or_else(|| {
                    self.arms.first().map(|a| match a {
                        WhenArm::Cond { body, .. }
                        | WhenArm::Is { body, .. }
                        | WhenArm::Else(body) => body.as_ref(),
                    })
                });
            body.map(|b| {
                let locals: std::collections::HashMap<IStr, crate::typeck::Ty> = ctx
                    .var_types
                    .borrow()
                    .iter()
                    .map(|(k, v)| (IStr::new(k.clone()), v.clone()))
                    .collect();
                let ty = ctx.ty_env.infer_expr(b, &locals);
                ty_to_mlir(&ty, ctx.mlir)
            })
            .unwrap_or_else(|| ctx.mlir.i64())
        };

        if let Some(subject_expr) = &self.subject {
            let subject = subject_expr.lower(ctx, block, symtab)?;
            let disc = block.append_op(ctx.mlir.llvm_extractvalue(subject, 0, ctx.mlir.i64()));
            lower_pattern_when(ctx, block, symtab, subject, disc, &self.arms, result_ty)
        } else {
            lower_boolean_when(ctx, block, symtab, &self.arms, result_ty)
        }
    }
}

fn lower_boolean_when<'c>(
    ctx: &CodegenContext<'_, 'c>,
    outer_block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    arms: &[WhenArm],
    result_ty: Type<'c>,
) -> Result<Value<'c, 'c>, CodegenSymptom> {
    let loc = ctx.location();

    let Some((head, tail)) = arms.split_first() else {
        return Ok(outer_block.const_i64(ctx.mlir, 0));
    };

    match head {
        WhenArm::Else(body) => body.lower(ctx, outer_block, symtab),

        WhenArm::Is { .. } => Err(CodegenSymptom::Internal(
            "Is arm in boolean when — use 'when subject is ...' form".to_string(),
        )),

        WhenArm::Cond { condition, body } => {
            let cond_val = condition.lower(ctx, outer_block, symtab)?;

            let value_producing = tail.iter().any(|a| matches!(a, WhenArm::Else(_)));
            let result_tys: Vec<Type<'c>> =
                if value_producing { vec![result_ty] } else { vec![] };

            let then_region = Region::new();
            {
                let blk = Block::new(&[]);
                then_region.append_block(blk);
                let blk_ref = then_region.first_block().unwrap();
                let val = body.lower(ctx, &blk_ref, &mut symtab.clone())?;
                if value_producing {
                    blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
                } else {
                    blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
                }
            }

            let else_region = Region::new();
            if !tail.is_empty() {
                let blk = Block::new(&[]);
                else_region.append_block(blk);
                let blk_ref = else_region.first_block().unwrap();
                let val =
                    lower_boolean_when(ctx, &blk_ref, &mut symtab.clone(), tail, result_ty)?;
                if value_producing {
                    blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
                } else {
                    blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
                }
            }

            let if_op = scf_dialect::r#if(cond_val, &result_tys, then_region, else_region, loc);
            let result_op = outer_block.append_operation(if_op);

            if value_producing {
                Ok(result_op.result(0).unwrap().into())
            } else {
                Ok(outer_block.const_i64(ctx.mlir, 0))
            }
        }
    }
}

/// Lower a pattern-matching `when` expression.
///
/// `subject` is the full union value (always `union_type()`).
/// `disc` is the pre-extracted discriminant (`i64`) from `subject[0]`.
///
/// Pattern matching always yields an `i64` value; non-exhaustive matches
/// fall through to `const_i64(0)` as a default.
fn lower_pattern_when<'c>(
    ctx: &CodegenContext<'_, 'c>,
    outer_block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    subject: Value<'c, 'c>,
    disc: Value<'c, 'c>,
    arms: &[WhenArm],
    result_ty: Type<'c>,
) -> Result<Value<'c, 'c>, CodegenSymptom> {
    use crate::intern::IStr;
    let loc = ctx.location();

    let Some((head, tail)) = arms.split_first() else {
        return Ok(outer_block.const_i64(ctx.mlir, 0));
    };

    match head {
        WhenArm::Else(body) => body.lower(ctx, outer_block, symtab),

        WhenArm::Cond { .. } => Err(CodegenSymptom::Internal(
            "Cannot mix condition arms in a pattern match (when subject is ...)".to_string(),
        )),

        WhenArm::Is { pattern, body } => {
            let variant_name = IStr::new(pattern.name().to_string());
            let (_, expected_disc, _) =
                ctx.ty_env.lookup_variant(variant_name).ok_or_else(|| {
                    CodegenSymptom::Internal(format!(
                        "Unknown variant '{}' in pattern",
                        variant_name.as_str()
                    ))
                })?;

            let expected_val = outer_block.const_i64(ctx.mlir, expected_disc as i64);
            let cond =
                outer_block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, disc, expected_val));

            let result_tys = vec![result_ty];

            // Build then-region: optionally bind payload fields, lower body.
            let then_region = Region::new();
            {
                let blk = Block::new(&[]);
                then_region.append_block(blk);
                let blk_ref = then_region.first_block().unwrap();
                let mut inner_symtab = symtab.clone();

                if let Tag::Generic(_, params) = pattern {
                    let payload_fields = ctx
                        .ty_env
                        .lookup_variant(variant_name)
                        .map(|(_, _, f)| f)
                        .unwrap_or(&[]);
                    for (slot, (param_name, _)) in params.iter().enumerate() {
                        if param_name.as_str() == "_" {
                            continue;
                        }
                        let field_mlir_ty = payload_fields
                            .get(slot)
                            .map(|(_, ty)| ty_to_mlir(ty, ctx.mlir))
                            .unwrap_or_else(|| ctx.mlir.i64());
                        let extracted = blk_ref.append_op(
                            ctx.mlir.llvm_extractvalue(subject, (slot + 1) as i64, field_mlir_ty),
                        );
                        inner_symtab.insert(param_name.as_str().to_string(), extracted);
                    }
                }

                let val = body.lower(ctx, &blk_ref, &mut inner_symtab)?;
                blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
            }

            // Build else-region: recurse on remaining arms.
            let else_region = Region::new();
            {
                let blk = Block::new(&[]);
                else_region.append_block(blk);
                let blk_ref = else_region.first_block().unwrap();
                let val = lower_pattern_when(
                    ctx, &blk_ref, &mut symtab.clone(), subject, disc, tail, result_ty,
                )?;
                blk_ref.append_operation(scf_dialect::r#yield(&[val], loc));
            }

            let if_op = scf_dialect::r#if(cond, &result_tys, then_region, else_region, loc);
            let result_op = outer_block.append_operation(if_op);
            Ok(result_op.result(0).unwrap().into())
        }
    }
}
