use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use ast::ConstValue;

use crate::DefId;
use crate::typed::TypedCallableSignature;

/// Cross-file answers a finishing pass read, captured per file.
#[derive(Default, Clone)]
pub(crate) struct FinishReadLog {
    pub signatures: HashMap<DefId, Option<TypedCallableSignature>>,
    pub constants: HashMap<DefId, Option<ConstValue>>,
    pub scanned: bool,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Direct-mapped "already recorded" cache. Finishing re-reads the same handful
/// of cross-file answers hundreds of thousands of times per file, so a single
/// array probe replaces a hash lookup on the common path. A slot collision only
/// falls through to the exact map, so the cache can never drop a dependency.
const SEEN_SLOTS: usize = 512;

thread_local! {
    static LOG: RefCell<Option<FinishReadLog>> = const { RefCell::new(None) };
    static SEEN_SIGNATURES: RefCell<[Option<DefId>; SEEN_SLOTS]> =
        const { RefCell::new([None; SEEN_SLOTS]) };
    static SEEN_CONSTANTS: RefCell<[Option<DefId>; SEEN_SLOTS]> =
        const { RefCell::new([None; SEEN_SLOTS]) };
}

fn slot_of(id: DefId) -> usize {
    let address = id.0.as_str().as_ptr() as usize;
    (address ^ (id.1 as usize).wrapping_mul(0x9E37_79B9_7F4A_7C15)) & (SEEN_SLOTS - 1)
}

/// Enable/disable recording for the whole parallel finishing phase. Must stay
/// enabled while any worker is capturing, so it is toggled around the phase,
/// not around each file.
pub(crate) fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::Relaxed);
}

pub(crate) fn begin() {
    LOG.with(|log| *log.borrow_mut() = Some(FinishReadLog::default()));
    SEEN_SIGNATURES.with(|seen| seen.borrow_mut().fill(None));
    SEEN_CONSTANTS.with(|seen| seen.borrow_mut().fill(None));
}

pub(crate) fn end() -> FinishReadLog {
    LOG.with(|log| log.borrow_mut().take().unwrap_or_default())
}

pub(crate) fn record_signature(id: DefId, signature: Option<&TypedCallableSignature>) {
    if !ACTIVE.load(Ordering::Relaxed) || first_seen(&SEEN_SIGNATURES, id) {
        return;
    }
    LOG.with(|log| {
        if let Some(log) = log.borrow_mut().as_mut() {
            log.signatures
                .entry(id)
                .or_insert_with(|| signature.cloned());
        }
    });
}

pub(crate) fn record_constant(id: DefId, answer: &Option<ConstValue>) {
    if !ACTIVE.load(Ordering::Relaxed) || first_seen(&SEEN_CONSTANTS, id) {
        return;
    }
    LOG.with(|log| {
        if let Some(log) = log.borrow_mut().as_mut() {
            log.constants.entry(id).or_insert_with(|| answer.clone());
        }
    });
}

fn first_seen(
    seen: &'static std::thread::LocalKey<RefCell<[Option<DefId>; SEEN_SLOTS]>>,
    id: DefId,
) -> bool {
    seen.with(|seen| {
        let mut seen = seen.borrow_mut();
        let slot = &mut seen[slot_of(id)];
        if *slot == Some(id) {
            true
        } else {
            *slot = Some(id);
            false
        }
    })
}

pub(crate) fn record_scan() {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    LOG.with(|log| {
        if let Some(log) = log.borrow_mut().as_mut() {
            log.scanned = true;
        }
    });
}
