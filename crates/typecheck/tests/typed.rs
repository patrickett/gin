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

fn field(base: ReferenceTargetGroup, index: usize) -> ReferenceTargetGroup {
    ReferenceTargetGroup::Field {
        base: Box::new(base),
        index,
    }
}

fn expect_symmetric_overlap(
    left: &ReferenceTargetGroup,
    right: &ReferenceTargetGroup,
    expected: Overlap,
) {
    let constraints = ConstraintEnv::default();
    assert_eq!(left.overlap(right, &constraints), expected);
    assert_eq!(right.overlap(left, &constraints), expected);
}

#[test]
fn nested_field_is_disjoint_from_a_sibling_of_its_parent() {
    let root = ReferenceTargetGroup::Local(PlaceId(0));
    let nested = field(field(root.clone(), 0), 0);
    let sibling = field(root, 2);
    expect_symmetric_overlap(&nested, &sibling, Overlap::Disjoint);
}

#[test]
fn deeply_nested_field_is_disjoint_from_a_shallower_branch() {
    let root = ReferenceTargetGroup::Param(GroupId(0));
    let nested = field(field(field(root.clone(), 0), 1), 2);
    let sibling = field(field(root, 1), 2);
    expect_symmetric_overlap(&nested, &sibling, Overlap::Disjoint);
}

#[test]
fn nested_fields_overlap_their_ancestors() {
    let root = ReferenceTargetGroup::Local(PlaceId(0));
    let parent = field(root.clone(), 0);
    let nested = field(field(parent.clone(), 1), 2);
    expect_symmetric_overlap(&nested, &parent, Overlap::Partial);
    expect_symmetric_overlap(&nested, &root, Overlap::Partial);
    expect_symmetric_overlap(&nested, &nested, Overlap::Equal);
}

#[test]
fn nested_item_is_disjoint_from_a_different_shallower_item() {
    let first = item(symbolic(NormalExpr::Value(int(0))));
    let nested = ReferenceTargetGroup::ItemRegion {
        base: Box::new(first.clone()),
        index: symbolic(NormalExpr::Value(int(1))),
    };
    let second = item(symbolic(NormalExpr::Value(int(2))));
    expect_symmetric_overlap(&nested, &second, Overlap::Disjoint);
    expect_symmetric_overlap(&nested, &first, Overlap::Partial);
}

#[test]
fn nested_variant_is_disjoint_from_a_different_shallower_variant() {
    let parent = variant("Some");
    let nested = ReferenceTargetGroup::Variant {
        base: Box::new(parent.clone()),
        name: Intern::from_ref("Some"),
    };
    expect_symmetric_overlap(&nested, &variant("None"), Overlap::Disjoint);
    expect_symmetric_overlap(&nested, &parent, Overlap::Partial);
}

#[test]
fn nested_ranges_preserve_disjointness_and_partial_overlap() {
    let parent = range(NormalExpr::Value(int(0)), NormalExpr::Value(int(4)));
    let nested = ReferenceTargetGroup::ItemRange {
        base: Box::new(parent.clone()),
        start: symbolic(NormalExpr::Value(int(0))),
        end: symbolic(NormalExpr::Value(int(2))),
    };
    let sibling = range(NormalExpr::Value(int(4)), NormalExpr::Value(int(8)));
    expect_symmetric_overlap(&nested, &sibling, Overlap::Disjoint);
    expect_symmetric_overlap(&nested, &parent, Overlap::Partial);
}

#[test]
fn nested_unknown_item_retains_unknown_overlap_with_shallower_item() {
    let nested = ReferenceTargetGroup::ItemRegion {
        base: Box::new(item(TargetIndex::Unknown)),
        index: symbolic(NormalExpr::Value(int(0))),
    };
    expect_symmetric_overlap(
        &nested,
        &item(symbolic(NormalExpr::Value(int(1)))),
        Overlap::Unknown,
    );
}

#[test]
fn nested_dereference_is_disjoint_from_sibling_pointer_storage() {
    let root = ReferenceTargetGroup::Local(PlaceId(0));
    let pointer = ReferenceTargetGroup::Deref(Box::new(field(root.clone(), 0)));
    let nested = ReferenceTargetGroup::Deref(Box::new(pointer.clone()));
    let sibling = ReferenceTargetGroup::Deref(Box::new(field(root, 1)));
    expect_symmetric_overlap(&nested, &sibling, Overlap::Disjoint);
    expect_symmetric_overlap(&nested, &pointer, Overlap::Partial);
}
