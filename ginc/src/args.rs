use clap::Parser;
use std::ffi::OsStr;
use std::path::PathBuf;

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
