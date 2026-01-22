use crate::{
    frontend::{parser::token_parser, prelude::*},
    source::Source,
};
use chumsky::input::Stream;
use std::{collections::BTreeMap, path::PathBuf};

pub trait Parsable<S> {
    fn to_ast(&self) -> Result<AstBranch, Vec<(ParserErrors<'_>, PathBuf)>>;
}

pub type ParserErrors<'a> = Vec<Rich<'a, Token<'a>>>;

impl<S: Lexable<S> + Source> Parsable<S> for S {
    fn to_ast(&self) -> Result<AstBranch, Vec<(ParserErrors<'_>, PathBuf)>> {
        let lexers = self.lex();

        let mut errs: Vec<(ParserErrors<'_>, PathBuf)> = Vec::new();
        let mut nodes = BTreeMap::new();

        for (lexer, path) in lexers {
            let token_stream = Stream::from_iter(lexer.map(|(t, _s)| t));
            let parser = token_parser();
            let (maybe_node, errors) = parser.parse(token_stream).into_output_errors();

            if let Some(node) = maybe_node {
                nodes.insert(path, node);
            } else {
                errs.push((errors, path));
            }
        }

        if !errs.is_empty() {
            return Err(errs);
        }

        // TODO: check imports, build branches from imports
        let branches = BTreeMap::new();

        Ok(AstBranch::new_root(branches, nodes))
    }
}
