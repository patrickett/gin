//! The corpus: every `.gin` file of the repository's `modules/`, the other standalone `.gin`
//! examples, and the Gin sources written as string literals in the parser's tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repository root")
}

pub fn gin_files(root: &Path, found: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(root) else { return };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            gin_files(&entry, found);
        } else if entry.extension().is_some_and(|e| e == "gin") {
            found.push(entry);
        }
    }
}

pub struct Input {
    pub name: String,
    pub text: String,
}

pub fn parses(text: &str) -> bool {
    parser::parse_strict(text).is_ok()
}

/// `.gin` files under `modules/`, `docs/dev/examples/` and `crates/*/examples/`.
pub fn files() -> Vec<Input> {
    let root = repository();
    let mut paths = Vec::new();
    gin_files(&root.join("modules"), &mut paths);
    gin_files(&root.join("docs/dev/examples"), &mut paths);
    gin_files(&root.join("crates/typecheck/examples"), &mut paths);
    paths
        .into_iter()
        .map(|path| Input {
            name: path.strip_prefix(&root).unwrap().display().to_string(),
            text: std::fs::read_to_string(&path).unwrap(),
        })
        .collect()
}

pub fn module_files() -> Vec<Input> {
    files().into_iter().filter(|input| input.name.starts_with("modules/")).collect()
}

/// The value of every plain or raw string literal in Rust source, unescaped.
pub fn string_literals(rust: &str) -> Vec<String> {
    let bytes = rust.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            }
            b'\'' => {
                // A char literal such as '"' or '\'' (a lifetime has no closing quote nearby).
                if bytes.get(at + 2) == Some(&b'\'') {
                    at += 3;
                } else if bytes.get(at + 1) == Some(&b'\\') && bytes.get(at + 3) == Some(&b'\'') {
                    at += 4;
                } else {
                    at += 1;
                }
            }
            b'r' if matches!(bytes.get(at + 1), Some(b'"' | b'#'))
                && (at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_')) =>
            {
                let mut hashes = 0;
                let mut cursor = at + 1;
                while bytes.get(cursor) == Some(&b'#') {
                    hashes += 1;
                    cursor += 1;
                }
                if bytes.get(cursor) != Some(&b'"') {
                    at += 1;
                    continue;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let body = cursor + 1;
                let Some(length) = rust[body..].find(&close) else { break };
                found.push(rust[body..body + length].to_owned());
                at = body + length + close.len();
            }
            b'"' => {
                let mut value = String::new();
                let mut cursor = at + 1;
                loop {
                    let Some(&byte) = bytes.get(cursor) else { return found };
                    match byte {
                        b'"' => break,
                        b'\\' => {
                            match bytes.get(cursor + 1) {
                                Some(b'n') => value.push('\n'),
                                Some(b't') => value.push('\t'),
                                Some(b'r') => value.push('\r'),
                                Some(b'0') => value.push('\0'),
                                Some(b'\\') => value.push('\\'),
                                Some(b'"') => value.push('"'),
                                Some(b'\'') => value.push('\''),
                                Some(b'\n') => {
                                    cursor += 2;
                                    while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\n')) {
                                        cursor += 1;
                                    }
                                    continue;
                                }
                                _ => {
                                    // \u{..} and \x..: not Gin source worth keeping.
                                    value.push('\u{fffd}');
                                }
                            }
                            cursor += 2;
                        }
                        _ => {
                            let ch = rust[cursor..].chars().next().unwrap();
                            value.push(ch);
                            cursor += ch.len_utf8();
                        }
                    }
                }
                found.push(value);
                at = cursor + 1;
            }
            _ => at += 1,
        }
    }
    found
}

fn rust_files(root: &Path, found: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(root) else { return };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            rust_files(&entry, found);
        } else if entry.extension().is_some_and(|e| e == "rs") {
            found.push(entry);
        }
    }
}

/// Literals from `crates/parser/tests` that the parser accepts without flaws.
pub fn parser_test_inputs() -> Vec<Input> {
    let root = repository();
    let mut paths = Vec::new();
    rust_files(&root.join("crates/parser/tests"), &mut paths);
    let mut inputs = Vec::new();
    for path in paths {
        let rust = std::fs::read_to_string(&path).unwrap();
        for (index, text) in string_literals(&rust).into_iter().enumerate() {
            if text.contains('\n') && parses(&text) {
                inputs.push(Input {
                    name: format!("{}#{index}", path.strip_prefix(&root).unwrap().display()),
                    text,
                });
            }
        }
    }
    inputs
}
