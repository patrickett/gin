//! JSON serialization helpers for Gin types and ASTs.
//! Used to format JSON responses for the LSP.

use ast::source::SourceExt;
use ast::ty::Ty;
use ast::{
    BindValue, ConstValue, DeclareValue, Expr, FileAst, HasFunctionKind, HasMember, HasSpanId,
    SpanId, SpanTable, UnionVariant,
};
use serde_json::Value;

pub struct JsonSerializer;

impl JsonSerializer {
    /// Serialize a resolved `Ty` to a JSON structure with kind, fields, size, and alignment.
    pub fn ty_to_json(ty: &Ty) -> Value {
        match ty {
            Ty::Int { width, signed, .. } => serde_json::json!({
                "kind": "Int", "width": width, "signed": signed,
            }),
            Ty::Float { .. } => serde_json::json!({
                "kind": "Float",
            }),
            Ty::Unit => serde_json::json!({ "kind": "Unit", "size": 0 }),
            Ty::Record { name, fields } => {
                let flds: Vec<Value> = fields
                    .iter()
                    .map(
                        |(fn_, ft)| serde_json::json!({ "name": fn_.as_str(), "type": Self::ty_to_json(ft) }),
                    )
                    .collect();
                serde_json::json!({
                    "kind": "Record", "name": name.as_str(),
                    "fields": flds,
                })
            }
            Ty::Union {
                name,
                variants,
                literal_values,
                ..
            } => {
                let vars: Vec<Value> = variants
                    .iter()
                    .map(|UnionVariant { name: vn, fields, .. }| {
                        let flds: Vec<Value> = fields
                            .iter()
                            .map(|(fn_, ft)| {
                                serde_json::json!({ "name": fn_.as_str(), "type": Self::ty_to_json(ft) })
                            })
                            .collect();
                        serde_json::json!({ "name": vn.as_str(), "fields": flds })
                    })
                    .collect();
                let mut obj = serde_json::json!({
                    "kind": "Union", "name": name.as_str(), "variants": vars,
                });
                if literal_values.is_some() {
                    obj["has_literal_values"] = Value::Bool(true);
                }
                obj
            }
            Ty::Opaque(name) => serde_json::json!({ "kind": "Opaque", "name": name.as_str() }),
            Ty::Array { elem, size } => serde_json::json!({
                "kind": "Array", "elem": Self::ty_to_json(elem), "length": size.to_string(),
            }),
            Ty::Ptr { inner } => serde_json::json!({
                "kind": "Ptr", "inner": Self::ty_to_json(inner),
            }),
            Ty::Ref { inner, mutable } => serde_json::json!({
                "kind": if *mutable { "Mut" } else { "Ref" },
                "inner": Self::ty_to_json(inner),
            }),
            Ty::Literal(cv) => {
                let value_str = match cv {
                    ConstValue::String(s) => s.clone(),
                    ConstValue::Int(n) => n.to_string(),
                    ConstValue::Float(f) => f.to_string(),
                    ConstValue::Tag { name: tn, .. } => tn.as_str().to_string(),
                    ConstValue::Record { .. } => "<record>".to_string(),
                    ConstValue::List(_) => "<list>".to_string(),
                };
                serde_json::json!({ "kind": "Literal", "value": value_str })
            }
            Ty::Tuple(items) => {
                let items_json: Vec<Value> = items.iter().map(Self::ty_to_json).collect();
                serde_json::json!({ "kind": "Tuple", "items": items_json })
            }
        }
    }

    /// Serialize a full AST to JSON (all levels).
    pub fn ast_to_json(ast: &FileAst, source: &str) -> Value {
        Self::ast_to_json_with_depth(ast, source, None)
    }

    /// Serialize an AST to JSON with an optional maximum recursion depth.
    pub fn ast_to_json_with_depth(ast: &FileAst, source: &str, max_depth: Option<usize>) -> Value {
        let span_table = &ast.span_table;

        let defs: Vec<Value> = ast
            .defs
            .iter()
            .map(|(name, bind)| {
                let mut obj = serde_json::json!({
                    "name": name.as_str(),
                    "kind": if bind.params.is_some() { "function" } else { "bind" },
                    "private": ast.private_defs.contains(name),
                });
                if let Some(params) = bind.params.as_ref() {
                    obj["params"] = Self::params_json(params);
                }
                if let Some(doc) = bind.doc_comment.as_ref() {
                    obj["doc"] = Value::String(doc.value.clone());
                }
                obj["value"] = Self::bind_val_json(&bind.value, span_table, source, 0, max_depth);
                obj
            })
            .collect();

        let tags: Vec<Value> = ast
            .tags
            .iter()
            .map(|(name, decl)| {
                let mut obj = serde_json::json!({
                    "name": name.as_str(), "kind": "tag",
                    "private": ast.private_tags.contains(name),
                });
                if let Some(params) = decl.params.as_ref() {
                    obj["params"] = Self::params_json(params);
                }
                if let Some(doc) = decl.doc_comment.as_ref() {
                    obj["doc"] = Value::String(doc.value.clone());
                }
                obj["value"] = Self::declare_value_json(&decl.value, span_table, source);
                obj
            })
            .collect();

        let uses: Vec<Value> = ast
            .uses
            .iter()
            .flat_map(|import| {
                import.0.iter().map(|mi| {
                    let (sl, sc) =
                        source.byte_offset_to_position(span_table.get(mi.span_id()).start());
                    serde_json::json!({
                        "source": format!("{:?}", mi.source),
                        "alias": mi.alias.as_ref().map(|a| a.as_str()),
                        "line": sl, "character": sc,
                    })
                })
            })
            .collect();

        let top_exprs: Vec<Value> = ast
            .exprs
            .iter()
            .map(|(e, sid)| Self::expr_json(e, *sid, span_table, source, 0, max_depth))
            .collect();

        serde_json::json!({
            "defs": defs, "tags": tags, "uses": uses,
            "top_level_exprs": top_exprs,
            "has_module_doc": ast.module_doc.is_some(),
            "module_doc": ast.module_doc.as_ref().map(|d| d.value.as_str()),
        })
    }

    fn declare_value_json(value: &DeclareValue, span_table: &SpanTable, source: &str) -> Value {
        match value {
            DeclareValue::Has(members) => {
                let mems: Vec<Value> = members
                    .iter()
                    .map(|m| match m {
                        HasMember::Property(p) => {
                            let ty_src = p.ty.as_ref().map(|ty| {
                                let sp = span_table.get(ty.span_id());
                                source.get(sp.start()..sp.end()).unwrap_or("<span err>")
                            });
                            serde_json::json!({
                                "kind": "property",
                                "name": p.name.as_str(),
                                "type": ty_src,
                            })
                        }
                        HasMember::Function(f) => {
                            let return_src = f.return_ty.as_ref().map(|rt| {
                                let sp = span_table.get(rt.span_id);
                                source.get(sp.start()..sp.end()).unwrap_or("<span err>")
                            });
                            let error_src = f.error_ty.as_ref().map(|et| {
                                let sp = span_table.get(et.span_id);
                                source.get(sp.start()..sp.end()).unwrap_or("<span err>")
                            });
                            let kind = match f.kind {
                                HasFunctionKind::Instance => "instance_method",
                                HasFunctionKind::Associated => "associated_method",
                            };
                            serde_json::json!({
                                "kind": kind,
                                "name": f.name.as_str(),
                                "params": Self::params_json(&f.params),
                                "return_ty": return_src,
                                "error_ty": error_src,
                            })
                        }
                    })
                    .collect();
                serde_json::json!({"kind": "has", "members": mems})
            }
            DeclareValue::Alias(sp) => {
                let span = span_table.get(sp.span_id);
                let src = source.get(span.start()..span.end()).unwrap_or("<span err>");
                serde_json::json!({"kind": "alias", "type_expr": src})
            }
            DeclareValue::Union { variants } => {
                let vars: Vec<Value> = variants
                    .iter()
                    .map(|v| {
                        let span = span_table.get(v.shape().span_id);
                        let shape_src =
                            source.get(span.start()..span.end()).unwrap_or("<span err>");
                        let mut obj = serde_json::json!({
                            "shape": shape_src,
                        });
                        match v {
                            ast::Variant::External { .. } => {
                                obj["kind"] = Value::String("external".into());
                            }
                            ast::Variant::Local { doc_comment, .. } => {
                                obj["kind"] = Value::String("local".into());
                                if let Some(doc) = doc_comment {
                                    obj["doc"] = Value::String(doc.value.clone());
                                }
                            }
                        }
                        obj
                    })
                    .collect();
                serde_json::json!({"kind": "union", "variants": vars})
            }
            DeclareValue::Set() => serde_json::json!({"kind": "set"}),
            DeclareValue::Range(start, end) => {
                serde_json::json!({"kind": "range", "start": start.to_string(), "end": end.to_string()})
            }
            DeclareValue::InRange(start, end) => {
                serde_json::json!({"kind": "in_range", "start": start.to_string(), "end": end.to_string()})
            }
            DeclareValue::When(_) => serde_json::json!({"kind": "when"}),
        }
    }

    fn params_json(params: &ast::Parameters) -> Value {
        Value::Array(
            params
                .iter()
                .map(|(name, kind)| {
                    serde_json::json!({ "name": name.as_str(), "kind": format!("{:?}", kind) })
                })
                .collect(),
        )
    }

    fn bind_val_json(
        value: &BindValue,
        span_table: &SpanTable,
        source: &str,
        depth: usize,
        max_depth: Option<usize>,
    ) -> Value {
        match value {
            BindValue::Expr(e) => {
                Self::expr_json(&e.value, e.span_id, span_table, source, depth, max_depth)
            }
            BindValue::Body { exprs, ret } => {
                let body: Vec<Value> = exprs
                    .iter()
                    .map(|e| {
                        Self::expr_json(&e.value, e.span_id, span_table, source, depth, max_depth)
                    })
                    .collect();
                let ret_val = ret.value.as_ref().map(|e| {
                    Self::expr_json(&e.value, e.span_id, span_table, source, depth, max_depth)
                });
                serde_json::json!({ "kind": "body", "body": body, "return": ret_val })
            }
            BindValue::Extern => serde_json::json!({ "kind": "extern" }),
            BindValue::Unassigned => serde_json::json!({ "kind": "unassigned" }),
        }
    }

    fn expr_json(
        expr: &Expr,
        span_id: SpanId,
        span_table: &SpanTable,
        source: &str,
        depth: usize,
        max_depth: Option<usize>,
    ) -> Value {
        let span = span_table.get(span_id);
        let src = source.get(span.start()..span.end()).unwrap_or("<span err>");
        let mut obj = serde_json::json!({ "kind": Self::expr_kind_name(expr), "source": src });

        if max_depth.is_some_and(|md| depth >= md) {
            return obj;
        }

        let nd = depth + 1;

        match expr {
            Expr::Lit(lit) => {
                obj["value"] = Value::String(format!("{lit:?}"));
            }
            Expr::FnCall(call) => {
                obj["name"] = Value::String(call.path.root.as_str().to_string());
                if let Some(args) = &call.args {
                    obj["args"] = Value::Array(
                        args.iter()
                            .map(|a| {
                                Self::expr_json(
                                    &a.value, a.span_id, span_table, source, nd, max_depth,
                                )
                            })
                            .collect(),
                    );
                }
            }
            Expr::Binary(bin) => {
                obj["op"] = Value::String(format!("{:?}", bin.op));
                obj["lhs"] = Self::expr_json(
                    &bin.lhs.value,
                    bin.lhs.span_id,
                    span_table,
                    source,
                    nd,
                    max_depth,
                );
                obj["rhs"] = Self::expr_json(
                    &bin.rhs.value,
                    bin.rhs.span_id,
                    span_table,
                    source,
                    nd,
                    max_depth,
                );
            }
            Expr::Bind(bind) => {
                obj["bind_name"] = Value::String(bind.name.as_str().to_string());
                if let Some(params) = bind.params.as_ref() {
                    obj["params"] = Self::params_json(params);
                }
                obj["value"] = Self::bind_val_json(&bind.value, span_table, source, nd, max_depth);
            }
            Expr::If(if_expr) => {
                obj["condition"] = Value::String(format!("{:?}", if_expr.subject));
                obj["pattern"] = if_expr
                    .pattern
                    .as_ref()
                    .map(|pattern| Value::String(format!("{:?}", pattern.value)))
                    .unwrap_or(Value::Null);
                obj["body"] = Value::Array(
                    if_expr
                        .body
                        .iter()
                        .map(|e| {
                            Self::expr_json(&e.value, e.span_id, span_table, source, nd, max_depth)
                        })
                        .collect(),
                );
            }
            Expr::When(w) => {
                if let Some(s) = &w.subject {
                    obj["subject"] =
                        Self::expr_json(&s.value, s.span_id, span_table, source, nd, max_depth);
                }
            }
            Expr::Loop(l) => {
                obj["loop_kind"] = Value::String(format!("{:?}", l));
            }
            Expr::FormatString(fs) => {
                obj["parts_count"] = Value::Number((fs.parts.len()).into());
            }
            Expr::TypeInRange(bounds) => {
                obj["bounds"] = Value::String(format!("{:?}", bounds));
            }
            Expr::TypeRef { inner, mutable } => {
                obj["inner"] = Value::String(format!("{:?}", inner));
                obj["mutable"] = Value::Bool(*mutable);
            }
            Expr::RecordGet { base, field } => {
                obj["base"] =
                    Self::expr_json(&base.value, base.span_id, span_table, source, nd, max_depth);
                obj["field"] = Value::String(field.as_str().to_string());
            }
            _ => {}
        }
        obj
    }

    fn expr_kind_name(expr: &Expr) -> &'static str {
        match expr {
            Expr::Loop(_) => "Loop",
            Expr::Binary(_) => "Binary",
            Expr::FnCall(_) => "FnCall",
            Expr::Lit(_) => "Lit",
            Expr::FormatString(_) => "FormatString",
            Expr::Range(_) => "Range",
            Expr::Bind(_) => "Bind",
            Expr::When(_) => "When",
            Expr::If(_) => "If",
            Expr::SelfRef => "SelfRef",
            Expr::TagCall(_) => "TagCall",
            Expr::AnonymousTag(_) => "AnonymousTag",
            Expr::TupleAlloc { .. } => "TupleAlloc",
            Expr::TupleGet { .. } => "TupleGet",
            Expr::TupleSet { .. } => "TupleSet",
            Expr::Cast { .. } => "Cast",
            Expr::BufGet { .. } => "BufGet",
            Expr::BufSet { .. } => "BufSet",
            Expr::TakePtr(_) => "TakePtr",
            Expr::Ref { .. } => "Ref",
            Expr::ConsumeArg(_) => "ConsumeArg",
            Expr::Eat(_) => "Eat",
            Expr::Deref(_) => "Deref",
            Expr::Negate(_) => "Negate",
            Expr::Asm(_) => "Asm",
            Expr::List(_) => "List",
            Expr::RecordLit(_) => "RecordLit",
            Expr::TupleLit(_) => "TupleLit",
            Expr::TypeNominal(_) => "TypeNominal",
            Expr::TypeQualified(_) => "TypeQualified",
            Expr::TypeGeneric { .. } => "TypeGeneric",
            Expr::TypeInRange(_) => "TypeInRange",
            Expr::TypeRef { .. } => "TypeRef",
            Expr::RecordGet { .. } => "RecordGet",
            Expr::RecordSet { .. } => "RecordSet",
            Expr::Destructure { .. } => "Destructure",
        }
    }
}
