use internment::Intern;
use typecheck::typed::{
    EffectTarget as Effect, GroupId, PlaceId, ReferenceTargetGroup as Target, TargetIndex,
};

fn field(base: Target, index: usize) -> Target {
    Target::Field {
        base: Box::new(base),
        index,
    }
}

#[test]
fn conversion_and_substitution_preserve_every_projection() {
    let project = |root| {
        field(
            Target::Deref(Box::new(Target::ItemRange {
                base: Box::new(Target::ItemRegion {
                    base: Box::new(Target::Variant {
                        base: Box::new(root),
                        name: Intern::from_ref("Some"),
                    }),
                    index: TargetIndex::Unknown,
                }),
                start: TargetIndex::Symbolic(1.into()),
                end: TargetIndex::Symbolic(3.into()),
            })),
            7,
        )
    };
    let effect = Effect::from_reference(&project(Target::Param(GroupId(2)))).unwrap();
    let actual = field(Target::Local(PlaceId(5)), 4);
    let applied = effect.substitute(GroupId(2), &actual).unwrap();
    assert_eq!(applied.target, project(actual));
    assert!(!applied.descendants);
    assert!(
        effect
            .substitute(GroupId(1), &Target::Local(PlaceId(5)))
            .is_none()
    );
    assert!(Effect::from_reference(&project(Target::Local(PlaceId(5)))).is_none());
}

#[test]
fn descendants_survive_projections_and_repeated_markers() {
    let root = Effect::Group(GroupId(0));
    let projected = Effect::Field {
        base: Box::new(Effect::Descendants(Box::new(root))),
        index: 3,
    };
    let nested = Effect::Descendants(Box::new(Effect::Descendants(Box::new(projected))));
    let actual = Target::Local(PlaceId(0));
    let applied = nested.substitute(GroupId(0), &actual).unwrap();
    assert_eq!(applied.target, field(actual, 3));
    assert!(applied.descendants);
    assert!(
        nested
            .substitute(GroupId(1), &Target::Local(PlaceId(0)))
            .is_none()
    );
}

fn deep_check(case: &str, test: &str) {
    const CHILD: &str = "GIN_EFFECT_TARGET_PATH_CHILD";
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
    let mut param = Target::Param(GroupId(0));
    let mut local = Target::Local(PlaceId(0));
    let mut effect = Effect::Group(GroupId(0));
    for index in 0..2000 {
        param = field(param, index);
        local = field(local, index);
        effect = Effect::Field {
            base: Box::new(effect),
            index,
        };
    }
    let (converted, substituted) = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn_scoped(scope, || match case {
                "convert" => (Effect::from_reference(&param), None),
                "local" => {
                    assert!(Effect::from_reference(&local).is_none());
                    (None, None)
                }
                "substitute" => (
                    None,
                    effect.substitute(GroupId(0), &Target::Local(PlaceId(0))),
                ),
                "mismatch" => {
                    assert!(
                        effect
                            .substitute(GroupId(1), &Target::Local(PlaceId(0)))
                            .is_none()
                    );
                    (None, None)
                }
                _ => panic!("unknown case"),
            })
            .unwrap()
            .join()
            .unwrap()
    });
    match case {
        "convert" => {
            let mut current = converted.as_ref().unwrap();
            for index in (0..2000).rev() {
                let Effect::Field {
                    base,
                    index: actual,
                } = current
                else {
                    panic!("missing field")
                };
                assert_eq!(*actual, index);
                current = base;
            }
            assert_eq!(*current, Effect::Group(GroupId(0)));
        }
        "substitute" => {
            let applied = substituted.as_ref().unwrap();
            assert!(!applied.descendants);
            let mut current = &applied.target;
            for index in (0..2000).rev() {
                let Target::Field {
                    base,
                    index: actual,
                } = current
                else {
                    panic!("missing field")
                };
                assert_eq!(*actual, index);
                current = base;
            }
            assert_eq!(*current, Target::Local(PlaceId(0)));
        }
        _ => {}
    }
}

#[test]
fn conversion_handles_deep_paths_on_small_stack() {
    deep_check("convert", "conversion_handles_deep_paths_on_small_stack");
}

#[test]
fn substitution_handles_deep_paths_on_small_stack() {
    deep_check(
        "substitute",
        "substitution_handles_deep_paths_on_small_stack",
    );
}

#[test]
fn mismatched_substitution_handles_deep_paths_on_small_stack() {
    deep_check(
        "mismatch",
        "mismatched_substitution_handles_deep_paths_on_small_stack",
    );
}

#[test]
fn local_conversion_handles_deep_paths_on_small_stack() {
    deep_check(
        "local",
        "local_conversion_handles_deep_paths_on_small_stack",
    );
}
