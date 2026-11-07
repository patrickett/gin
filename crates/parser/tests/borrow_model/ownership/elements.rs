use super::*;

impl Model {
    pub fn initialization(&self, owner: StorageOwner, index: u8) -> Result<Initialization, Error> {
        Ok(self.cell(owner, index)?.1.knowledge)
    }

    /// Region locator only, not an initialized T. Generic writes/destruction remain forbidden.
    pub fn borrow_storage(&mut self, owner: StorageOwner, mode: Mode) -> Result<Handle, Error> {
        let allocation = self.owner_index(owner)?;
        self.new_handle(self.ownership.allocations[allocation].backing, mode)
    }

    pub fn forget_initialization(&mut self, owner: StorageOwner, index: u8) -> Result<(), Error> {
        self.ownership_transaction(|model| {
            let (allocation, _) = model.cell(owner, index)?;
            let backing = model.ownership.allocations[allocation].backing;
            model.require_access(model.place_descriptor(backing, Mode::Write), Mode::Write)?;
            model.advance_revision(model.place_state(backing).domain)?;
            model.ownership.allocations[allocation].cells[usize::from(index)].knowledge =
                Initialization::Unknown;
            Ok(())
        })
    }

    pub fn initialize_element(
        &mut self,
        owner: StorageOwner,
        index: u8,
        value: OwnedValue,
    ) -> Result<(), Error> {
        self.ownership_transaction(|model| {
            let (allocation, cell) = model.cell(owner, index)?;
            Self::require_initialization(cell, Initialization::Uninitialized)?;
            let resource = model.value_resource(value)?;
            let backing = model.ownership.allocations[allocation].backing;
            model.require_access(model.place_descriptor(backing, Mode::Write), Mode::Write)?;
            let storage = model.place_state(backing);
            let domain = storage.domain;
            let place = model.new_place(domain, storage.extent, Some(backing))?;
            model.ownership.elements |= bit(place);
            model.advance_revision(domain)?;
            model.ownership.values[usize::from(value.0)].live = false;
            // Packing consumes standalone value responsibility; the cell now carries it.
            model.ownership.value_returns[usize::from(resource)] = 0;
            model.ownership.allocations[allocation].cells[usize::from(index)] = Cell {
                knowledge: Initialization::Initialized,
                value: Some((resource, place)),
            };
            Ok(())
        })
    }

    pub fn construct_element(
        &mut self,
        owner: StorageOwner,
        index: u8,
        outcome: Construction,
    ) -> Result<(), Error> {
        self.ownership_transaction(|model| {
            let (allocation, cell) = model.cell(owner, index)?;
            Self::require_initialization(cell, Initialization::Uninitialized)?;
            let backing = model.ownership.allocations[allocation].backing;
            model.require_access(model.place_descriptor(backing, Mode::Write), Mode::Write)?;
            if outcome == Construction::FailBeforeCommit {
                return Err(Error::ConstructionFailed);
            }
            let value = model.new_owned_value()?;
            model.initialize_element(owner, index, value)
        })
    }

    pub fn borrow_element(
        &mut self,
        owner: StorageOwner,
        index: u8,
        mode: Mode,
    ) -> Result<Handle, Error> {
        let (_, cell) = self.cell(owner, index)?;
        Self::require_initialization(cell, Initialization::Initialized)?;
        self.new_handle(cell.value.expect("initialized value").1, mode)
    }

    fn remove_element(&mut self, owner: StorageOwner, index: u8) -> Result<u8, Error> {
        let (allocation, cell) = self.cell(owner, index)?;
        Self::require_initialization(cell, Initialization::Initialized)?;
        let (resource, place) = cell.value.expect("initialized value");
        self.invalidate_owned_place(place)?;
        self.ownership.allocations[allocation].cells[usize::from(index)] = Cell {
            knowledge: Initialization::Uninitialized,
            value: None,
        };
        Ok(resource)
    }

    pub fn extract_element(&mut self, owner: StorageOwner, index: u8) -> Result<OwnedValue, Error> {
        self.ownership_transaction(|model| {
            let resource = model.remove_element(owner, index)?;
            model.push_value(resource, model.scope())
        })
    }

    pub fn destroy_element(&mut self, owner: StorageOwner, index: u8) -> Result<(), Error> {
        self.ownership_transaction(|model| {
            let resource = model.remove_element(owner, index)?;
            model.ownership.destroyed |= 1u16 << resource;
            Ok(())
        })
    }
}
