//! Bounded lifecycle ledger, not lowering or checking of source `own` syntax.
//! Domain authority permits access; these tokens account for lifecycle responsibility.

use super::*;

mod elements;
mod tokens;

pub const OWNED_CAPACITY_MAX: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocator(u8);
/// Copying this ID aliases one ledger token; only an explicit move issues its successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageOwner(u8);
/// Opaque, self-contained value responsibility, with no modeled borrowed/nested payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedValue(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Initialization {
    Uninitialized,
    Initialized,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construction {
    Succeed,
    FailBeforeCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipObligations {
    pub storage: u8,
    pub values: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Ledger {
    allocators: u8,
    allocations: Vec<Allocation>,
    owners: Vec<Token>,
    values: Vec<Token>,
    // Per-resource active-frame masks. Ordinary moves may not consume unwind capacity.
    owner_returns: [u8; LIMIT],
    value_returns: [u8; LIMIT],
    resources: u8,
    destroyed: u16,
    elements: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token {
    resource: u8,
    scope: (u8, u16),
    live: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Allocation {
    backing: Place,
    allocator: Allocator,
    cells: Vec<Cell>,
    live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cell {
    knowledge: Initialization,
    // Actual resource/place bookkeeping survives forgetting; it grants no program permission.
    value: Option<(u8, Place)>,
}

fn return_count(reservations: &[u8; LIMIT]) -> usize {
    reservations
        .iter()
        .map(|mask| mask.count_ones() as usize)
        .sum()
}

impl Model {
    /// Harness observation, not an initialization-refining operation available to source code.
    pub fn ownership_obligations(&self) -> OwnershipObligations {
        OwnershipObligations {
            storage: self
                .ownership
                .allocations
                .iter()
                .filter(|allocation| allocation.live)
                .count() as u8,
            values: self.ownership.resources - self.ownership.destroyed.count_ones() as u8,
        }
    }

    fn ownership_transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        // All state is bounded and effect-free here. This is not real destructor rollback.
        let mut trial = self.clone();
        let result = operation(&mut trial)?;
        trial.assert_invariants();
        *self = trial;
        Ok(result)
    }

    fn owner_index(&self, owner: StorageOwner) -> Result<usize, Error> {
        let token = self.ownership.owners[usize::from(owner.0)];
        if !token.live {
            return Err(Error::OwnerMoved);
        }
        if token.scope != self.scope() {
            return Err(Error::OwnerNotHeld);
        }
        Ok(usize::from(token.resource))
    }

    fn value_resource(&self, value: OwnedValue) -> Result<u8, Error> {
        let token = self.ownership.values[usize::from(value.0)];
        if !token.live {
            return Err(Error::ValueMoved);
        }
        if token.scope != self.scope() {
            return Err(Error::ValueNotHeld);
        }
        Ok(token.resource)
    }

    fn cell(&self, owner: StorageOwner, index: u8) -> Result<(usize, Cell), Error> {
        let allocation = self.owner_index(owner)?;
        let cell = self.ownership.allocations[allocation]
            .cells
            .get(usize::from(index))
            .ok_or(Error::ElementBounds)?;
        Ok((allocation, *cell))
    }

    fn require_initialization(cell: Cell, expected: Initialization) -> Result<(), Error> {
        match cell.knowledge {
            Initialization::Unknown => Err(Error::InitializationUnknown),
            actual if actual == expected => Ok(()),
            Initialization::Initialized => Err(Error::AlreadyInitialized),
            Initialization::Uninitialized => Err(Error::Uninitialized),
        }
    }

    pub(super) fn managed_backing(&self, place: Place) -> bool {
        self.ownership
            .allocations
            .iter()
            .any(|allocation| allocation.backing == place)
    }

    pub(super) fn managed_place(&self, place: Place) -> bool {
        self.managed_backing(place) || self.ownership.elements & bit(place) != 0
    }

    pub(super) fn require_no_frame_ownership(&self) -> Result<(), Error> {
        let live = self
            .ownership
            .owners
            .iter()
            .chain(&self.ownership.values)
            .any(|token| token.live && token.scope == self.scope());
        let pending = self
            .ownership
            .owner_returns
            .iter()
            .chain(&self.ownership.value_returns)
            .any(|mask| mask & (1u8 << self.frame()) != 0);
        if live || pending {
            Err(Error::OutstandingOwnership)
        } else {
            Ok(())
        }
    }

    pub fn check_ownership_complete(&self) -> Result<(), Error> {
        let live = self
            .ownership
            .allocations
            .iter()
            .any(|allocation| allocation.live)
            || self.ownership.values.iter().any(|token| token.live);
        if live {
            Err(Error::OutstandingOwnership)
        } else {
            Ok(())
        }
    }

    pub(super) fn assert_ownership_invariants(&self) {
        let ledger = &self.ownership;
        self.assert_return_capacity(&ledger.owners, &ledger.owner_returns);
        self.assert_return_capacity(&ledger.values, &ledger.value_returns);
        assert!(ledger.allocations.len() <= LIMIT);
        assert!(ledger.owners.len() <= LIMIT);
        assert!(ledger.values.len() <= LIMIT);
        assert!(usize::from(ledger.resources) <= LIMIT);
        for (index, allocation) in ledger.allocations.iter().enumerate() {
            let count = ledger
                .owners
                .iter()
                .filter(|token| token.live && usize::from(token.resource) == index)
                .count();
            assert_eq!(count, usize::from(allocation.live));
            assert_eq!(self.place_state(allocation.backing).alive, allocation.live);
            assert!(allocation.cells.len() <= usize::from(OWNED_CAPACITY_MAX));
            for cell in &allocation.cells {
                if cell.knowledge != Initialization::Unknown {
                    assert_eq!(
                        cell.value.is_some(),
                        cell.knowledge == Initialization::Initialized
                    );
                }
                if let Some((_, place)) = cell.value {
                    assert!(allocation.live);
                    assert!(self.place_state(place).alive);
                }
            }
        }
        self.assert_value_locations();
        for token in ledger
            .owners
            .iter()
            .chain(&ledger.values)
            .filter(|token| token.live)
        {
            assert!(token.scope.0 <= self.frame());
            assert_eq!(self.epochs[usize::from(token.scope.0)], token.scope.1);
        }
    }

    fn assert_value_locations(&self) {
        let ledger = &self.ownership;
        for resource in 0..ledger.resources {
            let tokens = ledger
                .values
                .iter()
                .filter(|token| token.live && token.resource == resource)
                .count();
            let cells = ledger
                .allocations
                .iter()
                .flat_map(|allocation| &allocation.cells)
                .filter(|cell| cell.value.is_some_and(|value| value.0 == resource))
                .count();
            let destroyed = usize::from(ledger.destroyed & (1u16 << resource) != 0);
            assert_eq!(tokens + cells + destroyed, 1);
        }
        for (index, place) in self.places.iter().enumerate() {
            let id = Place(index as u8);
            if ledger.elements & bit(id) != 0 {
                let cells = ledger
                    .allocations
                    .iter()
                    .flat_map(|allocation| &allocation.cells)
                    .filter(|cell| cell.value.is_some_and(|value| value.1 == id))
                    .count();
                assert_eq!(cells, usize::from(place.alive));
            }
        }
    }

    fn assert_return_capacity(&self, tokens: &[Token], reservations: &[u8; LIMIT]) {
        assert!(tokens.len() + return_count(reservations) <= LIMIT);
        for (resource, mask) in reservations.iter().enumerate() {
            assert_eq!(mask & 1, 0, "root needs no return reservation");
            assert_eq!(mask >> (CALL_DEPTH_MAX + 1), 0);
            for frame in 1..=CALL_DEPTH_MAX {
                if mask & (1u8 << frame) != 0 {
                    assert!(frame <= self.frame());
                    assert!(tokens.iter().any(|token| {
                        token.live
                            && usize::from(token.resource) == resource
                            && token.scope.0 >= frame
                    }));
                }
            }
        }
    }
}
