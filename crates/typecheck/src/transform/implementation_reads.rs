use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use ast::{FileAst, prelude::Intern};

use super::constant_queries::ConstantQueries;
use crate::DefId;

#[derive(Clone, Default)]
pub(super) struct ImplementationReads {
    pub bindings: HashSet<DefId>,
    pub trait_declarations: HashSet<Intern<String>>,
    pub trait_candidates: bool,
    pub constants: HashMap<DefId, Option<ast::ConstValue>>,
}

impl ImplementationReads {
    pub fn unchanged(&self, before: &FileAst, after: &FileAst, queries: &ConstantQueries) -> bool {
        (!self.trait_candidates || before.defs == after.defs)
            && self
                .bindings
                .iter()
                .all(|id| before.defs.get(id) == after.defs.get(id))
            && self
                .trait_declarations
                .iter()
                .all(|name| before.tags.get(name) == after.tags.get(name))
            && self
                .constants
                .iter()
                .all(|(id, answer)| queries.value(*id) == *answer)
    }
}

pub(crate) struct ImplementationReadRecorder {
    reads: RefCell<ImplementationReads>,
}

impl ImplementationReadRecorder {
    pub fn new() -> Self {
        Self {
            reads: RefCell::default(),
        }
    }

    pub fn binding(&self, id: impl Into<DefId>) {
        self.reads.borrow_mut().bindings.insert(id.into());
    }

    pub fn constant(&self, id: DefId, answer: &Option<ast::ConstValue>) {
        self.reads.borrow_mut().constants.insert(id, answer.clone());
    }

    pub fn trait_declaration(&self, name: Intern<String>) {
        self.reads.borrow_mut().trait_declarations.insert(name);
    }

    pub fn trait_candidates(&self) {
        self.reads.borrow_mut().trait_candidates = true;
    }

    pub(super) fn into_reads(self) -> ImplementationReads {
        self.reads.into_inner()
    }
}
