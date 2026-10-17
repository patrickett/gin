use super::*;
use crate::{InterpreterSession, Submission};

fn run_recipe(name: &str) {
    let recipe = language_recipe(name).unwrap();
    let mut session = InterpreterSession::default();
    for step in recipe.steps {
        let result = session.submit(step.source);
        match (&step.expected, &result) {
            (RecipeExpectation::DeclarationAccepted, Submission::DeclarationAccepted { .. }) => {}
            (RecipeExpectation::Evaluated { display }, Submission::Evaluated { value, .. }) => {
                assert_eq!(
                    value.display(),
                    *display,
                    "{}: {}",
                    recipe.name,
                    step.source
                );
            }
            (RecipeExpectation::Answered { contains }, Submission::Answered { text }) => {
                assert!(text.contains(contains), "{}: {text}", step.source);
            }
            _ => panic!("{}: {}: {result:?}", recipe.name, step.source),
        }
    }
    let accepted = session.declarations().to_owned();
    for rejection in recipe.rejections {
        let result = session.submit(rejection.source);
        let Submission::Rejected { diagnostics } = result else {
            panic!("{}: {}: {result:?}", recipe.name, rejection.source);
        };
        assert!(
            diagnostics
                .iter()
                .any(|entry| entry.diagnostic().code == rejection.code),
            "{}: {}: {diagnostics:?}",
            recipe.name,
            rejection.source,
        );
        assert_eq!(session.declarations(), accepted);
    }
}

#[test]
fn bounded_type_recipe_checks_known_arguments() {
    run_recipe("types");
}

#[test]
fn function_recipe_multiplies_values_and_rejects_invalid_quantity() {
    run_recipe("functions");
}

#[test]
fn record_recipe_constructs_and_reads_fields() {
    run_recipe("records");
}

#[test]
fn conditional_recipe_selects_both_branches() {
    run_recipe("conditionals");
}

#[test]
fn ownership_recipe_consumes_values_and_projects_integer_domains() {
    run_recipe("ownership");
}

#[test]
fn help_search_and_empty_completion_share_executable_recipes() {
    let mut session = InterpreterSession::default();
    assert!(matches!(
        session.submit("missing"),
        Submission::Rejected { .. }
    ));
    let completions = session.completions("");
    let index = crate::help(Some("language"));
    assert!(crate::help(Some("start")).contains("#help language"));
    for recipe in language_recipes() {
        let topic = format!("#help {}", recipe.name);
        assert!(index.contains(&topic));
        let help = crate::help(Some(recipe.name));
        for step in recipe.steps {
            assert!(help.contains(step.source));
        }
        for rejection in recipe.rejections {
            assert!(help.contains("Intentional compile-time rejection"));
            assert!(help.contains(&rejection.code.slug()));
        }
        for alias in recipe.aliases {
            assert_eq!(language_recipe(alias).unwrap().name, recipe.name);
            assert_eq!(crate::help(Some(alias)), help);
        }
        assert!(session.search(recipe.name).iter().any(|entry| {
            entry.name == recipe.name && entry.kind == "language_recipe" && entry.signature == topic
        }));
        let completion = completions
            .iter()
            .find(|entry| entry.kind == "language_recipe" && entry.label == recipe.name)
            .expect("recipe starter completion");
        let source = completion.insert_text.as_deref().expect("runnable source");
        assert_eq!(source, recipe.steps[0].source);
        let result = InterpreterSession::default().submit(source);
        assert!(
            matches!(result, Submission::DeclarationAccepted { .. }),
            "{source}: {result:?}"
        );
    }
    assert!(crate::help(Some("type")).starts_with("#type <expression>"));
    assert_eq!(session.pending(), Some("missing"));
    assert!(session.declarations().is_empty());
    assert!(
        session
            .completions("parcel.")
            .iter()
            .all(|entry| entry.kind != "language_recipe")
    );
}

#[test]
fn recipe_steps_and_expected_outcomes_are_machine_readable() {
    let recipe = serde_json::to_value(language_recipe("functions").unwrap()).unwrap();
    assert_eq!(
        recipe["steps"][0]["expected"]["kind"],
        "declaration_accepted"
    );
    assert_eq!(recipe["steps"][1]["expected"]["kind"], "evaluated");
    assert_eq!(recipe["steps"][1]["expected"]["display"], "375");
    assert_eq!(
        recipe["rejections"][0]["code"],
        "type-integer-literal-out-of-range"
    );
}
