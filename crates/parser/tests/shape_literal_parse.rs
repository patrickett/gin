//! Layer 1 shape-literal parsing tests (1.1–1.11).
//!
//! Tests covering the various forms of tag/variant construction with
//! named arguments, shorthand arguments, qualified variant paths,
//! bare-variant expressions, and nested shape literals.

use ast::{BindValue, Expr, TagCall};
use parser::query::SourceParseExt;

mod support;
use support::*;

#[test]
fn top_level_record_field_write_parses_as_record_set() {
    let out = "Int is in 1...400\n\nCoord has x Int, y Int, z Int\np: Coord(x: 1, y: 2, z: 3)\np.x:: 10\n"
        .parse_source_full();
    assert!(
        out.symptoms
            .iter()
            .all(|symptom| symptom.category != diagnostic::Category::Flaw),
        "symptoms: {:?}",
        out.symptoms,
    );
    assert!(
        matches!(out.ast.exprs.as_slice(), [expr] if matches!(expr.0, Expr::RecordSet { .. })),
        "expected RecordSet, got {:?}",
        out.ast.exprs,
    );
}

#[test]
fn shape_literal_named_fields() {
    let source = "\
Coord has x Int, y Int, z Int
origin := Coord(x: 0, y: 0, z: 0)
";
    let out = source.parse_source_full();
    let file = out.ast;

    // `origin` should be a constant bind whose value is a TagCall.
    let origin = file.defs.get(&intern("origin")).expect("origin bind");
    let BindValue::Expr(typed_expr) = &origin.value else {
        panic!("origin should be Expr, got {:?}", origin.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("origin value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Coord", "TagCall name should be Coord");
    assert!(qual_path.is_none(), "expected no qual_path for bare Coord");
    assert_eq!(args.len(), 3, "expected 3 named arguments");

    // Each named arg is a Bind expression: x: 0, y: 0, z: 0
    let field_names: Vec<&str> = args
        .iter()
        .map(|a| match &a.value {
            Expr::Bind(b) => b.name.as_str(),
            other => panic!("arg should be Expr::Bind, got {other:?}"),
        })
        .collect();
    assert_eq!(field_names, ["x", "y", "z"], "field names in source order");
}

#[test]
fn shape_literal_shorthand() {
    let source = "\
Coord has x Int, y Int, z Int
x := 1; y := 2; z := 3
p := Coord(x, y, z)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let p = file.defs.get(&intern("p")).expect("p bind");
    let BindValue::Expr(typed_expr) = &p.value else {
        panic!("p should be Expr, got {:?}", p.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("p value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Coord");
    assert!(qual_path.is_none());
    assert_eq!(args.len(), 3, "expected 3 shorthand args");

    // In Gin, shorthand identifiers in argument position parse as FnCall
    // (a bare-variable reference), *not* as Expr::Bind.
    for (i, arg) in args.iter().enumerate() {
        match &arg.value {
            Expr::FnCall(call) => {
                assert_eq!(
                    call.path.value.root.as_str(),
                    ["x", "y", "z"][i],
                    "shorthand arg {} name",
                    i
                );
                assert!(call.path.value.segments.is_empty(), "no path segments");
                assert!(call.args.is_none(), "no call args");
            }
            other => panic!("shorthand arg {} should be FnCall, got {:?}", i, other),
        }
    }
}

#[test]
fn shape_literal_mixed_shorthand_and_explicit() {
    let source = "\
Coord has x Int, y Int, z Int
x := 1
p := Coord(x, y: x + 1, z: x + 2)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let p = file.defs.get(&intern("p")).expect("p bind");
    let BindValue::Expr(typed_expr) = &p.value else {
        panic!("p should be Expr, got {:?}", p.value);
    };
    let Expr::TagCall(TagCall { name, args, .. }) = &typed_expr.value else {
        panic!("p value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Coord");
    assert_eq!(args.len(), 3, "expected 3 args");

    // First arg: shorthand `x` → FnCall
    assert!(
        matches!(&args[0].value, Expr::FnCall(c) if c.path.value.root.as_str() == "x"),
        "first arg should be reference to x, got {:?}",
        args[0].value
    );

    // Second arg: named `y: x + 1` → Bind with name "y" containing a Binary expr
    match &args[1].value {
        Expr::Bind(b) => {
            assert_eq!(b.name.as_str(), "y", "named arg should be y");
            match &b.value {
                BindValue::Expr(inner) => {
                    assert!(
                        matches!(&inner.value, Expr::Binary(_)),
                        "y value should be Binary, got {:?}",
                        inner.value
                    );
                }
                other => panic!("y value should be Expr, got {other:?}"),
            }
        }
        other => panic!("second arg should be Expr::Bind, got {other:?}"),
    }

    // Third arg: named `z: x + 2` → Bind with name "z" containing a Binary expr
    match &args[2].value {
        Expr::Bind(b) => {
            assert_eq!(b.name.as_str(), "z", "named arg should be z");
            match &b.value {
                BindValue::Expr(inner) => {
                    assert!(
                        matches!(&inner.value, Expr::Binary(_)),
                        "z value should be Binary, got {:?}",
                        inner.value
                    );
                }
                other => panic!("z value should be Expr, got {other:?}"),
            }
        }
        other => panic!("third arg should be Expr::Bind, got {other:?}"),
    }
}

#[test]
fn shape_literal_reordered_fields() {
    let source = "\
Coord has x Int, y Int, z Int
p := Coord(z: 3, x: 1, y: 2)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let p = file.defs.get(&intern("p")).expect("p bind");
    let BindValue::Expr(typed_expr) = &p.value else {
        panic!("p should be Expr, got {:?}", p.value);
    };
    let Expr::TagCall(TagCall { args, .. }) = &typed_expr.value else {
        panic!("p value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(args.len(), 3, "expected 3 args");
    let field_names: Vec<&str> = args
        .iter()
        .map(|a| match &a.value {
            Expr::Bind(b) => b.name.as_str(),
            other => panic!("arg should be Expr::Bind, got {other:?}"),
        })
        .collect();
    // Source order must be preserved: z, x, y
    assert_eq!(field_names, ["z", "x", "y"], "fields in source order");
}

// The parser does *not* reject bare expressions (e.g. `Coord(1, 2, 3)`) —
// they parse as positional TagCall arguments with Lit or other expression
// nodes.  Rejection of bare expressions is a typechecker concern.

#[test]
fn shape_literal_accepts_bare_expressions() {
    let source = "\
Coord has x Int, y Int, z Int
x: Coord(1, 2, 3)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let x = file.defs.get(&intern("x")).expect("x bind");
    let BindValue::Expr(typed_expr) = &x.value else {
        panic!("x should be Expr, got {:?}", x.value);
    };
    let Expr::TagCall(TagCall { name, args, .. }) = &typed_expr.value else {
        panic!("x value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Coord");
    assert_eq!(args.len(), 3, "three bare expression args");
    // Each bare expression is a Lit node (integer literal).
    for arg in args {
        assert!(
            matches!(&arg.value, Expr::Lit(_)),
            "bare arg should be Lit, got {:?}",
            arg.value
        );
    }
}

// Like 1.5, the parser accepts bare expressions.  The `x + 1` argument is
// parsed as an inline Binary expression.

#[test]
fn shape_literal_accepts_bare_non_identifier() {
    let source = "\
Coord has x Int, y Int, z Int
x := 1
p: Coord(x + 1, y, z)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let p = file.defs.get(&intern("p")).expect("p bind");
    let BindValue::Expr(typed_expr) = &p.value else {
        panic!("p should be Expr, got {:?}", p.value);
    };
    let Expr::TagCall(TagCall { name, args, .. }) = &typed_expr.value else {
        panic!("p value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Coord");
    assert_eq!(args.len(), 3, "three args");

    // First arg: bare expression `x + 1` → Binary
    assert!(
        matches!(&args[0].value, Expr::Binary(_)),
        "first arg should be Binary (x+1), got {:?}",
        args[0].value
    );
    // Remaining args: shorthand references
    assert!(matches!(&args[1].value, Expr::FnCall(c) if c.path.value.root.as_str() == "y"));
    assert!(matches!(&args[2].value, Expr::FnCall(c) if c.path.value.root.as_str() == "z"));
}

#[test]
fn variant_construction_named_fields() {
    let source = "\
Maybe(value) is Some(value) or None
val := Maybe.Some(value: 5)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let val = file.defs.get(&intern("val")).expect("val bind");
    let BindValue::Expr(typed_expr) = &val.value else {
        panic!("val should be Expr, got {:?}", val.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("val value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Some", "variant name is Some");
    let qp = qual_path.as_ref().expect("qual_path should be present");
    assert_eq!(qp.value.root.as_str(), "Maybe", "qual_path root is Maybe");
    assert_eq!(qp.value.segments.len(), 1, "one segment in qual_path");
    assert_eq!(qp.value.segments[0].as_str(), "Some", "segment is Some");
    assert_eq!(args.len(), 1, "expected 1 arg");

    match &args[0].value {
        Expr::Bind(b) => {
            assert_eq!(b.name.as_str(), "value", "named arg is 'value'");
        }
        other => panic!("arg should be Expr::Bind, got {other:?}"),
    }
}

#[test]
fn variant_construction_named_fields_with_type_annotation() {
    let source = "\
Maybe(value) is Some(value) or None
val Maybe(Int): Some(value: 5)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let val = file.defs.get(&intern("val")).expect("val bind");

    // The `Maybe(Int)` annotation is stored in `return_tag` as a TagCall.
    let return_tag = val
        .return_tag
        .as_ref()
        .expect("return_tag should capture Maybe(Int)");
    match &return_tag.value {
        Expr::TagCall(call) => {
            assert_eq!(call.name.as_str(), "Maybe", "return tag type is Maybe");
            assert_eq!(call.args.len(), 1, "one type param");
            assert!(
                matches!(call.args[0].value, Expr::AnonymousTag(name) if name.as_str() == "Int")
            );
        }
        other => panic!("return_tag should be TypeGeneric, got {other:?}"),
    }

    // The value `Some(value: 5)` is a TagCall without a qual_path (the qualification
    // comes from return_tag).
    let BindValue::Expr(typed_expr) = &val.value else {
        panic!("val should be Expr, got {:?}", val.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("val value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Some", "variant name is Some");
    // qual_path is None when the qualifier is in return_tag
    assert!(qual_path.is_none(), "qual_path is consumed by return_tag");
    assert_eq!(args.len(), 1, "expected 1 arg");

    match &args[0].value {
        Expr::Bind(b) => {
            assert_eq!(b.name.as_str(), "value", "named arg is 'value'");
        }
        other => panic!("arg should be Expr::Bind, got {other:?}"),
    }
}

#[test]
fn variant_construction_shorthand() {
    let source = "\
Maybe(value) is Some(value) or None
value := 5
val := Maybe.Some(value)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let val = file.defs.get(&intern("val")).expect("val bind");
    let BindValue::Expr(typed_expr) = &val.value else {
        panic!("val should be Expr, got {:?}", val.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("val value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Some");
    let qp = qual_path.as_ref().expect("qual_path should be present");
    assert_eq!(qp.value.root.as_str(), "Maybe");
    assert_eq!(args.len(), 1, "expected 1 arg");

    // Shorthand: `value` is a variable reference → FnCall
    match &args[0].value {
        Expr::FnCall(call) => {
            assert_eq!(call.path.value.root.as_str(), "value");
        }
        other => panic!("shorthand arg should be FnCall, got {other:?}"),
    }
}

#[test]
fn variant_construction_shorthand_with_type_annotation() {
    let source = "\
Maybe(value) is Some(value) or None
value := 5
val Maybe(Int): Some(value)
";
    let out = source.parse_source_full();
    let file = out.ast;

    let val = file.defs.get(&intern("val")).expect("val bind");

    // return_tag captures Maybe(Int)
    let return_tag = val
        .return_tag
        .as_ref()
        .expect("return_tag should capture Maybe(Int)");
    match &return_tag.value {
        Expr::TagCall(call) => {
            assert_eq!(call.name.as_str(), "Maybe");
            assert_eq!(call.args.len(), 1);
            assert!(
                matches!(call.args[0].value, Expr::AnonymousTag(name) if name.as_str() == "Int")
            );
        }
        other => panic!("return_tag should be TypeGeneric, got {other:?}"),
    }

    let BindValue::Expr(typed_expr) = &val.value else {
        panic!("val should be Expr, got {:?}", val.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("val value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "Some");
    assert!(qual_path.is_none(), "qual_path consumed by return_tag");
    assert_eq!(args.len(), 1);
}

#[test]
fn variant_no_fields_qualified() {
    let source = "\
Maybe(value) is Some(value) or None
none := Maybe.None
";
    let out = source.parse_source_full();
    let file = out.ast;

    let none = file.defs.get(&intern("none")).expect("none bind");
    let BindValue::Expr(typed_expr) = &none.value else {
        panic!("none should be Expr, got {:?}", none.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("none value should be TagCall, got {:?}", typed_expr.value);
    };

    assert_eq!(name.as_str(), "None");
    let qp = qual_path.as_ref().expect("qual_path should be present");
    assert_eq!(qp.value.root.as_str(), "Maybe");
    assert!(args.is_empty(), "no fields on None");
}

#[test]
fn variant_no_fields_bare_with_type_annotation() {
    let source = "\
Maybe(value) is Some(value) or None
none Maybe(Int): None
";
    let out = source.parse_source_full();
    let file = out.ast;

    let none = file.defs.get(&intern("none")).expect("none bind");

    // return_tag captures Maybe(Int)
    let return_tag = none
        .return_tag
        .as_ref()
        .expect("return_tag should capture Maybe(Int)");
    match &return_tag.value {
        Expr::TagCall(call) => {
            assert_eq!(call.name.as_str(), "Maybe");
            assert_eq!(call.args.len(), 1);
            assert!(
                matches!(call.args[0].value, Expr::AnonymousTag(name) if name.as_str() == "Int")
            );
        }
        other => panic!("return_tag should be TypeGeneric, got {other:?}"),
    }

    let BindValue::Expr(typed_expr) = &none.value else {
        panic!("none should be Expr, got {:?}", none.value);
    };

    // Bare `None` in this position should parse as AnonymousTag
    assert!(
        matches!(&typed_expr.value, Expr::AnonymousTag(name) if name.as_str() == "None"),
        "bare None should be AnonymousTag, got {:?}",
        typed_expr.value
    );
}

// The parser does *not* emit errors for unqualified `Some(value: 3)` in
// expression position — that is a typechecker / semantic concern.
// Qualified paths (`Maybe.Some(value: 3)`) and type-annotation forms
// (`Maybe(Int): Some(value: 3)`) parse without parse-level warnings.

#[test]
fn variant_construction_qualified_no_diag() {
    // Qualified `Maybe.Some(value: 3)` is valid — no parse-level warnings expected.
    let source = "Maybe(value) is Some(value) or None\nx: Maybe.Some(value: 3)\n";
    let out = source.parse_source_full();
    let parse_warnings_only = &out.ast.parse_warnings;
    let has_unexpected_warning = parse_warnings_only
        .iter()
        .any(|d| !d.message.contains("unused"));
    assert!(
        !has_unexpected_warning,
        "unexpected parse warnings for qualified variant: {:?}",
        parse_warnings_only
    );
}

#[test]
fn variant_construction_type_annotation_no_diag() {
    // `Maybe(Int): Some(value: 3)` uses the type annotation form — no warnings expected.
    let source = "Maybe(value) is Some(value) or None\nx Maybe(Int): Some(value: 3)\n";
    let out = source.parse_source_full();
    let parse_warnings_only = &out.ast.parse_warnings;
    let has_unexpected_warning = parse_warnings_only
        .iter()
        .any(|d| !d.message.contains("unused"));
    assert!(
        !has_unexpected_warning,
        "unexpected parse warnings for typed variant: {:?}",
        parse_warnings_only
    );
}

#[test]
fn nested_shape_literals() {
    let source = "\
Primitive has width Int, signed Bool
NamedTy has name String, ty Type
coord_ty := NamedTy(name: \"Coord\", ty: Primitive(width: 64, signed: True))
";
    let out = source.parse_source_full();
    let file = out.ast;

    let coord_ty = file.defs.get(&intern("coord_ty")).expect("coord_ty bind");
    let BindValue::Expr(typed_expr) = &coord_ty.value else {
        panic!("coord_ty should be Expr, got {:?}", coord_ty.value);
    };
    let Expr::TagCall(TagCall {
        name,
        qual_path,
        args,
    }) = &typed_expr.value
    else {
        panic!("coord_ty should be TagCall, got {:?}", typed_expr.value);
    };

    // Outer: NamedTy(...)
    assert_eq!(name.as_str(), "NamedTy");
    assert!(qual_path.is_none());
    assert_eq!(args.len(), 2, "NamedTy has 2 fields");

    // First field: `name: "Coord"` → Bind
    match &args[0].value {
        Expr::Bind(b) => {
            assert_eq!(b.name.as_str(), "name");
        }
        other => panic!("first arg should be Expr::Bind, got {other:?}"),
    }

    // Second field: `ty: Primitive(width: 64, signed: True)` → nested Bind/TagCall
    match &args[1].value {
        Expr::Bind(b) => {
            assert_eq!(b.name.as_str(), "ty");
            let BindValue::Expr(inner) = &b.value else {
                panic!("ty value should be Expr, got {:?}", b.value);
            };
            let Expr::TagCall(TagCall {
                name: inner_name,
                qual_path: inner_qp,
                args: inner_args,
            }) = &inner.value
            else {
                panic!("ty value should be TagCall, got {:?}", inner.value);
            };
            assert_eq!(inner_name.as_str(), "Primitive");
            assert!(inner_qp.is_none());
            assert_eq!(inner_args.len(), 2, "Primitive has 2 fields");
        }
        other => panic!("second arg should be Expr::Bind, got {other:?}"),
    }
}
