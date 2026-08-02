//! Parser tests for the ownership system: `eat` consume-parameter syntax
//! and `eat expr` call-site consume argument syntax.

use ast::{NormalExpr, Expr, ParamConvention, ParameterKind, PredicateExpr};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn test_parse_bare_param_defaults_to_ownership() {
    let src = "print(s String): 0
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("print")).unwrap();
    // Default ownership is represented by the absence of an explicit convention.
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
}

#[test]
fn test_parse_mixed_params() {
    let src = "process(eat db Database, txn Transaction, name String):
    return 0
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("process")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("db")),
        Some(&ParamConvention::Consume)
    );
    // Default ownership is not stored in the explicit-convention map.
    assert!(
        bind.param_conventions
            .get(&Intern::from_ref("txn"))
            .is_none()
    );
    assert!(
        bind.param_conventions
            .get(&Intern::from_ref("name"))
            .is_none()
    );
}

#[test]
fn test_parse_default_owned_param_with_return_type() {
    let src = "consume(s String) Int:
    return 0
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("consume")).unwrap();
    // Default ownership is not stored in the explicit-convention map.
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
    // Return tag should be set (capitalized type annotation)
    assert!(
        bind.return_tag.is_some(),
        "return_tag should be Some for Int return type"
    );
}

#[test]
fn test_parse_typed_hidden_state_parameters() {
    let src = "List(x, length PointerSize: ?, capacity PointerSize: ?, storage StorageIdentity: ?) has\n    pointer Pointer(x)\n";
    let output = src.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let declaration = output.ast.tags.get(&Intern::from_ref("List")).unwrap();
    let params = declaration.params.as_ref().unwrap();

    assert!(matches!(
        params.get(&Intern::from_ref("x")).map(|param| &param.kind),
        Some(&ParameterKind::Generic)
    ));
    for name in ["length", "capacity", "storage"] {
        assert!(matches!(
            params.get(&Intern::from_ref(name)).map(|param| &param.kind),
            Some(&ParameterKind::Inferred { .. })
        ));
    }
}

#[test]
fn test_parse_parameter_refinement_chain() {
    let src = "get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)\n";
    let output = src.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let bind = output.ast.defs.get(&Intern::from_ref("get")).unwrap();

    assert_eq!(
        bind.param_refinements.get(&Intern::from_ref("index")),
        Some(&PredicateExpr::And(vec![
            PredicateExpr::Ge(NormalExpr::from(0)),
            PredicateExpr::Lt(NormalExpr::Var(Intern::from_ref("n"))),
        ]))
    );
}

#[test]
fn test_parse_consume_arg_in_call() {
    // A call with `eat x` argument inside a function body
    let src = "\
main:
    x: 42
    dummy(eat x)
    return 0
return
";
    let ast = src.parse_source_full().ast;
    assert!(
        ast.defs.contains_key(&Intern::from_ref("main")),
        "main should be a def"
    );
    let main_def = ast.defs.get(&Intern::from_ref("main")).unwrap();
    if let ast::BindValue::Body { exprs, .. } = &main_def.value {
        let has_consume_arg = exprs.iter().any(|expr| {
            if let Expr::FnCall(call) = &expr.value
                && let Some(args) = &call.args
            {
                return args.iter().any(|a| matches!(&a.value, Expr::ConsumeArg(_)));
            }
            false
        });
        assert!(has_consume_arg, "expected ConsumeArg in call args");
    }
}

#[test]
fn test_tilde_consume_syntax_is_rejected() {
    let output = "consume(~value Entity): 0\n".parse_source_full();

    assert!(
        output
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "lex-unexpected-character")
    );
    let consume = output.ast.defs.get(&Intern::from_ref("consume")).unwrap();
    assert_ne!(
        consume.param_conventions.get(&Intern::from_ref("value")),
        Some(&ParamConvention::Consume)
    );
}

#[test]
fn test_parse_ref_param() {
    let src = "read(ref e Entity) Int:
    return e.hp
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("read")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("e")),
        Some(&ParamConvention::Observe)
    );
}

#[test]
fn test_parse_mut_param() {
    let src = "write(mut e Entity, hp Int):
    e.hp: hp
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("write")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("e")),
        Some(&ParamConvention::Mutate)
    );
    // hp is bare - should be Inferred (not stored)
    assert!(
        bind.param_conventions
            .get(&Intern::from_ref("hp"))
            .is_none()
    );
}

#[test]
fn test_parse_eat_param() {
    let src = "consume(eat e Entity) Int:
    return e.hp
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("consume")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("e")),
        Some(&ParamConvention::Consume)
    );
}

#[test]
fn test_parse_mixed_ref_and_bare_params() {
    let src = "attack(ref a Entity, ref d Entity):
    d.hp: d.hp - a.damage
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("attack")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("a")),
        Some(&ParamConvention::Observe)
    );
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("d")),
        Some(&ParamConvention::Observe)
    );
}

#[test]
fn test_parse_local_ref_type_annotation() {
    let src = "\
main:
    e: Entity(10, 10)
    ref r Entity: e
    return 0
return
";
    let ast = src.parse_source_full().ast;
    let main_def = ast.defs.get(&Intern::from_ref("main")).unwrap();
    // Look inside the body for the Bind with name "r"
    if let ast::BindValue::Body { exprs, .. } = &main_def.value {
        let r_bind = exprs
            .iter()
            .find_map(|expr| {
                if let Expr::Bind(bind) = &expr.value {
                    if bind.name.as_str() == "r" {
                        Some(bind)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .expect("expected a Bind for 'r'");

        // Check that the return_tag has an Expr::Ref wrapping Entity
        if let Some(return_tag) = &r_bind.return_tag {
            match &return_tag.value {
                Expr::Ref {
                    inner,
                    mutable,
                    group,
                } => {
                    assert!(!mutable, "expected immutable ref");
                    assert!(group.is_none());
                    match &inner.value {
                        Expr::AnonymousTag(name) => {
                            assert_eq!(name.as_str(), "Entity");
                        }
                        other => panic!("expected Nominal inside Ref, got {other:?}"),
                    }
                }
                other => panic!("expected Expr::Ref, got {other:?}"),
            }
        } else {
            panic!("expected return_tag to be set for ref type annotation");
        }
    } else {
        panic!("expected main to have a Body");
    }
}

#[test]
fn test_parse_local_mut_type_annotation() {
    let src = "\
main:
    e: Entity(10, 10)
    mut r Entity: e
    return 0
return
";
    let ast = src.parse_source_full().ast;
    let main_def = ast.defs.get(&Intern::from_ref("main")).unwrap();
    if let ast::BindValue::Body { exprs, .. } = &main_def.value {
        let r_bind = exprs
            .iter()
            .find_map(|expr| {
                if let Expr::Bind(bind) = &expr.value {
                    if bind.name.as_str() == "r" {
                        Some(bind)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .expect("expected a Bind for 'r'");

        if let Some(return_tag) = &r_bind.return_tag {
            match &return_tag.value {
                Expr::Ref {
                    inner,
                    mutable,
                    group,
                } => {
                    assert!(*mutable, "expected mutable ref");
                    assert!(group.is_none());
                    match &inner.value {
                        Expr::AnonymousTag(name) => {
                            assert_eq!(name.as_str(), "Entity");
                        }
                        other => panic!("expected Nominal inside Ref, got {other:?}"),
                    }
                }
                other => panic!("expected Expr::Ref, got {other:?}"),
            }
        } else {
            panic!("expected return_tag to be set for mut type annotation");
        }
    } else {
        panic!("expected main to have a Body");
    }
}

#[test]
fn test_parse_group_annotation_immutable() {
    let src = "\
process(ref{r} a Entity, ref{r} d Entity) Int:
    return a.hp + d.hp
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("process")).unwrap();
    assert_eq!(bind.group_params.len(), 1);
    assert_eq!(bind.group_params[0].path.to_string(), "r");
    assert_eq!(bind.group_params[0].ty_name.as_str(), "Entity");
    assert!(!bind.group_params[0].mutable);
}

#[test]
fn test_parse_group_annotation_mutable() {
    let src = "\
attack(mut{r} a Entity, mut{r} d Entity):
    d.hp: d.hp - a.calculate_damage(d)
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("attack")).unwrap();
    assert_eq!(bind.group_params.len(), 1);
    assert_eq!(bind.group_params[0].path.to_string(), "r");
    assert_eq!(bind.group_params[0].ty_name.as_str(), "Entity");
    assert!(bind.group_params[0].mutable);
}

#[test]
fn test_parse_semantic_group_path() {
    let output = "power_up(ref{entities} entity Entity, mut{entities.rings.items} ring Ring): 0\n"
        .parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let bind = output.ast.defs.get(&Intern::from_ref("power_up")).unwrap();
    let parameter = bind
        .params
        .as_ref()
        .unwrap()
        .get(&Intern::from_ref("ring"))
        .unwrap();
    let ast::ParameterKind::Tagged(parameter) = &parameter.kind else {
        panic!("expected tagged parameter");
    };
    let Expr::Ref {
        group: Some(group), ..
    } = &parameter.value
    else {
        panic!("expected grouped reference");
    };

    assert_eq!(group.root.as_str(), "entities");
    assert_eq!(
        group
            .segments
            .iter()
            .map(|segment| segment.as_str())
            .collect::<Vec<_>>(),
        vec!["rings", "items"]
    );
    assert_eq!(
        bind.param_groups.get(&Intern::from_ref("ring")),
        Some(group)
    );
}

#[test]
fn test_parse_multiple_group_annotations() {
    let src = "\
process(ref{e} entity Entity, mut{rr} ring Ring):
    ring.power: ring.power + entity.energy
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("process")).unwrap();
    assert_eq!(bind.group_params.len(), 2);
    assert_eq!(bind.group_params[0].path.to_string(), "e");
    assert!(!bind.group_params[0].mutable);
    assert_eq!(bind.group_params[1].path.to_string(), "rr");
    assert!(bind.group_params[1].mutable);
    // Check param_groups:
    assert_eq!(
        bind.param_groups
            .get(&Intern::from_ref("entity"))
            .map(ToString::to_string)
            .as_deref(),
        Some("e")
    );
    assert_eq!(
        bind.param_groups
            .get(&Intern::from_ref("ring"))
            .map(ToString::to_string)
            .as_deref(),
        Some("rr")
    );
}

#[test]
fn test_group_is_preserved_on_parameter_and_return_reference_types() {
    let src = "\
borrow(ref{r} value Entity) ref{r} Entity: value
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("borrow")).unwrap();
    let parameter = bind
        .params
        .as_ref()
        .unwrap()
        .get(&Intern::from_ref("value"))
        .unwrap();

    let ast::ParameterKind::Tagged(parameter_ty) = &parameter.kind else {
        panic!("expected tagged parameter");
    };
    let Expr::Ref {
        inner,
        mutable,
        group,
    } = &parameter_ty.value
    else {
        panic!("expected grouped parameter reference");
    };
    assert!(!mutable);
    assert_eq!(group.as_ref().map(ToString::to_string).as_deref(), Some("r"));
    assert!(matches!(&inner.value, Expr::AnonymousTag(name) if name.as_str() == "Entity"));

    let Expr::Ref {
        inner,
        mutable,
        group,
    } = &bind.return_tag.as_ref().unwrap().value
    else {
        panic!("expected grouped return reference");
    };
    assert!(!mutable);
    assert_eq!(group.as_ref().map(ToString::to_string).as_deref(), Some("r"));
    assert!(matches!(&inner.value, Expr::AnonymousTag(name) if name.as_str() == "Entity"));
}

#[test]
fn test_parse_and_has_copy_override() {
    let src = "Transaction has Copy\n    id Int\n    Copy.can_copy: False\n";
    let ast = src.parse_source_full().ast;
    let decl = ast.tags.get(&Intern::from_ref("Transaction")).unwrap();
    let pt = decl
        .provided_traits
        .iter()
        .find(|p| p.trait_name.as_str() == "Copy")
        .expect("Copy provided trait");
    assert_eq!(pt.fields.len(), 1);
    assert_eq!(pt.fields[0].0.as_str(), "can_copy");
}

#[test]
fn test_parse_and_is_not_copy_rejected() {
    let src = "Transaction has id Int\n     and is not Copy\n";
    let out = src.parse_source_full();
    assert!(
        out.symptoms
            .iter()
            .any(|d| d.message.contains("marker syntax removed")),
        "expected migration diagnostic, got: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn test_parse_group_annotation_param_groups_mapped() {
    let src = "\
process(ref{e} entity Entity, mut{rr} ring Ring):
    ring.power: ring.power + entity.energy
return
";
    let ast = src.parse_source_full().ast;
    let bind = ast.defs.get(&Intern::from_ref("process")).unwrap();

    // entity is ref in immutable group e
    assert_eq!(
        bind.param_groups
            .get(&Intern::from_ref("entity"))
            .map(ToString::to_string)
            .as_deref(),
        Some("e")
    );

    // ring is ref in mutable group rr
    assert_eq!(
        bind.param_groups
            .get(&Intern::from_ref("ring"))
            .map(ToString::to_string)
            .as_deref(),
        Some("rr")
    );

    // Verify conventions: ref entity → Ref(false), mut{rr} ring → Ref(true)
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("entity")),
        Some(&ParamConvention::Observe)
    );
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("ring")),
        Some(&ParamConvention::Mutate)
    );
}
