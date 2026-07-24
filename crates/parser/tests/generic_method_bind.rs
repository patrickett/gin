//! Parser tests for method binds with parameterized-type receivers,
//! e.g. `Range(x).new(start x, end x) Range(x): (start, end)`.

use ast::{DeclareValue, Expr, HasFunctionKind, HasMember, ParameterKind, TypeExpr};
use internment::Intern;

use parser::cursor::TokenCursor;
use parser::query::SourceParseExt;

fn intern(s: &str) -> Intern<String> {
    Intern::new(s.to_owned())
}

#[test]
fn parses_generic_method_bind_with_typevar_params_and_return() {
    let src =
        "Range(x) has start x, end x\n\nRange(x).new(start x, end x) Range(x): (start, end)\n";
    let ast = TokenCursor::parse_source(src);

    // Tag declaration is recorded as `Range`
    assert!(
        ast.tags.contains_key(&intern("Range")),
        "expected Range tag in tags(): {:?}",
        ast.tags.keys().collect::<Vec<_>>()
    );

    // Method def is mangled as `Range.new` (TypeGeneric collapses to base name)
    let bind = ast
        .defs
        .get(&intern("Range.new"))
        .expect("expected Range.new bind");

    // Receiver is TypeGeneric { name: Range, params: { x: Generic } }
    let recv = bind
        .receiver_type_surface()
        .expect("Range.new must have a receiver_type");
    match &recv.value {
        TypeExpr::Generic { name, params, .. } => {
            assert_eq!(name.as_str(), "Range");
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].0.as_str(), "x");
            assert!(matches!(params[0].1, ParameterKind::Generic));
        }
        other => panic!("receiver should be TypeGeneric, got {:?}", other),
    }

    // Params: start: x, end: x — both Tagged(TypeNominal("x"))
    let params = bind.params.as_ref().expect("Range.new must have params");
    assert_eq!(params.len(), 2);
    let starts: Vec<_> = params.iter().collect();
    let (k0, v0) = &starts[0];
    let (k1, v1) = &starts[1];
    assert_eq!(k0.as_str(), "start");
    assert_eq!(k1.as_str(), "end");
    for (k, v) in [(*k0, v0), (*k1, v1)] {
        match v {
            ParameterKind::Tagged(sp) => match &sp.value {
                TypeExpr::Nominal(n, _) => {
                    assert_eq!(n.as_str(), "x", "{} param type-var should be x", k.as_str());
                }
                other => panic!("{} param should be TypeNominal(x), got {:?}", k, other),
            },
            other => panic!("{} param kind should be Tagged, got {:?}", k, other),
        }
    }

    // Return type is TypeGeneric Range(x), stored on bind.return_tag
    let ret = bind
        .return_tag
        .as_ref()
        .expect("Range.new must have return_tag");
    match &ret.value {
        TypeExpr::Generic { name, params, .. } => {
            assert_eq!(name.as_str(), "Range");
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].0.as_str(), "x");
        }
        other => panic!("return_tag should be TypeGeneric Range(x), got {:?}", other),
    }
}

#[test]
fn parses_method_bind_with_nontypevar_params() {
    // Sanity: ensure the existing `Type.method` form still parses for a non-generic
    // receiver — i.e., we did not regress the bare-Tag receiver path.
    let src = "Bool is True or False\n\nBool.to_string Str := 'true'\n";
    let ast = TokenCursor::parse_source(src);

    let bind = ast
        .defs
        .get(&intern("Bool.to_string"))
        .expect("Bool.to_string should be present");

    let recv = bind.receiver_type_surface().expect("receiver must exist");
    match &recv.value {
        TypeExpr::Nominal(n, _) => assert_eq!(n.as_str(), "Bool"),
        other => panic!("receiver should be TypeNominal(Bool), got {:?}", other),
    }
}

#[test]
fn parses_custom_range_no_type_param_for_contrast() {
    // Per the design: `CustomRange has start, end` (no `(x)` after the type
    // name) leaves both fields independently generic. The corresponding method
    // `CustomRange.new(start, end) CustomRange: (start, end)` should likewise
    // parse cleanly without forcing the two params to share a type variable.
    let src =
        "CustomRange has start, end\n\nCustomRange.new(start, end) CustomRange: (start, end)\n";
    let ast = TokenCursor::parse_source(src);

    assert!(ast.tags.contains_key(&intern("CustomRange")));

    let bind = ast
        .defs
        .get(&intern("CustomRange.new"))
        .expect("CustomRange.new should be present");

    let params = bind.params.as_ref().expect("must have params");
    assert_eq!(params.len(), 2);
    let collected: Vec<_> = params.iter().collect();
    // No shared type variable: each param is `Generic` (no annotation).
    for (name, kind) in &collected {
        assert!(
            matches!(kind, ParameterKind::Generic),
            "{} should be Generic, got {:?}",
            name,
            kind
        );
    }
}

#[test]
fn doc_comment_attaches_to_method_bind() {
    let src = "\
Range(x) has start x, end x

--- create a new range
Range(x).new(start x, end x) Range(x): (start, end)
";
    let ast = TokenCursor::parse_source(src);
    let bind = ast
        .defs
        .get(&intern("Range.new"))
        .expect("Range.new bind should exist");
    assert_eq!(
        bind.doc_comment.as_ref().map(|doc| doc.value.as_str()),
        Some("create a new range")
    );
}

#[test]
fn parses_module_rooted_generic_method_call() {
    let src = "\
Range(x) has start x, end x

Range(x).new(start x, end x) Range(x): (start, end)

core.Range.new(12, 1200)
";
    let ast = TokenCursor::parse_source(src);

    let call = ast
        .exprs
        .iter()
        .find_map(|(expr, _)| match expr {
            Expr::FnCall(call) if call.path.root.as_str() == "core" => Some(call),
            _ => None,
        })
        .expect("expected core.Range.new call");

    assert_eq!(call.path.root.as_str(), "core");
    let segments: Vec<&str> = call.path.segments.iter().map(|seg| seg.as_str()).collect();
    assert_eq!(segments, ["Range", "new"]);
}

#[test]
fn method_with_typed_self_has_receiver_and_tagged_self_param() {
    let src = "\
Point has x Int, y Int\n\
\n\
Point.distance(self Point, other Point) Int:\
    return 0\n\
return\n";
    let out = src.parse_source_full();
    if !out.symptoms.is_empty() {
        eprintln!(
            "symptoms: {:?}",
            out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
    let ast = out.ast;
    let def_names: Vec<_> = ast.defs.keys().map(|k| k.as_str().to_string()).collect();
    let bind = ast
        .defs
        .values()
        .find(|b| b.name.as_str() == "distance")
        .unwrap_or_else(|| panic!("distance method def among {def_names:?}"));
    assert!(bind.is_method(), "expected receiver_type");
    let has_typed_self = bind.params.as_ref().is_some_and(|p| {
        matches!(
            p.get(&internment::Intern::from_ref("self")),
            Some(ast::ParameterKind::Tagged(_))
        )
    });
    assert!(has_typed_self, "self should be tagged");
    assert!(matches!(
        bind.value,
        ast::BindValue::Body { .. } | ast::BindValue::Expr(_)
    ));
}

#[test]
fn has_member_classifies_property() {
    let src = "Range(x) has start x\n";
    let ast = TokenCursor::parse_source(src);

    let range = ast
        .tags
        .get(&intern("Range"))
        .expect("Range tag should exist");

    let DeclareValue::Has(members) = &range.value else {
        panic!("expected DeclareValue::Has, got {:?}", range.value);
    };
    assert_eq!(members.len(), 1, "expected 1 member");
    let HasMember::Property(start) = &members[0] else {
        panic!("start should be a property member");
    };
    assert_eq!(start.name.as_str(), "start");
    assert!(start.ty.is_some());
}

#[test]
fn explicit_empty_params_classify_associated_function() {
    let out = "Factory has create() Factory\n".parse_source_full();
    assert!(out.symptoms.is_empty(), "symptoms: {:?}", out.symptoms);
    let factory = out.ast.tags.get(&intern("Factory")).expect("Factory");
    let DeclareValue::Has(members) = &factory.value else {
        panic!("expected has members");
    };
    let HasMember::Function(create) = &members[0] else {
        panic!("create() should be a function member");
    };
    assert!(create.params.is_empty());
    assert_eq!(create.kind, HasFunctionKind::Associated);
}

#[test]
fn has_member_classifies_property_and_instance_method() {
    let src = "Range(x) has start x, contains(self, value x) Bool\n";
    let ast = TokenCursor::parse_source(src);

    let range = ast
        .tags
        .get(&intern("Range"))
        .expect("Range tag should exist");

    let DeclareValue::Has(members) = &range.value else {
        panic!("expected DeclareValue::Has, got {:?}", range.value);
    };
    assert_eq!(members.len(), 2, "expected 2 members");
    let HasMember::Property(start) = &members[0] else {
        panic!("start should be a property member");
    };
    assert_eq!(start.name.as_str(), "start");
    assert!(start.body.is_none());

    let HasMember::Function(contains) = &members[1] else {
        panic!("contains should be a function member");
    };
    assert_eq!(contains.name.as_str(), "contains");
    assert!(contains.params.contains_key(&intern("self")));
    assert!(contains.body.is_none());
    assert!(
        contains.return_ty.is_some(),
        "contains should have return type"
    );
}

#[test]
fn has_block_body_retains_receiver_and_self_expression() {
    let out =
        "Region has\n    reset(ref self) Region:\n        self\n    return\n".parse_source_full();
    assert!(out.symptoms.is_empty(), "symptoms: {:?}", out.symptoms);
    let region = out.ast.tags.get(&intern("Region")).expect("Region");
    let DeclareValue::Has(members) = &region.value else {
        panic!("expected has members");
    };
    let HasMember::Function(reset) = &members[0] else {
        panic!("reset should be a function member");
    };
    assert!(reset.params.contains_key(&intern("self")));
    let Some(ast::HasMemberBody::Overrideable(body)) = &reset.body else {
        panic!("reset should have an overrideable body");
    };
    let ast::BindValue::Body { exprs, ret } = body else {
        panic!("reset should have a block body");
    };
    assert!(matches!(exprs.as_slice(), [body] if matches!(body.value, Expr::SelfRef)));
    assert!(ret.value.is_none());
}

#[test]
fn nested_has_method_registers_only_qualified_name() {
    let out =
        "Range(x) has\n    start x\n    end x\n\n    new(start x, end x) Self: Self(start, end)\n"
            .parse_source_full();
    assert!(out.symptoms.is_empty(), "symptoms: {:?}", out.symptoms);
    assert!(out.ast.defs.contains_key(&intern("Range.new")));
    assert!(!out.ast.defs.contains_key(&intern("new")));
    assert_eq!(out.ast.method_binds.len(), 1);
    let return_ty = out.ast.method_binds[0]
        .return_tag
        .as_ref()
        .expect("method return type");
    assert!(
        matches!(&return_ty.value, TypeExpr::Nominal(name, _) if name.as_str() == "Self"),
        "expected Self return type, got {:?}",
        return_ty.value
    );
}

#[test]
fn method_with_body_after_colon_parses() {
    let out = "Range(x).new(start x, end x): Range(start, end)\n".parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "expected method bind to parse, got symptoms: {:?}",
        out.symptoms,
    );
}

#[test]
fn range_gin_method_form_parses() {
    let src = "\
use '../'.Bounded

--- Range utilities.
Range(x) has start x, end x
Range(x).Bounded has
    min: self.start
    max: self.end

--- create a new range
Range(x).new(start x, end x): Range(start, end)
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "expected range snippet to parse, got symptoms: {:#?}",
        out.symptoms,
    );
    let range = out.ast.tags.get(&intern("Range")).expect("Range");
    let bounded = range
        .provided_traits
        .iter()
        .find(|provided| provided.trait_name.as_str() == "Bounded")
        .expect("Bounded provision");
    assert_eq!(bounded.fields.len(), 2);
    let min_span = out.ast.span_table.get(bounded.fields[0].1.span_id);
    let self_start = src.find("self.start").expect("self.start");
    assert!(min_span.contains(self_start));
    assert!(min_span.contains(self_start + "self.".len()));
}

#[test]
fn parenthesized_has_members_emit_removed_syntax_error() {
    let out = "Coord has (x Int, y Int)".parse_source_full();
    assert!(
        out.symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-removed-has-parens"),
        "expected parse-removed-has-parens, got symptoms: {:?}",
        out.symptoms,
    );
}

#[test]
fn paren_provision_syntax_emits_error() {
    let out = "Range.Bounded(min: self.start, max: self.end)\n".parse_source_full();
    assert!(
        out.symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-removed-provision-parens"),
        "expected parse-removed-provision-parens, got symptoms: {:?}",
        out.symptoms,
    );
}
