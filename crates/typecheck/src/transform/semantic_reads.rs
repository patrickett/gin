use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};

use ast::ty::{DeclarationKey, TypeId};

#[derive(Default)]
pub(crate) struct SemanticReads {
    pub declarations: HashSet<TypeId>,
    pub handles: HashSet<DeclarationKey>,
    pub scanned: bool,
    pub bodies: HashSet<crate::DefId>,
    pub references: HashMap<crate::ExprId, crate::typed::ReferenceValue>,
    pub written: HashSet<crate::ExprId>,
}

thread_local! {
    static READS: RefCell<Option<SemanticReads>> = const { RefCell::new(None) };
}

static ACTIVE_RECORDERS: AtomicU32 = AtomicU32::new(0);

pub(crate) struct SemanticReadGuard {
    active: bool,
}

impl SemanticReadGuard {
    pub fn begin() -> Self {
        READS.with_borrow_mut(|reads| {
            assert!(reads.is_none(), "semantic dependency recording cannot nest");
            *reads = Some(SemanticReads::default());
        });
        let previous = ACTIVE_RECORDERS.fetch_add(1, Ordering::Relaxed);
        assert!(previous < u32::MAX);
        Self { active: true }
    }

    pub fn finish(mut self) -> SemanticReads {
        let reads = READS.with_borrow_mut(|reads| {
            reads
                .take()
                .expect("semantic dependency recording not active")
        });
        self.active = false;
        assert!(ACTIVE_RECORDERS.fetch_sub(1, Ordering::Relaxed) > 0);
        reads
    }
}

impl Drop for SemanticReadGuard {
    fn drop(&mut self) {
        if self.active {
            READS.with_borrow_mut(|reads| {
                reads.take();
            });
            assert!(ACTIVE_RECORDERS.fetch_sub(1, Ordering::Relaxed) > 0);
        }
    }
}

fn record(update: impl FnOnce(&mut SemanticReads)) {
    if ACTIVE_RECORDERS.load(Ordering::Relaxed) == 0 {
        return;
    }
    READS.with_borrow_mut(|reads| {
        if let Some(reads) = reads {
            update(reads);
        }
    });
}

pub(crate) fn declaration(id: TypeId) {
    record(|reads| {
        reads.declarations.insert(id);
    });
}

pub(crate) fn handle(key: &DeclarationKey) {
    record(|reads| {
        reads.handles.insert(key.clone());
    });
}

pub(crate) fn scan() {
    record(|reads| {
        reads.scanned = true;
    });
}

pub(crate) fn body(id: crate::DefId) {
    record(|reads| {
        reads.bodies.insert(id);
    });
}

pub(crate) fn reference(id: crate::ExprId, value: &crate::typed::ReferenceValue) {
    record(|reads| {
        if !reads.written.contains(&id) {
            reads.references.entry(id).or_insert_with(|| value.clone());
        }
    });
}

pub(crate) fn reference_written(id: crate::ExprId) {
    record(|reads| {
        reads.written.insert(id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_dependencies_capture_only_values_read_before_local_writes() {
        let recording = SemanticReadGuard::begin();
        let initial = crate::typed::ReferenceValue::default();
        reference(crate::ExprId(0), &initial);
        reference_written(crate::ExprId(0));
        reference_written(crate::ExprId(1));
        let mut changed = initial.clone();
        changed
            .sources
            .insert(crate::typed::ReferenceSource::Unknown);
        reference(crate::ExprId(0), &changed);
        reference(crate::ExprId(1), &changed);
        let reads = recording.finish();
        assert_eq!(
            reads.references,
            HashMap::from([(crate::ExprId(0), initial)])
        );
        assert_eq!(
            reads.written,
            HashSet::from([crate::ExprId(0), crate::ExprId(1)])
        );
    }

    #[test]
    fn unwinding_clears_recording_before_the_next_query() {
        let panic = std::panic::catch_unwind(|| {
            let _recording = SemanticReadGuard::begin();
            scan();
            panic!("cancelled query");
        });
        assert!(panic.is_err());
        let recording = SemanticReadGuard::begin();
        let id = crate::DefId::named(internment::Intern::from_ref("missing"));
        body(id);
        let reads = recording.finish();
        assert_eq!(reads.bodies, HashSet::from([id]));
        assert!(!reads.scanned);
        assert!(reads.declarations.is_empty());
    }
}
