use clap::Parser;
use ginc::{Args, compile};

fn main() {
    match compile(Args::parse()) {
        Ok(warnings) => {
            for warning in warnings {
                eprintln!("{warning:#?}")
            }
        }
        Err(_) => todo!(),
    }
}
