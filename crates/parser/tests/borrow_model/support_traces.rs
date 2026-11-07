//! A separate small oracle for transitive support and domain-wide shared leases.
//! No call, policy-composition, or lifetime inference rules are introduced here.

use super::model::{Error, Handle, LIMIT, Lease, Mode, Model, Place, Policy};

const STEPS: u32 = 5;
const OPERATIONS: [Operation; 10] = [
    Operation::ReadChild,
    Operation::ReadGrandchild,
    Operation::WriteGrandchild,
    Operation::RetargetSlot,
    Operation::InvalidateSlot,
    Operation::InvalidateChild,
    Operation::ProtectGrandchild,
    Operation::ReleaseLatest,
    Operation::RecreateChild,
    Operation::ReadSlot,
];

#[derive(Debug, Clone, Copy)]
enum Operation {
    ReadChild,
    ReadGrandchild,
    WriteGrandchild,
    RetargetSlot,
    InvalidateSlot,
    InvalidateChild,
    ProtectGrandchild,
    ReleaseLatest,
    RecreateChild,
    ReadSlot,
}

/// Booleans describe this fixed support chain; no model bitsets/generations are reused.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SupportOracle {
    slot_alive: bool,
    child_alive: bool,
    support_unchanged: bool,
    leases: Vec<bool>,
}

impl SupportOracle {
    fn new() -> Self {
        Self {
            slot_alive: true,
            child_alive: true,
            support_unchanged: true,
            // Setup issues and ends one lease, so ReleaseLatest always has a real operand.
            leases: vec![false],
        }
    }

    fn child_access(&self) -> Result<(), Error> {
        if !self.slot_alive || !self.child_alive || !self.support_unchanged {
            return Err(Error::Invalidated);
        }
        Ok(())
    }

    fn slot_access(&self) -> Result<(), Error> {
        if !self.slot_alive {
            return Err(Error::Invalidated);
        }
        Ok(())
    }

    fn unprotected(&self) -> Result<(), Error> {
        if self.leases.contains(&true) {
            return Err(Error::LeaseConflict);
        }
        Ok(())
    }

    fn assert_invariants(&self) {
        assert!(!self.leases.is_empty());
        assert!(self.leases.len() <= 1 + STEPS as usize);
        if self.leases.contains(&true) {
            assert!(self.slot_alive);
            assert!(self.child_alive);
            assert!(self.support_unchanged);
        }
    }

    fn step(&mut self, operation: Operation) -> Result<(), Error> {
        match operation {
            Operation::ReadChild | Operation::ReadGrandchild | Operation::RecreateChild => {
                self.child_access()
            }
            Operation::ReadSlot => self.slot_access(),
            Operation::WriteGrandchild => {
                self.child_access()?;
                self.unprotected()
            }
            Operation::RetargetSlot => {
                self.slot_access()?;
                self.unprotected()?;
                self.support_unchanged = false;
                Ok(())
            }
            Operation::InvalidateSlot => {
                self.slot_access()?;
                self.unprotected()?;
                self.slot_alive = false;
                Ok(())
            }
            Operation::InvalidateChild => {
                self.child_access()?;
                self.unprotected()?;
                self.child_alive = false;
                Ok(())
            }
            Operation::ProtectGrandchild => {
                self.child_access()?;
                self.leases.push(true);
                Ok(())
            }
            Operation::ReleaseLatest => {
                let active = self.leases.last_mut().unwrap();
                if !*active {
                    return Err(Error::LeaseExpired);
                }
                *active = false;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SupportTrace {
    model: Model,
    initial_value: Handle,
    slot: Handle,
    child_place: Place,
    child: Handle,
    grandchild: Handle,
    latest: Lease,
}

impl SupportTrace {
    fn new(separate_domains: bool) -> Self {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let target_domain = model.new_domain().unwrap();
        let target = model.new_place(target_domain, 3, None).unwrap();
        let initial_value = model.new_handle(target, Mode::Write).unwrap();
        let slot_domain = model.new_domain().unwrap();
        let storage = model.new_place(slot_domain, 2, None).unwrap();
        model
            .new_slot(storage, initial_value, &[target], 3)
            .unwrap();
        let slot = model.new_handle(storage, Mode::Write).unwrap();
        let child_domain = if separate_domains {
            model.new_domain().unwrap()
        } else {
            slot_domain
        };
        let child_place = model.new_place(child_domain, 2, Some(storage)).unwrap();
        let child = model.new_handle(child_place, Mode::Write).unwrap();
        let nested = model.new_place(child_domain, 1, Some(child_place)).unwrap();
        let grandchild = model.new_handle(nested, Mode::Write).unwrap();
        let latest = model.protect(grandchild, Mode::Read).unwrap();
        model.release(latest).unwrap();
        Self {
            model,
            initial_value,
            slot,
            child_place,
            child,
            grandchild,
            latest,
        }
    }

    fn step(&mut self, operation: Operation) -> Result<(), Error> {
        match operation {
            Operation::ReadChild => self.model.read(self.child),
            Operation::ReadGrandchild => self.model.read(self.grandchild),
            Operation::WriteGrandchild => self.model.write(self.grandchild),
            Operation::RetargetSlot => self.model.retarget(self.slot, self.initial_value),
            Operation::InvalidateSlot => self.model.invalidate(self.slot),
            Operation::InvalidateChild => self.model.invalidate(self.child),
            Operation::ProtectGrandchild => {
                self.latest = self.model.protect(self.grandchild, Mode::Read)?;
                Ok(())
            }
            Operation::ReleaseLatest => self.model.release(self.latest),
            Operation::RecreateChild => self
                .model
                .new_handle(self.child_place, Mode::Read)
                .map(|_| ()),
            Operation::ReadSlot => self.model.read(self.slot),
        }
    }

    fn compare_probes(&self, oracle: &SupportOracle, prefix: &[Operation]) {
        assert_eq!(
            self.model.read(self.slot),
            oracle.slot_access(),
            "{prefix:?}"
        );
        assert_eq!(
            self.model.read(self.child),
            oracle.child_access(),
            "{prefix:?}"
        );
        assert_eq!(
            self.model.read(self.grandchild),
            oracle.child_access(),
            "{prefix:?}"
        );
        self.model.assert_invariants();
        oracle.assert_invariants();
    }
}

#[test]
fn support_chains_match_the_oracle_across_all_five_step_traces() {
    assert!(
        4 + STEPS as usize <= LIMIT,
        "handle setup plus one result per step"
    );
    assert!(
        (STEPS as usize) < LIMIT,
        "lease setup plus one result per step"
    );
    let traces = OPERATIONS.len().pow(STEPS);
    assert_eq!(traces, 100_000);
    for separate_domains in [false, true] {
        let initial = SupportTrace::new(separate_domains);
        for trace in 0..traces {
            let mut execution = initial.clone();
            let mut oracle = SupportOracle::new();
            let mut digits = trace;
            let mut prefix = Vec::with_capacity(STEPS as usize);
            for _ in 0..STEPS {
                let operation = OPERATIONS[digits % OPERATIONS.len()];
                digits /= OPERATIONS.len();
                prefix.push(operation);
                let before = execution.clone();
                let oracle_before = oracle.clone();
                let expected = oracle.step(operation);
                let result = execution.step(operation);
                assert_eq!(result, expected, "separate={separate_domains}: {prefix:?}");
                if result.is_err() {
                    assert_eq!(execution, before, "{prefix:?}");
                    assert_eq!(oracle, oracle_before, "{prefix:?}");
                }
                execution.compare_probes(&oracle, &prefix);
            }
        }
    }
}
