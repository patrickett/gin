use super::integer_cases::{Domain, Integer, domains};
use ast::ConstValue;
use ginc::cli::Profile;
use parser::cursor::TokenCursor;
use std::collections::HashSet;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, FileId};

mod narrowing;

#[derive(Clone, Copy, Debug)]
struct Case {
    source: Domain,
    target: Domain,
    value: Integer,
}

impl Case {
    fn symbol(self) -> String {
        format!("{}{}", self.source.symbol(), self.target.symbol())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Local,
    Parameter,
    Return,
    FieldRead,
    FieldWrite,
    AnonymousCast,
    NamedCast,
}

impl Route {
    const ALL: [Self; 7] = [
        Self::Local,
        Self::Parameter,
        Self::Return,
        Self::FieldRead,
        Self::FieldWrite,
        Self::AnonymousCast,
        Self::NamedCast,
    ];

    fn symbol(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Parameter => "parameter",
            Self::Return => "returned",
            Self::FieldRead => "field_read",
            Self::FieldWrite => "field_write",
            Self::AnonymousCast => "anonymous_cast",
            Self::NamedCast => "named_cast",
        }
    }

    fn definition(self, case: Case) -> String {
        let id = case.symbol();
        let body = match self {
            Self::Local => format!(
                "\n    converted is in {}: value\n    return converted",
                case.target
            ),
            Self::Parameter => format!(": return identity_{id}(value)"),
            Self::Return => ": return value".to_string(),
            Self::FieldRead => {
                format!(": return read_{id}(SourceBox{id}(number: value as Source{id}))")
            }
            Self::FieldWrite => format!(
                "\n    box: TargetBox{id}(number: value as Target{id})\n    raw is in {}: box.number\n    return raw",
                case.target
            ),
            Self::AnonymousCast => format!(
                "\n    converted Target{id}: value as Target{id}\n    raw is in {}: converted\n    return raw",
                case.target
            ),
            Self::NamedCast => format!(
                "\n    named Source{id}: value as Source{id}\n    converted Target{id}: named as Target{id}\n    raw is in {}: converted\n    return raw",
                case.target
            ),
        };
        let separator = if body.starts_with(':') { "" } else { ":" };
        format!(
            "{}_{id}(value is in {}) is in {}{separator}{body}\n",
            self.symbol(),
            case.source,
            case.target,
        )
    }
}

fn cases_for(source: Domain) -> Vec<Case> {
    domains()
        .filter(|target| target.width >= source.width && target.contains_domain(source))
        .flat_map(|target| {
            source.boundary_values().into_iter().map(move |value| Case {
                source,
                target,
                value,
            })
        })
        .collect()
}

fn declarations(case: Case) -> String {
    let id = case.symbol();
    format!(
        "Source{id} is in {}\nSource{id} has\n    finish(self):\n        return\nTarget{id} is in {}\nTarget{id} has\n    finish(self):\n        return\nSourceBox{id} has\n    number Source{id}\n    finish(self):\n        return\nTargetBox{id} has\n    number Target{id}\n    finish(self):\n        return\n",
        case.source, case.target,
    )
}

fn native_source(cases: &[Case], routes: &[Route]) -> String {
    let mut source = String::new();
    let mut seen = HashSet::new();
    for case in cases {
        let id = case.symbol();
        if !seen.insert(id.clone()) {
            continue;
        }
        source.push_str(&declarations(*case));
        if routes.contains(&Route::Parameter) {
            source.push_str(&format!(
                "identity_{id}(input is in {}) is in {}: return input\n",
                case.target, case.target,
            ));
        }
        if routes.contains(&Route::FieldRead) {
            source.push_str(&format!(
                "read_{id}(box SourceBox{id}) is in {}:\n    raw is in {}: box.number\n    return raw\n",
                case.target,
                case.target,
            ));
        }
        for route in routes {
            source.push_str(&route.definition(*case));
        }
    }
    source.push_str("#entry\nmain:\n");
    for case in cases {
        for route in routes {
            source.push_str(&format!(
                "    when not ({}_{}({}) = {}) then\n        return 1\n",
                route.symbol(),
                case.symbol(),
                case.value,
                case.value,
            ));
        }
    }
    source.push_str("    return 0\n");
    source
}

fn constant_source(cases: &[Case]) -> String {
    let mut source = String::new();
    let mut seen = HashSet::new();
    for (index, case) in cases.iter().enumerate() {
        let id = case.symbol();
        if seen.insert(id.clone()) {
            source.push_str(&declarations(*case));
        }
        source.push_str(&format!(
            "source_{index} is in {} := {}\nlocal_{index} is in {} := source_{index}\nanonymous_cast_{index} := source_{index} as Target{id}\nnamed_{index} := source_{index} as Source{id}\nnamed_cast_{index} := named_{index} as Target{id}\n",
            case.source, case.value, case.target,
        ));
    }
    source
}

#[test]
fn generated_constant_widening_preserves_numeric_values() {
    let mut checked = 0;
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
            let Some(index) = ["local_", "anonymous_cast_", "named_cast_"]
                .into_iter()
                .find_map(|prefix| name.strip_prefix(prefix))
            else {
                continue;
            };
            let case = cases[index.parse::<usize>().unwrap()];
            let BindBody::Expr(body) = definition.body else {
                panic!("expected a constant conversion: {name}");
            };
            let target = flask::CompileTarget::Library;
            let evaluated = typecheck::Evaluator::new(
                typed.resolved_program(),
                typecheck::CompileTimeHost::new(&target),
            )
            .eval(body)
            .unwrap_or_else(|error| panic!("{case:?}, {name}: {error:?}"));
            let typecheck::EvalOutcome::Complete(ConstValue::Int(actual)) = evaluated else {
                panic!("expected an integer constant: {case:?}, {name}: {evaluated:?}");
            };
            assert_eq!(
                actual.to_string(),
                case.value.to_string(),
                "{case:?}, {name}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 915);
}

#[test]
fn generated_native_widening_preserves_numeric_values() {
    let mut checked = 0;
    let failure = |cases: &[Case], routes: &[Route], profile| {
        let source = native_source(cases, routes);
        match super::try_run_program(&source, "integer_widening_matrix", profile) {
            Ok(output) if output.status.success() => None,
            Ok(output) => Some(format!("{output:?}")),
            Err(error) => Some(error),
        }
    };
    for domain in domains() {
        let cases = cases_for(domain);
        for profile in [Profile::Debug, Profile::Release] {
            if failure(&cases, &Route::ALL, profile).is_none() {
                checked += cases.len() * Route::ALL.len();
                continue;
            }
            let mut failing = cases.as_slice();
            while failing.len() > 1 {
                let middle = failing.len() / 2;
                let candidate = &failing[..middle];
                failing = if failure(candidate, &Route::ALL, profile).is_none() {
                    &failing[middle..]
                } else {
                    candidate
                };
            }
            for route in Route::ALL {
                if let Some(error) = failure(failing, &[route], profile) {
                    let source = native_source(failing, &[route]);
                    panic!(
                        "{profile:?}, {:?}, {route:?}: {error}\n{source}",
                        failing[0]
                    );
                }
            }
            panic!(
                "conversion batch failed, but the isolated case passed: {profile:?}, {:?}",
                failing[0]
            );
        }
    }
    assert_eq!(checked, 4_270);
}
