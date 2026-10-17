use internment::Intern;
use typecheck::ConstraintEnv;
use typecheck::typed::{
    GroupId, GroupProjection, Overlap, PlaceId, ReferenceTargetGroup as Target, TargetIndex,
};

fn field(base: Target, index: usize) -> Target {
    Target::Field {
        base: Box::new(base),
        index,
    }
}
fn item(base: Target, index: i128) -> Target {
    Target::ItemRegion {
        base: Box::new(base),
        index: TargetIndex::Symbolic(index.into()),
    }
}
fn range(base: Target, start: i128, end: i128) -> Target {
    Target::ItemRange {
        base: Box::new(base),
        start: TargetIndex::Symbolic(start.into()),
        end: TargetIndex::Symbolic(end.into()),
    }
}

#[test]
fn overlap_preserves_projection_relationships() {
    let root = Target::Local(PlaceId(0));
    let other = Target::Local(PlaceId(1));
    let variant = |name| Target::Variant {
        base: Box::new(root.clone()),
        name: Intern::from_ref(name),
    };
    let cases = vec![
        (root.clone(), root.clone(), Overlap::Equal),
        (root.clone(), field(root.clone(), 0), Overlap::Partial),
        (
            field(root.clone(), 0),
            field(root.clone(), 1),
            Overlap::Disjoint,
        ),
        (field(root.clone(), 0), field(other, 0), Overlap::Disjoint),
        (variant("First"), variant("Second"), Overlap::Disjoint),
        (
            Target::Deref(Box::new(root.clone())),
            field(root.clone(), 0),
            Overlap::Partial,
        ),
        (
            item(root.clone(), 0),
            item(root.clone(), 1),
            Overlap::Disjoint,
        ),
        (
            item(root.clone(), 1),
            range(root.clone(), 0, 2),
            Overlap::Partial,
        ),
        (
            item(root.clone(), 2),
            range(root.clone(), 0, 2),
            Overlap::Disjoint,
        ),
        (
            range(root.clone(), 0, 2),
            range(root.clone(), 0, 2),
            Overlap::Equal,
        ),
        (
            range(root.clone(), 0, 2),
            range(root.clone(), 2, 4),
            Overlap::Disjoint,
        ),
        (
            range(root.clone(), 0, 3),
            range(root.clone(), 2, 4),
            Overlap::Partial,
        ),
        (
            Target::ItemRegion {
                base: Box::new(root.clone()),
                index: TargetIndex::Unknown,
            },
            item(root, 1),
            Overlap::Unknown,
        ),
    ];
    let constraints = ConstraintEnv::default();
    for (left, right, expected) in cases {
        assert_eq!(left.overlap(&right, &constraints), expected);
        assert_eq!(right.overlap(&left, &constraints), expected);
        if matches!((&left, &right), (Target::Local(_), Target::Field { .. })) {
            continue;
        }
        assert_eq!(
            field(left.clone(), 7).overlap(&field(right.clone(), 7), &constraints),
            expected
        );
        assert_eq!(
            Target::Deref(Box::new(left.clone()))
                .overlap(&Target::Deref(Box::new(right.clone())), &constraints),
            expected
        );
        let different = if expected == Overlap::Equal {
            Overlap::Disjoint
        } else {
            expected
        };
        assert_eq!(
            field(left, 7).overlap(&field(right, 8), &constraints),
            different
        );
    }
}

#[test]
fn unequal_depth_overlap_compares_aligned_ancestors() {
    let root = Target::Local(PlaceId(0));
    let deep = field(field(root.clone(), 0), 1);
    let constraints = ConstraintEnv::default();
    for (ancestor, expected) in [
        (root.clone(), Overlap::Partial),
        (field(root.clone(), 0), Overlap::Partial),
        (field(root.clone(), 1), Overlap::Disjoint),
        (Target::Local(PlaceId(1)), Overlap::Disjoint),
    ] {
        assert_eq!(deep.overlap(&ancestor, &constraints), expected);
        assert_eq!(ancestor.overlap(&deep, &constraints), expected);
    }
    let unknown = Target::ItemRegion {
        base: Box::new(root.clone()),
        index: TargetIndex::Unknown,
    };
    let projected = field(item(root, 0), 1);
    assert_eq!(unknown.overlap(&projected, &constraints), Overlap::Unknown);
    assert_eq!(projected.overlap(&unknown, &constraints), Overlap::Unknown);
}

#[test]
fn projection_matching_requires_the_entire_path_and_equal_ancestor() {
    let root = Target::Local(PlaceId(0));
    let constraints = ConstraintEnv::default();
    let variant_name = Intern::from_ref("Some");
    let target = field(
        Target::Deref(Box::new(Target::Variant {
            base: Box::new(item(root.clone(), 0)),
            name: variant_name,
        })),
        3,
    );
    let projections = vec![
        GroupProjection::Items,
        GroupProjection::Variant { name: variant_name },
        GroupProjection::Pointee,
        GroupProjection::Field {
            name: Intern::from_ref("value"),
            index: 3,
        },
    ];
    assert!(target.matches_projection_from(&root, &projections, &constraints));
    assert!(!target.matches_projection_from(
        &Target::Local(PlaceId(1)),
        &projections,
        &constraints
    ));
    assert!(!target.matches_projection_from(&root, &projections[1..], &constraints));
    for replacement in [
        GroupProjection::Field {
            name: Intern::from_ref("value"),
            index: 4,
        },
        GroupProjection::Pointee,
        GroupProjection::Items,
        GroupProjection::Variant { name: variant_name },
    ] {
        let mut mismatched = projections.clone();
        *mismatched.last_mut().unwrap() = replacement;
        assert!(!target.matches_projection_from(&root, &mismatched, &constraints));
    }
    assert!(range(root.clone(), 0, 2).matches_projection_from(
        &root,
        &[GroupProjection::Items],
        &constraints
    ));
    assert!(!item(root.clone(), 0).matches_projection_from(&root, &[], &constraints));
}

fn deep_check(case: &str, test: &str) {
    const CHILD: &str = "GIN_REFERENCE_TARGET_PATH_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD, case)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let mut local = Target::Local(PlaceId(0));
    let mut param = Target::Param(GroupId(0));
    let mut projections = Vec::new();
    for index in 0..2000 {
        local = field(local, index);
        param = field(param, index);
        projections.push(GroupProjection::Field {
            name: Intern::from_ref("field"),
            index,
        });
    }
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn_scoped(scope, || {
                let constraints = ConstraintEnv::default();
                match case {
                    "roots" => {
                        assert_eq!(local.root_local(), Some(PlaceId(0)));
                        assert_eq!(local.root_param(), None);
                        assert_eq!(param.root_param(), Some(GroupId(0)));
                        assert_eq!(param.root_local(), None);
                    }
                    "overlap" => {
                        assert_eq!(local.overlap(&local, &constraints), Overlap::Equal);
                        assert_eq!(local.overlap(&param, &constraints), Overlap::Disjoint);
                        assert_eq!(
                            local.overlap(&Target::Local(PlaceId(0)), &constraints),
                            Overlap::Partial
                        );
                    }
                    "projections" => assert!(local.matches_projection_from(
                        &Target::Local(PlaceId(0)),
                        &projections,
                        &constraints
                    )),
                    _ => panic!("unknown case"),
                }
            })
            .unwrap()
            .join()
            .unwrap();
    });
}

#[test]
fn roots_handle_deep_paths_on_small_stack() {
    deep_check("roots", "roots_handle_deep_paths_on_small_stack");
}
#[test]
fn overlap_handles_deep_paths_on_small_stack() {
    deep_check("overlap", "overlap_handles_deep_paths_on_small_stack");
}
#[test]
fn projection_matching_handles_deep_paths_on_small_stack() {
    deep_check(
        "projections",
        "projection_matching_handles_deep_paths_on_small_stack",
    );
}
