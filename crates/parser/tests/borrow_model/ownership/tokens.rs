use super::*;

impl Model {
    pub fn new_allocator(&mut self) -> Result<Allocator, Error> {
        if self.frame() != 0 {
            return Err(Error::SetupOnly);
        }
        if usize::from(self.ownership.allocators) == LIMIT {
            return Err(Error::Limit);
        }
        let allocator = Allocator(self.ownership.allocators);
        self.ownership.allocators += 1;
        Ok(allocator)
    }

    pub fn reserve_storage(
        &mut self,
        domain: Domain,
        allocator: Allocator,
        capacity: u8,
        extent: u8,
    ) -> Result<StorageOwner, Error> {
        self.ownership_transaction(|model| {
            assert!(allocator.0 < model.ownership.allocators);
            if capacity > OWNED_CAPACITY_MAX || model.ownership.allocations.len() == LIMIT {
                return Err(Error::Limit);
            }
            let backing = model.new_place(domain, extent, None)?;
            model.require_access(model.place_descriptor(backing, Mode::Write), Mode::Write)?;
            let resource = model.ownership.allocations.len() as u8;
            model.ownership.allocations.push(Allocation {
                backing,
                allocator,
                cells: vec![
                    Cell {
                        knowledge: Initialization::Uninitialized,
                        value: None
                    };
                    usize::from(capacity)
                ],
                live: true,
            });
            model.push_owner(resource, model.scope())
        })
    }

    fn push_owner(&mut self, resource: u8, scope: (u8, u16)) -> Result<StorageOwner, Error> {
        if self.ownership.owners.len() + return_count(&self.ownership.owner_returns) >= LIMIT {
            return Err(Error::Limit);
        }
        let owner = StorageOwner(self.ownership.owners.len() as u8);
        self.ownership.owners.push(Token {
            resource,
            scope,
            live: true,
        });
        Ok(owner)
    }

    pub(super) fn push_value(
        &mut self,
        resource: u8,
        scope: (u8, u16),
    ) -> Result<OwnedValue, Error> {
        if self.ownership.values.len() + return_count(&self.ownership.value_returns) >= LIMIT {
            return Err(Error::Limit);
        }
        let value = OwnedValue(self.ownership.values.len() as u8);
        self.ownership.values.push(Token {
            resource,
            scope,
            live: true,
        });
        Ok(value)
    }

    pub fn new_owned_value(&mut self) -> Result<OwnedValue, Error> {
        self.ownership_transaction(|model| {
            if usize::from(model.ownership.resources) == LIMIT {
                return Err(Error::Limit);
            }
            let resource = model.ownership.resources;
            model.ownership.resources += 1;
            model.push_value(resource, model.scope())
        })
    }

    fn move_owner(&mut self, owner: StorageOwner, scope: (u8, u16)) -> Result<StorageOwner, Error> {
        let resource = self.ownership.owners[usize::from(owner.0)].resource;
        let result = self.push_owner(resource, scope)?;
        self.ownership.owners[usize::from(owner.0)].live = false;
        Ok(result)
    }

    fn move_value(&mut self, value: OwnedValue, scope: (u8, u16)) -> Result<OwnedValue, Error> {
        let resource = self.value_resource(value)?;
        let result = self.push_value(resource, scope)?;
        self.ownership.values[usize::from(value.0)].live = false;
        Ok(result)
    }

    pub fn transfer_owner(&mut self, owner: StorageOwner) -> Result<StorageOwner, Error> {
        self.ownership_transaction(|model| {
            model.owner_index(owner)?;
            model.move_owner(owner, model.scope())
        })
    }

    pub fn transfer_value(&mut self, value: OwnedValue) -> Result<OwnedValue, Error> {
        self.ownership_transaction(|model| model.move_value(value, model.scope()))
    }

    pub fn enter_owner_call(
        &mut self,
        owner: StorageOwner,
        domains: &[Domain],
    ) -> Result<StorageOwner, Error> {
        let (mut owners, values) = self.enter_ownership_call(&[owner], &[], domains)?;
        assert_eq!(owners.len(), 1);
        assert!(values.is_empty());
        Ok(owners.pop().unwrap())
    }

    pub fn enter_ownership_call(
        &mut self,
        owners: &[StorageOwner],
        values: &[OwnedValue],
        domains: &[Domain],
    ) -> Result<(Vec<StorageOwner>, Vec<OwnedValue>), Error> {
        self.ownership_transaction(|model| {
            model.require_ownership_arguments(owners, values)?;
            model.enter_call(domains)?;
            let frame = 1u8 << model.frame();
            for owner in owners {
                let resource = model.ownership.owners[usize::from(owner.0)].resource;
                model.ownership.owner_returns[usize::from(resource)] |= frame;
            }
            for value in values {
                let resource = model.ownership.values[usize::from(value.0)].resource;
                model.ownership.value_returns[usize::from(resource)] |= frame;
            }
            let mut moved_owners = Vec::with_capacity(owners.len());
            let mut moved_values = Vec::with_capacity(values.len());
            for owner in owners {
                moved_owners.push(model.move_owner(*owner, model.scope())?);
            }
            for value in values {
                // Validated in the caller; this private move must not recheck callee scope.
                let resource = model.ownership.values[usize::from(value.0)].resource;
                moved_values.push(model.push_value(resource, model.scope())?);
                model.ownership.values[usize::from(value.0)].live = false;
            }
            Ok((moved_owners, moved_values))
        })
    }

    fn require_ownership_arguments(
        &self,
        owners: &[StorageOwner],
        values: &[OwnedValue],
    ) -> Result<(), Error> {
        if owners.len() > LIMIT || values.len() > LIMIT {
            return Err(Error::Limit);
        }
        for (index, owner) in owners.iter().enumerate() {
            self.owner_index(*owner)?;
            if owners[..index].contains(owner) {
                return Err(Error::DuplicateOwnership);
            }
        }
        for (index, value) in values.iter().enumerate() {
            self.value_resource(*value)?;
            if values[..index].contains(value) {
                return Err(Error::DuplicateOwnership);
            }
        }
        Ok(())
    }

    pub fn return_owner(
        &mut self,
        owner: StorageOwner,
        required_extent: u8,
    ) -> Result<StorageOwner, Error> {
        self.ownership_transaction(|model| {
            if model.frame() == 0 {
                return Err(Error::NoCaller);
            }
            let allocation = model.owner_index(owner)?;
            let backing = model.ownership.allocations[allocation].backing;
            let place = model.place_state(backing);
            model.require_support(backing, place.dependencies, &place.support_generations)?;
            model.require_extent(place.dependencies, required_extent)?;
            model.ownership.owner_returns[allocation] &= !(1u8 << model.frame());
            let caller = model.frame() - 1;
            model.move_owner(owner, (caller, model.epochs[usize::from(caller)]))
        })
    }

    pub fn return_value(&mut self, value: OwnedValue) -> Result<OwnedValue, Error> {
        self.ownership_transaction(|model| {
            if model.frame() == 0 {
                return Err(Error::NoCaller);
            }
            let resource = model.value_resource(value)?;
            model.ownership.value_returns[usize::from(resource)] &= !(1u8 << model.frame());
            let caller = model.frame() - 1;
            model.move_value(value, (caller, model.epochs[usize::from(caller)]))
        })
    }

    pub fn destroy_value(&mut self, value: OwnedValue) -> Result<(), Error> {
        self.ownership_transaction(|model| {
            let resource = model.value_resource(value)?;
            model.ownership.values[usize::from(value.0)].live = false;
            model.ownership.value_returns[usize::from(resource)] = 0;
            model.ownership.destroyed |= 1u16 << resource;
            Ok(())
        })
    }

    pub fn reclaim_storage(
        &mut self,
        owner: StorageOwner,
        allocator: Allocator,
    ) -> Result<(), Error> {
        self.ownership_transaction(|model| {
            let allocation = model.owner_index(owner)?;
            let storage = &model.ownership.allocations[allocation];
            if storage.allocator != allocator {
                return Err(Error::WrongAllocator);
            }
            model.require_access(
                model.place_descriptor(storage.backing, Mode::Write),
                Mode::Write,
            )?;
            for cell in &storage.cells {
                if cell.knowledge == Initialization::Unknown {
                    return Err(Error::InitializationUnknown);
                }
                if cell.knowledge == Initialization::Initialized {
                    return Err(Error::LiveElements);
                }
            }
            model.invalidate_owned_place(storage.backing)?;
            model.ownership.allocations[allocation].live = false;
            model.ownership.owner_returns[allocation] = 0;
            model.ownership.owners[usize::from(owner.0)].live = false;
            Ok(())
        })
    }
}
