//! Independent finite call oracle; no source lowering or blanket exhaustiveness claim.
//! Six history pairs x two authority choices x four prefixes x 24^2 suffixes:
//! 27,648 traces, 55,296 suffix transitions, and 132 prefix attempts (82 accepted, 50 refused).
//! Prefixes of lengths 0/1/2/8 execute once, then all 24 operations occupy each suffix slot.
//! Logical prefix-plus-suffix lengths are 2/3/4/10 (131,328 logical transitions).
//! Enumeration fixture prefill adds 104 real public transfers, outside those counts.
//! Witnesses additionally exercise protected unwind, unrelated outputs, and frame reuse.
//! The distinct packing corpus is in `ownership_call_traces/packing.rs`.
//! Leases, constructors, and unknown initialization remain excluded there.

#[path = "ownership_call_traces/packing.rs"]
mod packing;

use super::model::{
    Allocator, Domain, Error, Initialization, Model, OwnedValue, Policy, StorageOwner,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Enter(bool),
    Batch(bool),
    SingleOwner,
    SingleValue,
    DuplicateOwner,
    DuplicateValue,
    Leave,
    MoveOwner(usize),
    MoveValue(usize),
    ReturnOwner(usize, u8),
    ReturnValue(usize),
    Reclaim(usize),
    Destroy(usize),
    OldOwner,
    OldValue,
    Pack(usize, usize),
    Extract(usize),
    DestroyCell(usize),
    OwnerCall(usize, bool),
}

const ALPHABET: [Step; 24] = [
    Step::Enter(false),
    Step::Enter(true),
    Step::Batch(false),
    Step::Batch(true),
    Step::SingleOwner,
    Step::SingleValue,
    Step::DuplicateOwner,
    Step::DuplicateValue,
    Step::Leave,
    Step::MoveOwner(0),
    Step::MoveOwner(1),
    Step::MoveValue(0),
    Step::MoveValue(1),
    Step::ReturnOwner(0, 3),
    Step::ReturnOwner(1, 3),
    Step::ReturnOwner(0, 4),
    Step::ReturnValue(0),
    Step::ReturnValue(1),
    Step::Reclaim(0),
    Step::Reclaim(1),
    Step::Destroy(0),
    Step::Destroy(1),
    Step::OldOwner,
    Step::OldValue,
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ticket {
    resource: usize,
    location: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Resource {
    alive: bool,
    borrowers: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct History {
    tickets: Vec<Ticket>,
    resources: [Resource; 2],
    current: [usize; 2],
}

impl History {
    fn new() -> Self {
        Self {
            tickets: (0..2)
                .map(|resource| Ticket {
                    resource,
                    location: Some(0),
                })
                .collect(),
            resources: std::array::from_fn(|_| Resource {
                alive: true,
                borrowers: vec![],
            }),
            current: [0, 1],
        }
    }

    fn held(&self, index: usize, activation: u32, owner: bool) -> Result<(), Error> {
        match self.tickets[index].location {
            None if owner => Err(Error::OwnerMoved),
            None => Err(Error::ValueMoved),
            Some(location) if location == activation => Ok(()),
            Some(_) if owner => Err(Error::OwnerNotHeld),
            Some(_) => Err(Error::ValueNotHeld),
        }
    }

    fn used(&self) -> usize {
        self.tickets.len()
            + self
                .resources
                .iter()
                .map(|resource| resource.borrowers.len())
                .sum::<usize>()
    }

    fn issue(&mut self, index: usize, location: u32) -> Result<(), Error> {
        if self.used() >= 16 {
            return Err(Error::Limit);
        }
        let resource = self.tickets[index].resource;
        self.tickets[index].location = None;
        self.current[resource] = self.tickets.len();
        self.tickets.push(Ticket {
            resource,
            location: Some(location),
        });
        Ok(())
    }

    fn terminate(&mut self, index: usize) {
        let resource = self.tickets[index].resource;
        self.tickets[index].location = None;
        self.resources[resource].alive = false;
        self.resources[resource].borrowers.clear();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Invocation {
    id: u32,
    authority_from: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Oracle {
    owners: History,
    values: History,
    cells: [Option<usize>; 2],
    stack: Vec<Invocation>,
    next_id: u32,
    authority: u32,
}

impl Oracle {
    fn new() -> Self {
        Self {
            owners: History::new(),
            values: History::new(),
            cells: [None; 2],
            stack: vec![Invocation {
                id: 0,
                authority_from: None,
            }],
            next_id: 1,
            authority: 0,
        }
    }

    fn here(&self) -> u32 {
        self.stack.last().unwrap().id
    }

    // The externally attempted oracle operation, not only its committed copy,
    // must be unchanged on refusal. The runner asserts that property.
    fn advance(&mut self, step: Step) -> Result<(), Error> {
        let mut trial = self.clone();
        trial.apply(step)?;
        *self = trial;
        Ok(())
    }

    fn enter(&mut self, owners: &[usize], values: &[usize], pass: bool) -> Result<(), Error> {
        for (history, inputs, owner) in
            [(&self.owners, owners, true), (&self.values, values, false)]
        {
            for (position, index) in inputs.iter().enumerate() {
                history.held(*index, self.here(), owner)?;
                if inputs[..position].contains(index) {
                    return Err(Error::DuplicateOwnership);
                }
            }
        }
        if self.stack.len() == 5 {
            return Err(Error::Limit);
        }
        if pass && self.authority != self.here() {
            return Err(Error::NoAuthority);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.stack.push(Invocation {
            id,
            authority_from: pass.then_some(self.authority),
        });
        if pass {
            self.authority = id;
        }
        for (history, inputs) in [(&mut self.owners, owners), (&mut self.values, values)] {
            for index in inputs {
                let resource = history.tickets[*index].resource;
                history.resources[resource].borrowers.push(id);
            }
            for index in inputs {
                history.issue(*index, id)?;
            }
        }
        Ok(())
    }

    fn leave(&mut self) -> Result<(), Error> {
        if self.stack.len() == 1 {
            return Err(Error::NoCaller);
        }
        for history in [&self.owners, &self.values] {
            if history
                .tickets
                .iter()
                .any(|ticket| ticket.location == Some(self.here()))
                || history
                    .resources
                    .iter()
                    .any(|resource| resource.borrowers.contains(&self.here()))
            {
                return Err(Error::OutstandingOwnership);
            }
        }
        if let Some(previous) = self.stack.pop().unwrap().authority_from {
            self.authority = previous;
        }
        Ok(())
    }

    fn return_ticket(&mut self, owner: bool, resource: usize, extent: u8) -> Result<(), Error> {
        if self.stack.len() == 1 {
            return Err(Error::NoCaller);
        }
        let here = self.here();
        let caller = self.stack[self.stack.len() - 2].id;
        let history = if owner {
            &mut self.owners
        } else {
            &mut self.values
        };
        let index = history.current[resource];
        history.held(index, here, owner)?;
        if owner && extent > 3 {
            return Err(Error::Validity);
        }
        history.resources[resource]
            .borrowers
            .retain(|borrower| *borrower != here);
        history.issue(index, caller)
    }

    fn apply(&mut self, step: Step) -> Result<(), Error> {
        let owners = self.owners.current;
        let values = self.values.current;
        match step {
            Step::Enter(pass) => self.enter(&[], &[], pass),
            Step::Batch(pass) => self.enter(&owners, &values, pass),
            Step::SingleOwner => self.enter(&owners[..1], &[], false),
            Step::SingleValue => self.enter(&[], &values[..1], false),
            Step::DuplicateOwner => self.enter(&[owners[0]; 2], &[], false),
            Step::DuplicateValue => self.enter(&[], &[values[0]; 2], false),
            Step::Leave => self.leave(),
            Step::ReturnOwner(resource, extent) => self.return_ticket(true, resource, extent),
            Step::ReturnValue(resource) => self.return_ticket(false, resource, 0),
            Step::MoveOwner(resource) => {
                self.owners.held(owners[resource], self.here(), true)?;
                self.owners.issue(owners[resource], self.here())
            }
            Step::MoveValue(resource) => {
                self.values.held(values[resource], self.here(), false)?;
                self.values.issue(values[resource], self.here())
            }
            Step::Reclaim(resource) => {
                self.owners.held(owners[resource], self.here(), true)?;
                if self.authority != self.here() {
                    return Err(Error::NoAuthority);
                }
                if self.cells[resource].is_some() {
                    return Err(Error::LiveElements);
                }
                self.owners.terminate(owners[resource]);
                Ok(())
            }
            Step::Destroy(resource) => {
                self.values.held(values[resource], self.here(), false)?;
                self.values.terminate(values[resource]);
                Ok(())
            }
            Step::OldOwner => {
                self.owners.held(0, self.here(), true)?;
                self.owners.issue(0, self.here())
            }
            Step::OldValue => {
                self.values.held(0, self.here(), false)?;
                self.values.terminate(0);
                Ok(())
            }
            Step::Pack(owner, value) => self.pack(owner, value),
            Step::Extract(owner) => self.extract(owner),
            Step::DestroyCell(owner) => self.destroy_cell(owner),
            Step::OwnerCall(owner, pass) => self.enter(&[owners[owner]], &[], pass),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Registers {
    owners: Vec<StorageOwner>,
    values: Vec<OwnedValue>,
    owner_current: [usize; 2],
    value_current: [usize; 2],
    domain: Domain,
    allocator: Allocator,
}

#[derive(Clone)]
struct Runner {
    model: Model,
    oracle: Oracle,
    registers: Registers,
}

impl Runner {
    fn unrelated_fixture() -> Self {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let domain = model.new_domain().unwrap();
        let allocator = model.new_allocator().unwrap();
        let mut owners = vec![model.reserve_storage(domain, allocator, 1, 3).unwrap()];
        let mut values = vec![model.new_owned_value().unwrap()];
        let mut oracle = Oracle::new();
        for history in [&mut oracle.owners, &mut oracle.values] {
            history.tickets.truncate(1);
            history.resources[1].alive = false;
        }
        for _ in 1..12 {
            owners.push(model.transfer_owner(*owners.last().unwrap()).unwrap());
            values.push(model.transfer_value(*values.last().unwrap()).unwrap());
            oracle.advance(Step::MoveOwner(0)).unwrap();
            oracle.advance(Step::MoveValue(0)).unwrap();
        }
        let (inputs, arguments) = model
            .enter_ownership_call(&[owners[11]], &[values[11]], &[domain])
            .unwrap();
        oracle.enter(&[11], &[11], true).unwrap();
        owners.extend(inputs);
        values.extend(arguments);
        owners.push(model.reserve_storage(domain, allocator, 1, 3).unwrap());
        values.push(model.new_owned_value().unwrap());
        let here = oracle.here();
        for history in [&mut oracle.owners, &mut oracle.values] {
            history.current[1] = history.tickets.len();
            history.tickets.push(Ticket {
                resource: 1,
                location: Some(here),
            });
            history.resources[1].alive = true;
        }
        let runner = Self {
            model,
            registers: Registers {
                owners,
                values,
                owner_current: oracle.owners.current,
                value_current: oracle.values.current,
                domain,
                allocator,
            },
            oracle,
        };
        runner.probe();
        runner
    }

    fn fixture(owner_budget: usize, value_budget: usize) -> Self {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let domain = model.new_domain().unwrap();
        let allocator = model.new_allocator().unwrap();
        let owners = (0..2)
            .map(|_| model.reserve_storage(domain, allocator, 1, 3).unwrap())
            .collect();
        let values = (0..2).map(|_| model.new_owned_value().unwrap()).collect();
        let mut runner = Self {
            model,
            oracle: Oracle::new(),
            registers: Registers {
                owners,
                values,
                owner_current: [0, 1],
                value_current: [0, 1],
                domain,
                allocator,
            },
        };
        for _ in 2..owner_budget {
            runner.step(Step::MoveOwner(0)).unwrap();
        }
        for _ in 2..value_budget {
            runner.step(Step::MoveValue(0)).unwrap();
        }
        runner.probe();
        runner
    }

    fn save_owner(&mut self, resource: usize, token: StorageOwner) {
        self.registers.owner_current[resource] = self.registers.owners.len();
        self.registers.owners.push(token);
    }

    fn save_value(&mut self, resource: usize, token: OwnedValue) {
        self.registers.value_current[resource] = self.registers.values.len();
        self.registers.values.push(token);
    }

    fn execute_entry(&mut self, step: Step) -> Result<(), Error> {
        let owners = self
            .registers
            .owner_current
            .map(|i| self.registers.owners[i]);
        let values = self
            .registers
            .value_current
            .map(|i| self.registers.values[i]);
        let domains = [self.registers.domain];
        let (owner_inputs, value_inputs, pass): (&[_], &[_], bool) = match step {
            Step::Batch(pass) => (&owners, &values, pass),
            Step::SingleOwner => {
                let token = self.model.enter_owner_call(owners[0], &[])?;
                self.save_owner(0, token);
                return Ok(());
            }
            Step::SingleValue => (&[], &values[..1], false),
            Step::DuplicateOwner => (&[owners[0]; 2], &[], false),
            Step::DuplicateValue => (&[], &[values[0]; 2], false),
            _ => unreachable!(),
        };
        let (owners, values) = self.model.enter_ownership_call(
            owner_inputs,
            value_inputs,
            if pass { &domains } else { &[] },
        )?;
        for (resource, token) in owners.into_iter().enumerate() {
            self.save_owner(resource, token);
        }
        for (resource, token) in values.into_iter().enumerate() {
            self.save_value(resource, token);
        }
        Ok(())
    }

    fn execute(&mut self, step: Step) -> Result<(), Error> {
        let owners = self
            .registers
            .owner_current
            .map(|i| self.registers.owners[i]);
        let values = self
            .registers
            .value_current
            .map(|i| self.registers.values[i]);
        match step {
            Step::Enter(pass) => {
                let domains = [self.registers.domain];
                self.model.enter_call(if pass { &domains } else { &[] })?;
            }
            Step::Leave => self.model.leave_call()?,
            Step::MoveOwner(resource) => {
                let token = self.model.transfer_owner(owners[resource])?;
                self.save_owner(resource, token);
            }
            Step::MoveValue(resource) => {
                let token = self.model.transfer_value(values[resource])?;
                self.save_value(resource, token);
            }
            Step::ReturnOwner(resource, extent) => {
                let token = self.model.return_owner(owners[resource], extent)?;
                self.save_owner(resource, token);
            }
            Step::ReturnValue(resource) => {
                let token = self.model.return_value(values[resource])?;
                self.save_value(resource, token);
            }
            Step::Reclaim(resource) => {
                self.model
                    .reclaim_storage(owners[resource], self.registers.allocator)?;
            }
            Step::Destroy(resource) => self.model.destroy_value(values[resource])?,
            Step::OldOwner => {
                let token = self.model.transfer_owner(self.registers.owners[0])?;
                self.save_owner(0, token);
            }
            Step::OldValue => self.model.destroy_value(self.registers.values[0])?,
            Step::Pack(owner, value) => {
                self.model
                    .initialize_element(owners[owner], 0, values[value])?;
            }
            Step::Extract(owner) => {
                let token = self.model.extract_element(owners[owner], 0)?;
                // Tokens are opaque; label this output by the expected source cell's resource.
                let resource = self.oracle.cells[owner].unwrap();
                self.save_value(resource, token);
            }
            Step::DestroyCell(owner) => self.model.destroy_element(owners[owner], 0)?,
            Step::OwnerCall(owner, pass) => {
                let domains = [self.registers.domain];
                let token = self
                    .model
                    .enter_owner_call(owners[owner], if pass { &domains } else { &[] })?;
                self.save_owner(owner, token);
            }
            _ => self.execute_entry(step)?,
        }
        Ok(())
    }

    fn step(&mut self, step: Step) -> Result<(), Error> {
        let before = self.clone();
        let mut attempted = self.oracle.clone();
        let expected = attempted.advance(step);
        let actual = self.execute(step);
        assert_eq!(actual, expected, "{step:?}, oracle: {:?}", before.oracle);
        if actual.is_err() {
            assert_eq!(attempted, before.oracle, "oracle refusal: {step:?}");
            assert_eq!(self.model, before.model, "model refusal: {step:?}");
            assert_eq!(self.registers, before.registers);
            assert_eq!(self.oracle, before.oracle);
        } else {
            self.oracle = attempted;
        }
        self.probe();
        actual
    }

    fn probe(&self) {
        assert_eq!(self.registers.owner_current, self.oracle.owners.current);
        assert_eq!(self.registers.value_current, self.oracle.values.current);
        assert_eq!(
            self.registers.owners.len(),
            self.oracle.owners.tickets.len()
        );
        assert_eq!(
            self.registers.values.len(),
            self.oracle.values.tickets.len()
        );
        for (index, owner) in self.registers.owners.iter().enumerate() {
            let expected = self.oracle.owners.held(index, self.oracle.here(), true);
            let trial = self.model.clone();
            assert_eq!(
                trial.initialization(*owner, 0),
                expected.map(|()| {
                    let resource = self.oracle.owners.tickets[index].resource;
                    if self.oracle.cells[resource].is_some() {
                        Initialization::Initialized
                    } else {
                        Initialization::Uninitialized
                    }
                }),
            );
            assert_eq!(trial, self.model);
        }
        for (index, value) in self.registers.values.iter().enumerate() {
            let expected = self.oracle.values.held(index, self.oracle.here(), false);
            let mut trial = self.model.clone();
            assert_eq!(trial.destroy_value(*value), expected);
            if expected.is_err() {
                assert_eq!(trial, self.model, "destroy probe refusal");
            }
            trial.assert_invariants();
        }
        let storage = self
            .oracle
            .owners
            .resources
            .iter()
            .filter(|r| r.alive)
            .count();
        let values = self
            .oracle
            .values
            .resources
            .iter()
            .filter(|r| r.alive)
            .count();
        let obligations = self.model.ownership_obligations();
        assert_eq!(usize::from(obligations.storage), storage);
        assert_eq!(usize::from(obligations.values), values);
        let complete = if storage + values == 0 {
            Ok(())
        } else {
            Err(Error::OutstandingOwnership)
        };
        assert_eq!(self.model.check_ownership_complete(), complete);
        self.model.assert_invariants();
        self.oracle.probe_locations();
    }
}

#[test]
fn finite_call_suffixes_match_independent_invocation_oracle() {
    let budgets = [(2, 2), (8, 8), (12, 12), (15, 15), (12, 15), (15, 12)];
    let mut traces = 0;
    let mut transitions = 0;
    let mut prefix_transitions = 0;
    let mut accepted_prefix_transitions = 0;
    for (owners, values) in budgets {
        let fixture = Runner::fixture(owners, values);
        for authority in [false, true] {
            let prefixes = [
                vec![],
                vec![Step::Enter(authority)],
                vec![Step::Batch(authority), Step::Batch(authority)],
                vec![
                    Step::Batch(authority),
                    Step::ReturnOwner(0, 3),
                    Step::ReturnOwner(1, 3),
                    Step::ReturnValue(0),
                    Step::ReturnValue(1),
                    Step::Leave,
                    Step::Enter(authority),
                    Step::Leave,
                ],
            ];
            for prefix in prefixes {
                let mut base = fixture.clone();
                for step in prefix {
                    if base.step(step).is_ok() {
                        accepted_prefix_transitions += 1;
                    }
                    prefix_transitions += 1;
                }
                for first in ALPHABET {
                    for second in ALPHABET {
                        let mut runner = base.clone();
                        let _ = runner.step(first);
                        let _ = runner.step(second);
                        traces += 1;
                        transitions += 2;
                    }
                }
            }
        }
    }
    assert_eq!(
        (traces, transitions, prefix_transitions),
        (27_648, 55_296, 132)
    );
    assert_eq!(accepted_prefix_transitions, 82);
    assert_eq!(prefix_transitions - accepted_prefix_transitions, 50);
}

#[test]
fn protected_nested_unwind_and_failed_bound_are_nonvacuous() {
    for authority in [false, true] {
        let mut runner = Runner::fixture(8, 8);
        runner.step(Step::Batch(authority)).unwrap();
        runner.step(Step::Batch(authority)).unwrap();
        assert_eq!(runner.oracle.owners.used(), 16);
        assert_eq!(runner.oracle.values.used(), 16);
        assert_eq!(runner.step(Step::MoveOwner(0)), Err(Error::Limit));
        assert_eq!(runner.step(Step::MoveValue(0)), Err(Error::Limit));
        assert_eq!(runner.step(Step::ReturnOwner(0, 4)), Err(Error::Validity));
        assert_eq!(runner.step(Step::Leave), Err(Error::OutstandingOwnership));
        for _ in 0..2 {
            for resource in 0..2 {
                runner.step(Step::ReturnOwner(resource, 3)).unwrap();
                runner.step(Step::ReturnValue(resource)).unwrap();
            }
            runner.step(Step::Leave).unwrap();
            if runner.oracle.stack.len() == 2 {
                assert_eq!(runner.step(Step::MoveOwner(0)), Err(Error::Limit));
                assert_eq!(runner.step(Step::MoveValue(0)), Err(Error::Limit));
            }
        }
        runner.step(Step::Enter(authority)).unwrap();
        assert_eq!(runner.step(Step::OldOwner), Err(Error::OwnerMoved));
        assert_eq!(runner.step(Step::OldValue), Err(Error::ValueMoved));
        runner.step(Step::Leave).unwrap();
        for resource in 0..2 {
            runner.step(Step::Reclaim(resource)).unwrap();
            runner.step(Step::Destroy(resource)).unwrap();
        }
        runner.model.check_ownership_complete().unwrap();
    }
}

#[test]
fn last_slots_and_midbatch_failures_preserve_inputs() {
    for (owners, values) in [(15, 2), (2, 15), (13, 2), (2, 13)] {
        let mut runner = Runner::fixture(owners, values);
        assert_eq!(runner.step(Step::Batch(true)), Err(Error::Limit));
        if owners == 15 {
            assert_eq!(runner.step(Step::SingleOwner), Err(Error::Limit));
        }
        if values == 15 {
            assert_eq!(runner.step(Step::SingleValue), Err(Error::Limit));
        }
        for resource in 0..2 {
            runner.step(Step::Reclaim(resource)).unwrap();
            runner.step(Step::Destroy(resource)).unwrap();
        }
        runner.model.check_ownership_complete().unwrap();
    }
}

#[test]
fn unrelated_outputs_cannot_spend_resource_specific_credits() {
    let mut runner = Runner::unrelated_fixture();
    // Zero has a protected input return; callee-created one has no such credit.
    runner.step(Step::MoveOwner(0)).unwrap();
    runner.step(Step::MoveValue(0)).unwrap();
    assert_eq!(runner.oracle.owners.used(), 16);
    assert_eq!(runner.oracle.values.used(), 16);
    assert_eq!(runner.step(Step::ReturnOwner(1, 3)), Err(Error::Limit));
    assert_eq!(runner.step(Step::ReturnValue(1)), Err(Error::Limit));
    runner.step(Step::ReturnOwner(0, 3)).unwrap();
    runner.step(Step::ReturnValue(0)).unwrap();
    runner.step(Step::Reclaim(1)).unwrap();
    runner.step(Step::Destroy(1)).unwrap();
    runner.step(Step::Leave).unwrap();
    runner.step(Step::Reclaim(0)).unwrap();
    runner.step(Step::Destroy(0)).unwrap();
    runner.model.check_ownership_complete().unwrap();
}

#[test]
fn suspended_ancestors_depth_and_reentry_are_nonvacuous() {
    let mut runner = Runner::fixture(2, 2);
    runner.step(Step::Enter(true)).unwrap();
    assert_eq!(runner.step(Step::SingleOwner), Err(Error::OwnerNotHeld));
    assert_eq!(runner.step(Step::SingleValue), Err(Error::ValueNotHeld));
    assert_eq!(runner.step(Step::MoveOwner(1)), Err(Error::OwnerNotHeld));
    assert_eq!(runner.step(Step::MoveValue(1)), Err(Error::ValueNotHeld));
    for _ in 0..3 {
        runner.step(Step::Enter(true)).unwrap();
    }
    assert_eq!(runner.step(Step::Enter(false)), Err(Error::Limit));
    for _ in 0..4 {
        runner.step(Step::Leave).unwrap();
    }
    runner.step(Step::SingleOwner).unwrap();
    assert_eq!(runner.step(Step::Reclaim(0)), Err(Error::NoAuthority));
    runner.step(Step::ReturnOwner(0, 3)).unwrap();
    runner.step(Step::Leave).unwrap();
    // Reuse exactly the former callee depth with a fresh invocation ID.
    runner.step(Step::SingleValue).unwrap();
    assert_eq!(runner.step(Step::MoveOwner(0)), Err(Error::OwnerNotHeld));
    runner.step(Step::ReturnValue(0)).unwrap();
    runner.step(Step::Leave).unwrap();
    runner.step(Step::Enter(false)).unwrap();
    runner.step(Step::Leave).unwrap();
    for resource in 0..2 {
        runner.step(Step::Reclaim(resource)).unwrap();
        runner.step(Step::Destroy(resource)).unwrap();
    }
    runner.model.check_ownership_complete().unwrap();
}

#[test]
fn terminal_cleanup_discharges_all_nested_resource_credits() {
    let mut runner = Runner::fixture(8, 8);
    runner.step(Step::Batch(true)).unwrap();
    runner.step(Step::Batch(true)).unwrap();
    for resource in 0..2 {
        runner.step(Step::Reclaim(resource)).unwrap();
        runner.step(Step::Destroy(resource)).unwrap();
    }
    assert_eq!(runner.oracle.owners.used(), 12);
    assert_eq!(runner.oracle.values.used(), 12);
    runner.model.check_ownership_complete().unwrap();
    runner.step(Step::Leave).unwrap();
    runner.step(Step::Leave).unwrap();
}
