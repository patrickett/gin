use ast::ConstValue;
use ast::NormalExpr;
use internment::Intern;
use typecheck::typed::GroupProjection;
use typecheck::{ConstraintEnv, GroupId, Overlap, PlaceId, ReferenceTargetGroup, TargetIndex};

fn int(value: i128) -> ConstValue {
    ConstValue::Int(value.into())
}

fn item(index: TargetIndex) -> ReferenceTargetGroup {
    ReferenceTargetGroup::ItemRegion {
        base: Box::new(ReferenceTargetGroup::Local(PlaceId(0))),
        index,
    }
}

fn symbolic(expr: NormalExpr) -> TargetIndex {
    TargetIndex::Symbolic(expr)
}

fn range(start: NormalExpr, end: NormalExpr) -> ReferenceTargetGroup {
    ReferenceTargetGroup::ItemRange {
        base: Box::new(ReferenceTargetGroup::Local(PlaceId(0))),
        start: symbolic(start),
        end: symbolic(end),
    }
}

fn variant(name: &str) -> ReferenceTargetGroup {
    ReferenceTargetGroup::Variant {
        base: Box::new(ReferenceTargetGroup::Param(GroupId(0))),
        name: Intern::from_ref(name),
    }
}

#[test]
fn variant_projection_matches_parent() {
    assert!(variant("Some").matches_projection_from(
        &ReferenceTargetGroup::Param(GroupId(0)),
        &[GroupProjection::Variant {
            name: Intern::from_ref("Some"),
        }],
        &ConstraintEnv::default(),
    ));
}

#[test]
fn variant_field_projection_matches_parent() {
    let target = ReferenceTargetGroup::Field {
        base: Box::new(variant("Some")),
        index: 0,
    };

    assert!(target.matches_projection_from(
        &ReferenceTargetGroup::Param(GroupId(0)),
        &[
            GroupProjection::Variant {
                name: Intern::from_ref("Some"),
            },
            GroupProjection::Field {
                name: Intern::from_ref("value"),
                index: 0,
            },
        ],
        &ConstraintEnv::default(),
    ));
}

#[test]
fn different_variants_of_same_union_are_disjoint() {
    assert_eq!(
        variant("Some").overlap(&variant("None"), &ConstraintEnv::default()),
        Overlap::Disjoint
    );
}

#[test]
fn variant_partially_overlaps_union_parent() {
    assert_eq!(
        variant("Some").overlap(
            &ReferenceTargetGroup::Param(GroupId(0)),
            &ConstraintEnv::default()
        ),
        Overlap::Partial
    );
}

#[test]
fn variant_preserves_root_parameter() {
    assert_eq!(variant("Some").root_param(), Some(GroupId(0)));
}

#[test]
fn distinct_constant_indices_are_disjoint() {
    let left = item(symbolic(NormalExpr::Value(int(1))));
    let right = item(symbolic(NormalExpr::Value(int(2))));

    assert_eq!(
        left.overlap(&right, &ConstraintEnv::default()),
        Overlap::Disjoint
    );
}

#[test]
fn fields_beneath_distinct_items_are_disjoint() {
    let field = |index: i128| ReferenceTargetGroup::Field {
        base: Box::new(item(symbolic(NormalExpr::Value(ConstValue::Int(
            index.into(),
        ))))),
        index: 0,
    };

    assert_eq!(
        field(0).overlap(&field(1), &ConstraintEnv::default()),
        Overlap::Disjoint
    );
}

#[test]
fn same_symbolic_index_is_equal() {
    let index = symbolic(NormalExpr::Var(Intern::from_ref("i")));

    assert_eq!(
        item(index.clone()).overlap(&item(index), &ConstraintEnv::default()),
        Overlap::Equal
    );
}

#[test]
fn constrained_unequal_indices_are_disjoint() {
    let i = NormalExpr::Var(Intern::from_ref("i"));
    let j = NormalExpr::Var(Intern::from_ref("j"));
    let constraints = ConstraintEnv {
        known_lt: vec![(i.clone(), j.clone())],
        ..ConstraintEnv::default()
    };

    assert_eq!(
        item(symbolic(i)).overlap(&item(symbolic(j)), &constraints),
        Overlap::Disjoint
    );
}

#[test]
fn constrained_equal_indices_are_equal() {
    let i = NormalExpr::Var(Intern::from_ref("i"));
    let j = NormalExpr::Var(Intern::from_ref("j"));
    let constraints = ConstraintEnv {
        known_eq: vec![(i.clone(), j.clone())],
        ..ConstraintEnv::default()
    };

    assert_eq!(
        item(symbolic(i)).overlap(&item(symbolic(j)), &constraints),
        Overlap::Equal
    );
}

#[test]
fn unrelated_symbolic_indices_have_unknown_overlap() {
    let left = symbolic(NormalExpr::Var(Intern::from_ref("i")));
    let right = symbolic(NormalExpr::Var(Intern::from_ref("j")));

    assert_eq!(
        item(left).overlap(&item(right), &ConstraintEnv::default()),
        Overlap::Unknown
    );
}

#[test]
fn item_before_range_is_disjoint() {
    let index = NormalExpr::Value(int(1));
    let start = NormalExpr::Value(int(2));
    let end = NormalExpr::Value(int(5));

    assert_eq!(
        item(symbolic(index)).overlap(&range(start, end), &ConstraintEnv::default()),
        Overlap::Disjoint
    );
}

#[test]
fn item_inside_range_partially_overlaps() {
    let index = NormalExpr::Value(int(3));
    let start = NormalExpr::Value(int(2));
    let end = NormalExpr::Value(int(5));

    assert_eq!(
        item(symbolic(index)).overlap(&range(start, end), &ConstraintEnv::default()),
        Overlap::Partial
    );
}

#[test]
fn adjacent_ranges_are_disjoint() {
    let left = range(NormalExpr::Value(int(0)), NormalExpr::Value(int(2)));
    let right = range(NormalExpr::Value(int(2)), NormalExpr::Value(int(4)));

    assert_eq!(
        left.overlap(&right, &ConstraintEnv::default()),
        Overlap::Disjoint
    );
}

#[test]
fn equal_ranges_are_equal() {
    let left = range(NormalExpr::Value(int(1)), NormalExpr::Value(int(4)));
    let right = left.clone();

    assert_eq!(
        left.overlap(&right, &ConstraintEnv::default()),
        Overlap::Equal
    );
}

#[test]
fn intersecting_ranges_partially_overlap() {
    let left = range(NormalExpr::Value(int(1)), NormalExpr::Value(int(4)));
    let right = range(NormalExpr::Value(int(3)), NormalExpr::Value(int(6)));

    assert_eq!(
        left.overlap(&right, &ConstraintEnv::default()),
        Overlap::Partial
    );
}

#[test]
fn unavailable_indices_have_unknown_overlap() {
    assert_eq!(
        item(TargetIndex::Unknown).overlap(&item(TargetIndex::Unknown), &ConstraintEnv::default()),
        Overlap::Unknown
    );
}
