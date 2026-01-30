use clap::Parser;
use ginc::{Args, GinCompiler};

fn main() {
    let mut args = Args::parse();
    // let folder_name = args.folder_name();

    // info!("    {} {}", "Compiling".green().bold(), folder_name);
    // match GinCompiler::compile(&mut args) {
    //     Ok((warnings, _)) => {
    //         warnings.print();
    //         info!("    {} {}", "Compiled".green().bold(), folder_name);
    //     }
    //     Err(flaw) => {
    //         flaw.print();
    //         info!(
    //             "{} {} {}",
    //             "flaw:".red().bold(),
    //             "could not compile",
    //             folder_name
    //         );
    //     }
    // }
    GinCompiler::compile(&mut args);
}
