use std::{
    borrow::BorrowMut,
    fs::{self, canonicalize, File},
    path::Path,
    time::SystemTime,
};

use std::io::prelude::*;

use super::{
    compiler_error::CompilerError,
    parser::{ast::Node, Parser},
    user_input::ask_yes_no,
    validator::validate,
};
use crate::handle_error;

// TODO: read_file -> Result<SourceFile, UserDeny>

// having this file wrapper allows us to do some cool things in the future
// 1. dynamically mutate files based on user imput
// 2. reload files with their changes
pub struct SourceFile {
    full_path: String,
    last_modified: Option<SystemTime>,
    content: Option<String>,
}

impl SourceFile {
    pub fn full_path(&self) -> String {
        self.full_path.clone()
    }

    pub fn get_current_modified(&self) -> SystemTime {
        let metadata = fs::metadata(self.full_path.to_string()).expect("user will have corrected");
        let modified = metadata.modified().expect("has last modified date");
        modified
    }

    pub fn been_modified(&mut self) -> bool {
        let current_modified = self.get_current_modified();
        let last_modified = match self.last_modified {
            Some(m) => m,
            None => {
                self.read_from_disk();
                self.last_modified.expect("failed to read last_modified")
            }
        };

        current_modified > last_modified
    }

    pub fn new(path: &Path) -> Self {
        let path = canonicalize(path).expect("failed to get real path");
        let full_path = path.to_str().expect("msg").to_string();

        Self {
            full_path,
            content: None,
            last_modified: None,
        }
    }

    pub fn to_module(&mut self, parser: &mut Parser) -> Vec<Node> {
        parser.set_content(self);
        let ast_attempt: Vec<Result<Node, CompilerError>> = parser.borrow_mut().collect();
        let maybe_ast = handle_error(ast_attempt);
        if let Some(ast) = maybe_ast {
            let validate_result = validate(ast);
            match validate_result {
                Ok(compile_ready_ast) => compile_ready_ast,
                Err(compiler_error) => {
                    eprint!("{}", compiler_error);
                    panic!("");
                }
            }
        } else {
            panic!("")
        }
    }

    pub fn get_content(&mut self) -> String {
        // check modified
        let modified = false;

        if modified {
            let question = format!("The file {} has been modified since it was last used. Do you want to use the new changes? ", &self.full_path);
            let use_modified_file = ask_yes_no(&question);

            if use_modified_file {
                self.read_from_disk();
            }
        }

        let c = self.content.clone();

        match c {
            Some(content) => content,
            None => {
                self.read_from_disk();
                self.content.clone().unwrap()
            }
        }
    }

    fn read_from_disk(&mut self) {
        // TODO: check modified
        let mut content = String::new();

        let mut file = File::open(&self.full_path).expect("failed to open file");
        let metadata = file.metadata().expect("failed to get metadata for file");
        self.last_modified = Some(
            metadata
                .modified()
                .expect("failed to read modified metadata for file"),
        );

        file.read_to_string(&mut content)
            .expect("failed to read file to string");

        self.content = Some(content);
    }
}
