//! ASM builder intrinsics — fold `AsmBuilder` method chains and inject inline
//! assembly expressions.
//!
//! The `AsmBuilder` compile-time flow works in two phases:
//! 1. [`fold_compile_time_binds`][crate::analysis::const_expr::fold_compile_time_binds]
//!    evaluates the `AsmBuilder::new → .input/.inout → .build` chain into a
//!    `ConstValue::Record` (the "AsmSpec").
//! 2. [`inject_asm_exprs`] rewrites `asm(spec)` calls to populate their
//!    template, operands, and clobbers fields from the folded spec.

use ControlFlow::Continue;
use std::collections::HashMap;
use std::ops::ControlFlow;

use internment::Intern;

use ast::ConstValue;
use ast::FileAst;
use ast::expr::{BindValue, Expr, Typed};
use ast::folder::Folder;

/// Collect all foldable asm spec binds from the AST.
fn collect_folded_specs(ast: &FileAst) -> HashMap<Intern<String>, ConstValue> {
    ast.defs
        .iter()
        .filter_map(|(name, bind)| {
            if !bind.is_compile_time {
                return None;
            }
            match &bind.value {
                BindValue::Expr(e) => e.const_value.clone().map(|cv| (*name, cv)),
                _ => None,
            }
        })
        .collect()
}

/// Rewrite `asm()` calls that reference a folded AsmSpec constant into
/// fully populated `Expr::Asm` nodes.
///
/// Looks for `Expr::Asm` nodes where `spec_expr` is set, resolves the
/// spec to its const-folded value, and fills in `template`, `operands`,
/// and `clobbers` from the `AsmSpec` record.
pub fn inject_asm_exprs(ast: &mut FileAst) {
    // Phase 1: Collect all folded constant values (immutable borrow)
    let folded: HashMap<Intern<String>, ConstValue> = collect_folded_specs(ast);

    // Phase 2: Walk and modify (mutable borrow, using only the folded map)
    for bind in ast.defs.values_mut() {
        inject_asm_exprs_in_value(&mut bind.value, &folded);
    }
}

struct AsmInjector<'a> {
    folded: &'a HashMap<Intern<String>, ConstValue>,
}

impl Folder for AsmInjector<'_> {
    fn visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        if let Expr::Asm(asm) = expr {
            let Some(spec) = asm.spec_expr.take() else {
                return Continue(());
            };
            let cv = resolve_spec_const_only(&spec, self.folded);
            if let Some(ConstValue::Record { fields }) = cv {
                let template = fields
                    .iter()
                    .find(|(n, _)| n.as_str() == "template")
                    .and_then(|(_, v)| match v {
                        ConstValue::String(s) => Some(Intern::new(s.clone())),
                        _ => None,
                    });
                let operands_list = fields
                    .iter()
                    .find(|(n, _)| n.as_str() == "operands")
                    .and_then(|(_, v)| match v {
                        ConstValue::List(items) => Some(items.clone()),
                        _ => None,
                    });
                let clobbers_list = fields
                    .iter()
                    .find(|(n, _)| n.as_str() == "clobbers")
                    .and_then(|(_, v)| match v {
                        ConstValue::List(items) => Some(items.clone()),
                        _ => None,
                    });

                if let (Some(template), Some(operands), Some(clobbers)) =
                    (template, operands_list, clobbers_list)
                {
                    asm.template = template;
                    asm.operands = convert_operands(&operands);
                    asm.clobbers = convert_clobbers(&clobbers);
                }
            }
            Continue(())
        } else {
            ast::folder::walk_expr_mut(self, expr)
        }
    }
}

fn inject_asm_exprs_in_value(value: &mut BindValue, folded: &HashMap<Intern<String>, ConstValue>) {
    let mut injector = AsmInjector { folded };
    let _ = ast::folder::walk_bind_value_mut(&mut injector, value);
}

/// Resolve an `AsmSpec` expression to its const-folded `ConstValue`.
fn resolve_spec_const_only(
    spec: &Typed<Expr>,
    folded: &HashMap<Intern<String>, ConstValue>,
) -> Option<ConstValue> {
    match &spec.value {
        Expr::Bind(b) => folded.get(&b.name).cloned(),
        Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty() => {
            folded.get(&call.path.value.root).cloned()
        }
        _ => None,
    }
}

/// Convert a list of `ConstValue` operand descriptors to `Vec<OperandSpec>`.
fn convert_operands(operands: &[ConstValue]) -> Vec<ast::expr::OperandSpec> {
    use ast::expr::{OperandKind, OperandSpec};
    operands
        .iter()
        .filter_map(|op| {
            let kind = op.get_field("kind").and_then(|k| match k {
                ConstValue::Tag { name, .. } => match name.as_str() {
                    "Input" => Some(OperandKind::Input),
                    "Output" => Some(OperandKind::Output),
                    "InOut" => Some(OperandKind::InOut),
                    "LateOut" => Some(OperandKind::LateOut),
                    _ => None,
                },
                _ => None,
            })?;
            let register = op
                .get_field("register")
                .and_then(|r| r.as_str())
                .map(|s| Intern::new(s.to_string()))?;
            Some(OperandSpec { kind, register })
        })
        .collect()
}

/// Convert a list of `ConstValue` clobber descriptors to `Vec<ClobberSpec>`.
fn convert_clobbers(clobbers: &[ConstValue]) -> Vec<ast::expr::ClobberSpec> {
    use ast::expr::ClobberSpec;
    clobbers
        .iter()
        .filter_map(|c| match c {
            ConstValue::Tag { name, args, .. } if name.as_str() == "ClobberMemory" => {
                Some(ClobberSpec::Memory)
            }
            ConstValue::Tag { name, args, .. } if name.as_str() == "ClobberRegister" => {
                let reg = args.first().and_then(|a| a.as_str())?;
                Some(ClobberSpec::Register(Intern::new(reg.to_string())))
            }
            _ => None,
        })
        .collect()
}

/// Try to fold an AsmBuilder method call (new, input, inout, output, clobber,
/// clobber_memory, build) into a `ConstValue::Record`.
///
/// This is `pub(crate)` because it's called from
/// [`analysis::const_expr`][crate::analysis::const_expr] during compile-time
/// expression evaluation.
pub(crate) fn try_fold_asm_builder(method: &str, args: &[ConstValue]) -> Option<ConstValue> {
    match method {
        "new" if args.len() == 1 => {
            let template = &args[0];
            let template_str = match template {
                ConstValue::String(s) => s.clone(),
                ConstValue::Tag { args, .. } if args.len() == 1 => {
                    if let ConstValue::String(s) = &args[0] {
                        s.clone()
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            Some(ConstValue::Record {
                fields: vec![
                    (
                        Intern::new("template".to_string()),
                        ConstValue::String(template_str),
                    ),
                    (
                        Intern::new("operands".to_string()),
                        ConstValue::List(Vec::new()),
                    ),
                    (
                        Intern::new("clobbers".to_string()),
                        ConstValue::List(Vec::new()),
                    ),
                ],
            })
        }
        "input" | "output" | "inout" if args.len() >= 2 => {
            let builder = &args[0];
            let builder_template = builder.get_field("template")?.clone();
            let mut builder_operands = builder.get_field("operands")?.as_list()?.to_vec();
            let builder_clobbers = builder.get_field("clobbers")?.as_list()?.to_vec();

            let reg = extract_register_name(&args[1])?;
            let kind_str = match method {
                "input" => "Input",
                "output" => "Output",
                "inout" => "InOut",
                _ => unreachable!(),
            };
            builder_operands.push(ConstValue::Record {
                fields: vec![
                    (
                        Intern::new("kind".to_string()),
                        ConstValue::Tag {
                            name: Intern::new(kind_str.to_string()),
                            qual_path: None,
                            args: Vec::new(),
                        },
                    ),
                    (Intern::new("register".to_string()), ConstValue::String(reg)),
                ],
            });
            Some(ConstValue::Record {
                fields: vec![
                    (Intern::new("template".to_string()), builder_template),
                    (
                        Intern::new("operands".to_string()),
                        ConstValue::List(builder_operands),
                    ),
                    (
                        Intern::new("clobbers".to_string()),
                        ConstValue::List(builder_clobbers),
                    ),
                ],
            })
        }
        "lateout" if args.len() >= 2 => {
            let builder = &args[0];
            let builder_template = builder.get_field("template")?.clone();
            let mut builder_operands = builder.get_field("operands")?.as_list()?.to_vec();
            let builder_clobbers = builder.get_field("clobbers")?.as_list()?.to_vec();

            let reg = extract_register_name(&args[1])?;
            builder_operands.push(ConstValue::Record {
                fields: vec![
                    (
                        Intern::new("kind".to_string()),
                        ConstValue::Tag {
                            name: Intern::new("LateOut".to_string()),
                            qual_path: None,
                            args: Vec::new(),
                        },
                    ),
                    (Intern::new("register".to_string()), ConstValue::String(reg)),
                ],
            });
            Some(ConstValue::Record {
                fields: vec![
                    (Intern::new("template".to_string()), builder_template),
                    (
                        Intern::new("operands".to_string()),
                        ConstValue::List(builder_operands),
                    ),
                    (
                        Intern::new("clobbers".to_string()),
                        ConstValue::List(builder_clobbers),
                    ),
                ],
            })
        }
        "clobber" if args.len() >= 2 => {
            let builder = &args[0];
            let builder_template = builder.get_field("template")?.clone();
            let builder_operands = builder.get_field("operands")?.as_list()?.to_vec();
            let mut builder_clobbers = builder.get_field("clobbers")?.as_list()?.to_vec();

            let reg = extract_register_name(&args[1])?;
            builder_clobbers.push(ConstValue::Tag {
                name: Intern::new("ClobberRegister".to_string()),
                qual_path: None,
                args: vec![ConstValue::String(reg)],
            });
            Some(ConstValue::Record {
                fields: vec![
                    (Intern::new("template".to_string()), builder_template),
                    (
                        Intern::new("operands".to_string()),
                        ConstValue::List(builder_operands),
                    ),
                    (
                        Intern::new("clobbers".to_string()),
                        ConstValue::List(builder_clobbers),
                    ),
                ],
            })
        }
        "clobber_memory" if !args.is_empty() => {
            let builder = &args[0];
            let builder_template = builder.get_field("template")?.clone();
            let builder_operands = builder.get_field("operands")?.as_list()?.to_vec();
            let mut builder_clobbers = builder.get_field("clobbers")?.as_list()?.to_vec();

            builder_clobbers.push(ConstValue::Tag {
                name: Intern::new("ClobberMemory".to_string()),
                qual_path: None,
                args: Vec::new(),
            });
            Some(ConstValue::Record {
                fields: vec![
                    (Intern::new("template".to_string()), builder_template),
                    (
                        Intern::new("operands".to_string()),
                        ConstValue::List(builder_operands),
                    ),
                    (
                        Intern::new("clobbers".to_string()),
                        ConstValue::List(builder_clobbers),
                    ),
                ],
            })
        }
        "build" if !args.is_empty() => {
            let builder = &args[0];
            let builder_template = builder.get_field("template")?.clone();
            let builder_operands = builder.get_field("operands")?.as_list()?.to_vec();
            let builder_clobbers = builder.get_field("clobbers")?.as_list()?.to_vec();

            Some(ConstValue::Record {
                fields: vec![
                    (Intern::new("template".to_string()), builder_template),
                    (
                        Intern::new("operands".to_string()),
                        ConstValue::List(builder_operands),
                    ),
                    (
                        Intern::new("clobbers".to_string()),
                        ConstValue::List(builder_clobbers),
                    ),
                ],
            })
        }
        _ => None,
    }
}

/// Extract the register name from a ConstValue (handles `Tag { name: "X0", args: [] }`
/// or `Record { fields: [("value", String("x0"))] }`).
fn extract_register_name(cv: &ConstValue) -> Option<String> {
    match cv {
        ConstValue::Tag { args, .. } => {
            if args.is_empty() {
                None
            } else {
                match &args[0] {
                    ConstValue::String(s) => Some(s.clone()),
                    _ => None,
                }
            }
        }
        ConstValue::Record { fields } => fields
            .iter()
            .find(|(name, _)| name.as_str() == "value")
            .and_then(|(_, v)| match v {
                ConstValue::String(s) => Some(s.clone()),
                _ => None,
            }),
        ConstValue::String(s) => Some(s.clone()),
        _ => None,
    }
}
