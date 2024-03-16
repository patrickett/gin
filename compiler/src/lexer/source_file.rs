use std::{
    char,
    fs::{self, canonicalize},
    path::Path,
};

use crate::{expr::Expr, module::GinModule, parse::Parser, token::Token};

use super::Lexer;

// having this file wrapper allows us to do some cool things in the future
// 1. dynamically mutate files based on user imput
// 2. reload files with their changes
#[derive(Debug, Clone)]
pub struct SourceFile {
    // we keep the full path in case something else imports
    // so we can just reuse the same already parsed file
    full_path: String,
    // we use a vec here so we can mutate in the future
    // Chars iter is readonly
    chars: Vec<char>,
    index: usize,
}
// TODO: ask runtime to run SourceFile.to_module() -> GinModule

impl SourceFile {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn full_path(&self) -> &String {
        &self.full_path
    }

    pub fn new(p: String) -> Self {
        let path = Path::new(&p);
        let path = canonicalize(path).expect("failed to get real path");
        if !path.exists() {
            // TODO: prompt user don't error
            eprintln!("No such file or directory: {}", path.display());
        }

        let content = fs::read_to_string(path.clone()).expect("msg");

        Self {
            // this is scary if the file is really large
            chars: content.chars().collect(),
            index: 0,
            full_path: path.to_str().expect("msg").to_string(),
        }
    }

    pub fn to_module(&mut self) -> GinModule {
        GinModule::new(self.full_path().to_owned(), self.debug())
    }

    // if changes to file were made on disk, this allows us to re-lex-parse the file
    pub fn reload() {}

    // Print ast for source file
    pub fn debug(&self) -> Vec<Expr> {
        let mut lexer = Lexer::new();
        lexer.set_content(self.to_owned());
        let parser = Parser::new(Some(lexer));
        parser.collect()
    }

    // Print ast for source file
    pub fn tokens(&self) -> Vec<Token> {
        let mut lexer = Lexer::new();
        lexer.set_content(self.to_owned());
        lexer.collect()
    }
}

impl Iterator for SourceFile {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.chars.len() {
            return None;
        }
        let a = Some(self.chars[self.index]);
        self.index += 1;
        a
    }
}
