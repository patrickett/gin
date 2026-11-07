use ginfmt::{Options, print_unverified, verify};
use std::io::Read;

fn main() {
    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source).unwrap();
    let formatted = print_unverified(&source, Options::default()).unwrap();
    print!("{formatted}");
    eprintln!("semantics {:?}", verify::check_semantics(&source, &formatted));
    eprintln!("comments {:?}", verify::check_comments(&source, &formatted));
    let again = print_unverified(&formatted, Options::default()).map_err(|error| error.to_string());
    eprintln!("stable {}", again.as_ref().is_ok_and(|text| *text == formatted));
    if let Ok(again) = again {
        for (index, (left, right)) in formatted.lines().zip(again.lines()).enumerate() {
            if left != right {
                eprintln!("line {index}: {left:?} vs {right:?}");
                break;
            }
        }
    }
}
