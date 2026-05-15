//! Parser tests for method binds with parameterized-type receivers,
//! e.g. `Range[x].new(start x, end x) Range[x]: (start, end)`.

use ast::{Expr, ParameterKind, TypeExpr};
use internment::Intern;
use parser::parse_from_str as parse_str;

fn intern(s: &str) -> Intern<String> {
    Intern::new(s.to_owned())
}

#[test]
fn parses_generic_method_bind_with_typevar_params_and_return() {
    let src =
        "Range[x] has (start x, end x)\n\nRange[x].new(start x, end x) Range[x]: (start, end)\n";
    let ast = parse_str(src);

    // Tag declaration is recorded as `Range`
    assert!(
        ast.tags().contains_key(&intern("Range")),
        "expected Range tag in tags(): {:?}",
        ast.tags().keys().collect::<Vec<_>>()
    );

    // Method def is mangled as `Range.new` (TypeGeneric collapses to base name)
    let bind = ast
        .defs()
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
    let params = bind.params().as_ref().expect("Range.new must have params");
    assert_eq!(params.len(), 2);
    let starts: Vec<_> = params.iter().collect();
    let (k0, v0) = &starts[0];
    let (k1, v1) = &starts[1];
    assert_eq!(k0.as_str(), "start");
    assert_eq!(k1.as_str(), "end");
    for (k, v) in [(*k0, v0), (*k1, v1)] {
        match v {
            ParameterKind::Tagged(sp) => match sp.value.as_type_expr() {
                Some(TypeExpr::Nominal(n, _)) => {
                    assert_eq!(n.as_str(), "x", "{} param type-var should be x", k.as_str());
                }
                other => panic!("{} param should be TypeNominal(x), got {:?}", k, other),
            },
            other => panic!("{} param kind should be Tagged, got {:?}", k, other),
        }
    }

    // Return type is TypeGeneric Range[x], stored on bind.return_tag
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
        other => panic!("return_tag should be TypeGeneric Range[x], got {:?}", other),
    }
}

#[test]
fn parses_method_bind_with_nontypevar_params() {
    // Sanity: ensure the existing `Type.method` form still parses for a non-generic
    // receiver — i.e., we did not regress the bare-Tag receiver path.
    let src = "Bool is True or False\n\nBool.to_string Str := 'true'\n";
    let ast = parse_str(src);

    let bind = ast
        .defs()
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
    // Per the design: `CustomRange has (start, end)` (no `(x)` after the type
    // name) leaves both fields independently generic. The corresponding method
    // `CustomRange.new(start, end) CustomRange: (start, end)` should likewise
    // parse cleanly without forcing the two params to share a type variable.
    let src =
        "CustomRange has (start, end)\n\nCustomRange.new(start, end) CustomRange: (start, end)\n";
    let ast = parse_str(src);

    assert!(ast.tags().contains_key(&intern("CustomRange")));

    let bind = ast
        .defs()
        .get(&intern("CustomRange.new"))
        .expect("CustomRange.new should be present");

    let params = bind.params().as_ref().expect("must have params");
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
Range[x] has (start x, end x)

--- create a new range
Range[x].new(start x, end x) Range[x]: (start, end)
";
    let ast = parse_str(src);
    let bind = ast
        .defs()
        .get(&intern("Range.new"))
        .expect("Range.new bind should exist");
    assert_eq!(
        bind.doc_comment().map(|doc| doc.value.as_str()),
        Some("create a new range")
    );
}

#[test]
fn parses_module_rooted_generic_method_call() {
    let src = "\
Range[x] has (start x, end x)

Range[x].new(start x, end x) Range[x]: (start, end)

core.Range.new(12, 1200)
";
    let ast = parse_str(src);

    let call = ast
        .top_level_exprs()
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
