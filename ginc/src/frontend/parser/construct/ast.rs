use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use chumsky::container::Container;

use crate::frontend::prelude::*;

pub type TagMap = HashMap<TagName, Params<TagValue>>;
pub type DefMap = HashMap<DefName, Params<DefValue>>;

impl Container<Item> for (TagMap, DefMap) {
    fn push(&mut self, item: Item) {
        match item.value {
            ItemValue::TagValue(tag_name, params) => {
                self.0.insert(tag_name, params);
            }
            ItemValue::DefValue(def_name, params) => {
                self.1.insert(def_name, params);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
/// Output of parsing a gin file
pub struct ParsedFile {
    pub imports: Vec<Import>, // import specifiers
    pub tags: TagMap,
    pub defs: DefMap,
    // pub path: PathBuf,
    // pub root: Option<NodeRef>,         // root node of this file's AST
    // pub nodes: HashMap<Uuid, AstNode>, // all nodes keyed by id
    // pub diagnostics: Vec<String>,
    // pub version: u64, // for incremental updates
}

#[derive(Debug, Clone)]
pub struct ParsedFolder {
    pub subfolders: BTreeMap<PathBuf, ParsedFolder>,
    pub files: BTreeMap<PathBuf, ParsedFile>, // keeps files sorted for deterministic iteration
                                              // pub symbol_index: HashMap<String, SymbolInfo>,
                                              // pub module_deps: Vec<String>,
}
