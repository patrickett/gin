use super::integer_cases::{Domain, Integer, domains};
use std::cmp::Ordering;
use std::collections::HashSet;

use ast::ConstValue;
use ginc::cli::Profile;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, FileId};

#[derive(Clone, Copy, Debug)]
enum Comparison {
    Less,
    LessOrEqual,
    Equal,
    NotEqual,
    GreaterOrEqual,
    Greater,
}

impl Comparison {
    const ALL: [Self; 6] = [
        Self::Less,
        Self::LessOrEqual,
        Self::Equal,
        Self::NotEqual,
        Self::GreaterOrEqual,
        Self::Greater,
    ];

    fn value_expression(self, left: &str, right: &str) -> String {
        let operator = match self {
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Equal => "=",
            Self::NotEqual => unreachable!("inequality is expressed as a negated condition"),
            Self::GreaterOrEqual => ">=",
            Self::Greater => ">",
        };
        format!("{left} {operator} {right}")
    }

    fn expression(self, left: &str, right: &str) -> String {
        if matches!(self, Self::NotEqual) {
            format!("not ({left} = {right})")
        } else {
            self.value_expression(left, right)
        }
    }

    fn expected(self, left: Integer, right: Integer) -> bool {
        let order = left.compare(right);
        match self {
            Self::Less => order == Ordering::Less,
            Self::LessOrEqual => order != Ordering::Greater,
            Self::Equal => order == Ordering::Equal,
            Self::NotEqual => order != Ordering::Equal,
            Self::GreaterOrEqual => order != Ordering::Less,
            Self::Greater => order == Ordering::Greater,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Case {
    left_domain: Domain,
    right_domain: Domain,
    left: Integer,
    right: Integer,
}

fn cases_for(left_domain: Domain) -> Vec<Case> {
    let mut cases = Vec::new();
    for right_domain in domains() {
        for left in left_domain.boundary_values() {
            for right in right_domain.boundary_values() {
                cases.push(Case {
                    left_domain,
                    right_domain,
                    left,
                    right,
                });
            }
        }
    }
    cases
}

fn constant_source(cases: &[Case]) -> String {
    let mut source = String::new();
    for (index, case) in cases.iter().enumerate() {
        source.push_str(&format!(
            "left_{index} is in {} := {}\nright_{index} is in {} := {}\n",
            case.left_domain, case.left, case.right_domain, case.right,
        ));
        for (operator, comparison) in Comparison::ALL.into_iter().enumerate() {
            if matches!(comparison, Comparison::NotEqual) {
                continue;
            }
            let expression =
                comparison.value_expression(&format!("left_{index}"), &format!("right_{index}"));
            source.push_str(&format!("result_{index}_{operator} := {expression}\n"));
        }
    }
    source
}

fn native_source(cases: &[Case], comparisons: &[Comparison]) -> String {
    let mut source = String::new();
    let mut seen = HashSet::new();
    let function_name = |case: &Case| {
        format!(
            "compare_{}_{}_{}_{}",
            case.left_domain.width,
            u8::from(case.left_domain.signed),
            case.right_domain.width,
            u8::from(case.right_domain.signed)
        )
    };
    for case in cases {
        if !seen.insert((case.left_domain, case.right_domain)) {
            continue;
        }
        source.push_str(&format!(
            "{}(left is in {}, right is in {}) is in 0 to 255:\n",
            function_name(case),
            case.left_domain,
            case.right_domain,
        ));
        for (operator, comparison) in comparisons.iter().enumerate() {
            source.push_str(&format!(
                "    flag_{operator} is in 0 to 255: when {} then {} else 0\n",
                comparison.expression("left", "right"),
                1 << operator,
            ));
        }
        let flags = (0..comparisons.len())
            .map(|operator| format!("flag_{operator}"))
            .collect::<Vec<_>>()
            .join(" + ");
        source.push_str(&format!("    return ({flags}) & 255\n"));
    }
    source.push_str("#entry\nmain:\n");
    for case in cases {
        let expected: u8 =
            comparisons
                .iter()
                .enumerate()
                .fold(0, |bits, (operator, comparison)| {
                    bits | (u8::from(comparison.expected(case.left, case.right)) << operator)
                });
        source.push_str(&format!(
            "    when not ({}({}, {}) = {expected}) then\n        return 1\n",
            function_name(case),
            case.left,
            case.right,
        ));
    }
    source.push_str("    return 0\n");
    source
}

#[test]
fn generated_constant_comparisons_match_independent_reference() {
    let mut comparisons_checked = 0;
    for domain in domains() {
        let cases = cases_for(domain);
        let source = constant_source(&cases);
        let parsed = TokenCursor::parse_source(&source);
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        assert!(
            typed.all_flaws().is_empty(),
            "{domain}: {:?}",
            typed.all_flaws()
        );
        for definition in typed.defs.values() {
            let name = definition.name.as_str();
            let Some(indices) = name.strip_prefix("result_") else {
                continue;
            };
            let (case_index, operator_index) = indices.split_once('_').unwrap();
            let case = cases[case_index.parse::<usize>().unwrap()];
            let comparison = Comparison::ALL[operator_index.parse::<usize>().unwrap()];
            let BindBody::Expr(body) = definition.body else {
                panic!("expected a constant expression: {name}");
            };
            let constant = &typed.exprs.facts.const_facts.const_value[body.as_usize()];
            let Some(ConstValue::Proposition(value)) = constant else {
                panic!("expected a constant comparison: {case:?}, {comparison:?}: {constant:?}");
            };
            assert_eq!(
                *value,
                comparison.expected(case.left, case.right),
                "{case:?}, {comparison:?}"
            );
            comparisons_checked += 1;
        }
    }
    assert_eq!(comparisons_checked, 28_125);
}

#[test]
fn generated_native_comparisons_match_independent_reference() {
    let mut comparisons_checked = 0;
    let failure = |cases: &[Case], comparisons: &[Comparison], profile| {
        let source = native_source(cases, comparisons);
        match super::try_run_program(&source, "integer_comparison_matrix", profile) {
            Ok(output) if output.status.success() => None,
            Ok(output) => Some(format!("{output:?}")),
            Err(error) => Some(error),
        }
    };
    for domain in domains() {
        let cases = cases_for(domain);
        for batch in cases.chunks(128) {
            for profile in [Profile::Debug, Profile::Release] {
                if failure(batch, &Comparison::ALL, profile).is_none() {
                    comparisons_checked += batch.len() * Comparison::ALL.len();
                    continue;
                }
                let mut failing = batch;
                while failing.len() > 1 {
                    let middle = failing.len() / 2;
                    let candidate = &failing[..middle];
                    failing = if failure(candidate, &Comparison::ALL, profile).is_none() {
                        &failing[middle..]
                    } else {
                        candidate
                    };
                }
                for comparison in Comparison::ALL {
                    if let Some(error) = failure(failing, &[comparison], profile) {
                        let source = native_source(failing, &[comparison]);
                        panic!(
                            "{profile:?}, {:?}, {comparison:?}: {error}\n{source}",
                            failing[0]
                        );
                    }
                }
                panic!(
                    "comparison batch failed, but the isolated case passed: {profile:?}, {:?}\n{}",
                    failing[0],
                    native_source(batch, &Comparison::ALL)
                );
            }
        }
    }
    assert_eq!(comparisons_checked, 67_500);
}
