use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    SessionCommand,
    CompilerQuery,
    DeclarationAttribute,
    ExpressionDirective,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CapabilityArgument {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CapabilityExample {
    pub source: &'static str,
    pub setup: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Capability {
    pub name: &'static str,
    pub kind: CapabilityKind,
    pub syntax: &'static str,
    pub summary: &'static str,
    pub arguments: &'static [CapabilityArgument],
    pub examples: &'static [CapabilityExample],
    pub prerequisites: &'static str,
    pub result: &'static str,
}

pub fn capabilities() -> &'static [Capability] {
    CAPABILITIES
}

pub fn query_catalog(filter: Option<&str>) -> Vec<&'static Capability> {
    let filter = normalize_filter(filter.unwrap_or_default());
    CAPABILITIES
        .iter()
        .filter(|entry| entry.kind == CapabilityKind::CompilerQuery)
        .filter(|entry| entry.matches(&filter))
        .collect()
}

impl Capability {
    pub(crate) fn matches(&self, filter: &str) -> bool {
        [
            self.name,
            self.syntax,
            self.summary,
            self.prerequisites,
            self.result,
        ]
        .iter()
        .any(|text| text.to_lowercase().contains(filter))
            || self.arguments.iter().any(|argument| {
                argument.name.contains(filter)
                    || argument.description.to_lowercase().contains(filter)
            })
            || self
                .examples
                .iter()
                .any(|example| example.source.to_lowercase().contains(filter))
    }
}

pub(crate) fn normalize_filter(filter: &str) -> String {
    filter.trim().trim_start_matches('#').to_lowercase()
}

const fn argument(
    name: &'static str,
    description: &'static str,
    required: bool,
) -> CapabilityArgument {
    CapabilityArgument {
        name,
        description,
        required,
    }
}

const fn example(source: &'static str) -> CapabilityExample {
    CapabilityExample { source, setup: &[] }
}

const fn command(
    name: &'static str,
    syntax: &'static str,
    summary: &'static str,
    arguments: &'static [CapabilityArgument],
    prerequisites: &'static str,
    result: &'static str,
) -> Capability {
    Capability {
        name,
        kind: CapabilityKind::SessionCommand,
        syntax,
        summary,
        arguments,
        examples: &[],
        prerequisites,
        result,
    }
}

const fn attribute(
    name: &'static str,
    syntax: &'static str,
    summary: &'static str,
    arguments: &'static [CapabilityArgument],
    prerequisites: &'static str,
) -> Capability {
    Capability {
        name,
        kind: CapabilityKind::DeclarationAttribute,
        syntax,
        summary,
        arguments,
        examples: &[],
        prerequisites,
        result: "declaration metadata; not an expression result",
    }
}

const EXPRESSION: &[CapabilityArgument] = &[argument(
    "expression",
    "Gin expression using the current session and package",
    true,
)];
const TASK: &[CapabilityArgument] = &[argument(
    "task-id",
    "background task identifier; omit for the foreground condition",
    false,
)];
const TYPE: &[CapabilityArgument] = &[argument(
    "T",
    "Gin type expression, for example @() or a declared type name",
    true,
)];

const CAPABILITIES: &[Capability] = &[
    command(
        "help",
        "#help [topic]",
        "show commands, examples, or details for a catalog entry",
        &[argument(
            "topic",
            "start, commands, queries, attributes, jsonl, or a capability name",
            false,
        )],
        "none",
        "help text",
    ),
    command(
        "queries",
        "#queries [filter]",
        "list compiler queries or inspect matching queries",
        &[argument(
            "filter",
            "query name or text, with an optional leading #",
            false,
        )],
        "none",
        "compiler query catalog",
    ),
    command(
        "type",
        "#type <expression>",
        "check an expression and show its type",
        EXPRESSION,
        "expression must typecheck; this session command differs from compiler query expressions",
        "formatted type",
    ),
    command(
        "inspect",
        "#inspect <expression>",
        "inspect a value without committing evaluator changes",
        EXPRESSION,
        "compile-time evaluable expression",
        "structured value",
    ),
    command(
        "scope",
        "#scope",
        "list session declarations, package root declarations, and imported names",
        &[],
        "none",
        "names and signatures",
    ),
    command(
        "doc",
        "#doc <symbol>",
        "inspect a declaration's signature and documentation",
        &[argument(
            "symbol",
            "exact name from #scope, including a qualified method name",
            true,
        )],
        "symbol must be visible",
        "signature and documentation",
    ),
    command(
        "search",
        "#search <text>",
        "search visible names, documentation, and the capability catalog",
        &[argument("text", "case-insensitive substring", true)],
        "none",
        "matching symbols and capabilities",
    ),
    command(
        "complete",
        "#complete <source>",
        "suggest completions at the end of a Gin source fragment",
        &[argument(
            "source",
            "possibly incomplete Gin source; the cursor is at its end",
            true,
        )],
        "none; compiler diagnostics in incomplete source do not reject this query",
        "completion candidates",
    ),
    command(
        "suggest",
        "#suggest [source]",
        "suggest edits for source, the pending submission, or the image",
        &[argument(
            "source",
            "Gin source; defaults to pending source, then accepted declarations",
            false,
        )],
        "none",
        "diagnostics and byte-range replacement edits; edits are not applied",
    ),
    command(
        "frames",
        "#frames [task-id]",
        "list captured evaluator frames, innermost first",
        TASK,
        "a failed evaluation with captured frames",
        "frame indices and source locations",
    ),
    command(
        "frame",
        "#frame <index> [task-id]",
        "inspect a captured frame and its locals",
        &[
            argument("index", "zero-based frame index from #frames", true),
            argument(
                "task-id",
                "background task identifier; omit for the foreground condition",
                false,
            ),
        ],
        "a failed evaluation with captured frames",
        "captured locals; not a live debugger frame",
    ),
    command(
        "retry",
        "#retry [task-id]",
        "retry a failed submission against current declarations",
        TASK,
        "a pending condition",
        "whole-submission retry; does not resume a captured frame",
    ),
    command(
        "abort",
        "#abort [task-id]",
        "discard a pending condition",
        TASK,
        "none",
        "condition discarded; accepted declarations remain",
    ),
    command(
        "save",
        "#save <path>",
        "check and write accepted declarations as Gin source",
        &[argument("path", "output file path", true)],
        "all holes must be filled and current package sources must check",
        "source file; excludes evaluation history and heap state",
    ),
    command(
        "run",
        "#run <expression>",
        "evaluate asynchronously",
        EXPRESSION,
        "compile-time evaluable expression",
        "task identifier",
    ),
    command(
        "tasks",
        "#tasks",
        "list running tasks",
        &[],
        "none",
        "running task identifiers and expressions",
    ),
    command(
        "stop",
        "#stop <task-id>",
        "request cancellation of a running task",
        &[argument("task-id", "identifier from #run or #tasks", true)],
        "task must still be running",
        "cancellation request",
    ),
    command(
        "clear",
        "#clear",
        "clear declarations and evaluator state; running tasks continue",
        &[],
        "none",
        "empty session using the same package context",
    ),
    command(
        "reset",
        "#reset",
        "clear declarations, evaluator state, and running tasks",
        &[],
        "none",
        "empty session using the same package context",
    ),
    command(
        "exit",
        "#exit | #quit | #q",
        "exit the interpreter",
        &[],
        "none",
        "session termination",
    ),
    Capability {
        name: "size",
        kind: CapabilityKind::CompilerQuery,
        syntax: "#size(T)",
        summary: "query the storage size of a type in bytes",
        arguments: TYPE,
        examples: &[example("#size(@())"), example("#size(())")],
        prerequisites: "T must have a known layout for the current target; \
                        no package is required for @()",
        result: "compile-time integer, in bytes",
    },
    Capability {
        name: "alignment",
        kind: CapabilityKind::CompilerQuery,
        syntax: "#alignment(T)",
        summary: "query the alignment of a type in bytes",
        arguments: TYPE,
        examples: &[example("#alignment(@())"), example("#alignment(())")],
        prerequisites: "T must have a known layout for the current target; \
                        no package is required for @()",
        result: "compile-time integer, in bytes",
    },
    Capability {
        name: "index_bits",
        kind: CapabilityKind::CompilerQuery,
        syntax: "#index_bits(T)",
        summary: "query the target address/index width in bits",
        arguments: TYPE,
        examples: &[example("#index_bits(@())"), example("#index_bits(())")],
        prerequisites: "a target layout; the current implementation returns the target index width \
                        independently of T",
        result: "compile-time integer, in bits",
    },
    Capability {
        name: "target",
        kind: CapabilityKind::CompilerQuery,
        syntax: "#target[.arch|.vendor|.os|.triple]",
        summary: "inspect target architecture, vendor, operating system, and target triple",
        arguments: &[argument(
            "field",
            "optional field: arch, vendor, os, or triple",
            false,
        )],
        examples: &[
            example("#target"),
            example("#target.arch"),
            example("#target.vendor"),
            example("#target.os"),
            example("#target.triple"),
        ],
        prerequisites: "none; scratch sessions use the host target and packages use their \
                        configured target",
        result: "target record; each selected field is text",
    },
    attribute(
        "bits",
        "#bits(width)",
        "set an integer type's representation width",
        &[argument(
            "width",
            "compile-time integer width from 1 through 128 bits",
            true,
        )],
        "place directly before an integer type declaration whose validity fits the representation",
    ),
    attribute(
        "phantom",
        "#phantom(parameter, ...)",
        "mark generic parameters as phantom",
        &[argument("parameter", "generic parameter name", true)],
        "place directly before the declaration",
    ),
    attribute(
        "fixed_array",
        "#fixed_array(element, count)",
        "describe a fixed-array carrier",
        &[
            argument("element", "element type parameter", true),
            argument("count", "length parameter", true),
        ],
        "requires a compatible carrier declaration",
    ),
    attribute(
        "sequence_builder",
        "#sequence_builder(element, empty, push)",
        "describe a sequence builder carrier",
        &[
            argument("element", "element type parameter", true),
            argument("empty", "empty constructor", true),
            argument("push", "append operation", true),
        ],
        "requires a compatible carrier declaration and operations",
    ),
    attribute(
        "entry",
        "#entry",
        "mark a program entry function",
        &[],
        "place directly before the function declaration",
    ),
    attribute(
        "test",
        "#test",
        "mark a test declaration",
        &[],
        "place directly before the declaration; does not run a test in the interpreter",
    ),
    attribute(
        "inline",
        "#inline",
        "request function inlining",
        &[],
        "place directly before the function declaration",
    ),
    attribute(
        "debug",
        "#debug",
        "mark a declaration for debug builds",
        &[],
        "place directly before the declaration",
    ),
    attribute(
        "infix",
        "#infix('symbol')",
        "declare a function's infix operator symbol",
        &[argument("symbol", "operator text literal", true)],
        "place directly before a compatible operator function",
    ),
    attribute(
        "complexity",
        "#complexity(expression)",
        "annotate a function's complexity",
        &[argument(
            "expression",
            "complexity expression, for example Constant or Linear(n)",
            true,
        )],
        "place directly before the function declaration",
    ),
    attribute(
        "private",
        "#private[(module, ...)]",
        "restrict member visibility",
        &[argument("module", "optional allowed module scope", false)],
        "member declaration; top-level private sections use the private keyword",
    ),
    Capability {
        name: "todo",
        kind: CapabilityKind::ExpressionDirective,
        syntax: "#todo",
        summary: "mark an unfinished implementation",
        arguments: &[],
        examples: &[],
        prerequisites: "use in an expression position; declarations containing holes \
                        remain queryable",
        result: "hole diagnostic when executed; #save rejects unfilled holes",
    },
    Capability {
        name: "emit",
        kind: CapabilityKind::ExpressionDirective,
        syntax: "#emit(code, arguments)",
        summary: "describe target-specific runtime code",
        arguments: &[
            argument("code", "emission code description", true),
            argument("arguments", "emission input values", true),
        ],
        examples: &[],
        prerequisites: "requires valid emission contracts; runtime-only code cannot execute \
                        in the typed interpreter",
        result: "runtime code; compile-time evaluation reports a runtime dependency",
    },
];
