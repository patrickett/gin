use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::typed::{DefId, TypedCallableSignature};

use super::finish_reads;

pub(super) struct CallableSignatureScope<'a> {
    shared: &'a HashMap<DefId, TypedCallableSignature>,
    local: Cow<'a, HashMap<DefId, TypedCallableSignature>>,
    reads: Option<&'a SignatureReads>,
}

impl<'a> CallableSignatureScope<'a> {
    pub fn borrowed(
        shared: &'a HashMap<DefId, TypedCallableSignature>,
        local: &'a HashMap<DefId, TypedCallableSignature>,
    ) -> Self {
        Self {
            shared,
            local: Cow::Borrowed(local),
            reads: None,
        }
    }

    pub fn new(
        shared: &'a HashMap<DefId, TypedCallableSignature>,
        local: impl IntoIterator<Item = (DefId, TypedCallableSignature)>,
    ) -> Self {
        Self {
            shared,
            local: Cow::Owned(local.into_iter().collect()),
            reads: None,
        }
    }

    pub fn inputs(
        &self,
    ) -> (
        &HashMap<DefId, TypedCallableSignature>,
        &HashMap<DefId, TypedCallableSignature>,
    ) {
        (self.shared, self.local.as_ref())
    }

    pub fn recorded<'scope>(
        shared: &'scope HashMap<DefId, TypedCallableSignature>,
        local: &'scope HashMap<DefId, TypedCallableSignature>,
        reads: &'scope SignatureReads,
    ) -> CallableSignatureScope<'scope> {
        CallableSignatureScope {
            shared,
            local: Cow::Borrowed(local),
            reads: Some(reads),
        }
    }

    pub fn replace(&mut self, id: DefId, signature: TypedCallableSignature) {
        self.local.to_mut().insert(id, signature);
    }

    pub fn get(&self, id: &DefId) -> Option<&TypedCallableSignature> {
        if let Some(reads) = self.reads {
            reads.lookups.borrow_mut().insert(*id);
        }
        if let Some(local) = self.local.get(id) {
            return Some(local);
        }
        let shared = self.shared.get(id);
        finish_reads::record_signature(*id, shared);
        shared
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DefId, &TypedCallableSignature)> {
        if let Some(reads) = self.reads {
            reads.scanned.set(true);
        }
        finish_reads::record_scan();
        self.local.iter().chain(
            self.shared
                .iter()
                .filter(|(id, _)| !self.local.contains_key(id)),
        )
    }
}

#[derive(Default)]
pub(super) struct SignatureReads {
    lookups: RefCell<HashSet<DefId>>,
    scanned: Cell<bool>,
}

impl SignatureReads {
    pub fn into_parts(self) -> (HashSet<DefId>, bool) {
        (self.lookups.into_inner(), self.scanned.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::query::SourceParseExt;

    #[test]
    fn reads_include_failed_lookups_and_candidate_scans() {
        let shared = HashMap::new();
        let scope = CallableSignatureScope::new(&shared, []);
        let reads = SignatureReads::default();
        let (shared, local) = scope.inputs();
        let tracked = CallableSignatureScope::recorded(shared, local, &reads);
        let absent = DefId::named(ast::prelude::Intern::from_ref("absent"));
        assert!(tracked.get(&absent).is_none());
        assert_eq!(tracked.iter().count(), 0);
        let (lookups, scanned) = reads.into_parts();
        assert_eq!(lookups, HashSet::from([absent]));
        assert!(scanned);
    }

    #[test]
    fn lookup_matches_materialization_including_shadowing_and_overload_ordinals() {
        let parsed = format!("{}choose(x Word) Word: return x\nchoose(x ref Word) Word: return 1\nother(x Word) Word: return x\n", test_fixtures::preludes::WORD_PRELUDE)
            .parse_source_full();
        let typed = crate::transform::transform_file(parsed.ast, crate::FileId(0));
        let shared: HashMap<_, _> = typed
            .defs
            .iter()
            .map(|(id, bind)| (*id, TypedCallableSignature::from(bind)))
            .collect();
        assert!(shared.keys().any(|id| id.1 == 1));
        let mut local: Vec<_> = shared
            .iter()
            .filter(|(id, _)| id.1 == 1)
            .map(|(id, signature)| (*id, signature.clone()))
            .collect();
        local[0].1.call_capability = crate::typed::CallCapability::RuntimeOnly;
        let scope = CallableSignatureScope::new(&shared, local.clone());
        let mut materialized = shared.clone();
        materialized.extend(local);
        for id in shared
            .keys()
            .copied()
            .chain([DefId::named(ast::prelude::Intern::from_ref("missing"))])
        {
            assert_eq!(scope.get(&id), materialized.get(&id));
        }
        assert!(std::ptr::eq(
            scope
                .get(&DefId::named(ast::prelude::Intern::from_ref("other")))
                .unwrap(),
            shared
                .get(&DefId::named(ast::prelude::Intern::from_ref("other")))
                .unwrap()
        ));
    }
}
