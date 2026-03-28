use crate::ast::ModPath;
use crate::codegen::{prelude::*, ty_to_mlir};
use crate::parse::delimited_list;
use crate::prelude::*;
use crate::typeck::Ty;
use chumsky::span::SimpleSpan;

/// A capitalized variant constructor call, e.g. `Some(5)` or `Maybe.Some(3)`.
///
/// Distinct from [`FnCall`] (which uses lowercase identifiers) — this constructs
/// a tagged union value with the given variant name and arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TagCall {
    /// Simple variant name (e.g., "Some") - used for variant lookup
    pub name: IStr,
    /// Optional qualified path (e.g., ModPath { root: "Maybe", segments: ["Some"] })
    pub qual_path: Option<ModPath>,
    pub args: Vec<Spanned<Expr>>,
    pub span: SimpleSpan,
}

pub fn tag_call<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, TagCall, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let args = delimited_list(Token::ParenOpen, expr, Token::Comma, Token::ParenClose);

    // Qualified form: Maybe.Some(x), Result.Ok(x) — uses Tag.Tag pattern
    let qualified =
        crate::ast::tag_variant_path()
            .then(args.clone())
            .map_with(|(path, args), e| {
                let variant_name = *path.segments.last().unwrap_or(&path.root);
                TagCall {
                    name: variant_name,
                    qual_path: Some(path),
                    args,
                    span: e.span(),
                }
            });

    // Simple form: Some(x), None()
    let simple = select! { Token::Tag(name) => IStr::new(name.to_string()) }
        .then(args)
        .map_with(|(name, args), e| TagCall {
            name,
            qual_path: None,
            args,
            span: e.span(),
        });

    // Prefer qualified to avoid ambiguity
    choice((qualified, simple))
}

impl<'c> Lower<'c> for TagCall {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        // Try union variant construction first.
        if let Some((union_name, discriminant, payload_fields)) =
            ctx.ty_env.lookup_variant(self.name)
        {
            let union_ty = ctx.ty_env.lookup_tag(union_name);
            let union_mlir_ty = union_ty
                .as_ref()
                .map(|ty| ty_to_mlir(ty, ctx.mlir))
                .unwrap_or_else(|| ctx.mlir.union_type());

            // Check if this is an optimized union with no fields (like Bool)
            let is_optimized_simple = union_ty.as_ref().is_some_and(|ty| {
                matches!(ty, Ty::Union { variants, .. } if variants.iter().all(|(_, fields)| fields.is_empty()))
            });

            if is_optimized_simple {
                // For optimized unions (like Bool), create a simple integer constant
                let variant_count = union_ty
                    .as_ref()
                    .and_then(|ty| {
                        if let Ty::Union { variants, .. } = ty {
                            Some(variants.len())
                        } else {
                            None
                        }
                    })
                    .unwrap_or(2);

                return if variant_count == 2 {
                    // Use i1 for 2-variant unions like Bool
                    let i1_attr = melior::ir::attribute::IntegerAttribute::new(
                        IntegerType::new(ctx.mlir, 1).into(),
                        discriminant as i64,
                    )
                    .into();
                    Some(block.append_op(ctx.mlir.const_op(i1_attr, ctx.mlir.i1())))
                } else if variant_count <= 256 {
                    // Use i8 for 3-256 variant unions
                    let i8_ty = IntegerType::new(ctx.mlir, 8).into();
                    let i8_attr =
                        melior::ir::attribute::IntegerAttribute::new(i8_ty, discriminant as i64)
                            .into();
                    Some(block.append_op(ctx.mlir.const_op(i8_attr, i8_ty)))
                } else {
                    // Fall back to i64 for larger unions
                    Some(block.const_i64(ctx.mlir, discriminant as i64))
                };
            }

            // Standard union construction with struct representation
            let disc_val = block.const_i64(ctx.mlir, discriminant as i64);
            let mut val = block.append_op(ctx.mlir.llvm_undef(union_mlir_ty));
            val = block.append_op(ctx.mlir.llvm_insertvalue(val, disc_val, 0));
            for (slot, (arg, (_, field_ty))) in
                self.args.iter().zip(payload_fields.iter()).enumerate()
            {
                let lowered = arg.lower(ctx, block, symtab)?;
                let field_mlir_ty = ty_to_mlir(field_ty, ctx.mlir);
                let coerced = if field_mlir_ty == ctx.mlir.i1() {
                    let loc = ctx.location();
                    let extend_op = match OperationBuilder::new("arith.extui", loc)
                        .add_operands(&[lowered])
                        .add_results(&[ctx.mlir.i64()])
                        .build()
                    {
                        Ok(op) => op,
                        Err(e) => {
                            ctx.emit_internal(format!("extui build failed: {e}"));
                            return None;
                        }
                    };
                    block.append_op(extend_op)
                } else {
                    lowered
                };
                val = block.append_op(ctx.mlir.llvm_insertvalue(val, coerced, (slot + 1) as i64));
            }
            return Some(val);
        }

        // Fall back to record construction.
        // Note: unknown tag diagnostics are emitted by typeck; codegen just fails gracefully.
        let record_ty = match ctx.ty_env.lookup_tag(self.name).cloned() {
            Some(ty) => ty,
            None => return None,
        };

        match &record_ty {
            Ty::Record { .. } => {
                let fields = record_ty.record_fields_sorted();
                let struct_type = ty_to_mlir(&record_ty, ctx.mlir);
                let mut val = block.append_op(ctx.mlir.llvm_undef(struct_type));

                // Named construction: `Tag(field: val, ...)` — args parse as Bind expressions.
                let is_named = self.args.iter().any(|a| matches!(&a.0, Expr::Bind(_)));
                if is_named {
                    for arg in &self.args {
                        let Expr::Bind(bind) = &arg.0 else {
                            ctx.emit_internal(format!(
                                "Mixed named/positional args in record '{}' constructor",
                                self.name.as_str()
                            ));
                            return None;
                        };
                        let BindValue::Expr(value_expr) = bind.value() else {
                            ctx.emit_internal("Named record arg must be a simple expression");
                            return None;
                        };
                        let field_name = bind.name();
                        let idx = match fields.iter().position(|(fname, _)| **fname == field_name) {
                            Some(i) => i,
                            None => {
                                ctx.emit_internal(format!(
                                    "No field '{}' on record '{}'",
                                    field_name.as_str(),
                                    self.name.as_str()
                                ));
                                return None;
                            }
                        };
                        let arg_val = value_expr.lower(ctx, block, symtab)?;
                        val = block.append_op(ctx.mlir.llvm_insertvalue(val, arg_val, idx as i64));
                    }
                } else {
                    // Positional construction: `Tag(val1, val2, ...)` — args by layout order.
                    for (i, _) in fields.iter().enumerate() {
                        let arg = match self.args.get(i) {
                            Some(a) => a,
                            None => {
                                ctx.emit_internal(format!(
                                    "Not enough args for record '{}': expected {}, got {}",
                                    self.name.as_str(),
                                    fields.len(),
                                    self.args.len()
                                ));
                                return None;
                            }
                        };
                        let arg_val = arg.lower(ctx, block, symtab)?;
                        val = block.append_op(ctx.mlir.llvm_insertvalue(val, arg_val, i as i64));
                    }
                }
                Some(val)
            }
            _ => None,
        }
    }
}
