use diagnostic::DiagnosticCode;
use diagnostic::code::{IntegerCode, OperatorCode, TypeCode};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LanguageRecipe {
    pub name: &'static str,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub steps: &'static [RecipeStep],
    pub rejections: &'static [RecipeRejection],
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeStep {
    pub source: &'static str,
    pub expected: RecipeExpectation,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecipeExpectation {
    DeclarationAccepted,
    Evaluated { display: &'static str },
    Answered { contains: &'static str },
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeRejection {
    pub source: &'static str,
    pub code: DiagnosticCode,
    pub explanation: &'static str,
}

pub fn language_recipes() -> &'static [LanguageRecipe] {
    RECIPES
}

pub fn language_recipe(topic: &str) -> Option<&'static LanguageRecipe> {
    let topic = crate::catalog::normalize_filter(topic);
    RECIPES
        .iter()
        .find(|recipe| recipe.name == topic || recipe.aliases.contains(&topic.as_str()))
}

impl LanguageRecipe {
    pub(crate) fn matches(&self, filter: &str) -> bool {
        [self.name, self.title, self.summary]
            .into_iter()
            .chain(self.aliases.iter().copied())
            .chain(self.steps.iter().map(|step| step.source))
            .any(|text| text.to_lowercase().contains(filter))
    }

    pub(crate) fn help(&self) -> String {
        let mut text = format!(
            "{}\n{}\n\nStart a fresh isolated session; submit each numbered source separately.",
            self.title, self.summary
        );
        for (index, step) in self.steps.iter().enumerate() {
            let expected = match step.expected {
                RecipeExpectation::DeclarationAccepted => "declaration accepted".to_owned(),
                RecipeExpectation::Evaluated { display } => format!("value: {display}"),
                RecipeExpectation::Answered { contains } => format!("answer contains: {contains}"),
            };
            text.push_str(&format!(
                "\n\n{}. Submit:\n{}\nExpected {expected}.",
                index + 1,
                step.source
            ));
        }
        for rejection in self.rejections {
            text.push_str(&format!(
                "\n\nIntentional compile-time rejection:\n{}\nExpected diagnostic: {}.\n{}",
                rejection.source, rejection.code, rejection.explanation
            ));
        }
        text
    }
}

pub(crate) fn language_help() -> String {
    let topics = RECIPES
        .iter()
        .map(|recipe| format!("#help {}  {}", recipe.name, recipe.title))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Language recipes run without a package or imports. Each topic provides complete\n\
         submissions, expected results, and separately labeled compiler rejection examples.\n\
         Start with bounded anonymous integers for arithmetic. Gin evaluates computations\n\
         with known inputs at compile time; parameter bounds reject invalid inputs.\n\n{topics}"
    )
}

const RECIPES: &[LanguageRecipe] = &[
    LanguageRecipe {
        name: "types",
        title: "Bounded integer types",
        aliases: &["integers", "integer", "bounds", "numbers"],
        summary: "Write `value in 0 to 255` to give a parameter an inclusive integer domain. \
            These anonymous bounded types support primitive arithmetic without imports. \
            The compiler checks known arguments against the domain. Use #type to inspect \
            an expression's inferred type; use #help ownership for named nominal types.",
        steps: &[
            RecipeStep {
                source: "identity(value in 0 to 255): return value",
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "identity(7)",
                expected: RecipeExpectation::Evaluated { display: "7" },
            },
            RecipeStep {
                source: "#type identity(7)",
                expected: RecipeExpectation::Answered {
                    contains: "integer in 0 to 255",
                },
            },
        ],
        rejections: &[RecipeRejection {
            source: "identity(256)",
            // type-integer-literal-out-of-range
            code: DiagnosticCode::Type(TypeCode::Integer(IntegerCode::LiteralOutOfRange)),
            explanation: "256 is outside the parameter's inclusive 0 to 255 domain. \
                The accepted identity declaration remains available after rejection.",
        }],
    },
    LanguageRecipe {
        name: "functions",
        title: "Typed functions and compile-time arithmetic",
        aliases: &["function", "arithmetic", "parameters", "calls"],
        summary: "A function declares parameters in parentheses and a body after a colon. \
            An inline expression body may omit `return`; an indented multiline body needs \
            `return` to produce a result. Bounded anonymous \
            parameters are the simplest way to write primitive arithmetic. The compiler \
            infers the result type. A call with known inputs, such as order_total(3, 125), \
            runs at compile time. Named arguments use a colon, as in quantity: 3.",
        steps: &[
            RecipeStep {
                source: concat!(
                    "order_total(quantity in 1 to 10, price in 0 to 10000):\n",
                    "    return quantity * price"
                ),
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "order_total(3, 125)",
                expected: RecipeExpectation::Evaluated { display: "375" },
            },
            RecipeStep {
                source: "order_total(quantity: 3, price: 125)",
                expected: RecipeExpectation::Evaluated { display: "375" },
            },
        ],
        rejections: &[RecipeRejection {
            source: "order_total(0, 125)",
            // type-integer-literal-out-of-range
            code: DiagnosticCode::Type(TypeCode::Integer(IntegerCode::LiteralOutOfRange)),
            explanation: "Zero is outside quantity's inclusive 1 to 10 domain, so this \
                call is rejected during compilation instead of failing at runtime.",
        }],
    },
    LanguageRecipe {
        name: "records",
        title: "Records, fields, and construction",
        aliases: &["record", "struct", "fields", "constructor"],
        summary: "Declare a record with `Name has` and indented fields. A field's bound \
            follows its name without a colon. Include a terminal method with the type in \
            the same submission; see #help ownership. Construct a value with Name(field: \
            value, ...) and read fields with value.field. Submit the value binding separately \
            from the type so each can be replaced independently.",
        steps: &[
            RecipeStep {
                source: concat!(
                    "Parcel has\n    weight in 0 to 100\n    priority in 0 to 1\n",
                    "    finish(self):\n        return"
                ),
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "parcel := Parcel(weight: 4, priority: 1)",
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "parcel.weight",
                expected: RecipeExpectation::Evaluated { display: "4" },
            },
            RecipeStep {
                source: "parcel.priority",
                expected: RecipeExpectation::Evaluated { display: "1" },
            },
            RecipeStep {
                source: "#type parcel",
                expected: RecipeExpectation::Answered { contains: "Parcel" },
            },
        ],
        rejections: &[RecipeRejection {
            source: "Parcel(weight: 101, priority: 1)",
            // type-out-of-range
            code: DiagnosticCode::Type(TypeCode::Integer(IntegerCode::OutOfRange)),
            explanation: "The constructor rejects a weight outside its field's inclusive \
                0 to 100 domain during compilation.",
        }],
    },
    LanguageRecipe {
        name: "conditionals",
        title: "Conditional values with when",
        aliases: &["conditional", "when", "if", "branches", "comparison"],
        summary: "Use `when condition then value else value` to select a result. \
            `is` compares values in this example. Gin uses `when`; the `if` keyword has \
            been removed. In a multiline function body, explicitly return the conditional \
            result. This function accepts only the two priority values 0 and 1.",
        steps: &[
            RecipeStep {
                source: "route(priority in 0 to 1): return when priority is 1 then 10 else 20",
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "route(1)",
                expected: RecipeExpectation::Evaluated { display: "10" },
            },
            RecipeStep {
                source: "route(0)",
                expected: RecipeExpectation::Evaluated { display: "20" },
            },
        ],
        rejections: &[RecipeRejection {
            source: "route(2)",
            // type-integer-literal-out-of-range
            code: DiagnosticCode::Type(TypeCode::Integer(IntegerCode::LiteralOutOfRange)),
            explanation: "The compiler rejects an unknown priority before the conditional runs.",
        }],
    },
    LanguageRecipe {
        name: "ownership",
        title: "Named integers, terminal methods, and domain projection",
        aliases: &[
            "terminal",
            "terminal_methods",
            "finish",
            "nominal",
            "projection",
        ],
        summary: "`Count is in 0 to 255` declares a distinct nominal integer type. Submit \
            it together with its terminal method: `finish(self)` consumes a value and \
            returns no result. Invoke a zero-argument method as value.finish. Nominal \
            arithmetic needs an explicitly provided operator method. To reuse Count's \
            bounds as an anonymous integer domain, write `value is in Count`; those \
            parameters support primitive arithmetic without nominal operator dispatch.",
        steps: &[
            RecipeStep {
                source: "Count is in 0 to 255\nCount has\n    finish(self):\n        return",
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "consume(value Count):\n    value.finish\n    return",
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "consume(7)",
                expected: RecipeExpectation::Evaluated {
                    display: "completed (no value)",
                },
            },
            RecipeStep {
                source: "raw_count(value is in Count): return value",
                expected: RecipeExpectation::DeclarationAccepted,
            },
            RecipeStep {
                source: "raw_count(8)",
                expected: RecipeExpectation::Evaluated { display: "8" },
            },
        ],
        rejections: &[RecipeRejection {
            source: "double(value Count): return value + value",
            // type-nominal-operator-unavailable
            code: DiagnosticCode::Type(TypeCode::Operator(OperatorCode::NominalUnavailable)),
            explanation: "Count has no addition operator method. Use a parameter `value \
                is in Count` for primitive arithmetic, or define the nominal operator.",
        }],
    },
];

#[cfg(test)]
#[path = "tests/language_help_tests.rs"]
mod tests;
