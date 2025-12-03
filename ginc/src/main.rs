use clap::Parser;
use ginc::{Args, compile};

fn main() {
    match compile(Args::parse()) {
        Ok((warnings, _)) => {
            for warning in warnings {
                eprintln!("{warning:#?}")
            }
        }
        Err(_) => todo!(),
    }
}
