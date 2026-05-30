//! Parser tests for the ownership system: `eat` consume-parameter syntax
//! and `eat expr` call-site consume argument syntax.

use ast::{Expr, ParamConvention, TypeExpr};
use internment::Intern;
use parser::parse_from_str;

#[test]
fn test_parse_bare_param_is_inferred_by_default() {
    let src = "print(s String): 0
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("print")).unwrap();
    // Bare params default to Inferred; the parser only stores non-default conventions,
    // so nothing should be in the map for this param.
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
}

#[test]
fn test_parse_mixed_params() {
    let src = "process(eat db Database, txn Transaction, name String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("process")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("db")),
        Some(&ParamConvention::Eat)
    );
    // Bare params (Inferred, default) are not stored in the map.
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
fn test_parse_own_param_with_return_type() {
    let src = "consume(s String) Int:
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("consume")).unwrap();
    // Bare param defaults to Inferred; convention not stored in the map.
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
    // Return tag should be set (capitalized type annotation)
    assert!(
        bind.return_tag.is_some(),
        "return_tag should be Some for Int return type"
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
    let ast = parse_from_str(src);
    assert!(
        ast.defs().contains_key(&Intern::from_ref("main")),
        "main should be a def"
    );
    let main_def = ast.defs().get(&Intern::from_ref("main")).unwrap();
    if let ast::BindValue::Body { exprs, .. } = main_def.value() {
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
fn test_parse_ref_param() {
    let src = "read(ref e Entity) Int:
    return e.hp
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("read")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("e")),
        Some(&ParamConvention::Ref(false))
    );
}

#[test]
fn test_parse_mut_param() {
    let src = "write(mut e Entity, hp Int):
    e.hp: hp
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("write")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("e")),
        Some(&ParamConvention::Ref(true))
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
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("consume")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("e")),
        Some(&ParamConvention::Eat)
    );
}

#[test]
fn test_parse_mixed_ref_and_bare_params() {
    let src = "attack(ref a Entity, ref d Entity):
    d.hp: d.hp - a.damage
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("attack")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("a")),
        Some(&ParamConvention::Ref(false))
    );
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("d")),
        Some(&ParamConvention::Ref(false))
    );
}

#[test]
fn test_parse_local_ref_type_annotation() {
    // ref type annotation on a local bind: `r ref Entity: ref e`
    let src = "\
main:
    e: Entity(10, 10)
    r ref Entity: ref e
    return 0
return
";
    let ast = parse_from_str(src);
    let main_def = ast.defs().get(&Intern::from_ref("main")).unwrap();
    // Look inside the body for the Bind with name "r"
    if let ast::BindValue::Body { exprs, .. } = main_def.value() {
        let r_bind = exprs
            .iter()
            .find_map(|expr| {
                if let Expr::Bind(bind) = &expr.value {
                    if bind.name().as_str() == "r" {
                        Some(bind)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .expect("expected a Bind for 'r'");

        // Check that the return_tag has TypeExpr::Ref wrapping Entity
        if let Some(return_tag) = &r_bind.return_tag {
            match &return_tag.value {
                TypeExpr::Ref { inner, mutable } => {
                    assert!(!mutable, "expected immutable ref");
                    match &inner.value {
                        TypeExpr::Nominal(name, _) => {
                            assert_eq!(name.as_str(), "Entity");
                        }
                        other => panic!("expected Nominal inside Ref, got {other:?}"),
                    }
                }
                other => panic!("expected TypeExpr::Ref, got {other:?}"),
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
    // mut type annotation: `r mut Entity: mut e`
    let src = "\
main:
    e: Entity(10, 10)
    r mut Entity: mut e
    return 0
return
";
    let ast = parse_from_str(src);
    let main_def = ast.defs().get(&Intern::from_ref("main")).unwrap();
    if let ast::BindValue::Body { exprs, .. } = main_def.value() {
        let r_bind = exprs
            .iter()
            .find_map(|expr| {
                if let Expr::Bind(bind) = &expr.value {
                    if bind.name().as_str() == "r" {
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
                TypeExpr::Ref { inner, mutable } => {
                    assert!(*mutable, "expected mutable ref");
                    match &inner.value {
                        TypeExpr::Nominal(name, _) => {
                            assert_eq!(name.as_str(), "Entity");
                        }
                        other => panic!("expected Nominal inside Ref, got {other:?}"),
                    }
                }
                other => panic!("expected TypeExpr::Ref, got {other:?}"),
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
process[r Entity](ref[r] a Entity, ref[r] d Entity) Int:
    return a.hp + d.hp
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("process")).unwrap();
    assert_eq!(bind.group_params.len(), 1);
    assert_eq!(bind.group_params[0].name.as_str(), "r");
    assert_eq!(bind.group_params[0].ty_name.as_str(), "Entity");
    assert!(!bind.group_params[0].mutable);
}

#[test]
fn test_parse_group_annotation_mutable() {
    let src = "\
attack[mut r Entity](ref[r] a Entity, ref[r] d Entity):
    d.hp: d.hp - a.calculate_damage(d)
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("attack")).unwrap();
    assert_eq!(bind.group_params.len(), 1);
    assert_eq!(bind.group_params[0].name.as_str(), "r");
    assert!(bind.group_params[0].mutable);
}

#[test]
fn test_parse_multiple_group_annotations() {
    let src = "\
process[e Entity, mut rr Ring](ref[e] entity Entity, mut[rr] ring Ring):
    ring.power: ring.power + entity.energy
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("process")).unwrap();
    assert_eq!(bind.group_params.len(), 2);
    assert_eq!(bind.group_params[0].name.as_str(), "e");
    assert!(!bind.group_params[0].mutable);
    assert_eq!(bind.group_params[1].name.as_str(), "rr");
    assert!(bind.group_params[1].mutable);
    // Check param_groups:
    assert_eq!(
        bind.param_groups.get(&Intern::from_ref("entity")),
        Some(&Intern::from_ref("e"))
    );
    assert_eq!(
        bind.param_groups.get(&Intern::from_ref("ring")),
        Some(&Intern::from_ref("rr"))
    );
}

#[test]
fn test_parse_and_has_copy_override() {
    let src = "Transaction has (id Int) and\n    has Copy(can_copy: False)\n";
    let ast = parse_from_str(src);
    let decl = ast.tags().get(&Intern::from_ref("Transaction")).unwrap();
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
    let src = "Transaction has (id Int)\n     and is not Copy\n";
    let out = parser::parse_source_full(src);
    assert!(
        out.symptoms.iter().any(|d| d.message.contains("marker syntax removed")),
        "expected migration diagnostic, got: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn test_parse_group_annotation_param_groups_mapped() {
    // Verify that param_groups correctly maps param names to group names.
    let src = "\
process[e Entity, mut rr Ring](ref[e] entity Entity, mut[rr] ring Ring):
    ring.power: ring.power + entity.energy
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("process")).unwrap();

    // entity is ref in immutable group e
    assert_eq!(
        bind.param_groups.get(&Intern::from_ref("entity")),
        Some(&Intern::from_ref("e"))
    );

    // ring is ref in mutable group rr
    assert_eq!(
        bind.param_groups.get(&Intern::from_ref("ring")),
        Some(&Intern::from_ref("rr"))
    );

    // Verify conventions: ref entity → Ref(false), mut[rr] ring → Ref(true)
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("entity")),
        Some(&ParamConvention::Ref(false))
    );
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("ring")),
        Some(&ParamConvention::Ref(true))
    );
}
