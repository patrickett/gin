use super::*;

impl Model {
    pub fn new_domain(&mut self) -> Result<Domain, Error> {
        if self.frame() != 0 {
            return Err(Error::SetupOnly);
        }
        if self.owners.len() == LIMIT {
            return Err(Error::Limit);
        }
        let domain = Domain(self.owners.len() as u8);
        self.owners.push(0);
        self.revisions.push(0);
        Ok(domain)
    }

    pub fn new_place(
        &mut self,
        domain: Domain,
        extent: u8,
        support: Option<Place>,
    ) -> Result<Place, Error> {
        self.require_authority(domain)?;
        if self.places.len() == LIMIT {
            return Err(Error::Limit);
        }
        let place = Place(self.places.len() as u8);
        let mut dependencies = bit(place);
        let mut support_generations = [0; LIMIT];
        if let Some(support) = support {
            let state = self.place_state(support);
            self.require_support(support, state.dependencies, &state.support_generations)?;
            if extent > state.extent {
                return Err(Error::Validity);
            }
            dependencies |= state.dependencies;
            support_generations = state.support_generations;
            support_generations[usize::from(support.0)] = state.generation;
        }
        self.places.push(PlaceState {
            domain,
            extent,
            dependencies,
            support_generations,
            generation: 0,
            alive: true,
            slot: None,
        });
        Ok(place)
    }

    pub fn new_handle(&mut self, target: Place, mode: Mode) -> Result<Handle, Error> {
        let descriptor = self.place_descriptor(target, mode);
        self.require_access(descriptor, mode)?;
        self.push_handle(descriptor)
    }

    pub(super) fn place_descriptor(&self, target: Place, mode: Mode) -> Descriptor {
        Descriptor {
            target,
            mode,
            scope: self.scope(),
            dependencies: self.place_state(target).dependencies,
            support_generations: self.place_state(target).support_generations,
            lease: None,
        }
    }

    pub fn copy_handle(&mut self, handle: Handle) -> Result<Handle, Error> {
        let descriptor = self.descriptor(handle);
        self.require_valid(descriptor)?;
        if let Some(lease) = descriptor.lease
            && self.lease_state(lease).descriptor.mode == Mode::Write
        {
            return Err(Error::ExclusiveCopy);
        }
        // Copying a locator does not read its target or create access authority.
        self.push_handle(descriptor)
    }

    pub fn check_escape(&self, handle: Handle, required_extent: u8) -> Result<(), Error> {
        let descriptor = self.descriptor(handle);
        self.require_valid(descriptor)?;
        if descriptor.lease.is_some() {
            return Err(Error::OutstandingLease);
        }
        self.require_extent(descriptor.dependencies, required_extent)
    }

    pub(super) fn require_extent(&self, dependencies: u16, required: u8) -> Result<(), Error> {
        for (index, place) in self.places.iter().enumerate() {
            if dependencies & (1u16 << index) != 0 && place.extent < required {
                return Err(Error::Validity);
            }
        }
        Ok(())
    }

    /// Bind a scenario's slot/family identities without exposing managed Place IDs.
    pub fn new_slot_from_handles(
        &mut self,
        storage: Handle,
        initial: Handle,
        family: &[Handle],
        required_extent: u8,
    ) -> Result<(), Error> {
        if family.len() > LIMIT {
            return Err(Error::Limit);
        }
        let storage = self.descriptor(storage);
        self.require_access(storage, Mode::Write)?;
        self.require_valid(self.descriptor(initial))?;
        let mut targets = Vec::with_capacity(family.len());
        for handle in family {
            let descriptor = self.descriptor(*handle);
            self.require_valid(descriptor)?;
            targets.push(descriptor.target);
        }
        self.new_slot(storage.target, initial, &targets, required_extent)
    }

    pub fn new_slot(
        &mut self,
        storage: Place,
        initial: Handle,
        family: &[Place],
        required_extent: u8,
    ) -> Result<(), Error> {
        let place = self.place_state(storage);
        if self.managed_place(storage) {
            return Err(Error::OwnershipRequired);
        }
        self.require_authority(place.domain)?;
        self.require_support(storage, place.dependencies, &place.support_generations)?;
        if required_extent < place.extent {
            return Err(Error::Validity);
        }
        assert!(
            place.slot.is_none(),
            "a scenario initializes each slot once"
        );
        if family.len() > LIMIT {
            return Err(Error::Limit);
        }
        let mut family_bits = 0u16;
        for origin in family {
            self.place_state(*origin);
            family_bits |= bit(*origin);
        }
        let slot = SlotState {
            value: self.descriptor(initial),
            family: family_bits,
            mode: self.descriptor(initial).mode,
            required_extent,
        };
        self.require_replacement(&slot, initial)?;
        self.places[usize::from(storage.0)].slot = Some(slot);
        Ok(())
    }

    pub fn load_slot(&mut self, access: Handle) -> Result<Handle, Error> {
        let access = self.descriptor(access);
        self.require_access(access, Mode::Read)?;
        let slot = self
            .place_state(access.target)
            .slot
            .as_ref()
            .ok_or(Error::NotSlot)?;
        // A checked stored value does not borrow the variable it was copied from.
        let mut descriptor = Descriptor {
            scope: self.scope(),
            ..slot.value
        };
        self.require_valid(descriptor)?;
        assert!(descriptor.lease.is_none(), "lease views cannot be stored");
        if self.policy == Policy::SharedView && access.mode == Mode::Read {
            descriptor.mode = Mode::Read;
        }
        self.push_handle(descriptor)
    }

    pub fn retarget(&mut self, access: Handle, replacement: Handle) -> Result<(), Error> {
        let access = self.descriptor(access);
        self.require_access(access, Mode::Write)?;
        self.require_unprotected_support(access.target, access.lease)?;
        let slot = self
            .place_state(access.target)
            .slot
            .as_ref()
            .ok_or(Error::NotSlot)?;
        self.require_replacement(slot, replacement)?;
        let descriptor = self.descriptor(replacement);
        if descriptor.target != access.target && descriptor.dependencies & bit(access.target) != 0 {
            return Err(Error::SupportConflict);
        }
        let generation = self
            .place_state(access.target)
            .generation
            .checked_add(1)
            .ok_or(Error::Limit)?;
        self.advance_revision(self.domain(access))?;
        let place = &mut self.places[usize::from(access.target.0)];
        place.generation = generation;
        place.slot.as_mut().unwrap().value = descriptor;
        Ok(())
    }

    fn require_replacement(&self, slot: &SlotState, replacement: Handle) -> Result<(), Error> {
        let replacement_state = self.descriptor(replacement);
        self.check_escape(replacement, slot.required_extent)?;
        if replacement_state.mode != slot.mode {
            return Err(Error::HandleType);
        }
        if slot.family & bit(replacement_state.target) == 0 {
            return Err(Error::OriginFamily);
        }
        Ok(())
    }
}
