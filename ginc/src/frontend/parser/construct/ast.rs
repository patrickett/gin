use crate::frontend::prelude::*;
use chumsky::container::Container;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

pub type TagMap = HashMap<TagName, Documented<Params<TagValue>>>;
pub type DefMap = HashMap<DefName, Documented<Params<DefValue>>>;

impl Container<Item> for (TagMap, DefMap) {
    fn push(&mut self, item: Item) {
        match item.value {
            ItemValue::TagValue(tag_name, params) => {
                let doc = Documented {
                    item: params,
                    doc: item.doc_comment,
                };

                self.0.insert(tag_name, doc);
            }
            ItemValue::DefValue(def_name, params) => {
                let doc = Documented {
                    item: params,
                    doc: item.doc_comment,
                };
                self.1.insert(def_name, doc);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
/// Output of parsing a gin file
pub struct AstNode {
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
pub struct AstBranch {
    root: bool,
    /// These are folders on the filesystem
    pub branches: BTreeMap<PathBuf, AstBranch>,
    /// These are files on the filesystem
    pub nodes: BTreeMap<PathBuf, AstNode>, // keeps files sorted for deterministic iteration
                                           // pub symbol_index: HashMap<String, SymbolInfo>,
                                           // pub module_deps: Vec<String>,
}

impl AstBranch {
    pub fn is_root(&self) -> bool {
        self.root
    }

    pub fn new_root(
        branches: BTreeMap<PathBuf, AstBranch>,
        nodes: BTreeMap<PathBuf, AstNode>,
    ) -> Self {
        Self {
            root: true,
            branches,
            nodes,
        }
    }

    pub fn new(branches: BTreeMap<PathBuf, AstBranch>, nodes: BTreeMap<PathBuf, AstNode>) -> Self {
        Self {
            root: false,
            branches,
            nodes,
        }
    }
}
