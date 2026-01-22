pub mod backend;
pub mod cache;
pub mod diagnostic;
pub mod frontend;
pub mod source;
use crate::diagnostic::*;
use crate::frontend::parser::Parsable;
use clap::*;
use std::ffi::OsStr;
use std::path::PathBuf;

pub const GIN_FILE_EXT: &str = "gin";
pub const BINARY_ENTRY_FILE_NAME: &str = "main.gin";

pub type GincResult<'a, T> = Result<(GincWarnings, T), GincFlaw<'a>>;

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

impl Args {
    pub fn folder_name(&self) -> String {
        self.input
            .file_name()
            .unwrap_or(OsStr::new("project"))
            .to_string_lossy()
            .to_string()
    }
}

/// Analagous to the `ginc` command
pub struct GinCompiler;

impl GinCompiler {
    pub fn compile(args: &'_ mut Args) -> GincResult<'_, ()> {
        let warnings = Vec::new();
        match &args.input.to_ast() {
            Ok(ast) => println!("{:#?}", ast),
            Err(errors) => return Err(GincFlaw::ParseError(errors.clone())),
        }

        // println!("{:#?}", ast);

        // let _folder = parse(args.input)?;

        //         // do correctness analysis
        //         // 1. typecheck
        //         // 2. borrow check
        //         // return any warnings these cause

        Ok((warnings, ()))
    }
}

// http -> download -> .gin_cache/deps/http@version/ -> fingeprint that dir
