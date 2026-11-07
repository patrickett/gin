//! The names of the session commands. Everything that talks to the session starts with `$`;
//! `#` belongs to the language (`#size`, `#entry`, `#todo`, ...). The language has no use for a
//! `$` outside a comment or a string literal (`#instruction('bl $target')`), so a submission or
//! an `op` that starts with `$` is never Gin, and a submission that starts with `#` is never a
//! command. `tests/commands.rs` checks both against the real lexer and parser.

/// The operations of the JSONL protocol, without the `$`.
pub const JSONL_OPS: [&str; 14] = [
    "describe", "check", "holes", "eval", "declare", "remove", "scope", "inspect", "image", "save", "restore", "load", "reset", "quit",
];

/// The commands of the REPL, without the `$`.
pub const REPL_COMMANDS: [&str; 14] = [
    "check", "eval", "type", "holes", "scope", "inspect", "image", "load", "save", "restore", "reset", "remove", "help", "quit",
];

/// Every command name, in either mode, without the `$`.
pub fn all_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = JSONL_OPS.iter().chain(REPL_COMMANDS.iter()).copied().collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// `(name, rest)` when `text` starts with `$` and a REPL command name (followed by whitespace or
/// the end): the command and the rest of its line. A `$` anywhere else, and every `#`, is the
/// language's.
pub fn repl_command(text: &str) -> Option<(&str, &str)> {
    let after = text.trim_start().strip_prefix('$')?;
    let end = after.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(after.len());
    let (name, rest) = after.split_at(end);
    let boundary = rest.is_empty() || rest.starts_with(char::is_whitespace);
    (boundary && REPL_COMMANDS.contains(&name)).then_some((name, rest.trim()))
}

/// The `$` spelling to suggest for an operation written without it, or with `:` or `#`.
pub fn suggestion(op: &str) -> Option<String> {
    let bare = op.trim_start_matches([':', '#', '$']);
    JSONL_OPS.contains(&bare).then(|| format!("${bare}"))
}
