use super::*;

impl Model {
    pub fn enter_call(&mut self, domains: &[Domain]) -> Result<(), Error> {
        if self.frame() == CALL_DEPTH_MAX || domains.len() > LIMIT {
            return Err(Error::Limit);
        }
        let mut seen = 0u16;
        for domain in domains {
            self.require_authority(*domain)?;
            let bit = 1u16 << domain.0;
            if seen & bit != 0 {
                return Err(Error::DuplicateAuthority);
            }
            seen |= bit;
            for lease in &self.leases {
                if lease.active && self.domain(lease.descriptor) == *domain {
                    return Err(Error::LeaseConflict);
                }
            }
        }
        let next_frame = self.frame() + 1;
        let epoch = self.epochs[usize::from(next_frame)]
            .checked_add(1)
            .ok_or(Error::Limit)?;
        for domain in domains {
            self.owners[usize::from(domain.0)] = next_frame;
        }
        self.epochs[usize::from(next_frame)] = epoch;
        self.calls.push(domains.to_vec());
        self.assert_invariants();
        Ok(())
    }

    pub fn leave_call(&mut self) -> Result<(), Error> {
        if self.frame() == 0 {
            return Err(Error::NoCaller);
        }
        self.require_no_frame_ownership()?;
        for lease in &self.leases {
            if lease.active && lease.frame == self.frame() {
                return Err(Error::OutstandingLease);
            }
        }
        let caller = self.frame() - 1;
        let domains = self.calls.pop().expect("a caller exists");
        for domain in domains {
            self.owners[usize::from(domain.0)] = caller;
        }
        self.assert_invariants();
        Ok(())
    }

    /// The required extent is an explicit caller result contract supplied by the scenario.
    pub fn return_handle(&mut self, handle: Handle, required_extent: u8) -> Result<Handle, Error> {
        if self.frame() == 0 {
            return Err(Error::NoCaller);
        }
        self.check_escape(handle, required_extent)?;
        let returned = self.push_handle(self.descriptor(handle))?;
        let caller = self.frame() - 1;
        self.handles[usize::from(returned.0)].scope = (caller, self.epochs[usize::from(caller)]);
        Ok(returned)
    }

    /// Call-side contract application only; does not prove an implementation fits its summary.
    pub fn apply_summary(&mut self, effects: &[Effect]) -> Result<(), Error> {
        if effects.len() > LIMIT {
            return Err(Error::Limit);
        }
        let mut trial = self.clone();
        for effect in effects {
            match *effect {
                Effect::Read(handle) => trial.read(handle)?,
                Effect::Write(handle) => trial.write(handle)?,
                Effect::Invalidate(handle) => trial.invalidate(handle)?,
                Effect::Unknown => return Err(Error::UnknownEffect),
            }
        }
        trial.assert_invariants();
        *self = trial;
        Ok(())
    }
}
