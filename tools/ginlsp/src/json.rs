//! JSON serialization helpers for Gin types and ASTs.
//! Used to format JSON responses for the LSP.

use ast::flow::ConstValue;
use ast::ty::Ty;
use ast::{BindValue, DeclareValue, Expr, FileAst, HasSpanId, SpanId, SpanTable};
use serde_json::Value;

/// Serialize a resolved `Ty` to a JSON structure with kind, fields, size, and alignment.
pub fn ty_to_json(ty: &Ty) -> Value {
    match ty {
        Ty::Int {
            width,
            signed,
            value: _,
        } => serde_json::json!({
            "kind": "Int", "width": width, "signed": signed,
            "size": ast::ty::ty_byte_size_static(ty),
        }),
        Ty::Float { .. } => serde_json::json!({
            "kind": "Float", "size": ast::ty::ty_byte_size_static(ty),
        }),
        Ty::Bool => serde_json::json!({ "kind": "Bool", "size": 1 }),
        Ty::Unit => serde_json::json!({ "kind": "Unit", "size": 0 }),
        Ty::Record { name, fields } => {
            let flds: Vec<Value> = fields
                .iter()
                .map(
                    |(fn_, ft)| serde_json::json!({ "name": fn_.as_str(), "type": ty_to_json(ft) }),
                )
                .collect();
            serde_json::json!({
                "kind": "Record", "name": name.as_str(),
                "fields": flds, "size": ast::ty::ty_byte_size_static(ty),
                "align": ast::ty::ty_alignment(ty),
            })
        }
        Ty::Union { name, variants } => {
            let vars: Vec<Value> = variants
                .iter()
                .map(|(vn, fields)| {
                    let flds: Vec<Value> = fields
                        .iter()
                        .map(|(fn_, ft)| {
                            serde_json::json!({ "name": fn_.as_str(), "type": ty_to_json(ft) })
                        })
                        .collect();
                    serde_json::json!({ "name": vn.as_str(), "fields": flds })
                })
                .collect();
            serde_json::json!({
                "kind": "Union", "name": name.as_str(), "variants": vars,
                "size": ast::ty::ty_byte_size_static(ty),
                "align": ast::ty::ty_alignment(ty),
            })
        }
        Ty::Opaque(name) => serde_json::json!({ "kind": "Opaque", "name": name.as_str() }),
        Ty::Array { elem, size } => serde_json::json!({
            "kind": "Array", "elem": ty_to_json(elem), "length": size,
        }),
        Ty::Ptr { inner } => serde_json::json!({
            "kind": "Ptr", "inner": ty_to_json(inner),
        }),
        Ty::Ref { inner, mutable } => serde_json::json!({
            "kind": if *mutable { "Mut" } else { "Ref" },
            "inner": ty_to_json(inner),
        }),
        Ty::ConstUnion { name, base, values } => {
            let vals: Vec<Value> = values
                .iter()
                .map(|v| match v {
                    ConstValue::String(s) => serde_json::json!(s),
                    ConstValue::Int(n) => serde_json::json!(n),
                    ConstValue::Float(f) => serde_json::json!(f),
                    ConstValue::Tag { name: tn, .. } => serde_json::json!(tn.as_str()),
                    ConstValue::Record { .. } => serde_json::json!("<record>"),
                    ConstValue::List(_) => serde_json::json!("<list>"),
                })
                .collect();
            serde_json::json!({
                "kind": "ConstUnion", "name": name.as_str(),
                "base": ty_to_json(base), "values": vals,
                "size": ast::ty::ty_byte_size_static(ty),
                "align": ast::ty::ty_alignment(ty),
            })
        }
        Ty::Tuple(items) => {
            let items_json: Vec<Value> = items.iter().map(ty_to_json).collect();
            serde_json::json!({ "kind": "Tuple", "items": items_json })
        }
    }
}

/// Serialize a full AST to JSON (all levels).
pub fn ast_to_json(ast: &FileAst, source: &str) -> Value {
    ast_to_json_with_depth(ast, source, None)
}

/// Serialize an AST to JSON with an optional maximum recursion depth.
pub fn ast_to_json_with_depth(ast: &FileAst, source: &str, max_depth: Option<usize>) -> Value {
    let span_table = ast.span_table();

    let defs: Vec<Value> = ast
        .defs()
        .iter()
        .map(|(name, bind)| {
            let mut obj = serde_json::json!({
                "name": name.as_str(),
                "kind": if bind.params().is_some() { "function" } else { "bind" },
                "private": ast.private_defs().contains(name),
            });
            if let Some(params) = bind.params().as_ref() {
                obj["params"] = params_json(params);
            }
            if let Some(doc) = bind.doc_comment() {
                obj["doc"] = Value::String(doc.value.clone());
            }
            obj["value"] = bind_val_json(bind.value(), span_table, source, 0, max_depth);
            obj
        })
        .collect();

    let tags: Vec<Value> = ast
        .tags()
        .iter()
        .map(|(name, decl)| {
            let mut obj = serde_json::json!({
                "name": name.as_str(), "kind": "tag",
                "private": ast.private_tags().contains(name),
            });
            if let Some(params) = decl.params().as_ref() {
                obj["params"] = params_json(params);
            }
            if let Some(doc) = decl.doc_comment() {
                obj["doc"] = Value::String(doc.value.clone());
            }
            obj["value"] = declare_value_json(decl.value(), span_table, source);
            obj
        })
        .collect();

    let uses: Vec<Value> = ast
        .uses()
        .iter()
        .flat_map(|import| {
            import.0.iter().map(|mi| {
                let (sl, sc) =
                    ast::byte_offset_to_position(span_table.get(mi.span_id()).start, source);
                serde_json::json!({
                    "source": format!("{:?}", mi.source),
                    "alias": mi.alias.as_ref().map(|a| a.as_str()),
                    "line": sl, "character": sc,
                })
            })
        })
        .collect();

    let top_exprs: Vec<Value> = ast
        .top_level_exprs()
        .iter()
        .map(|(e, sid)| expr_json(e, *sid, span_table, source, 0, max_depth))
        .collect();

    serde_json::json!({
        "defs": defs, "tags": tags, "uses": uses,
        "top_level_exprs": top_exprs,
        "has_module_doc": ast.module_doc().is_some(),
        "module_doc": ast.module_doc().map(|d| d.value.as_str()),
    })
}

// ---- Internal serialization helpers -------------------------------------

fn declare_value_json(value: &DeclareValue, span_table: &SpanTable, source: &str) -> Value {
    match value {
        DeclareValue::Alias(sp) => {
            let span = span_table.get(sp.span_id);
            let src = source.get(span.start..span.end).unwrap_or("<span err>");
            serde_json::json!({"kind": "alias", "type_expr": src})
        }
        DeclareValue::Record(params) => {
            let fields: Vec<Value> = params
                .iter()
                .map(|(name, kind)| {
                    serde_json::json!({"name": name.as_str(), "kind": format!("{:?}", kind)})
                })
                .collect();
            serde_json::json!({"kind": "record", "fields": fields})
        }
        DeclareValue::Union { variants } => {
            let vars: Vec<Value> = variants
                .iter()
                .map(|v| {
                    let span = span_table.get(v.shape().span_id);
                    let shape_src = source.get(span.start..span.end).unwrap_or("<span err>");
                    let mut obj = serde_json::json!({
                        "shape": shape_src,
                    });
                    match v {
                        ast::Variant::External(_) => {
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
        BindValue::Expr(e) => expr_json(&e.value, e.span_id, span_table, source, depth, max_depth),
        BindValue::Body { exprs, ret } => {
            let body: Vec<Value> = exprs
                .iter()
                .map(|e| expr_json(&e.value, e.span_id, span_table, source, depth, max_depth))
                .collect();
            let ret_val = ret
                .value
                .as_ref()
                .map(|e| expr_json(&e.value, e.span_id, span_table, source, depth, max_depth));
            serde_json::json!({ "kind": "body", "body": body, "return": ret_val })
        }
        BindValue::Extern => serde_json::json!({ "kind": "extern" }),
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
    let src = source.get(span.start..span.end).unwrap_or("<span err>");
    let mut obj = serde_json::json!({ "kind": expr_kind_name(expr), "source": src });

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
                        .map(|a| expr_json(&a.value, a.span_id, span_table, source, nd, max_depth))
                        .collect(),
                );
            }
        }
        Expr::Binary(bin) => {
            obj["op"] = Value::String(format!("{:?}", bin.op));
            obj["lhs"] = expr_json(
                &bin.lhs.value,
                bin.lhs.span_id,
                span_table,
                source,
                nd,
                max_depth,
            );
            obj["rhs"] = expr_json(
                &bin.rhs.value,
                bin.rhs.span_id,
                span_table,
                source,
                nd,
                max_depth,
            );
        }
        Expr::Bind(bind) => {
            obj["bind_name"] = Value::String(bind.name().as_str().to_string());
            if let Some(params) = bind.params().as_ref() {
                obj["params"] = params_json(params);
            }
            obj["value"] = bind_val_json(bind.value(), span_table, source, nd, max_depth);
        }
        Expr::If(if_expr) => {
            obj["condition"] = Value::String(format!("{:?}", if_expr.condition));
            obj["body"] = Value::Array(
                if_expr
                    .body
                    .iter()
                    .map(|e| expr_json(&e.value, e.span_id, span_table, source, nd, max_depth))
                    .collect(),
            );
        }
        Expr::When(w) => {
            if let Some(s) = &w.subject {
                obj["subject"] = expr_json(&s.value, s.span_id, span_table, source, nd, max_depth);
            }
        }
        Expr::Loop(l) => {
            obj["loop_kind"] = Value::String(format!("{:?}", l));
        }
        Expr::FormatString(fs) => {
            obj["parts_count"] = Value::Number((fs.parts.len()).into());
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
        Expr::TupleLit(_) => "TupleLit",
        Expr::TypeNominal(_) => "TypeNominal",
        Expr::TypeQualified(_) => "TypeQualified",
        Expr::TypeGeneric { .. } => "TypeGeneric",
    }
}
