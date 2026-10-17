use super::*;
use parser::query::SourceParseExt;

#[test]
fn symbolic_sequence_type_lengths_preserve_qualified_names() {
    for name in ["count", "sizes.count", "package.sizes.count"] {
        let mut segments = name.split('.');
        let root = Intern::from_ref(segments.next().unwrap());
        let expr = ast::Expr::FnCall(ast::FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: ast::Spanned::new(
                ast::ModPath::new(root, segments.map(Intern::from_ref).collect()),
                ast::SpanId::INVALID,
            ),
            args: None,
        });
        let expected = NormalExpr::Var(Intern::from_ref(name));
        assert_eq!(
            size_expr_to_normal(&expr, SizeCallName::Mangled, true),
            Some(expected.clone())
        );
        let expr = ast::Expr::Binary(ast::Binary {
            op: ast::BinOp::Add,
            lhs: Box::new(ast::Typed::infer(expr, ast::SpanId::INVALID)),
            rhs: Box::new(ast::Typed::infer(
                ast::Expr::Lit(ast::Literal::Int(1_i128.into())),
                ast::SpanId::INVALID,
            )),
        });
        assert_eq!(
            size_expr_to_normal(&expr, SizeCallName::Mangled, true),
            Some(NormalExpr::Add(Box::new(expected), Box::new(1_i128.into())))
        );
    }
}

#[test]
fn associated_sequence_type_lengths_preserve_qualified_names() {
    for name in ["Sizes.count", "Sizes.dimensions.count"] {
        let parsed = format!("value: [1; {name}]\n").parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let ast::BindValue::Expr(expr) = &parsed.ast.defs[&Intern::from_ref("value")].value else {
            panic!("expected expression");
        };
        let tags = HashMap::new();
        let Ty::Array { size, .. } = TypeEnv::new(&tags).resolve_expr(&expr.value) else {
            panic!("expected array type");
        };
        assert_eq!(size, NormalExpr::Var(Intern::from_ref(name)));
    }
}

fn resolve_node(nominal: bool) -> Ty {
    let source = if nominal {
        "Node(element) has\n    next ref Node(element)\n"
    } else {
        "Node(element) has\n    value element\n"
    };
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let node = Intern::from_ref("Node");
    let byte = Intern::from_ref("Byte");
    let element = Intern::from_ref("element");
    let mut types = HashMap::from([(byte, Ty::anonymous_integer_for_width(8, false))]);
    if nominal {
        types.insert(
            node,
            Ty::Named {
                instance: ast::ty::NamedTypeInstance {
                    declaration: ast::ty::TypeId {
                        file: 0,
                        name: node,
                    },
                    arguments: vec![],
                },
                name: node,
            },
        );
    }
    let parameters = HashMap::from([(node, parsed.ast.tags[&node].params.clone().unwrap())]);
    TypeEnv::new(&types)
        .with_tag_params(&parameters)
        .with_tag_decls(&parsed.ast.tags)
        .resolve_generic(
            &node,
            &[(
                element,
                ParameterKind::Default(Box::new(ast::Typed::infer(
                    ast::Expr::AnonymousTag(byte),
                    ast::SpanId::INVALID,
                ))),
            )],
        )
}

#[test]
fn recursive_nominal_application_preserves_identity() {
    const CHILD: &str = "GIN_TEST_RECURSIVE_NOMINAL_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "analysis::type_surface::nominal_tests::recursive_nominal_application_preserves_identity", "--nocapture"])
            .env(CHILD, "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let node = Intern::from_ref("Node");
    assert_eq!(
        resolve_node(true),
        Ty::Named {
            instance: ast::ty::NamedTypeInstance {
                declaration: ast::ty::TypeId {
                    file: 0,
                    name: node
                },
                arguments: vec![(
                    Intern::from_ref("element"),
                    TyArg::Type(Box::new(Ty::anonymous_integer_for_width(8, false)))
                )],
            },
            name: node,
        }
    );
}

#[test]
fn structural_generic_application_still_resolves_its_fields() {
    let ty = resolve_node(false);
    let Ty::Record {
        fields,
        resolved_params,
        ..
    } = ty
    else {
        panic!("expected structural record")
    };
    assert_eq!(
        fields,
        vec![(
            Intern::from_ref("value"),
            Box::new(Ty::anonymous_integer_for_width(8, false))
        )]
    );
    assert_eq!(
        resolved_params,
        Some(vec![(
            Intern::from_ref("element"),
            TyArg::Type(Box::new(Ty::anonymous_integer_for_width(8, false)))
        )])
    );
}

#[test]
fn borrowed_declarations_survive_type_environment_derivation() {
    let local = "Local is 1\n".parse_source_full().ast.tags;
    let package = "Fallback is Local\n".parse_source_full().ast.tags;
    let declarations = crate::tag_declarations::TagDeclarations::overlay(&local, &package);
    let types = HashMap::new();
    let parameters = HashMap::new();
    let registry = crate::TypeRegistry::default();
    let env = TypeEnv::new(&types).with_tag_declaration_lookup(declarations);
    let binder = ast::BinderId::new(
        0,
        ast::BinderOwner::Parameter {
            definition: ast::DefinitionId::named(Intern::from_ref("caller")),
            slot: 0,
        },
    );
    for derived in [
        env.with_subst(&types),
        env.with_tag_params(&parameters),
        env.with_opt_tag_params(Some(&parameters)),
        env.with_opt_tag_params(None),
        env.with_subst(&types).with_registry(&registry),
        env.with_dependent_binder(binder),
    ] {
        let lookup = derived.tag_declarations().unwrap();
        assert!(std::ptr::eq(
            lookup.get(&Intern::from_ref("Local")).unwrap(),
            &local[&Intern::from_ref("Local")]
        ));
        assert!(std::ptr::eq(
            lookup.get(&Intern::from_ref("Fallback")).unwrap(),
            &package[&Intern::from_ref("Fallback")]
        ));
    }
    assert!(
        env.with_tag_decls(&local)
            .tag_declarations()
            .unwrap()
            .get(&Intern::from_ref("Fallback"))
            .is_none()
    );
}
