use crate::codegen::{prelude::*, ty_to_mlir};
use crate::diagnostic::codegen::CodegenSymptom;
use crate::parse::delimited_list;
use crate::prelude::*;
use crate::typeck::Ty;
use crate::ast::ModPath;

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
    pub args: Vec<Expr>,
}

pub fn tag_call<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, TagCall, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let args = delimited_list(Token::ParenOpen, expr, Token::Comma, Token::ParenClose);

    // Qualified form: Maybe.Some(x), Result.Ok(x) — uses Tag.Tag pattern
    let qualified = crate::ast::tag_variant_path()
        .then(args.clone())
        .map(|(path, args)| {
            let variant_name = *path.segments.last().unwrap_or(&path.root);
            TagCall {
                name: variant_name,
                qual_path: Some(path),
                args,
            }
        });

    // Simple form: Some(x), None()
    let simple = select! { Token::Tag(name) => IStr::new(name.to_string()) }
        .then(args)
        .map(|(name, args)| TagCall {
            name,
            qual_path: None,
            args,
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
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        // Try union variant construction first.
        if let Some((union_name, discriminant, payload_fields)) = ctx.ty_env.lookup_variant(self.name) {
            let union_mlir_ty = ctx
                .ty_env
                .lookup_tag(union_name)
                .map(|ty| ty_to_mlir(ty, ctx.mlir))
                .unwrap_or_else(|| ctx.mlir.union_type());
            let disc_val = block.const_i64(ctx.mlir, discriminant as i64);
            let mut val = block.append_op(ctx.mlir.llvm_undef(union_mlir_ty));
            val = block.append_op(ctx.mlir.llvm_insertvalue(val, disc_val, 0));
            for (slot, (arg, (_, field_ty))) in self.args.iter().zip(payload_fields.iter()).enumerate() {
                let lowered = arg.lower(ctx, block, symtab)?;
                let field_mlir_ty = ty_to_mlir(field_ty, ctx.mlir);
                let coerced = if field_mlir_ty == ctx.mlir.i1() {
                    let loc = ctx.location();
                    let extend_op = OperationBuilder::new("arith.extui", loc)
                        .add_operands(&[lowered])
                        .add_results(&[ctx.mlir.i64()])
                        .build()
                        .map_err(|e| CodegenSymptom::Internal(format!("extui build failed: {e}")))?;
                    block.append_op(extend_op)
                } else {
                    lowered
                };
                val = block.append_op(ctx.mlir.llvm_insertvalue(val, coerced, (slot + 1) as i64));
            }
            return Ok(val);
        }

        // Fall back to record construction.
        let record_ty = ctx
            .ty_env
            .lookup_tag(self.name)
            .cloned()
            .ok_or_else(|| {
                CodegenSymptom::Internal(format!(
                    "Unknown type '{}' — not declared as a union variant or record",
                    self.name.as_str()
                ))
            })?;

        match &record_ty {
            Ty::Record { .. } => {
                let fields = record_ty.record_fields_sorted();
                let struct_type = ty_to_mlir(&record_ty, ctx.mlir);
                let mut val = block.append_op(ctx.mlir.llvm_undef(struct_type));

                // Named construction: `Tag(field: val, ...)` — args parse as Bind expressions.
                let is_named = self.args.iter().any(|a| matches!(a, Expr::Bind(_)));
                if is_named {
                    for arg in &self.args {
                        let Expr::Bind(bind) = arg else {
                            return Err(CodegenSymptom::Internal(format!(
                                "Mixed named/positional args in record '{}' constructor",
                                self.name.as_str()
                            )));
                        };
                        let BindValue::Expr(value_expr) = bind.value() else {
                            return Err(CodegenSymptom::Internal(
                                "Named record arg must be a simple expression".to_string(),
                            ));
                        };
                        let field_name = bind.name();
                        let idx = fields
                            .iter()
                            .position(|(fname, _)| **fname == field_name)
                            .ok_or_else(|| {
                                CodegenSymptom::Internal(format!(
                                    "No field '{}' on record '{}'",
                                    field_name.as_str(),
                                    self.name.as_str()
                                ))
                            })?;
                        let arg_val = value_expr.lower(ctx, block, symtab)?;
                        val = block.append_op(ctx.mlir.llvm_insertvalue(val, arg_val, idx as i64));
                    }
                } else {
                    // Positional construction: `Tag(val1, val2, ...)` — args by layout order.
                    for (i, _) in fields.iter().enumerate() {
                        let arg_val = self
                            .args
                            .get(i)
                            .ok_or_else(|| {
                                CodegenSymptom::Internal(format!(
                                    "Not enough args for record '{}': expected {}, got {}",
                                    self.name.as_str(),
                                    fields.len(),
                                    self.args.len()
                                ))
                            })?
                            .lower(ctx, block, symtab)?;
                        val = block.append_op(ctx.mlir.llvm_insertvalue(val, arg_val, i as i64));
                    }
                }
                Ok(val)
            }
            _ => Err(CodegenSymptom::Internal(format!(
                "Tag '{}' is not a union variant or record constructor",
                self.name.as_str()
            ))),
        }
    }
}
