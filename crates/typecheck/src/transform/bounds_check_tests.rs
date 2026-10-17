#[cfg(test)]
mod tests {
    use crate::{FileId, TagId, TypeRegistry};
    use ast::{Ty, TyArg};
    use internment::Intern;
    use parser::query::SourceParseExt;

    fn fixture() -> (TypeRegistry, Ty, Ty) {
        let parsed = "First is in 0 to 255\nSecond is in 0 to 255\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = crate::transform::transform_file(parsed.ast, FileId(0));
        let first = typed.tags[&TagId(Intern::from_ref("First"))]
            .resolved_ty
            .clone();
        let second = typed.tags[&TagId(Intern::from_ref("Second"))]
            .resolved_ty
            .clone();
        (typed.type_registry, first, second)
    }

    fn with_arguments(mut ty: Ty, arguments: Vec<(&str, Ty)>) -> Ty {
        let Ty::Named { instance, .. } = &mut ty else {
            panic!("nominal integer");
        };
        instance.arguments = arguments
            .into_iter()
            .map(|(name, ty)| (Intern::from_ref(name), TyArg::Type(Box::new(ty))))
            .collect();
        ty
    }

    fn accepts(registry: &TypeRegistry, actual: &Ty, expected: &Ty) -> bool {
        let mut flaws = Vec::new();
        super::super::check_value_against_param_ty(actual, None, expected, registry, &mut flaws);
        flaws.is_empty()
    }

    #[test]
    fn distinct_nominal_integers_do_not_match_empty_arguments() {
        let (registry, first, second) = fixture();
        assert!(!accepts(&registry, &first, &second));
        assert!(accepts(&registry, &first, &first));
    }

    #[test]
    fn projected_arguments_do_not_override_declaration_identity() {
        let (registry, first, second) = fixture();
        let actual = with_arguments(first, vec![("unit", Ty::Unit)]);
        let expected = with_arguments(
            second,
            vec![("unit", Ty::bounded_int(0.into(), 255.into()))],
        );
        assert!(!accepts(&registry, &actual, &expected));
    }

    #[test]
    fn argument_counts_and_names_must_match() {
        let (registry, first, _) = fixture();
        let expected = with_arguments(
            first.clone(),
            vec![("unit", Ty::bounded_int(0.into(), 255.into()))],
        );
        assert!(!accepts(&registry, &first, &expected));
        let wrong_name = with_arguments(first, vec![("other", Ty::Unit)]);
        assert!(!accepts(&registry, &wrong_name, &expected));
    }

    #[test]
    fn opaque_argument_does_not_hide_another_argument_mismatch() {
        let (registry, first, _) = fixture();
        let expected = with_arguments(
            first.clone(),
            vec![
                ("element", Ty::Opaque(Intern::from_ref("element"))),
                ("unit", Ty::Unit),
            ],
        );
        let actual = with_arguments(
            first,
            vec![
                ("element", Ty::Unit),
                ("unit", Ty::bounded_int(0.into(), 255.into())),
            ],
        );
        assert!(!accepts(&registry, &actual, &expected));
    }

    #[test]
    fn matching_opaque_and_projected_arguments_remain_accepted() {
        let (registry, first, _) = fixture();
        let actual = with_arguments(first.clone(), vec![("unit", Ty::Unit)]);
        for argument in [
            Ty::Opaque(Intern::from_ref("unit")),
            Ty::bounded_int(0.into(), 255.into()),
        ] {
            let expected = with_arguments(first.clone(), vec![("unit", argument)]);
            assert!(accepts(&registry, &actual, &expected));
        }
    }

    #[test]
    fn bounded_scalar_parameters_reject_known_noninteger_types() {
        let (registry, expected, _) = fixture();
        for actual in [
            Ty::Unit,
            Ty::Float { value: None },
            Ty::Array {
                elem: Box::new(Ty::Unit),
                size: ast::NormalExpr::from(1_i128),
            },
        ] {
            assert!(
                !accepts(&registry, &actual, &expected),
                "accepted {actual:?}"
            );
        }
    }

    #[test]
    fn bounded_scalar_checks_defer_unresolved_integer_types() {
        let (registry, expected, _) = fixture();
        for actual in [
            Ty::Opaque(Intern::from_ref("value")),
            Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer),
        ] {
            assert!(accepts(&registry, &actual, &expected), "{actual:?}");
        }
    }

    #[test]
    fn bounded_scalar_checks_defer_anonymous_integer_references() {
        let (registry, expected, _) = fixture();
        let expected = Ty::Ref {
            inner: Box::new(expected),
            mutable: false,
        };
        let actual = Ty::Ref {
            inner: Box::new(Ty::i64()),
            mutable: false,
        };
        assert!(accepts(&registry, &actual, &expected));
    }
}
