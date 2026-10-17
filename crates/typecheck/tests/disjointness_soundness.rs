use std::collections::HashSet;

use internment::Intern;
use typecheck::ConstraintEnv;
use typecheck::typed::{
    GroupId, Overlap, PlaceId, ReferenceTargetGroup as Target, ReferenceTargetSet, TargetIndex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coordinate {
    Field(usize),
    Variant(&'static str),
    Item(i128),
    Pointee,
}

#[derive(Clone, Copy)]
enum Selection {
    Field(usize),
    Variant(&'static str),
    Item(Option<i128>),
    Range(Option<i128>, Option<i128>),
    Deref,
}

impl Selection {
    fn apply(self, base: Target) -> Target {
        let base = Box::new(base);
        match self {
            Self::Field(index) => Target::Field { base, index },
            Self::Variant(name) => Target::Variant {
                base,
                name: Intern::from_ref(name),
            },
            Self::Item(value) => Target::ItemRegion {
                base,
                index: value.map_or(TargetIndex::Unknown, |value| {
                    TargetIndex::Symbolic(value.into())
                }),
            },
            Self::Range(start, end) => Target::ItemRange {
                base,
                start: start.map_or(TargetIndex::Unknown, |value| {
                    TargetIndex::Symbolic(value.into())
                }),
                end: end.map_or(TargetIndex::Unknown, |value| {
                    TargetIndex::Symbolic(value.into())
                }),
            },
            Self::Deref => Target::Deref(base),
        }
    }

    fn contains(self, coordinate: Coordinate) -> bool {
        match (self, coordinate) {
            (Self::Field(expected), Coordinate::Field(actual)) => expected == actual,
            (Self::Variant(expected), Coordinate::Variant(actual)) => expected == actual,
            (Self::Item(expected), Coordinate::Item(actual)) => {
                expected.is_none_or(|expected| expected == actual)
            }
            (Self::Range(start, end), Coordinate::Item(actual)) => {
                start.is_none_or(|start| start <= actual) && end.is_none_or(|end| actual < end)
            }
            (Self::Deref, Coordinate::Pointee) => true,
            _ => false,
        }
    }
}

struct Sample {
    target: Target,
    root: usize,
    selections: Vec<Selection>,
}

fn samples() -> Vec<Sample> {
    let selections = [
        Selection::Field(0),
        Selection::Field(1),
        Selection::Variant("Left"),
        Selection::Variant("Right"),
        Selection::Item(None),
        Selection::Item(Some(-1)),
        Selection::Item(Some(0)),
        Selection::Item(Some(1)),
        Selection::Item(Some(i128::MIN)),
        Selection::Item(Some(i128::MAX)),
        Selection::Range(Some(-1), Some(1)),
        Selection::Range(Some(1), Some(2)),
        Selection::Range(Some(1), Some(1)),
        Selection::Range(Some(2), Some(1)),
        Selection::Range(None, Some(1)),
        Selection::Range(Some(0), None),
        Selection::Range(None, None),
        Selection::Deref,
    ];
    let roots = [
        Target::Static,
        Target::Local(PlaceId(0)),
        Target::Local(PlaceId(1)),
        Target::Param(GroupId(0)),
    ];
    let mut samples = Vec::new();
    for (root, target) in roots.into_iter().enumerate() {
        samples.push(Sample {
            target: target.clone(),
            root,
            selections: Vec::new(),
        });
        for selection in selections {
            let projected = selection.apply(target.clone());
            samples.push(Sample {
                target: projected.clone(),
                root,
                selections: vec![selection],
            });
            if root == 1 {
                for child in [Selection::Field(0), Selection::Deref] {
                    samples.push(Sample {
                        target: child.apply(projected.clone()),
                        root,
                        selections: vec![selection, child],
                    });
                }
            }
        }
    }
    samples
}

fn locations() -> Vec<(usize, Vec<Coordinate>)> {
    let coordinates = [
        Coordinate::Field(0),
        Coordinate::Field(1),
        Coordinate::Variant("Left"),
        Coordinate::Variant("Right"),
        Coordinate::Item(-1),
        Coordinate::Item(0),
        Coordinate::Item(1),
        Coordinate::Item(2),
        Coordinate::Item(i128::MIN),
        Coordinate::Item(i128::MAX),
        Coordinate::Pointee,
    ];
    let mut locations = Vec::new();
    for root in 0..4 {
        locations.push((root, Vec::new()));
        for first in coordinates {
            locations.push((root, vec![first]));
            for second in coordinates {
                locations.push((root, vec![first, second]));
            }
        }
    }
    locations
}

#[test]
fn disjointness_and_equality_match_enumerated_locations() {
    let samples = samples();
    let locations = locations();
    let footprints: Vec<HashSet<usize>> = samples
        .iter()
        .map(|sample| {
            locations
                .iter()
                .enumerate()
                .filter_map(|(index, (root, coordinates))| {
                    (*root == sample.root
                        && coordinates.len() >= sample.selections.len()
                        && sample
                            .selections
                            .iter()
                            .zip(coordinates)
                            .all(|(selection, coordinate)| selection.contains(*coordinate)))
                    .then_some(index)
                })
                .collect()
        })
        .collect();
    assert!(footprints[0].len() > 1);
    assert!(footprints.iter().any(HashSet::is_empty));
    let constraints = ConstraintEnv::default();
    for (left_index, left) in samples.iter().enumerate() {
        for (right_index, right) in samples.iter().enumerate() {
            let relation = left.target.overlap(&right.target, &constraints);
            assert_eq!(relation, right.target.overlap(&left.target, &constraints));
            match relation {
                Overlap::Disjoint => assert!(
                    footprints[left_index].is_disjoint(&footprints[right_index]),
                    "false separation: {:?}, {:?}",
                    left.target,
                    right.target,
                ),
                Overlap::Equal => assert_eq!(footprints[left_index], footprints[right_index]),
                Overlap::Partial | Overlap::Unknown => {}
            }
        }
    }
}

#[test]
fn target_sets_preserve_pairwise_results_including_empty_sets() {
    let samples = samples();
    let constraints = ConstraintEnv::default();
    let sets: Vec<_> = samples
        .chunks(5)
        .map(|chunk| {
            let mut set = ReferenceTargetSet::default();
            for sample in chunk {
                set.extend(&ReferenceTargetSet::singleton(sample.target.clone()));
            }
            set
        })
        .chain([ReferenceTargetSet::default()])
        .collect();
    for left in &sets {
        for right in &sets {
            let expected = left.iter().all(|left| {
                right
                    .iter()
                    .all(|right| left.overlap(right, &constraints) == Overlap::Disjoint)
            });
            assert_eq!(left.is_disjoint(right, &constraints), expected);
            assert_eq!(right.is_disjoint(left, &constraints), expected);
        }
    }
}

#[test]
fn symbolic_indices_keep_their_meaning_across_paths_and_target_sets() {
    let left_index = ast::NormalExpr::Var(Intern::from_ref("left"));
    let right_index = ast::NormalExpr::Var(Intern::from_ref("right"));
    let item = |index: ast::NormalExpr| Target::ItemRegion {
        base: Box::new(Target::Local(PlaceId(0))),
        index: TargetIndex::Symbolic(index),
    };
    let left = item(left_index.clone());
    let right = item(right_index.clone());
    assert_eq!(
        left.overlap(&right, &ConstraintEnv::default()),
        Overlap::Unknown
    );
    let mut constraints = ConstraintEnv::default();
    constraints.known_lt.push((left_index, right_index));
    assert_eq!(left.overlap(&right, &constraints), Overlap::Disjoint);
    let left_set = ReferenceTargetSet::from([left.clone(), Selection::Field(0).apply(left)]);
    let right_set = ReferenceTargetSet::from([right.clone(), Selection::Field(1).apply(right)]);
    assert!(left_set.is_disjoint(&right_set, &constraints));
    assert!(right_set.is_disjoint(&left_set, &constraints));
    assert!(!left_set.is_disjoint(&right_set, &ConstraintEnv::default()));
}
