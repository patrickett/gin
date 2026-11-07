//! `inspect`: the shape and size of things in the session image. Every number here comes from
//! the checker (`#size`, `#alignment`, the checked aggregates and functions) or from the
//! lowering (`ginc::register_layout`, `ginc::instance_layouts`); nothing is derived a second
//! time. What is not computable yet is a typed "not available because" reason, as in `eval`.

use std::collections::BTreeMap;

use interpreter::{Interpreter, Program};
use serde_json::{Value as Json, json};
use typecheck::{
    AggregateKind, Borrowing, Capability, CheckedSourceBodyKind, CheckedSourceInstanceHead, CheckedSourceModule,
    DomainClaim, SourceType,
};

use crate::check::{Checked, show_type};
use crate::image::Image;
use crate::session::{check_once, value_json};
use crate::source::Decl;

/// Why something cannot be said yet, as data.
pub fn unavailable(kind: &str, detail: impl Into<String>) -> Json {
    json!({"available": false, "reason": {"kind": kind, "detail": detail.into()}})
}

struct Ctx<'a> {
    module: &'a CheckedSourceModule,
    checked: &'a Checked,
    base: &'a str,
}

/// An integer the checker computes for `expression` (a `#size(..)` query), or its explanation.
/// A query with known arguments is recorded as a constant, so it is evaluated like any body.
pub(crate) fn query_int(base: &str, expression: &str) -> Result<i128, String> {
    let text = format!("{base}\n__gin_q() in 0 to 1000000000000000: return {expression}\n");
    let module = check_once(&text).map_err(|flaws| flaws.first().map_or("the query failed".to_owned(), |f| f.message.clone()))?;
    let program = Program::from_modules(vec![module]);
    match Interpreter::new(&program).call("__gin_q", &[], &[]) {
        Ok(outcome) => outcome.value.as_integer().map(|n| n.as_i128()).ok_or_else(|| "the query is not a number".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

fn size_json(result: Result<i128, String>) -> Json {
    match result {
        Ok(n) => json!({"available": true, "value": n, "unit": "bytes"}),
        Err(detail) => unavailable("not_defined_yet", detail),
    }
}

fn claim_text(claim: &DomainClaim) -> String {
    use ast::BinaryOperator as B;
    match claim {
        DomainClaim::Multiple(modulus) => format!("self % {modulus} = 0"),
        DomainClaim::Compare(op, number) => {
            let symbol = match op {
                B::Equal => "=",
                B::Less => "<",
                B::LessOrEqual => "<=",
                B::Greater => ">",
                B::GreaterOrEqual => ">=",
                B::Remainder => "%",
                _ => "?",
            };
            format!("self {symbol} {number}")
        }
    }
}

pub fn inspect(checked: &Checked, name: &str) -> Json {
    let Some(module) = &checked.module else {
        return json!({"found": false, "reason": {"kind": "flawed", "detail": "the session does not check; fix its flaws first"}});
    };
    let base = checked.source.to_string();
    let ctx = Ctx { module, checked, base: &base };
    let file = module.checked();
    let (head, applied) = match name.split_once('(') {
        Some((head, _)) => (head.trim(), true),
        None => (name.trim(), false),
    };
    if let Some(schema) = file.aggregates.iter().position(|a| a.name == head) {
        return type_json(&ctx, schema, if applied { Some(name.trim()) } else { None });
    }
    if let Some((_, target)) = module.aliases().iter().find(|(n, _)| n == head) {
        return json!({"found": true, "kind": "alias", "name": head, "target": show_type(file, *target, &[]), "transparent": true,
            "target_layout": query_size(&ctx, &show_type(file, *target, &[]))});
    }
    if let Some((_, value)) = module.constants().iter().find(|(n, _)| n == head) {
        return json!({"found": true, "kind": "constant", "name": head, "known": true, "folded": true,
            "value": {"kind": "integer", "value": value.to_string()}, "type": format!("{value} to {value}")});
    }
    let functions: Vec<usize> = (0..file.functions.len())
        .filter(|i| file.functions[*i].name == head && module.function_home(*i).is_some_and(|(h, o)| h == module.name() && o as usize == *i))
        .collect();
    if !functions.is_empty() {
        return function_json(&ctx, head, &functions);
    }
    json!({"found": false, "reason": {"kind": "unknown_name", "detail": format!("`{name}` is not a type, alias, constant or function of the session")}})
}

fn query_size(ctx: &Ctx<'_>, type_text: &str) -> Json {
    json!({"size": size_json(query_int(ctx.base, &format!("#size({type_text})"))),
           "alignment": size_json(query_int(ctx.base, &format!("#alignment({type_text})")))})
}

fn capability(c: Capability) -> &'static str {
    match c {
        Capability::Linear => "linear",
        Capability::Unrestricted => "unrestricted",
    }
}

fn type_json(ctx: &Ctx<'_>, schema: usize, applied: Option<&str>) -> Json {
    let module = ctx.module;
    let file = module.checked();
    let a = &file.aggregates[schema];
    let params = &a.parameters;
    let generic = !params.is_empty();
    let kind = match a.kind {
        AggregateKind::Record => "record",
        AggregateKind::Sum => "sum",
        AggregateKind::Domain => "domain",
    };
    let fields: Vec<Json> = a
        .fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            json!({"index": i, "name": f.name, "type": show_type(file, f.ty, params), "private": f.private,
                   "register": f.register, "borrow": f.borrow.map(|b| format!("{b:?}").to_lowercase()), "length_field": f.length})
        })
        .collect();
    let member_of = |ty: &SourceType| -> Option<usize> {
        let SourceType::Aggregate { id } = ty else { return None };
        match file.instances.get(*id as usize)?.head {
            CheckedSourceInstanceHead::Nominal { schema } => Some(schema as usize),
            _ => None,
        }
    };
    let variants: Vec<Json> = a
        .variants
        .iter()
        .filter_map(|v| {
            let m = &file.aggregates[member_of(v)?];
            Some(json!({"name": m.name.rsplit('.').next().unwrap_or(&m.name), "qualified": m.name,
                "payload": m.fields.iter().map(|f| json!({"name": f.name, "type": show_type(file, f.ty, params)})).collect::<Vec<_>>()}))
        })
        .collect();
    let domain = (a.kind == AggregateKind::Domain).then(|| {
        let bounds = match a.fields.first().map(|f| f.ty) {
            Some(SourceType::Integer { lower, upper }) => json!({"lower": lower.to_string(), "upper": upper.to_string()}),
            _ => Json::Null,
        };
        let claims: Vec<String> = a.domain_claims.iter().map(claim_text).collect();
        json!({"bounds": bounds, "claims": claims, "wraps": a.wrapping})
    });
    let methods: Vec<Json> = (0..file.functions.len())
        .filter(|i| file.functions[*i].name.starts_with(&format!("{}.", a.name)))
        .map(|i| {
            let f = &file.functions[i];
            let (home, _) = module.function_home(i).unwrap_or(("?", 0));
            json!({"name": f.name, "signature": signature(file, i), "private": f.private, "infix": f.infix.map(|o| o.symbol()), "defined_in": home,
                "origin": match &f.provided_by { Some(t) => json!({"kind": "trait", "trait": t}), None => json!({"kind": "declared"}) }})
        })
        .collect();
    let parameters: Vec<Json> = params
        .iter()
        .enumerate()
        .map(|(i, p)| json!({"name": p, "bound": a.bounds.get(i).cloned().flatten(),
            "default": a.defaults.get(i).and_then(|d| d.map(|d| show_type(file, d, params))),
            "number_range": a.numbers.get(i).and_then(|n| n.map(|(l, u)| json!({"lower": l.to_string(), "upper": u.to_string()})))}))
        .collect();
    let layout = layout_json(ctx, schema, applied);
    json!({
        "found": true, "kind": "type", "name": a.name, "type_kind": if generic && applied.is_none() { "generic" } else { kind },
        "shape": kind, "generic": generic, "parameters": parameters,
        "linearity": capability(a.capability),
        "borrowing": match a.borrowing { Borrowing::None => Json::Null, other => json!(format!("{other:?}").to_lowercase()) },
        "fields": fields, "variants": variants, "domain": domain,
        "provides": a.provides,
        "methods": methods,
        "methods_note": "`origin` says whether the type wrote a method or a provided trait added it",
        "origin": module.layout_home(schema),
        "layout": layout,
    })
}

fn signature(file: &typecheck::CheckedSourceFile, index: usize) -> String {
    let f = &file.functions[index];
    let params = f.parameters.iter().map(|t| show_type(file, *t, &f.type_parameters)).collect::<Vec<_>>().join(", ");
    format!("{}({params}) -> {}", f.name, show_type(file, f.result, &f.type_parameters))
}

/// The lowering's register layout of the type written `text`. The session's own checked file
/// holds an instance only for a type something uses, so a probe function that borrows one is
/// appended and checked, and the layout is read from that.
fn registers_of(ctx: &Ctx<'_>, text: &str) -> Json {
    let probe = format!("{}\n__gin_probe(v ref {text}) in 0 to 0: return 0\n", ctx.base);
    let module = match check_once(&probe) {
        Ok(module) => module,
        Err(flaws) => return unavailable("no_register_form", flaws.first().map_or("the type cannot be named".to_owned(), |f| f.message.clone())),
    };
    match ginc::parameter_layout(&module, "__gin_probe", 0) {
        Some(layout) => registers_json(&layout),
        None => unavailable("no_register_form", "the type has no parameter form"),
    }
}

fn registers_json(layout: &ginc::RegisterLayout) -> Json {
    let sum = layout.sum.as_ref().map(|s| {
        json!({
            "total_registers": s.total, "tag_registers": 1, "widest_payload": s.widest_payload,
            "members": s.members.iter().map(|m| json!({"name": m.name, "tag": m.tag, "payload_registers": m.payload_width,
                "padding": s.widest_payload.zip(m.payload_width).map(|(w, p)| w - p)})).collect::<Vec<_>>(),
        })
    });
    json!({"available": layout.width.is_some(), "registers": layout.width, "limit": layout.limit,
        "register_groups": layout.registers, "sum": sum,
        "reason": layout.reason.as_ref().map(|r| json!({"kind": "no_register_form", "detail": r}))})
}

fn layout_json(ctx: &Ctx<'_>, schema: usize, applied: Option<&str>) -> Json {
    let module = ctx.module;
    let file = module.checked();
    let a = &file.aggregates[schema];
    if !a.parameters.is_empty() && applied.is_none() {
        // Symbolic until instantiated; the applications the session makes have layouts.
        let applications: Vec<Json> = file
            .instances
            .iter()
            .filter(|i| i.head == CheckedSourceInstanceHead::Nominal { schema: schema as u32 } && !i.arguments.is_empty())
            .filter(|i| !i.arguments.iter().any(|t| matches!(t, SourceType::Generic { .. })))
            .map(|i| {
                let text = format!("{}({})", a.name, i.arguments.iter().map(|t| show_type(file, *t, &[])).collect::<Vec<_>>().join(", "));
                json!({"type": text, "size": size_json(query_int(ctx.base, &format!("#size({text})"))),
                       "alignment": size_json(query_int(ctx.base, &format!("#alignment({text})")))})
            })
            .collect();
        let why = unavailable("symbolic", format!("it depends on the type parameters {}; a size exists once they are given", a.parameters.join(", ")));
        return json!({"size": why, "alignment": why, "registers": why, "applications": applications});
    }
    let text = applied.map_or_else(|| a.name.clone(), str::to_owned);
    let size = query_int(ctx.base, &format!("#size({text})"));
    let alignment = query_int(ctx.base, &format!("#alignment({text})"));
    let registers = registers_of(ctx, &text);
    let offsets = if a.kind == AggregateKind::Record && applied.is_none() { offsets_json(ctx, schema, &size) } else { Json::Null };
    json!({"size": size_json(size), "alignment": size_json(alignment), "field_offsets": offsets, "registers": registers})
}

/// Field offsets from the checker's own `#size`/`#alignment` of each field, placed in order as
/// the checker places them; the total is checked against the checker's `#size` of the record.
fn offsets_json(ctx: &Ctx<'_>, schema: usize, total: &Result<i128, String>) -> Json {
    let file = ctx.module.checked();
    let a = &file.aggregates[schema];
    let mut declarations = String::new();
    let mut texts = Vec::new();
    for (i, f) in a.fields.iter().enumerate() {
        texts.push(match f.ty {
            SourceType::Integer { lower, upper } => {
                declarations.push_str(&format!("\n#unrestricted\nGinField{i} is in {lower} to {upper}\n"));
                Some(format!("GinField{i}"))
            }
            SourceType::Aggregate { .. } => Some(show_type(file, f.ty, &[])),
            _ => None,
        });
    }
    let base = format!("{}{declarations}", ctx.base);
    let (mut offset, mut align) = (0i128, 1i128);
    let mut out = Vec::new();
    for (f, text) in a.fields.iter().zip(&texts) {
        let Some(text) = text else {
            return unavailable("not_defined_yet", format!("the field `{}` has a type the layout queries do not name yet", f.name));
        };
        let (Ok(size), Ok(alignment)) = (query_int(&base, &format!("#size({text})")), query_int(&base, &format!("#alignment({text})"))) else {
            return unavailable("not_defined_yet", format!("the checker defines no layout for the field `{}`", f.name));
        };
        offset = (offset + alignment - 1) / alignment * alignment;
        out.push(json!({"name": f.name, "offset": offset, "size": size, "alignment": alignment}));
        offset += size;
        align = align.max(alignment);
    }
    let end = (offset + align - 1) / align * align;
    let consistent = matches!(total, Ok(t) if *t == end);
    json!({"available": true, "fields": out, "consistent_with_size": consistent})
}

fn function_json(ctx: &Ctx<'_>, name: &str, indexes: &[usize]) -> Json {
    let module = ctx.module;
    let file = module.checked();
    let decls: Vec<&Decl> = ctx.checked.decls.iter().filter(|d| d.name == name).collect();
    let member = ctx.checked.decls.iter().flat_map(|d| d.members.iter()).find(|m| m.name == name);
    let instances = ginc::instance_layouts(&[module]);
    let overloads: Vec<Json> = indexes
        .iter()
        .map(|&i| {
            let f = &file.functions[i];
            let written = decls.iter().find(|d| d.params.len() == f.parameters.len()).map(|d| (&d.params, &d.result, &d.contract, &d.alternatives)).or_else(|| member.map(|m| (&m.params, &m.result, &m.contract, &m.alternatives)));
            let params: Vec<Json> = f
                .parameters
                .iter()
                .enumerate()
                .map(|(p, t)| {
                    let w = written.and_then(|w| w.0.get(p));
                    json!({"name": w.map(|w| w.name.clone()), "convention": w.and_then(|w| w.convention.clone()), "type": show_type(file, *t, &f.type_parameters),
                           "mut": f.mutated.contains(&(p as u32))})
                })
                .collect();
            let body = match &file.bodies[i] {
                Some(b) => json!({"recorded": true, "nodes": b.expressions.len(), "effects": b.effects,
                    "holes": b.expressions.iter().filter(|e| matches!(e.kind, CheckedSourceBodyKind::Hole)).count()}),
                None if f.instruction.is_some() => json!({"recorded": false, "reason": {"kind": "machine_instruction", "detail": "a `#instruction` function has no body; it is the instruction"}}),
                None => match file.declines.iter().find(|(at, _)| *at as usize == i) {
                    Some((_, d)) => json!({"recorded": false, "reason": {
                        "kind": "unrecorded_body",
                        "declined_construct": d.construct,
                        "declined_kind": format!("{:?}", d.kind).to_lowercase(),
                        "span": d.span.map(|s| json!({"byte_start": s.start(), "byte_end": s.end()})),
                        "detail": format!("the recorder declined this body at {}", d.construct)}}),
                    None => json!({"recorded": false, "reason": {"kind": "unrecorded_body", "detail": "the recorder declined this body and named no construct"}}),
                },
            };
            let mine: Json = match &instances {
                Ok(list) => json!(list.iter().filter(|l| l.name.starts_with(&format!("{}.{}<", module.name(), f.name))).map(|l| json!({
                    "name": l.name, "requested_by": l.requested_by,
                    "parameters": l.parameters.iter().map(|(t, w)| json!({"type": t, "registers": w})).collect::<Vec<_>>(),
                    "result": {"type": l.result.0, "registers": l.result.1},
                })).collect::<Vec<_>>()),
                Err(e) => unavailable("instantiation_failed", e.clone()),
            };
            json!({
                "signature": signature(file, i), "parameters": params,
                "result": {"type": show_type(file, f.result, &f.type_parameters), "written": written.and_then(|w| w.1.clone())},
                "result_alternatives": written.map(|w| w.3.clone()).unwrap_or_default(),
                "claims": written.and_then(|w| w.2.clone()),
                "type_parameters": f.type_parameters.iter().enumerate().map(|(k, p)| json!({"name": p, "bound": f.type_bounds.get(k).cloned().flatten()})).collect::<Vec<_>>(),
                "private": f.private, "infix": f.infix.map(|o| o.symbol()), "instruction": f.instruction,
                "defined_in": module.function_home(i).map(|h| h.0), "provided_by": f.provided_by,
                "body": body, "instances": mine,
            })
        })
        .collect();
    json!({"found": true, "kind": "function", "name": name, "overloads": overloads})
}

/// A zero-parameter function that can be evaluated: its value, or why not.
pub fn value_of_function(checked: &Checked, name: &str) -> Option<Json> {
    let module = checked.module.as_ref()?;
    let file = module.checked();
    let index = (0..file.functions.len()).find(|i| file.functions[*i].name == name && module.function_home(*i).is_some_and(|(h, o)| h == module.name() && o as usize == *i))?;
    let f = &file.functions[index];
    if !f.parameters.is_empty() || !f.type_parameters.is_empty() {
        return None;
    }
    let fresh = check_once(&checked.source).ok()?;
    let program = Program::from_modules(vec![fresh]);
    let ty = show_type(file, f.result, &[]);
    Some(match Interpreter::new(&program).call(name, &[], &[]) {
        Ok(outcome) => json!({"known": true, "value": value_json(&program, &outcome.value), "display": program.render(&outcome.value), "type": ty}),
        Err(error) => json!({"known": false, "type": ty, "reason": crate::session::reason_of(&error)}),
    })
}

/// The whole image: definitions and their dependencies, the constants and aliases folded, the
/// target facts the checker folds, and the instances monomorphization has asked for.
pub fn image_json(checked: &Checked, image: &Image, kept: &dyn Fn(usize) -> Option<bool>, revision: u64) -> Json {
    let target = typecheck::Limits::default().target;
    let (constants, aliases, instances) = match &checked.module {
        Some(m) => (
            m.constants().iter().map(|(n, v)| json!({"name": n, "value": v.to_string()})).collect::<Vec<_>>(),
            m.aliases().iter().map(|(n, t)| json!({"name": n, "target": show_type(m.checked(), *t, &[])})).collect::<Vec<_>>(),
            ginc::instance_layouts(&[m]).map(|l| l.into_iter().map(|i| json!({"name": i.name, "requested_by": i.requested_by})).collect::<Vec<_>>()).unwrap_or_default(),
        ),
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for d in &image.defs {
        *by_kind.entry(d.kind).or_default() += 1;
    }
    json!({
        "revision": revision,
        "modules": [{"name": "main", "imports": image.defs.iter().filter(|d| d.kind == "import").map(|d| d.name.clone()).collect::<Vec<_>>()}],
        "definitions": image.json(Some(kept)),
        "counts": by_kind,
        "folded": {"constants": constants, "aliases": aliases,
            "target": {"triple": target.triple, "register_bits": target.register_bits,
                "note": "the `#target` facts the checker folds into bodies; definitions that mention `#target` are marked"}},
        "instances": instances,
        "dependency_note": "`uses` via `checker` or `folded` are exact; via `text` is over-approximate and only appears for a function whose body the recorder declined, and for types and constants",
    })
}
