use crate::codegen::prelude::*;
use crate::codegen::{prelude::scf_dialect, ty_to_mlir};
use crate::diagnostic::codegen::CodegenSymptom;
use crate::intern::IStr;
use crate::prelude::*;
use crate::typeck::Ty;

use crate::ast::expr::r#return::Return;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IfCondition {
    Bool(Box<Expr>),
    Pattern { subject: Box<Expr>, tag: Tag },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IfExpr {
    pub condition: IfCondition,
    pub body: Vec<Expr>,
    pub ret: Return,
}

pub fn if_expr<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, IfExpr, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;
    use crate::ast::expr::r#return::r#return;

    // Parse: If <expr> [is <tag>]
    let condition = expr
        .clone()
        .then(just(Is).ignore_then(tag(expr.clone())).or_not())
        .map(|(subject, maybe_tag)| match maybe_tag {
            None => IfCondition::Bool(Box::new(subject)),
            Some(t) => IfCondition::Pattern {
                subject: Box::new(subject),
                tag: t,
            },
        });

    // Custom parser for if blocks:
    // - If <condition>
    // - Newline (repeated, to handle blank lines)
    // - Two forms:
    //   1. Indented form: Indent + body + Dedent (required) + return
    //   2. Non-indented form: body + (optional Dedent) + return
    // This ensures the if parser only consumes a dedent if it opened one.
    let indented_form = just(Indent)
        .ignore_then(expr.clone().repeated().collect::<Vec<_>>())
        .then_ignore(just(Dedent))  // REQUIRED when we saw Indent
        .then(r#return(expr.clone()))
        .map(|(body, ret)| (body, ret));

    let non_indented_form = expr
        .clone()
        .repeated()
        .collect::<Vec<_>>()
        .then_ignore(just(Dedent).or_not())  // Skip dedent if present (belongs to parent)
        .then(r#return(expr.clone()))
        .map(|(body, ret)| (body, ret));

    just(If)
        .ignore_then(condition)
        .then_ignore(just(Newline).repeated())  // Consume all newlines after condition
        .then(choice((
            indented_form,
            non_indented_form,
        )))
        .map(|(condition, (body, ret))| IfExpr { condition, body, ret })
}

impl<'c> Lower<'c> for IfExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        let loc = ctx.location();

        let cond_val = match &self.condition {
            IfCondition::Bool(expr) => {
                let cond_i64 = expr.lower(ctx, block, symtab)?;
                let loc = ctx.location();
                // Truncate i64 to i1 (boolean) for scf.if condition
                block.append_op(
                    OperationBuilder::new("arith.trunci", loc)
                        .add_operands(&[cond_i64])
                        .add_results(&[ctx.mlir.i1()])
                        .build()
                        .map_err(|e| CodegenSymptom::Internal(format!("arith.trunci: {e}")))?
                )
            }
            IfCondition::Pattern { subject, tag } => {
                let subject_val = subject.lower(ctx, block, symtab)?;
                let variant_name = IStr::new(tag.name().to_string());
                let (_, expected_disc, _) =
                    ctx.ty_env.lookup_variant(variant_name).ok_or_else(|| {
                        CodegenSymptom::Internal(format!(
                            "Unknown variant '{}' in if pattern",
                            tag.name()
                        ))
                    })?;
                let disc =
                    block.append_op(ctx.mlir.llvm_extractvalue(subject_val, 0, ctx.mlir.i64()));
                let expected_val = block.const_i64(ctx.mlir, expected_disc as i64);
                block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, disc, expected_val))
            }
        };

        // Infer the return type from the if block's return expression
        let ret_ty = self.ret.0.as_ref()
            .map(|e| ctx.ty_env.infer_expr(e, &std::collections::HashMap::new()))
            .unwrap_or(Ty::Int(64));
        let result_mlir = ty_to_mlir(&ret_ty, ctx.mlir);

        let then_region = Region::new();
        {
            let blk = Block::new(&[]);
            then_region.append_block(blk);
            let blk_ref = then_region.first_block().unwrap();
            let mut inner_symtab = symtab.clone();

            // Bind pattern variables if this is a pattern condition.
            if let IfCondition::Pattern { subject, tag } = &self.condition
                && let Tag::Generic(_, params) = tag
            {
                let subject_val = subject.lower(ctx, block, symtab)?;
                let variant_name = IStr::new(tag.name().to_string());
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
                    let extracted = blk_ref.append_op(ctx.mlir.llvm_extractvalue(
                        subject_val,
                        (slot + 1) as i64,
                        field_mlir_ty,
                    ));
                    inner_symtab.insert(param_name.as_str().to_string(), extracted);
                }
            }

            // Execute body expressions
            for expr in &self.body {
                expr.lower(ctx, &blk_ref, &mut inner_symtab)?;
            }

            // Yield the return value instead of calling llvm.return
            let ret_val = match &self.ret.0 {
                Some(expr) => expr.lower(ctx, &blk_ref, &mut inner_symtab)?,
                None => blk_ref.unit_value(ctx),
            };
            blk_ref.append_operation(scf_dialect::r#yield(&[ret_val], loc));
        }

        // Else region: yield a dummy value (won't be used if we return)
        let else_region = Region::new();
        {
            let blk = Block::new(&[]);
            else_region.append_block(blk);
            let blk_ref = else_region.first_block().unwrap();
            // Else block should never execute when we have early return semantics
            // Yield unit as a placeholder
            blk_ref.append_operation(scf_dialect::r#yield(&[blk_ref.unit_value(ctx)], loc));
        }

        // Emit scf.if that produces the return value
        let if_op = block.append_operation(scf_dialect::r#if(
            cond_val,
            &[result_mlir],
            then_region,
            else_region,
            loc,
        ));

        // Return the produced value from the if expression
        Ok(if_op.result(0).unwrap().into())
    }
}
