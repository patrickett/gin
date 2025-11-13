pub mod cache;
pub mod frontend;
pub mod module;
use crate::module::{Parsable, ToGin};
// use ariadne::{Label, Report, ReportKind};
use cache::COMPILER_CACHE;
use clap::*;
use std::path::PathBuf;

pub const GIN_FILE_EXT: &str = "gin";
pub const BINARY_ENTRY_FILE_NAME: &str = "main.gin";

#[derive(Debug)]
pub enum GincError {
    Other(String),
}
#[derive(Debug)]
pub enum GincWarning {}

pub type GincResult<T> = Result<T, GincError>;

#[derive(Parser, Debug, Default)]
#[command(version, about)]
pub struct Args {
    pub input: PathBuf,
    // /// Write output to <OUTPUT>
    // // TODO: change OUTPUT to FILENAME
    // #[arg(short, long)]
    // output: Option<PathBuf>,
    #[arg(short, long)]
    pub verbose: Option<bool>,
    // #[arg(short, long)]
    // target: Option<TargetPlatform>,
}

// Analagous to the `ginc` command
pub fn compile(args: Args) -> GincResult<Vec<GincWarning>> {
    debug_assert!(args.input.exists());

    if !args.input.is_dir() {
        return Err(GincError::Other("can only compile folders".to_string()));
    }

    // check cache first
    let cached_result = COMPILER_CACHE.get(&args.input);

    if let Some(_cached_item) = cached_result {
        todo!()
    }

    let Some(folder) = args.input.to_gin_folder() else {
        todo!()
    };

    let parsed_folder = folder.parse().expect("msg");

    COMPILER_CACHE.insert(folder.as_path().to_path_buf(), parsed_folder);

    println!("{:#?}", COMPILER_CACHE);
    // let source_code = fs::read_to_string(&args.input).expect("file had content");
    // let filename = args
    //     .input
    //     .file_name()
    //     .expect("has filename")
    //     .to_str()
    //     .expect("msg")
    //     .to_string();

    // let token_iter = GinLexer::new(&source_code).map(|(tok, span)| (tok, span.into())); // Range -> SimpleSpan

    // let token_stream =
    //     Stream::from_iter(token_iter).map((0..source_code.len()).into(), |(t, s): (_, _)| (t, s));

    // let (maybe_output, errors) = token_parser().parse(token_stream).into_output_errors();

    // // can only have ast when no errors
    // debug_assert!(
    //     (maybe_output.is_none() && !errors.is_empty())
    //         || (maybe_output.is_some() && errors.is_empty())
    // );

    // if let Some(ast) = maybe_output {
    //     println!("{ast:#?}");
    // }

    // let mut cache = ariadne::sources([(filename.clone(), &source_code)]);

    // for err in errors.into_iter() {
    //     let span = err.span();
    //     let (start, end) = (span.start(), span.end());

    //     let ariadne_span = (filename.clone(), Range { start, end });
    //     let msg = format!("{err:?}");

    //     let report = Report::build(
    //         ReportKind::Custom("error", ariadne::Color::Red),
    //         ariadne_span.clone(),
    //     )
    //     .with_message(msg.clone())
    //     // TODO: better error msgs
    //     .with_label(Label::new(ariadne_span).with_message("here"))
    //     .finish();

    //     report.eprint(&mut cache).unwrap();
    // }

    //         // do correctness analysis
    //         // 1. typecheck
    //         // 2. borrow check
    //         // return any warnings these cause

    Ok(Vec::new())
}

// http -> download -> .gin_cache/deps/http@version/ -> fingeprint that dir

// dont save ast to disk
// you would just be reading bytes from a file again. might as well just reparse the original file
