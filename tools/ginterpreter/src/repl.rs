//! The interactive loop. Declarations extend the session; an expression is checked and, when
//! it can be known at compile time, evaluated; flaws are printed the way `ginc --check-new`
//! prints them. Commands start with `$`.

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use crate::check::read_text;
use crate::flaw::Flaw;
use crate::session::{EvalOutcome, EvalWhat, Session, Target};
use crate::source::{Split, split};

pub const HELP: &str = "\
Gin session. Type a declaration or definition to extend the session, or an expression to
check it and (when its inputs are known now) evaluate it.

Commands start with `$`; `#` is the language's. They are recognised by name at the
start of a submission. A directive inside an expression (`#size(Point)`) is the language's.

  $check [path]        the flaws and warnings of the session, or of a file or package
  $eval <expression>   check and evaluate an expression (a bare expression does the same)
  $type <expression>   the type the checker proves for an expression
  $holes               the `#todo` holes of the session, with expected types, scope and facts
  $scope               what the session defines
  $inspect <name>      shape, size and layout of a type; signature of a function; value of a constant
  $image               definitions, what each uses and what a change would affect
  $load <path>         add the declarations of a .gin file
  $save <path>         write the image to a snapshot file
  $restore <path> [--verify]  load a snapshot (re-checked; derived state kept where the closure
                       hash matches; --verify also re-derives and compares each kept entry)
  $remove <name>       remove declarations by name
  $reset               forget everything
  $help                this text
  $quit                leave

A declaration is accepted only if the whole session still checks; a rejected one is reported
and the session is unchanged. A multi-line submission ends with a blank line.";

thread_local! {
    static FLAWS_SHOWN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many flaws (not warnings) this thread has printed. A batch run compares it before and
/// after a submission to know whether the submission failed.
pub fn flaws_shown() -> u64 {
    FLAWS_SHOWN.with(std::cell::Cell::get)
}

fn show_failure() {
    FLAWS_SHOWN.with(|count| count.set(count.get() + 1));
}

fn print_flaws(flaws: &[Flaw]) {
    for flaw in flaws {
        if flaw.is_flaw() {
            show_failure();
        }
        flaw.print();
    }
}

/// Run one submission. Returns false when the session should end.
pub fn submit(session: &mut Session, text: &str, out: &mut impl Write) -> std::io::Result<bool> {
    let text = text.trim_end();
    if text.trim().is_empty() {
        return Ok(true);
    }
    if let Some((name, rest)) = crate::commands::repl_command(text) {
        return command_line(session, name, rest, out);
    }
    match split(text) {
        // `#size(Point)` on its own line is a directive used as an expression, which the file
        // parser reads as a (misplaced) top-level directive: it is the language's, so it is
        // evaluated as an expression rather than reported as a syntax error.
        Err(_) if text.trim_start().starts_with('#') && !text.contains('\n') => {
            return evaluate(session, text.trim(), out);
        }
        Err(diagnostics) => {
            let source: std::sync::Arc<str> = std::sync::Arc::from(text);
            let flaws: Vec<Flaw> = diagnostics.iter().map(|d| crate::flaw::from_diagnostic("parse", d, "<input>", &source, &[])).collect();
            print_flaws(&flaws);
        }
        Ok(Split::Empty) => {}
        Ok(Split::Expression(expression)) => return evaluate(session, &expression, out),
        Ok(Split::Declarations(_)) => match session.declare(text) {
            Err(flaws) => {
                print_flaws(&flaws);
                writeln!(out, "rejected; the session is unchanged")?;
            }
            Ok(declared) if declared.accepted => {
                print_flaws(&declared.warnings);
                let mut parts = Vec::new();
                if !declared.added.is_empty() {
                    parts.push(format!("added {}", declared.added.join(", ")));
                }
                if !declared.replaced.is_empty() {
                    parts.push(format!("replaced {}", declared.replaced.join(", ")));
                }
                writeln!(out, "ok: {}", parts.join("; "))?;
            }
            Ok(declared) => {
                print_flaws(&declared.flaws);
                writeln!(out, "rejected: {} flaw(s); the session is unchanged", declared.flaws.len())?;
            }
        },
    }
    Ok(true)
}

/// Evaluate an expression against the session and print what is known.
fn evaluate(session: &mut Session, expression: &str, out: &mut impl Write) -> std::io::Result<bool> {
    match session.eval(&Target::Session, &EvalWhat::Expression(expression.to_owned())) {
        EvalOutcome::Value { display, ty, .. } => writeln!(out, "{display} : {ty}")?,
        EvalOutcome::Unknown { reason, text } => {
            writeln!(out, "not known at compile time ({}): {text}", reason["kind"].as_str().unwrap_or("unknown"))?;
        }
        EvalOutcome::Flawed(flaws) => {
            print_flaws(&flaws);
            writeln!(out, "{} flaw(s); nothing was run", flaws.iter().filter(|f| f.is_flaw()).count())?;
        }
    }
    Ok(true)
}

fn command_line(session: &mut Session, name: &str, rest: &str, out: &mut impl Write) -> std::io::Result<bool> {
    match name {
        "quit" => return Ok(false),
        "help" => writeln!(out, "{HELP}")?,
        "eval" => return evaluate(session, rest, out),
        "check" => {
            let checked = if rest.is_empty() {
                session.check()
            } else {
                let path = Path::new(rest);
                if path.is_dir() {
                    crate::check::check_package(path)
                } else {
                    match read_text(path) {
                        Ok(text) => crate::check::check_text(rest, &text),
                        Err(message) => {
                            show_failure();
                            writeln!(out, "{message}")?;
                            return Ok(true);
                        }
                    }
                }
            };
            print_flaws(&checked.flaws);
            print_flaws(&checked.warnings);
            writeln!(out, "{} flaw(s), {} warning(s), {} hole(s)", checked.flaws.len(), checked.warnings.len(), checked.holes.len())?;
        }
        "reset" => {
            session.reset();
            writeln!(out, "session cleared")?;
        }
        "type" => match session.type_of(rest) {
            Ok(ty) => writeln!(out, "{ty}")?,
            Err(flaws) => print_flaws(&flaws),
        },
        "flaws" => {
            if session.last_flaws.is_empty() {
                writeln!(out, "no flaws")?;
            } else {
                print_flaws(&session.last_flaws.clone());
            }
        }
        "holes" => {
            let checked = session.holes();
            print_flaws(&checked.flaws);
            if checked.holes.is_empty() {
                writeln!(out, "no holes")?;
            }
            for hole in &checked.holes {
                writeln!(
                    out,
                    "hole in {} at {}:{}: expected {}",
                    hole.construct,
                    hole.span.start.0,
                    hole.span.start.1,
                    hole.expected.as_deref().unwrap_or("(not published)")
                )?;
                for fact in &hole.facts {
                    writeln!(out, "  known: {fact}")?;
                }
                for p in &hole.params {
                    writeln!(out, "  parameter {}{}", p.name, p.ty.as_ref().map(|t| format!(" {t}")).unwrap_or_default())?;
                }
                for (name, ty) in &hole.locals {
                    writeln!(out, "  local {name}{}", ty.as_ref().map(|t| format!(" {t}")).unwrap_or_default())?;
                }
            }
        }
        "scope" => {
            for entry in session.scope() {
                writeln!(out, "{} {}", entry.kind, entry.signature)?;
            }
        }
        "inspect" => {
            let data = session.inspect(rest);
            writeln!(out, "{}", serde_json::to_string_pretty(&data).unwrap_or_default())?;
        }
        "image" => {
            let (data, _) = session.image();
            writeln!(out, "{}", serde_json::to_string_pretty(&data).unwrap_or_default())?;
        }
        "save" => match session.save(Path::new(rest)) {
            Ok(data) => writeln!(out, "saved {} declaration(s), {} cached entr(ies) to {rest}", data["declarations"], data["cached_entries"])?,
            Err(notice) => writeln!(out, "not saved: {}", notice["detail"].as_str().unwrap_or(""))?,
        },
        "restore" => {
            let (path, verify) = match rest.strip_suffix("--verify") {
                Some(path) => (path.trim(), true),
                None => (rest, false),
            };
            let data = session.restore(Path::new(path), verify);
            if data["restored"] == true {
                writeln!(out, "restored {} declaration(s); kept {} derived entr(ies)", data["declarations"], data["kept"].as_array().map_or(0, Vec::len))?;
            } else {
                writeln!(out, "not restored")?;
            }
            for n in data["notices"].as_array().into_iter().flatten() {
                writeln!(out, "  {}: {}", n["kind"].as_str().unwrap_or(""), n["detail"].as_str().unwrap_or(""))?;
            }
        }
        "remove" => {
            let removed = session.remove(rest);
            writeln!(out, "removed {}", if removed.is_empty() { "nothing".to_owned() } else { removed.join(", ") })?;
        }
        "load" => match read_text(Path::new(rest)) {
            Err(message) => {
                show_failure();
                writeln!(out, "{message}")?;
            }
            Ok(text) => match session.load_text(&text) {
                Err(flaws) => print_flaws(&flaws),
                Ok(declared) if declared.accepted => writeln!(out, "loaded {} declaration(s)", declared.added.len() + declared.replaced.len())?,
                Ok(declared) => {
                    print_flaws(&declared.flaws);
                    writeln!(out, "not loaded: {} flaw(s)", declared.flaws.len())?;
                }
            },
        },
        other => {
            show_failure();
            writeln!(out, "unknown command `${other}`; try $help")?;
        }
    }
    Ok(true)
}

/// Whether `buffer` is a whole submission: it parses, and does not end inside an indented block.
fn complete(buffer: &str) -> bool {
    // A command is one line.
    if crate::commands::repl_command(buffer).is_some() {
        return true;
    }
    let last = buffer.lines().last().unwrap_or("");
    if last.starts_with([' ', '\t']) {
        return false;
    }
    // A header or a marker line promises a body or a declaration on the lines that follow.
    let trimmed = last.trim();
    // A bare `#word` (or `#instruction('..')`) is a marker for the line after it; `#size(x)` is a
    // directive in an expression, which is whole.
    let marker = trimmed.strip_prefix('#').is_some_and(|rest| rest.chars().all(|c| c.is_alphanumeric() || c == '_') && !rest.is_empty())
        || trimmed.starts_with("#instruction(");
    if marker || [" has", " is", ":", " or", " then", " else", " do"].iter().any(|end| trimmed.ends_with(end)) {
        return false;
    }
    // A single line that is a directive in an expression (`#size(Point)`) is whole.
    if trimmed.starts_with('#') && !buffer.contains('\n') {
        return true;
    }
    parser::parse_strict(buffer).is_ok()
}

/// Run each submission whole, in order, against one session: the batch form (`--eval`). A failed
/// submission does not stop the rest, so a later one can repair it. Returns whether every
/// submission was free of flaws; a `$quit` ends the batch early.
pub fn run_batch(session: &mut Session, submissions: &[String], out: &mut impl Write) -> std::io::Result<bool> {
    let mut succeeded = true;
    for text in submissions {
        let before = flaws_shown();
        let keep_going = submit(session, text, out)?;
        succeeded &= flaws_shown() == before;
        if !keep_going {
            break;
        }
    }
    Ok(succeeded)
}

/// Read submissions from `input` (a terminal gets line editing) until it ends or `:quit`.
pub fn run(session: &mut Session, out: &mut impl Write) -> std::io::Result<()> {
    let interactive = std::io::stdin().is_terminal();
    if interactive {
        writeln!(out, "ginterpreter: a Gin session. :help for commands, :quit to leave.")?;
        let mut editor = match rustyline::DefaultEditor::new() {
            Ok(editor) => editor,
            Err(error) => return Err(std::io::Error::other(error.to_string())),
        };
        let mut buffer = String::new();
        loop {
            let prompt = if buffer.is_empty() { "gin> " } else { "...  " };
            match editor.readline(prompt) {
                Ok(line) => {
                    let _ = editor.add_history_entry(&line);
                    if !step(session, &mut buffer, &line, out)? {
                        return Ok(());
                    }
                }
                Err(rustyline::error::ReadlineError::Interrupted) => buffer.clear(),
                Err(_) => break,
            }
        }
        if !buffer.is_empty() {
            submit(session, &buffer, out)?;
        }
        return Ok(());
    }
    run_lines(session, std::io::stdin().lock(), out)
}

/// The same loop over any reader, for pipes and tests.
pub fn run_lines(session: &mut Session, input: impl BufRead, out: &mut impl Write) -> std::io::Result<()> {
    let mut buffer = String::new();
    for line in input.lines() {
        if !step(session, &mut buffer, &line?, out)? {
            return Ok(());
        }
    }
    if !buffer.trim().is_empty() {
        submit(session, &buffer, out)?;
    }
    Ok(())
}

fn step(session: &mut Session, buffer: &mut String, line: &str, out: &mut impl Write) -> std::io::Result<bool> {
    if line.trim().is_empty() {
        if buffer.trim().is_empty() {
            buffer.clear();
            return Ok(true);
        }
        let text = std::mem::take(buffer);
        return submit(session, &text, out);
    }
    if !buffer.is_empty() {
        buffer.push('\n');
    }
    buffer.push_str(line);
    if complete(buffer) {
        let text = std::mem::take(buffer);
        return submit(session, &text, out);
    }
    Ok(true)
}
