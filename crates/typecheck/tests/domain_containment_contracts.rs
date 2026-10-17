use ast::integer::{IntegerDomain, IntegerValidity};
use ast::{NormalExpr, Ty};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TagId, TypedFileAst};

fn interval(min: i128, max: i128) -> IntegerDomain {
    IntegerDomain::bounded(min.into(), max.into()).unwrap()
}

fn checked(source: IntegerDomain, target: IntegerDomain, program: &str) -> TypedFileAst {
    let parsed = program.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut context = TransformCtx::new();
    for (name, domain) in [("Source", source), ("Target", target)] {
        context.cross_file_tag_types.insert(
            TagId(Intern::from_ref(name)),
            Ty::AnonymousInteger {
                validity: IntegerValidity::new(domain),
            },
        );
    }
    transform(&parsed.ast, FileId(0), &context)
}

#[test]
fn conversion_rejects_a_union_gap_and_accepts_complete_union_coverage() {
    for (source, target, valid) in [
        (
            interval(-1, 1),
            IntegerDomain::union([interval(-4, -2), interval(2, 4)]),
            false,
        ),
        (
            interval(1, 4),
            IntegerDomain::union([interval(1, 2), interval(3, 4)]),
            true,
        ),
    ] {
        let typed = checked(
            source,
            target,
            "convert(value Source) Target: return value as Target\n",
        );
        let flaws = typed.all_flaws();
        if valid {
            assert!(flaws.is_empty(), "{flaws:?}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-integer-narrowing-failed"),
                "union gap accepted: {flaws:?}"
            );
        }
    }
}

#[test]
fn conversion_does_not_assume_unknown_membership_is_absence() {
    let uncertain = IntegerDomain::from_refinement(&ast::PredicateExpr::And(vec![
        ast::PredicateExpr::Ge(1_i128.into()),
        ast::PredicateExpr::Le(4_i128.into()),
        ast::PredicateExpr::Ne(NormalExpr::Var(Intern::from_ref("unknown"))),
    ]));
    let typed = checked(
        IntegerDomain::union([uncertain, interval(8, 8)]),
        IntegerDomain::excluding(1.into(), 8.into(), vec![2.into()]).unwrap(),
        "convert(value Source) Target: return value as Target\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-integer-narrowing-unproven"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn arguments_require_complete_containment_in_a_union_parameter() {
    let typed = checked(
        interval(-1, 1),
        IntegerDomain::union([interval(-4, -2), interval(2, 4)]),
        "accept(value Target): return\ninvoke(value Source):\n    accept(value)\n    return\n",
    );
    assert!(!typed.all_flaws().is_empty(), "union-gap argument accepted");
}
