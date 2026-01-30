use crate::database::{
    accumulator::Diagnostic,
    input_database::{Db, InputDatabase},
};
use crossbeam_channel::unbounded;
use salsa::Setter;
use std::path::PathBuf;

mod accumulator;
mod input_database;

#[salsa::input]
pub struct File {
    path: PathBuf,
    #[returns(ref)]
    contents: String,
}

#[salsa::tracked]
pub struct ParsedFile<'db> {
    // AST goes here
    value: u32,
    #[returns(ref)]
    links: Vec<ParsedFile<'db>>,
}

#[salsa::tracked]
fn compile(db: &dyn Db, input: File) -> u32 {
    let parsed = parse(db, input);
    sum(db, parsed)
}

#[salsa::tracked]
fn parse(db: &dyn Db, input: File) -> ParsedFile<'_> {
    let mut lines = input.contents(db).lines();
    let value = match lines.next().map(|line| (line.parse::<u32>(), line)) {
        Some((Ok(num), _)) => num,
        Some((Err(_e), line)) => {
            Diagnostic::push_error(
                db,
                input,
                format!("First line ({line}) could not be parsed as an integer"),
            );
            0
        }
        None => {
            Diagnostic::push_error(db, input, "File must contain an integer".to_string());
            0
        }
    };
    let links = lines
        .filter_map(|path| {
            let relative_path = match path.parse::<PathBuf>() {
                Ok(path) => path,
                Err(_err) => {
                    Diagnostic::push_error(db, input, format!("Failed to parse path: {path}"));
                    return None;
                }
            };
            let link_path = input.path(db).parent().unwrap().join(relative_path);
            match db.input(link_path) {
                Ok(file) => Some(parse(db, file)),
                Err(err) => {
                    Diagnostic::push_error(db, input, err);
                    None
                }
            }
        })
        .collect();
    ParsedFile::new(db, value, links)
}

#[salsa::tracked]
fn sum<'db>(db: &'db dyn Db, input: ParsedFile<'db>) -> u32 {
    input.value(db)
        + input
            .links(db)
            .iter()
            .map(|&file| sum(db, file))
            .sum::<u32>()
}

fn cached_compile() -> Result<(), String> {
    // Create the channel to receive file change events.
    let (tx, rx) = unbounded();
    let mut db = InputDatabase::new(tx);

    let initial_file_path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| format!("Usage: ./lazy-input <input-file>"))?;

    // Create the initial input using the input method so that changes to it
    // will be watched like the other files.
    let initial = db.input(initial_file_path.into())?;
    loop {
        // Compile the code starting at the provided input, this will read other
        // needed files using the on-demand mechanism.
        let sum = compile(&db, initial);
        let diagnostics = compile::accumulated::<Diagnostic>(&db, initial);
        if diagnostics.is_empty() {
            println!("Sum is: {sum}");
        } else {
            for diagnostic in diagnostics {
                println!("{}", diagnostic.0);
            }
        }

        // for log in db.logs.lock().unwrap().drain(..) {
        //     eprintln!("{log}");
        // }

        // Wait for file change events, the output can't change unless the
        // inputs change.
        for event in rx.recv().unwrap().unwrap() {
            let path = event.path.canonicalize().unwrap();
            // .wrap_err_with(|| {format!("Failed to canonicalize path {}", event.path.display())})?;
            let file = match db.files.get(&path) {
                Some(file) => *file,
                None => continue,
            };
            // `path` has changed, so read it and update the contents to match.
            // This creates a new revision and causes the incremental algorithm
            // to kick in, just like any other update to a salsa input.
            let contents = std::fs::read_to_string(path).unwrap();
            // .wrap_err_with(|| format!("Failed to read file {}", event.path.display()))?;
            file.set_contents(&mut db).to(contents);
        }
    }
}
