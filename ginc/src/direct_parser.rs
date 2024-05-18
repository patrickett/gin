use super::{compiler_error::CompilerError, parser::ast::Node};
use std::{
    fs::File,
    io::{self, BufReader, Read},
};

pub struct DirectParser {
    reader: BufReader<File>,
}

impl DirectParser {
    /// Create a parser for a given path.
    /// This will open the file and hold a BufReader
    pub fn new(file_path: &str) -> io::Result<Self> {
        let file = File::open(file_path)?;
        let reader = BufReader::new(file);
        Ok(Self { reader })
    }

    // fn next_char(&mut self) -> Option<char> {}

    fn next_word(&mut self) -> Option<io::Result<String>> {
        let mut word = String::new();
        let mut byte_buf = [0; 1]; // Buffer to read one byte at a time

        loop {
            match self.reader.read_exact(&mut byte_buf) {
                Ok(_) => {
                    let c = byte_buf[0] as char;
                    if c == ' ' {
                        if word.is_empty() {
                            return None;
                        } else {
                            return Some(Ok(word));
                        }
                    }
                    word.push(c);
                }
                Err(_) => {
                    if word.is_empty() {
                        return None;
                    } else {
                        return Some(Ok(word));
                    }
                }
            }
        }
    }

    fn next_node(&self) -> Option<Result<Node, CompilerError>> {
        None
    }
}

impl Iterator for DirectParser {
    type Item = Result<Node, CompilerError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_node()
    }
}
