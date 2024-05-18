use std::io::{self, Write};

pub fn ask_yes_no(question: &str) -> bool {
    loop {
        print!("{} [Y/n]: ", question);
        io::stdout().flush().expect("Failed to flush stdout");

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read line");

        let response = input.trim().to_lowercase();

        match response.as_str() {
            "y" | "yes" | "" => {
                break true;
            }
            "n" => {
                return false;
            }
            _ => {
                println!("Invalid input. Please enter 'yes', 'no', or 'retry'.");
            }
        }
    }
}

// fn ask_yes_no_retry(question: String) {
//     loop {
//         println!("Do you want to proceed? (yes/no)");

//         let mut input = String::new();
//         io::stdin()
//             .read_line(&mut input)
//             .expect("Failed to read line");

//         let response = input.trim().to_lowercase();

//         match response.as_str() {
//             "yes" => {
//                 println!("Proceeding...");
//                 break;
//             }
//             "no" => {
//                 println!("Exiting...");
//                 return;
//             }
//             "retry" => {
//                 println!("Retrying...");
//             }
//             _ => {
//                 println!("Invalid input. Please enter 'yes', 'no', or 'retry'.");
//             }
//         }
//     }
// }
