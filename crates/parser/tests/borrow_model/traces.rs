use super::model::{Domain, Effect, Error, Fact, Handle, Mode, Model, Policy};

const OPERATIONS: [Operation; 9] = [
    Operation::Read,
    Operation::Write,
    Operation::Invalidate,
    Operation::Protect,
    Operation::Enter,
    Operation::EnterEmpty,
    Operation::Leave,
    Operation::Unknown,
    Operation::CheckFact,
];
const STEPS: u32 = 5;

#[derive(Debug, Clone, Copy)]
enum Operation {
    Read,
    Write,
    Invalidate,
    Protect,
    Enter,
    EnterEmpty,
    Leave,
    Unknown,
    CheckFact,
}

/// Independent, single-allocation reference table, not a second full borrow checker.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Oracle {
    alive: bool,
    depth: u8,
    authority_at: u8,
    protected_at: Option<u8>,
    fact_current: bool,
}

impl Oracle {
    fn access(&self) -> Result<(), Error> {
        if !self.alive {
            return Err(Error::Invalidated);
        }
        if self.authority_at != self.depth {
            return Err(Error::NoAuthority);
        }
        if self.protected_at.is_some() {
            return Err(Error::LeaseConflict);
        }
        Ok(())
    }

    fn step(&mut self, operation: Operation) -> Result<(), Error> {
        match operation {
            Operation::Read => self.access(),
            Operation::Write => {
                self.access()?;
                self.fact_current = false;
                Ok(())
            }
            Operation::Invalidate => {
                self.access()?;
                self.alive = false;
                Ok(())
            }
            Operation::Protect => {
                self.access()?;
                self.protected_at = Some(self.depth);
                Ok(())
            }
            Operation::Enter => self.enter(true),
            Operation::EnterEmpty => self.enter(false),
            Operation::Leave => self.leave(),
            Operation::Unknown => Err(Error::UnknownEffect),
            Operation::CheckFact => {
                self.access()?;
                if !self.fact_current {
                    return Err(Error::ForgottenFact);
                }
                Ok(())
            }
        }
    }

    fn enter(&mut self, lend: bool) -> Result<(), Error> {
        if self.depth == 4 {
            return Err(Error::Limit);
        }
        if lend {
            if self.authority_at != self.depth {
                return Err(Error::NoAuthority);
            }
            if self.protected_at.is_some() {
                return Err(Error::LeaseConflict);
            }
            self.authority_at += 1;
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) -> Result<(), Error> {
        if self.depth == 0 {
            return Err(Error::NoCaller);
        }
        if self.protected_at == Some(self.depth) {
            return Err(Error::OutstandingLease);
        }
        if self.authority_at == self.depth {
            self.authority_at -= 1;
        }
        self.depth -= 1;
        Ok(())
    }

    fn assert_invariants(&self) {
        assert!(self.depth <= 4);
        assert!(self.authority_at <= self.depth);
        if let Some(frame) = self.protected_at {
            assert!(frame <= self.depth);
            assert_eq!(frame, self.authority_at);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelTrace {
    model: Model,
    domain: Domain,
    handle: Handle,
    fact: Fact,
}

impl ModelTrace {
    fn new() -> Self {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let domain = model.new_domain().unwrap();
        let place = model.new_place(domain, 3, None).unwrap();
        let handle = model.new_handle(place, Mode::Write).unwrap();
        let fact = model.remember(handle).unwrap();
        Self {
            model,
            domain,
            handle,
            fact,
        }
    }

    fn step(&mut self, operation: Operation) -> Result<(), Error> {
        match operation {
            Operation::Read => self.model.read(self.handle),
            Operation::Write => self.model.write(self.handle),
            Operation::Invalidate => self.model.invalidate(self.handle),
            Operation::Protect => self.model.protect(self.handle, Mode::Write).map(|_| ()),
            Operation::Enter => self.model.enter_call(&[self.domain]),
            Operation::EnterEmpty => self.model.enter_call(&[]),
            Operation::Leave => self.model.leave_call(),
            Operation::Unknown => self.model.apply_summary(&[Effect::Unknown]),
            Operation::CheckFact => self.model.check_fact(self.fact),
        }
    }
}

#[test]
fn all_five_step_traces_match_the_small_oracle_and_errors_preserve_state() {
    let initial = ModelTrace::new();
    let initial_oracle = Oracle {
        alive: true,
        depth: 0,
        authority_at: 0,
        protected_at: None,
        fact_current: true,
    };
    let traces = OPERATIONS.len().pow(STEPS);
    assert_eq!(traces, 59_049);
    for trace in 0..traces {
        let mut execution = initial.clone();
        let mut oracle = initial_oracle.clone();
        let mut digits = trace;
        for step in 0..STEPS {
            let operation = OPERATIONS[digits % OPERATIONS.len()];
            digits /= OPERATIONS.len();
            let before = execution.clone();
            let oracle_before = oracle.clone();
            let expected = oracle.step(operation);
            let result = execution.step(operation);
            assert_eq!(
                result, expected,
                "trace {trace}, step {step}: {operation:?}"
            );
            if result.is_err() {
                assert_eq!(execution, before, "failed transition mutated trace {trace}");
                assert_eq!(
                    oracle, oracle_before,
                    "oracle rejected after mutating trace {trace}"
                );
            }
            execution.model.assert_invariants();
            oracle.assert_invariants();
        }
    }
}
