//! Independent finite experiment: five places without support dependencies.
//! No leases, facts, summaries, resource exhaustion, parser inputs, or inferred lifetimes.
//! Constructors are trusted setup.
//! Enumerates every three-operation word, plus every two-operation suffix of four six-step
//! prefixes. Both policies and separate/coalesced target domains run the identical corpus.
//! Per fixture: 24^3 three-step traces and 4 * 24^2 eight-step traces. Four fixtures
//! total 64,512 traces / 239,616 transitions; explicit witnesses add 100 transitions.
//! Targets A/B have extent 3, the short family member extent 1, and the outsider extent 3.
//! Slot storage has extent 1 but requires stored values of extent 3. A and B either share
//! a domain or B shares the slot domain. At most 11 handles, five places, two domains,
//! four call frames, and no leases/facts are allocated; no resource refusal is expected.
use super::model::{Domain, Error, Handle, Mode, Model, Policy};

#[derive(Debug, Clone, Copy)]
enum Op {
    Enter(u8),
    Leave,
    Copy(usize),
    Return(usize, u8),
    Load(bool),
    Retarget(usize),
    Read(usize),
    Write(usize),
    Invalidate(usize),
}

const OPS: [Op; 24] = [
    Op::Enter(0),
    Op::Enter(1),
    Op::Enter(3),
    Op::Leave,
    Op::Copy(0),
    Op::Copy(7),
    Op::Return(7, 3),
    Op::Return(2, 3),
    Op::Load(false),
    Op::Load(true),
    Op::Retarget(1),
    Op::Retarget(2),
    Op::Retarget(3),
    Op::Retarget(6),
    Op::Retarget(7),
    Op::Read(7),
    Op::Read(8),
    Op::Write(7),
    Op::Write(8),
    Op::Invalidate(0),
    Op::Invalidate(1),
    Op::Read(0),
    Op::Read(1),
    Op::Write(4),
];

const PREFIXES: [[Op; 6]; 4] = [
    [
        Op::Enter(3),
        Op::Copy(1),
        Op::Return(7, 3),
        Op::Leave,
        Op::Read(8),
        Op::Write(8),
    ],
    [
        Op::Enter(1),
        Op::Copy(1),
        Op::Retarget(7),
        Op::Leave,
        Op::Load(false),
        Op::Read(7),
    ],
    [
        Op::Load(false),
        Op::Retarget(1),
        Op::Invalidate(0),
        Op::Read(7),
        Op::Load(true),
        Op::Write(7),
    ],
    [
        Op::Enter(3),
        Op::Copy(0),
        Op::Leave,
        Op::Enter(3),
        Op::Read(7),
        Op::Return(7, 3),
    ],
];

// A locator names an allocation and the unique activation that created it. Active
// activation identities, rather than Model's depth/epoch table, decide visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Locator {
    allocation: usize,
    writable: bool,
    activation: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Oracle {
    live: [bool; 5],
    domains: [usize; 5],
    stack: Vec<(usize, u8)>,
    serial: usize,
    registers: [Locator; 9],
    stored: Locator,
    shared: bool,
}

impl Oracle {
    fn visible(&self, value: Locator) -> Result<(), Error> {
        if !self.stack.iter().any(|entry| entry.0 == value.activation) {
            return Err(Error::InaccessibleHandle);
        }
        if !self.live[value.allocation] {
            return Err(Error::Invalidated);
        }
        Ok(())
    }

    fn access(&self, value: Locator, write: bool) -> Result<(), Error> {
        self.visible(value)?;
        let domain_bit = 1 << self.domains[value.allocation];
        let owner = self
            .stack
            .iter()
            .rposition(|entry| entry.1 & domain_bit != 0);
        if owner != Some(self.stack.len() - 1) {
            return Err(Error::NoAuthority);
        }
        if write && !value.writable {
            return Err(Error::ReadOnly);
        }
        Ok(())
    }

    fn extent(&self, value: Locator, required: u8) -> Result<(), Error> {
        self.visible(value)?;
        let extents = [3, 3, 1, 3, 1];
        if extents[value.allocation] < required {
            return Err(Error::Validity);
        }
        Ok(())
    }

    fn enter(&mut self, mask: u8) -> Result<(), Error> {
        if self.stack.len() == 5 {
            return Err(Error::Limit);
        }
        for domain in 0..2 {
            if mask & (1 << domain) != 0 {
                let owner = self
                    .stack
                    .iter()
                    .rposition(|entry| entry.1 & (1 << domain) != 0);
                if owner != Some(self.stack.len() - 1) {
                    return Err(Error::NoAuthority);
                }
            }
        }
        self.serial += 1;
        self.stack.push((self.serial, mask));
        Ok(())
    }

    fn step(&mut self, op: Op) -> Result<(), Error> {
        let activation = self.stack.last().unwrap().0;
        match op {
            Op::Enter(mask) => self.enter(mask),
            Op::Leave => {
                if self.stack.len() == 1 {
                    return Err(Error::NoCaller);
                }
                self.stack.pop();
                Ok(())
            }
            Op::Copy(register) => {
                let value = self.registers[register];
                self.visible(value)?;
                self.registers[7] = Locator {
                    activation,
                    ..value
                };
                Ok(())
            }
            Op::Return(register, required) => {
                if self.stack.len() == 1 {
                    return Err(Error::NoCaller);
                }
                let value = self.registers[register];
                self.extent(value, required)?;
                let activation = self.stack[self.stack.len() - 2].0;
                self.registers[8] = Locator {
                    activation,
                    ..value
                };
                Ok(())
            }
            Op::Load(read_only) => {
                self.access(self.registers[if read_only { 5 } else { 4 }], false)?;
                // Storage holds a value, not a reference to its source activation.
                let value = Locator {
                    activation,
                    ..self.stored
                };
                self.visible(value)?;
                self.registers[7] = Locator {
                    writable: value.writable && !(read_only && self.shared),
                    ..value
                };
                Ok(())
            }
            Op::Retarget(register) => self.replace(register),
            Op::Read(register) => self.access(self.registers[register], false),
            Op::Write(register) => {
                let value = self.registers[register];
                self.access(value, true)?;
                if value.allocation == 4 {
                    return Err(Error::SlotWrite);
                }
                Ok(())
            }
            Op::Invalidate(register) => {
                let value = self.registers[register];
                self.access(value, true)?;
                self.live[value.allocation] = false;
                Ok(())
            }
        }
    }

    fn replace(&mut self, register: usize) -> Result<(), Error> {
        self.access(self.registers[4], true)?;
        let value = self.registers[register];
        self.extent(value, 3)?;
        if !value.writable {
            return Err(Error::HandleType);
        }
        if ![0, 1, 2].contains(&value.allocation) {
            return Err(Error::OriginFamily);
        }
        self.stored = value;
        Ok(())
    }

    fn assert_invariants(&self) {
        assert!((1..=5).contains(&self.stack.len()));
        assert_eq!(self.stack[0], (0, 3));
        assert!([0, 1].contains(&self.stored.allocation));
        for value in self.registers {
            assert!(value.allocation < self.live.len());
            assert!(value.activation <= self.serial);
        }
    }
}

#[derive(Clone)]
struct Runner {
    model: Model,
    domains: [Domain; 2],
    registers: [Handle; 9],
    oracle: Oracle,
}

impl Runner {
    fn new(policy: Policy, coalesced: bool) -> Self {
        let mut model = Model::new(policy);
        let domains = [model.new_domain().unwrap(), model.new_domain().unwrap()];
        let mapping = [1, usize::from(coalesced), 1, 1, 0];
        let places = [0, 1, 2, 3, 4].map(|index| {
            let extent = [3, 3, 1, 3, 1][index];
            model
                .new_place(domains[mapping[index]], extent, None)
                .unwrap()
        });
        let initial = places.map(|place| model.new_handle(place, Mode::Write).unwrap());
        model
            .new_slot(places[4], initial[0], &places[..3], 3)
            .unwrap();
        let reader = model.new_handle(places[4], Mode::Read).unwrap();
        let target_reader = model.new_handle(places[1], Mode::Read).unwrap();
        let registers = [
            initial[0],
            initial[1],
            initial[2],
            initial[3],
            initial[4],
            reader,
            target_reader,
            initial[0],
            initial[0],
        ];
        let oracle_registers = [0, 1, 2, 3, 4, 4, 1, 0, 0].map(|allocation| Locator {
            allocation,
            writable: true,
            activation: 0,
        });
        let mut oracle = Oracle {
            live: [true; 5],
            domains: mapping,
            stack: vec![(0, 3)],
            serial: 0,
            registers: oracle_registers,
            stored: oracle_registers[0],
            shared: policy == Policy::SharedView,
        };
        oracle.registers[5].writable = false;
        oracle.registers[6].writable = false;
        Self {
            model,
            domains,
            registers,
            oracle,
        }
    }

    fn model_step(&mut self, op: Op) -> Result<(), Error> {
        match op {
            Op::Enter(mask) => {
                let domains: Vec<_> = (0..2)
                    .filter(|i| mask & (1 << i) != 0)
                    .map(|i| self.domains[i])
                    .collect();
                self.model.enter_call(&domains)
            }
            Op::Leave => self.model.leave_call(),
            Op::Copy(register) => {
                self.registers[7] = self.model.copy_handle(self.registers[register])?;
                Ok(())
            }
            Op::Return(register, required) => {
                self.registers[8] = self
                    .model
                    .return_handle(self.registers[register], required)?;
                Ok(())
            }
            Op::Load(read_only) => {
                let access = self.registers[if read_only { 5 } else { 4 }];
                self.registers[7] = self.model.load_slot(access)?;
                Ok(())
            }
            Op::Retarget(register) => self
                .model
                .retarget(self.registers[4], self.registers[register]),
            Op::Read(register) => self.model.read(self.registers[register]),
            Op::Write(register) => self.model.write(self.registers[register]),
            Op::Invalidate(register) => self.model.invalidate(self.registers[register]),
        }
    }

    fn step(&mut self, op: Op) -> Result<(), Error> {
        let before = self.clone();
        let expected = self.oracle.step(op);
        let actual = self.model_step(op);
        assert_eq!(
            actual, expected,
            "operation {op:?}; oracle before: {:?}",
            before.oracle
        );
        if actual.is_err() {
            assert_eq!(self.model, before.model);
            assert_eq!(self.registers, before.registers);
            assert_eq!(self.oracle, before.oracle);
        }
        self.model.assert_invariants();
        self.oracle.assert_invariants();
        actual
    }
}

#[test]
fn bounded_multi_place_words_and_composed_suffixes() {
    let mut traces = 0;
    let mut steps = 0;
    let mut rejections = [0; 3];
    for policy in [Policy::SharedView, Policy::ReadOnlySlot] {
        for coalesced in [false, true] {
            let initial = Runner::new(policy, coalesced);
            for word in 0..OPS.len().pow(3) {
                let mut runner = initial.clone();
                let mut digits = word;
                for _ in 0..3 {
                    let result = runner.step(OPS[digits % OPS.len()]);
                    count_rejection(result, &mut rejections);
                    digits /= OPS.len();
                    steps += 1;
                }
                traces += 1;
            }
            for prefix in PREFIXES {
                for word in 0..OPS.len().pow(2) {
                    let mut runner = initial.clone();
                    for op in prefix {
                        count_rejection(runner.step(op), &mut rejections);
                        steps += 1;
                    }
                    let mut digits = word;
                    for _ in 0..2 {
                        count_rejection(runner.step(OPS[digits % OPS.len()]), &mut rejections);
                        digits /= OPS.len();
                        steps += 1;
                    }
                    traces += 1;
                }
            }
        }
    }
    assert_eq!(traces, 64_512);
    assert_eq!(steps, 239_616);
    assert!(rejections.iter().all(|count| *count > 0));
}

fn count_rejection(result: Result<(), Error>, counts: &mut [usize; 3]) {
    match result {
        Err(Error::OriginFamily) => counts[0] += 1,
        Err(Error::Validity) => counts[1] += 1,
        Err(Error::Invalidated) => counts[2] += 1,
        _ => {}
    }
}

#[test]
fn composed_witnesses_check_return_and_loaded_target_identity() {
    let mut witnesses = [0; 3];
    for policy in [Policy::SharedView, Policy::ReadOnlySlot] {
        for coalesced in [false, true] {
            let mut returned = Runner::new(policy, coalesced);
            for op in PREFIXES[0] {
                returned.step(op).unwrap();
            }
            returned.step(Op::Invalidate(0)).unwrap();
            returned.step(Op::Read(8)).unwrap();
            returned.step(Op::Invalidate(1)).unwrap();
            assert_eq!(returned.step(Op::Read(8)), Err(Error::Invalidated));
            witnesses[0] += 1;
            let mut stored = Runner::new(policy, coalesced);
            for op in PREFIXES[1] {
                stored.step(op).unwrap();
            }
            stored.step(Op::Invalidate(0)).unwrap();
            stored.step(Op::Read(7)).unwrap();
            stored.step(Op::Invalidate(1)).unwrap();
            assert_eq!(stored.step(Op::Read(7)), Err(Error::Invalidated));
            witnesses[1] += 1;
            let mut stale = Runner::new(policy, coalesced);
            for op in PREFIXES[3].into_iter().take(4) {
                stale.step(op).unwrap();
            }
            assert_eq!(stale.step(Op::Read(7)), Err(Error::InaccessibleHandle));
            witnesses[2] += 1;
        }
    }
    assert_eq!(witnesses, [4; 3]);
}
