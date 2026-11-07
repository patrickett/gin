//! Finite root-frame validation, not semantic proof or source lowering.
//! Four fixtures x 14^4 words = 153,664 traces and 614,656 transitions.
//! Calls, leases, authority conflicts, larger capacities, and ledger exhaustion are excluded.
//! Knowledge drives permission; actual contents drive only obligation accounting.

use super::model::{
    Allocator, Construction, Error, Handle, Initialization, Mode, Model, OwnedValue, Policy,
    StorageOwner,
};

#[derive(Clone, Copy, Debug)]
enum Step {
    MoveOwner,
    MoveValue,
    Initialize,
    Construct,
    Fail,
    Borrow,
    Extract,
    Destroy,
    DestroyValue,
    Forget,
    Reclaim,
    WrongAllocator,
    OldOwner,
    OldValue,
}

const STEPS: [Step; 14] = [
    Step::MoveOwner,
    Step::MoveValue,
    Step::Initialize,
    Step::Construct,
    Step::Fail,
    Step::Borrow,
    Step::Extract,
    Step::Destroy,
    Step::DestroyValue,
    Step::Forget,
    Step::Reclaim,
    Step::WrongAllocator,
    Step::OldOwner,
    Step::OldValue,
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Oracle {
    owner_held: bool,
    knowledge: Initialization,
    actual: bool,
    values: Vec<bool>,
    current_value: usize,
    generation: u8,
    views: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Registers {
    owners: Vec<StorageOwner>,
    values: Vec<OwnedValue>,
    views: Vec<Handle>,
    allocator: Allocator,
    wrong_allocator: Allocator,
}

struct Runner {
    model: Model,
    registers: Registers,
    oracle: Oracle,
}

impl Oracle {
    fn known(&self, initialized: bool) -> Result<(), Error> {
        match self.knowledge {
            Initialization::Unknown => Err(Error::InitializationUnknown),
            Initialization::Initialized if !initialized => Err(Error::AlreadyInitialized),
            Initialization::Uninitialized if initialized => Err(Error::Uninitialized),
            _ => Ok(()),
        }
    }

    fn advance(&mut self, step: Step) -> Result<(), Error> {
        match step {
            Step::OldValue => return Err(Error::ValueMoved),
            Step::DestroyValue | Step::MoveValue => {
                if !self.values[self.current_value] {
                    return Err(Error::ValueMoved);
                }
                self.values[self.current_value] = false;
                if matches!(step, Step::MoveValue) {
                    self.values.push(true);
                    self.current_value = self.values.len() - 1;
                }
                return Ok(());
            }
            _ => {}
        }
        if matches!(step, Step::OldOwner) || !self.owner_held {
            return Err(Error::OwnerMoved);
        }
        match step {
            Step::Initialize | Step::Construct | Step::Fail => {
                self.known(false)?;
                if matches!(step, Step::Fail) {
                    return Err(Error::ConstructionFailed);
                }
                if matches!(step, Step::Initialize) {
                    if !self.values[self.current_value] {
                        return Err(Error::ValueMoved);
                    }
                    self.values[self.current_value] = false;
                }
                self.actual = true;
                self.knowledge = Initialization::Initialized;
            }
            Step::Borrow | Step::Extract | Step::Destroy => self.remove_or_borrow(step)?,
            Step::Forget => self.knowledge = Initialization::Unknown,
            Step::WrongAllocator => return Err(Error::WrongAllocator),
            Step::Reclaim => {
                match self.knowledge {
                    Initialization::Unknown => return Err(Error::InitializationUnknown),
                    Initialization::Initialized => return Err(Error::LiveElements),
                    Initialization::Uninitialized => {}
                }
                self.owner_held = false;
            }
            Step::MoveOwner => {}
            Step::OldOwner | Step::OldValue | Step::DestroyValue | Step::MoveValue => {
                unreachable!()
            }
        }
        Ok(())
    }

    fn remove_or_borrow(&mut self, step: Step) -> Result<(), Error> {
        self.known(true)?;
        if matches!(step, Step::Borrow) {
            self.views.push(self.generation);
        } else {
            self.actual = false;
            self.knowledge = Initialization::Uninitialized;
            self.generation += 1;
            if matches!(step, Step::Extract) {
                // Keep every old register: overwriting the input cannot erase an obligation.
                self.values.push(true);
                self.current_value = self.values.len() - 1;
            }
        }
        Ok(())
    }
}

impl Runner {
    fn fixture(live: bool, unknown: bool) -> Self {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let domain = model.new_domain().unwrap();
        let allocator = model.new_allocator().unwrap();
        let wrong_allocator = model.new_allocator().unwrap();
        let first = model.reserve_storage(domain, allocator, 1, 3).unwrap();
        let owner = model.transfer_owner(first).unwrap();
        model
            .construct_element(owner, 0, Construction::Succeed)
            .unwrap();
        let view = model.borrow_element(owner, 0, Mode::Read).unwrap();
        if !live {
            model.destroy_element(owner, 0).unwrap();
        }
        let value_old = model.new_owned_value().unwrap();
        let value = model.transfer_value(value_old).unwrap();
        let mut knowledge = Initialization::Uninitialized;
        if live {
            knowledge = Initialization::Initialized;
        }
        if unknown {
            model.forget_initialization(owner, 0).unwrap();
            knowledge = Initialization::Unknown;
        }
        Self {
            model,
            registers: Registers {
                owners: vec![first, owner],
                values: vec![value_old, value],
                views: vec![view],
                allocator,
                wrong_allocator,
            },
            oracle: Oracle {
                owner_held: true,
                knowledge,
                actual: live,
                values: vec![false, true],
                current_value: 1,
                generation: u8::from(!live),
                views: vec![0],
            },
        }
    }

    fn execute(&mut self, step: Step) -> Result<(), Error> {
        let owner = *self.registers.owners.last().unwrap();
        let value = *self.registers.values.last().unwrap();
        match step {
            Step::MoveOwner => {
                let next = self.model.transfer_owner(owner)?;
                self.registers.owners.push(next);
            }
            Step::MoveValue => {
                let next = self.model.transfer_value(value)?;
                self.registers.values.push(next);
            }
            Step::Initialize => self.model.initialize_element(owner, 0, value)?,
            Step::Construct => {
                self.model
                    .construct_element(owner, 0, Construction::Succeed)?;
            }
            Step::Fail => {
                self.model
                    .construct_element(owner, 0, Construction::FailBeforeCommit)?;
            }
            Step::Borrow => {
                self.registers
                    .views
                    .push(self.model.borrow_element(owner, 0, Mode::Read)?);
            }
            Step::Extract => {
                self.registers
                    .values
                    .push(self.model.extract_element(owner, 0)?);
            }
            Step::Destroy => self.model.destroy_element(owner, 0)?,
            Step::DestroyValue => self.model.destroy_value(value)?,
            Step::Forget => self.model.forget_initialization(owner, 0)?,
            Step::Reclaim => self
                .model
                .reclaim_storage(owner, self.registers.allocator)?,
            Step::WrongAllocator => {
                self.model
                    .reclaim_storage(owner, self.registers.wrong_allocator)?;
            }
            Step::OldOwner => {
                self.model.transfer_owner(self.registers.owners[0])?;
            }
            Step::OldValue => self.model.destroy_value(self.registers.values[0])?,
        }
        Ok(())
    }

    fn step(&mut self, step: Step) -> Result<(), Error> {
        let before = (
            self.model.clone(),
            self.registers.clone(),
            self.oracle.clone(),
        );
        let mut expected = self.oracle.clone();
        let result = expected.advance(step);
        let actual = self.execute(step);
        assert_eq!(actual, result, "step {step:?}, oracle {:?}", before.2);
        if result.is_ok() {
            self.oracle = expected;
        } else {
            assert_eq!(
                expected, before.2,
                "oracle rejection changed state: {step:?}"
            );
            assert_eq!(
                (&self.model, &self.registers, &self.oracle),
                (&before.0, &before.1, &before.2),
                "rejection changed state: {step:?}",
            );
        }
        self.probe();
        actual
    }

    fn probe(&mut self) {
        let oracle = &self.oracle;
        let current = self.registers.owners.len() - 1;
        for (index, owner) in self.registers.owners.iter().enumerate() {
            let expected = if index == current && oracle.owner_held {
                Ok(oracle.knowledge)
            } else {
                Err(Error::OwnerMoved)
            };
            assert_eq!(self.model.initialization(*owner, 0), expected);
        }
        let obligations = self.model.ownership_obligations();
        assert_eq!(obligations.storage, u8::from(oracle.owner_held));
        let values = oracle.values.iter().filter(|held| **held).count() as u8;
        assert_eq!(obligations.values, values + u8::from(oracle.actual));
        let complete = if obligations.storage == 0 && values == 0 && !oracle.actual {
            Ok(())
        } else {
            Err(Error::OutstandingOwnership)
        };
        assert_eq!(self.model.check_ownership_complete(), complete);
        for (view, generation) in self.registers.views.iter().zip(&oracle.views) {
            let expected = if oracle.actual && *generation == oracle.generation {
                Ok(())
            } else {
                Err(Error::Invalidated)
            };
            assert_eq!(self.model.read(*view), expected);
        }
        self.probe_values();
    }

    fn probe_values(&self) {
        assert_eq!(self.registers.values.len(), self.oracle.values.len());
        for (value, held) in self.registers.values.iter().zip(&self.oracle.values) {
            let mut receiver = self.model.clone();
            let expected = if *held {
                Ok(())
            } else {
                Err(Error::ValueMoved)
            };
            assert_eq!(receiver.destroy_value(*value), expected);
            if !held {
                assert_eq!(receiver, self.model);
            }
        }
    }
}

#[test]
fn ownership_words_match_independent_root_oracle() {
    for live in [false, true] {
        for unknown in [false, true] {
            for word in 0..14usize.pow(4) {
                let mut runner = Runner::fixture(live, unknown);
                runner.probe();
                let mut digits = word;
                for _ in 0..4 {
                    let step = STEPS[digits % STEPS.len()];
                    digits /= STEPS.len();
                    let _ = runner.step(step);
                }
            }
        }
    }
}

#[test]
fn ownership_witnesses_are_nonvacuous() {
    let mut empty = Runner::fixture(false, false);
    assert_eq!(empty.step(Step::Fail), Err(Error::ConstructionFailed));
    assert_eq!(empty.step(Step::Initialize), Ok(()));
    assert_eq!(empty.step(Step::Extract), Ok(()));
    assert_eq!(empty.step(Step::Reclaim), Ok(()));
    assert_eq!(empty.model.ownership_obligations().values, 1);
    assert_eq!(empty.step(Step::DestroyValue), Ok(()));
    assert_eq!(empty.model.check_ownership_complete(), Ok(()));

    let mut live = Runner::fixture(true, false);
    assert_eq!(live.step(Step::Extract), Ok(()));
    assert_eq!(live.step(Step::Reclaim), Ok(()));
    assert_eq!(live.model.ownership_obligations().values, 2);
    assert_eq!(live.step(Step::DestroyValue), Ok(()));
    assert_eq!(
        live.model.check_ownership_complete(),
        Err(Error::OutstandingOwnership)
    );

    for actual in [false, true] {
        let mut unknown = Runner::fixture(actual, true);
        assert_eq!(
            unknown.step(Step::Initialize),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            unknown.step(Step::Extract),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            unknown.step(Step::Destroy),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            unknown.step(Step::Borrow),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            unknown.step(Step::Reclaim),
            Err(Error::InitializationUnknown)
        );
    }
}

#[test]
fn earlier_state_and_stale_views_survive_failed_construction() {
    let mut runner = Runner::fixture(true, false);
    assert_eq!(runner.step(Step::Extract), Ok(()));
    let earlier = (
        runner.model.clone(),
        runner.registers.clone(),
        runner.oracle.clone(),
    );
    assert_eq!(runner.step(Step::Fail), Err(Error::ConstructionFailed));
    assert_eq!(
        (&runner.model, &runner.registers, &runner.oracle),
        (&earlier.0, &earlier.1, &earlier.2),
    );
    assert_eq!(runner.step(Step::Construct), Ok(()));
    assert_eq!(
        runner.model.read(runner.registers.views[0]),
        Err(Error::Invalidated)
    );
    assert_eq!(runner.step(Step::Borrow), Ok(()));
    assert_eq!(runner.step(Step::Forget), Ok(()));
    assert_eq!(
        runner.model.read(*runner.registers.views.last().unwrap()),
        Ok(())
    );
}
