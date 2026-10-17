use crate::catalog::{Capability, CapabilityKind, capabilities, normalize_filter, query_catalog};

const STARTER: &str = "\
40 + 2
answer := 40 + 2
#type answer
#scope
#doc answer
answer";

pub fn help(topic: Option<&str>) -> String {
    let topic = normalize_filter(topic.unwrap_or_default());
    match topic.as_str() {
        "" => format!(
            "{}\n\n{}",
            catalog_summary(CapabilityKind::SessionCommand),
            start_help()
        ),
        "commands" => catalog_summary(CapabilityKind::SessionCommand),
        "start" => start_help(),
        "language" | "recipes" => crate::language_help::language_help(),
        "queries" => query_help(None),
        "attributes" => format!(
            "Declaration attributes precede their declaration without a separating blank line.\n\
             They are not compiler query expressions. \
             Use #help <name> for arguments and restrictions.\n\n{}",
            catalog_summary(CapabilityKind::DeclarationAttribute),
        ),
        "jsonl" => jsonl_help().to_owned(),
        "quit" | "q" => capability_help("exit"),
        topic => crate::language_recipe(topic)
            .map(|recipe| recipe.help())
            .unwrap_or_else(|| capability_help(topic)),
    }
}

fn capability_help(topic: &str) -> String {
    let name = topic.strip_prefix("target.").map_or(topic, |_| "target");
    capabilities()
        .iter()
        .find(|entry| entry.name == name)
        .map(capability_detail)
        .unwrap_or_else(|| {
            format!(
                "unknown help topic `{topic}`; use #help start, #help commands, #help queries, \
             #help language, #help attributes, #help jsonl, or #search <text>"
            )
        })
}

fn catalog_summary(kind: CapabilityKind) -> String {
    capabilities()
        .iter()
        .filter(|entry| entry.kind == kind)
        .map(|entry| format!("{}  {}", entry.syntax, entry.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn query_help(filter: Option<&str>) -> String {
    let entries = query_catalog(filter);
    if entries.is_empty() {
        return "no matching compiler queries; use #queries to list them".to_owned();
    }
    let details = entries
        .into_iter()
        .map(capability_detail)
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Compiler queries are Gin expressions, usable in declarations and type constraints.\n\
         Submit a query directly to inspect its value. @() is a pointer to the unit type.\n\n\
         {details}\n\n\
         #type <expression> is a session command. #todo and #emit are expression directives.\n\
         Declaration attributes have separate rules; use #help attributes."
    )
}

fn capability_detail(entry: &Capability) -> String {
    let kind = match entry.kind {
        CapabilityKind::SessionCommand => "session command",
        CapabilityKind::CompilerQuery => "compiler query expression",
        CapabilityKind::DeclarationAttribute => "declaration attribute",
        CapabilityKind::ExpressionDirective => "expression directive",
    };
    let mut text = format!("{} ({kind})\n{}", entry.syntax, entry.summary);
    for argument in entry.arguments {
        text.push_str(&format!(
            "\nArgument {} ({}): {}",
            argument.name,
            if argument.required {
                "required"
            } else {
                "optional"
            },
            argument.description
        ));
    }
    text.push_str(&format!(
        "\nPrerequisites: {}\nResult: {}",
        entry.prerequisites, entry.result
    ));
    for example in entry.examples {
        for setup in example.setup {
            text.push_str(&format!("\nSetup submission:\n{setup}"));
        }
        text.push_str(&format!("\nExample: {}", example.source));
    }
    text
}

fn start_help() -> String {
    format!(
        "Try these submissions in order; no package or imports are needed:\n{STARTER}\n\n\
         Scratch sessions do not load imports. Start in a package directory or pass its\n\
         path to ginterpreter when you need imports and package dependencies. Package root\n\
         declarations appear in #scope; import nested modules to use their public names.\n\n\
         Submit declarations and expressions separately. A blank line finishes multiline input.\n\
         Submit function calls directly, for example f(1). Submit types together with their\n\
         terminal methods. Replace compatible declarations by resubmitting them.\n\
         Replacing a declaration preserves the other declarations from its submission.\n\
         Resubmit a complete type or shared import when replacing one of its members.\n\
         Failed submissions preserve accepted declarations. Inspect with #type, #scope, #doc,\n\
         #search, #complete, or #suggest, then use #retry.\n\n\
         Gin evaluates computations with compile-time-known inputs during compilation.\n\
         This interpreter uses the typed evaluator; runtime-only operations such as #emit\n\
         cannot execute here. #inspect evaluates without committing evaluator state.\n\
         #todo marks unfinished implementations; queries still work, execution stops at a hole,\n\
         and #save requires every hole filled.\n\n\
         Use #queries for compiler queries and #help <name> for arguments and examples.\n\
         Use #help language for executable recipes: types, functions, records, conditionals,\n\
         and ownership. #help functions starts with typed arithmetic and rejected bad inputs.\n\
         Use #help jsonl for the structured agent interface.\n\
         Ctrl-C cancels the current input; Ctrl-D exits."
    )
}

fn jsonl_help() -> &'static str {
    "Start the structured interface with:\n\
     ginterpreter --jsonl --isolated\n\n\
     Send one JSON object per line, with a string or integer id:\n\
     {\"id\":1,\"op\":\"describe\"}\n\
     {\"id\":2,\"op\":\"submit\",\"source\":\"#queries\"}\n\
     {\"id\":3,\"op\":\"inspect\",\"expression\":\"1 < 2\"}\n\n\
     Responses contain the request id and structured results. Use describe to discover\n\
     capabilities, query syntax, prerequisites, and examples. Requests share one session."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InterpreterSession, RuntimeStorage, Submission};

    #[test]
    fn starter_submissions_work_without_a_package() {
        let mut session = InterpreterSession::default();
        for input in STARTER.lines() {
            let result = session.submit(input);
            assert!(
                matches!(
                    result,
                    Submission::DeclarationAccepted { .. }
                        | Submission::Evaluated { .. }
                        | Submission::Answered { .. }
                ),
                "{input}: {result:?}"
            );
        }
        assert!(
            matches!(session.submit("answer"), Submission::Evaluated { value, .. }
            if value.storage == RuntimeStorage::Int64(42))
        );
    }
}
