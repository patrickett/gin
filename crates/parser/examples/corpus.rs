//! Parses each `.gin` file named on the command line, or found under a named directory, with
//! the new parser alone, and prints one tab-separated line per file:
//! `path`, `ok` or `fail`, the number of flaws, the first flaw's line, and its message.
//! Package resolution and checking are not involved, so one unsupported import cannot hide
//! what the parser makes of the rest of the file. With `--all` it prints one line per flaw
//! instead: `path`, `flaw`, the line, and the message, so kinds of problem can be counted.
use std::path::{Path, PathBuf};

fn files(path: &Path, found: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for entry in entries {
            files(&entry, found);
        }
    } else if path.extension().is_some_and(|extension| extension == "gin") {
        found.push(path.to_owned());
    }
}

fn main() {
    let mut found = Vec::new();
    let mut all = false;
    for argument in std::env::args().skip(1) {
        if argument == "--all" {
            all = true;
            continue;
        }
        files(Path::new(&argument), &mut found);
    }
    for path in found {
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.display();
        match parser::parse(&text) {
            Err(failure) => println!("{name}\tfail\t1\t0\t{failure:?}"),
            Ok(output) => {
                let flaws: Vec<_> = output
                    .diagnostics
                    .iter()
                    .filter(|d| d.category == diagnostic::Category::Flaw)
                    .collect();
                if all {
                    for flaw in &flaws {
                        let line = text[..flaw.span.start().min(text.len())]
                            .matches('\n')
                            .count()
                            + 1;
                        let message = flaw.message.replace(['\n', '\t'], " ");
                        println!("{name}\tflaw\t{line}\t{message}");
                    }
                    continue;
                }
                match flaws.first() {
                    None => println!("{name}\tok\t0\t\t"),
                    Some(first) => {
                        let line = text[..first.span.start().min(text.len())]
                            .matches('\n')
                            .count()
                            + 1;
                        let message = first.message.replace(['\n', '\t'], " ");
                        println!("{name}\tfail\t{}\t{line}\t{message}", flaws.len());
                    }
                }
            }
        }
    }
}
