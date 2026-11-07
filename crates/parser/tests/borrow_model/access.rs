use super::*;

impl Model {
    pub fn read(&self, handle: Handle) -> Result<(), Error> {
        self.require_access(self.descriptor(handle), Mode::Read)
    }

    pub fn write(&mut self, handle: Handle) -> Result<(), Error> {
        let descriptor = self.descriptor(handle);
        self.require_access(descriptor, Mode::Write)?;
        if self.managed_backing(descriptor.target) {
            return Err(Error::OwnershipRequired);
        }
        if self.place_state(descriptor.target).slot.is_some() {
            // A generic write must not bypass a slot's replacement contract.
            return Err(Error::SlotWrite);
        }
        self.advance_revision(self.domain(descriptor))
    }

    pub fn invalidate(&mut self, handle: Handle) -> Result<(), Error> {
        let descriptor = self.descriptor(handle);
        self.require_access(descriptor, Mode::Write)?;
        if self.managed_place(descriptor.target) {
            return Err(Error::OwnershipRequired);
        }
        self.invalidate_owned_place(descriptor.target)
    }

    pub(super) fn invalidate_owned_place(&mut self, target: Place) -> Result<(), Error> {
        let descriptor = self.place_descriptor(target, Mode::Write);
        self.require_access(descriptor, Mode::Write)?;
        self.require_unprotected_support(descriptor.target, None)?;
        self.advance_revision(self.domain(descriptor))?;
        self.places[usize::from(descriptor.target.0)].alive = false;
        Ok(())
    }

    pub fn protect(&mut self, handle: Handle, mode: Mode) -> Result<Lease, Error> {
        if self.leases.len() == LIMIT {
            return Err(Error::Limit);
        }
        let mut descriptor = self.descriptor(handle);
        self.require_access(descriptor, mode)?;
        if descriptor.lease.is_some() {
            return Err(Error::LeaseConflict);
        }
        descriptor.mode = mode;
        let lease = Lease(self.leases.len() as u8);
        self.leases.push(LeaseState {
            descriptor,
            frame: self.frame(),
            active: true,
            view_issued: false,
        });
        Ok(lease)
    }

    pub fn view(&mut self, lease: Lease) -> Result<Handle, Error> {
        let state = self.lease_state(lease);
        if !state.active {
            return Err(Error::LeaseExpired);
        }
        if state.frame != self.frame() {
            return Err(Error::NoAuthority);
        }
        if state.descriptor.mode == Mode::Write && state.view_issued {
            return Err(Error::ExclusiveCopy);
        }
        let descriptor = Descriptor {
            lease: Some(lease),
            ..state.descriptor
        };
        self.require_valid(descriptor)?;
        let handle = self.push_handle(descriptor)?;
        self.leases[usize::from(lease.0)].view_issued = true;
        Ok(handle)
    }

    pub fn release(&mut self, lease: Lease) -> Result<(), Error> {
        let state = self.lease_state(lease);
        if !state.active {
            return Err(Error::LeaseExpired);
        }
        if state.frame != self.frame() {
            return Err(Error::NoAuthority);
        }
        self.leases[usize::from(lease.0)].active = false;
        Ok(())
    }

    pub fn remember(&mut self, handle: Handle) -> Result<Fact, Error> {
        self.read(handle)?;
        if self.facts.len() == LIMIT {
            return Err(Error::Limit);
        }
        let fact = Fact(self.facts.len() as u8);
        let domain = self.domain(self.descriptor(handle));
        self.facts.push(FactState {
            handle,
            revision: self.revisions[usize::from(domain.0)],
        });
        Ok(fact)
    }

    pub fn check_fact(&self, fact: Fact) -> Result<(), Error> {
        assert!(usize::from(fact.0) < self.facts.len());
        let fact = self.facts[usize::from(fact.0)];
        self.read(fact.handle)?;
        let domain = self.domain(self.descriptor(fact.handle));
        if fact.revision != self.revisions[usize::from(domain.0)] {
            return Err(Error::ForgottenFact);
        }
        Ok(())
    }

    pub(super) fn lease_state(&self, lease: Lease) -> LeaseState {
        assert!(usize::from(lease.0) < self.leases.len());
        self.leases[usize::from(lease.0)]
    }

    fn require_live(&self, dependencies: u16) -> Result<(), Error> {
        for (index, place) in self.places.iter().enumerate() {
            if dependencies & (1u16 << index) != 0 && !place.alive {
                return Err(Error::Invalidated);
            }
        }
        Ok(())
    }

    pub(super) fn require_valid(&self, descriptor: Descriptor) -> Result<(), Error> {
        let (frame, epoch) = descriptor.scope;
        if frame > self.frame() || self.epochs[usize::from(frame)] != epoch {
            return Err(Error::InaccessibleHandle);
        }
        self.require_support(
            descriptor.target,
            descriptor.dependencies,
            &descriptor.support_generations,
        )?;
        if let Some(lease) = descriptor.lease {
            let lease = self.lease_state(lease);
            if !lease.active {
                return Err(Error::LeaseExpired);
            }
            if lease.frame != self.frame() {
                return Err(Error::NoAuthority);
            }
        }
        Ok(())
    }

    pub(super) fn require_support(
        &self,
        target: Place,
        dependencies: u16,
        generations: &[u16; LIMIT],
    ) -> Result<(), Error> {
        self.require_live(dependencies)?;
        for (index, place) in self.places.iter().enumerate() {
            if index != usize::from(target.0)
                && dependencies & (1u16 << index) != 0
                && generations[index] != place.generation
            {
                return Err(Error::Invalidated);
            }
        }
        Ok(())
    }

    pub(super) fn require_unprotected_support(
        &self,
        target: Place,
        except: Option<Lease>,
    ) -> Result<(), Error> {
        for (index, lease) in self.leases.iter().enumerate() {
            if lease.active
                && lease.descriptor.dependencies & bit(target) != 0
                && except != Some(Lease(index as u8))
            {
                return Err(Error::LeaseConflict);
            }
        }
        Ok(())
    }

    pub(super) fn require_access(&self, descriptor: Descriptor, mode: Mode) -> Result<(), Error> {
        self.require_valid(descriptor)?;
        self.require_authority(self.domain(descriptor))?;
        if mode == Mode::Write && descriptor.mode != Mode::Write {
            return Err(Error::ReadOnly);
        }
        for (index, lease) in self.leases.iter().enumerate() {
            if lease.active
                && self.domain(lease.descriptor) == self.domain(descriptor)
                && descriptor.lease != Some(Lease(index as u8))
                && (mode == Mode::Write || lease.descriptor.mode == Mode::Write)
            {
                return Err(Error::LeaseConflict);
            }
        }
        Ok(())
    }

    pub(super) fn advance_revision(&mut self, domain: Domain) -> Result<(), Error> {
        let revision = &mut self.revisions[usize::from(domain.0)];
        *revision = revision.checked_add(1).ok_or(Error::Limit)?;
        Ok(())
    }
}
