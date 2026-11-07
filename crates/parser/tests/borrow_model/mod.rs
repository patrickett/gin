//! A bounded semantic experiment, not a parser feature or production borrow checker.
//!
//! Places, authority, effects, and validity bounds are supplied by test scenarios.
//! They are not inferred from source text. No legacy frontend types are used.

mod access;
mod calls;
mod memory;
mod ownership;
pub use ownership::OWNED_CAPACITY_MAX;
pub use ownership::{
    Allocator, Construction, Initialization, OwnedValue, OwnershipObligations, StorageOwner,
};

pub const LIMIT: usize = 16;
const CALL_DEPTH_MAX: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Domain(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lease(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fact(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    SharedView,
    ReadOnlySlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Limit,
    NoAuthority,
    DuplicateAuthority,
    ReadOnly,
    Invalidated,
    LeaseConflict,
    LeaseExpired,
    ExclusiveCopy,
    OutstandingLease,
    Validity,
    OriginFamily,
    HandleType,
    SupportConflict,
    NotSlot,
    SlotWrite,
    ForgottenFact,
    UnknownEffect,
    NoCaller,
    SetupOnly,
    InaccessibleHandle,
    OwnerMoved,
    ValueMoved,
    DuplicateOwnership,
    OwnerNotHeld,
    ValueNotHeld,
    ElementBounds,
    AlreadyInitialized,
    Uninitialized,
    InitializationUnknown,
    WrongAllocator,
    LiveElements,
    OutstandingOwnership,
    ConstructionFailed,
    OwnershipRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Read(Handle),
    Write(Handle),
    Invalidate(Handle),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlaceState {
    domain: Domain,
    /// Larger values mean longer validity; these are supplied contracts, not lifetimes inferred.
    extent: u8,
    dependencies: u16,
    support_generations: [u16; LIMIT],
    generation: u16,
    alive: bool,
    slot: Option<SlotState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotState {
    value: Descriptor,
    family: u16,
    mode: Mode,
    required_extent: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Descriptor {
    target: Place,
    mode: Mode,
    scope: (u8, u16),
    dependencies: u16,
    support_generations: [u16; LIMIT],
    lease: Option<Lease>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LeaseState {
    descriptor: Descriptor,
    frame: u8,
    active: bool,
    view_issued: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FactState {
    handle: Handle,
    revision: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    policy: Policy,
    epochs: [u16; CALL_DEPTH_MAX as usize + 1],
    owners: Vec<u8>,
    revisions: Vec<u16>,
    places: Vec<PlaceState>,
    handles: Vec<Descriptor>,
    leases: Vec<LeaseState>,
    facts: Vec<FactState>,
    calls: Vec<Vec<Domain>>,
    ownership: ownership::Ledger,
}

impl Model {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            epochs: [0; CALL_DEPTH_MAX as usize + 1],
            owners: Vec::new(),
            revisions: Vec::new(),
            places: Vec::new(),
            handles: Vec::new(),
            leases: Vec::new(),
            facts: Vec::new(),
            calls: Vec::new(),
            ownership: ownership::Ledger::default(),
        }
    }

    fn frame(&self) -> u8 {
        assert!(self.calls.len() <= usize::from(CALL_DEPTH_MAX));
        self.calls.len() as u8
    }

    fn scope(&self) -> (u8, u16) {
        let frame = self.frame();
        (frame, self.epochs[usize::from(frame)])
    }

    fn place_state(&self, place: Place) -> &PlaceState {
        assert!(usize::from(place.0) < self.places.len());
        &self.places[usize::from(place.0)]
    }

    fn descriptor(&self, handle: Handle) -> Descriptor {
        assert!(usize::from(handle.0) < self.handles.len());
        self.handles[usize::from(handle.0)]
    }

    pub fn handle_mode(&self, handle: Handle) -> Mode {
        self.descriptor(handle).mode
    }

    fn domain(&self, descriptor: Descriptor) -> Domain {
        self.place_state(descriptor.target).domain
    }

    fn require_authority(&self, domain: Domain) -> Result<(), Error> {
        assert!(usize::from(domain.0) < self.owners.len());
        if self.owners[usize::from(domain.0)] != self.frame() {
            return Err(Error::NoAuthority);
        }
        Ok(())
    }

    fn push_handle(&mut self, mut descriptor: Descriptor) -> Result<Handle, Error> {
        if self.handles.len() == LIMIT {
            return Err(Error::Limit);
        }
        descriptor.scope = self.scope();
        let handle = Handle(self.handles.len() as u8);
        self.handles.push(descriptor);
        assert!(self.handles.len() <= LIMIT);
        Ok(handle)
    }

    pub fn assert_invariants(&self) {
        self.assert_ownership_invariants();
        assert!(self.owners.len() <= LIMIT);
        assert!(self.places.len() <= LIMIT);
        assert!(self.handles.len() <= LIMIT);
        assert!(self.leases.len() <= LIMIT);
        assert!(self.facts.len() <= LIMIT);
        assert_eq!(self.owners.len(), self.revisions.len());
        for (index, owner) in self.owners.iter().enumerate() {
            assert!(*owner <= self.frame());
            let domain = Domain(index as u8);
            let mut expected = 0u8;
            for (frame, domains) in self.calls.iter().enumerate() {
                if domains.contains(&domain) {
                    expected = frame as u8 + 1;
                }
            }
            assert_eq!(*owner, expected);
        }
        for descriptor in &self.handles {
            assert!(usize::from(descriptor.target.0) < self.places.len());
            assert_ne!(descriptor.dependencies & bit(descriptor.target), 0);
        }
        for lease in &self.leases {
            if lease.active {
                assert!(lease.frame <= self.frame());
                let domain = self.domain(lease.descriptor);
                assert_eq!(self.owners[usize::from(domain.0)], lease.frame);
                assert!(self.require_valid(lease.descriptor).is_ok());
            }
        }
        for place in &self.places {
            if let Some(slot) = &place.slot {
                assert!(slot.required_extent >= place.extent);
            }
        }
    }
}

fn bit(place: Place) -> u16 {
    assert!(usize::from(place.0) < LIMIT);
    1u16 << place.0
}
