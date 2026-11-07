//! Five (owner,value) historical ticket budgets: (2,2), (8,8), (12,12),
//! (2,15), (15,2). Each starts with two live empty owners and two detached values.
//! Prefill: 58 transfers. Each budget has both passed/withheld authority and
//! cached attempted prefixes of lengths 0,3,5. Their 80 attempts accept 61/refuse 19.
//! All 28^2 short suffixes: 23,520 traces / 47,040 transitions.
//! Logical lengths 2,5,7 total 109,760 transitions, excluding 80 cached attempts
//! and prefill. This bounded corpus is separate from the original call corpus.
//! At most three packs are attempted per path: two backing places plus <=3 new
//! incarnations cannot exhaust 16 places. Place exhaustion is excluded.

use super::*;

impl Oracle {
    pub(super) fn pack(&mut self, owner: usize, value: usize) -> Result<(), Error> {
        self.owners
            .held(self.owners.current[owner], self.here(), true)?;
        if self.cells[owner].is_some() {
            return Err(Error::AlreadyInitialized);
        }
        let index = self.values.current[value];
        self.values.held(index, self.here(), false)?;
        if self.authority != self.here() {
            return Err(Error::NoAuthority);
        }
        self.values.tickets[index].location = None;
        self.values.resources[value].borrowers.clear();
        self.cells[owner] = Some(value);
        Ok(())
    }

    pub(super) fn extract(&mut self, owner: usize) -> Result<(), Error> {
        self.owners
            .held(self.owners.current[owner], self.here(), true)?;
        let value = self.cells[owner].ok_or(Error::Uninitialized)?;
        if self.authority != self.here() {
            return Err(Error::NoAuthority);
        }
        self.values.issue(self.values.current[value], self.here())?;
        self.cells[owner] = None;
        Ok(())
    }

    pub(super) fn destroy_cell(&mut self, owner: usize) -> Result<(), Error> {
        self.owners
            .held(self.owners.current[owner], self.here(), true)?;
        let value = self.cells[owner].ok_or(Error::Uninitialized)?;
        if self.authority != self.here() {
            return Err(Error::NoAuthority);
        }
        self.cells[owner] = None;
        self.values.resources[value].alive = false;
        Ok(())
    }

    pub(super) fn probe_locations(&self) {
        for resource in 0..2 {
            let detached = self
                .values
                .tickets
                .iter()
                .filter(|ticket| ticket.resource == resource && ticket.location.is_some())
                .count();
            let packed = self
                .cells
                .iter()
                .filter(|cell| **cell == Some(resource))
                .count();
            let destroyed = usize::from(!self.values.resources[resource].alive);
            assert_eq!(detached + packed + destroyed, 1);
            if packed == 1 {
                assert!(self.values.resources[resource].borrowers.is_empty());
            }
        }
    }
}

const PACKING_ALPHABET: [Step; 28] = [
    Step::Pack(0, 0),
    Step::Pack(0, 1),
    Step::Pack(1, 0),
    Step::Pack(1, 1),
    Step::Extract(0),
    Step::Extract(1),
    Step::Batch(false),
    Step::Batch(true),
    Step::Enter(false),
    Step::Enter(true),
    Step::Leave,
    Step::ReturnOwner(0, 3),
    Step::ReturnOwner(1, 3),
    Step::ReturnOwner(0, 4),
    Step::ReturnValue(0),
    Step::ReturnValue(1),
    Step::MoveValue(0),
    Step::MoveValue(1),
    Step::Destroy(0),
    Step::Destroy(1),
    Step::Reclaim(0),
    Step::Reclaim(1),
    Step::DestroyCell(0),
    Step::DestroyCell(1),
    Step::OwnerCall(0, false),
    Step::OwnerCall(0, true),
    Step::OwnerCall(1, false),
    Step::OwnerCall(1, true),
];

#[test]
fn packing_suffixes_match_independent_resource_locations() {
    let mut counts = [0; 4];
    let mut prefill = 0;
    let mut logical_transitions = 0;
    for (owners, values) in [(2, 2), (8, 8), (12, 12), (2, 15), (15, 2)] {
        let fixture = Runner::fixture(owners, values);
        prefill += owners + values - 4;
        for pass in [false, true] {
            let prefixes = [
                vec![],
                vec![Step::Batch(pass), Step::Batch(pass), Step::Pack(1, 0)],
                vec![
                    Step::Pack(0, 1),
                    Step::SingleOwner,
                    Step::ReturnOwner(0, 3),
                    Step::Leave,
                    Step::Extract(0),
                ],
            ];
            for prefix in prefixes {
                logical_transitions += 28 * 28 * (prefix.len() + 2);
                let mut base = fixture.clone();
                for step in prefix {
                    counts[0] += 1;
                    counts[1] += usize::from(base.step(step).is_ok());
                }
                for first in PACKING_ALPHABET {
                    for second in PACKING_ALPHABET {
                        let mut runner = base.clone();
                        let _ = runner.step(first);
                        let _ = runner.step(second);
                        counts[2] += 1;
                        counts[3] += 2;
                    }
                }
            }
        }
    }
    assert_eq!(counts, [80, 61, 23_520, 47_040]);
    assert_eq!(counts[0] - counts[1], 19);
    assert_eq!((prefill, logical_transitions), (58, 109_760));
}

#[test]
fn populated_owner_inputs_and_refusals_preserve_resources() {
    let mut withheld = Runner::fixture(2, 2);
    withheld.step(Step::Batch(false)).unwrap();
    assert_eq!(withheld.step(Step::Pack(1, 0)), Err(Error::NoAuthority));
    assert_eq!(withheld.oracle.cells, [None; 2]);
    assert_eq!(withheld.step(Step::Destroy(0)), Ok(()));
    let mut runner = Runner::fixture(2, 2);
    assert_eq!(runner.step(Step::Extract(1)), Err(Error::Uninitialized));
    runner.step(Step::Pack(1, 0)).unwrap();
    assert_eq!(
        runner.step(Step::Pack(1, 1)),
        Err(Error::AlreadyInitialized)
    );
    assert_eq!(runner.step(Step::Reclaim(1)), Err(Error::LiveElements));
    runner.step(Step::OwnerCall(1, false)).unwrap();
    assert_eq!(runner.step(Step::Pack(0, 1)), Err(Error::OwnerNotHeld));
    assert_eq!(runner.step(Step::Extract(1)), Err(Error::NoAuthority));
    runner.step(Step::ReturnOwner(1, 3)).unwrap();
    runner.step(Step::Leave).unwrap();
    runner.step(Step::OwnerCall(1, true)).unwrap();
    runner.step(Step::Extract(1)).unwrap();
    assert_eq!(runner.step(Step::Pack(1, 1)), Err(Error::ValueNotHeld));
    runner.step(Step::Pack(1, 0)).unwrap();
    runner.step(Step::ReturnOwner(1, 3)).unwrap();
    runner.step(Step::Leave).unwrap();
    assert_eq!(runner.step(Step::Batch(false)), Err(Error::ValueMoved));
    runner.step(Step::OwnerCall(0, false)).unwrap();
    runner.step(Step::ReturnOwner(0, 3)).unwrap();
    runner.step(Step::Leave).unwrap();
    runner.step(Step::DestroyCell(1)).unwrap();
    runner.step(Step::Destroy(1)).unwrap();
    for resource in 0..2 {
        runner.step(Step::Reclaim(resource)).unwrap();
    }
    runner.model.check_ownership_complete().unwrap();
}

#[test]
fn packing_clears_only_its_nested_input_credits() {
    let mut runner = Runner::fixture(8, 8);
    runner.step(Step::Batch(true)).unwrap();
    runner.step(Step::Batch(true)).unwrap();
    assert_eq!(runner.oracle.values.used(), 16);
    runner.step(Step::Pack(1, 0)).unwrap();
    assert!(runner.oracle.values.resources[0].borrowers.is_empty());
    assert_eq!(runner.oracle.values.resources[1].borrowers.len(), 2);
    for _ in 0..2 {
        runner.step(Step::MoveValue(1)).unwrap();
    }
    assert_eq!(runner.step(Step::MoveValue(1)), Err(Error::Limit));
    assert_eq!(runner.step(Step::Extract(1)), Err(Error::Limit));
    assert_eq!(runner.oracle.cells[1], Some(0));
    for _ in 0..2 {
        runner.step(Step::ReturnOwner(0, 3)).unwrap();
        runner.step(Step::ReturnOwner(1, 3)).unwrap();
        runner.step(Step::ReturnValue(1)).unwrap();
        runner.step(Step::Leave).unwrap();
        assert_eq!(runner.model.ownership_obligations().values, 2);
    }
    runner.step(Step::DestroyCell(1)).unwrap();
    runner.step(Step::Destroy(1)).unwrap();
    for resource in 0..2 {
        runner.step(Step::Reclaim(resource)).unwrap();
    }
    runner.model.check_ownership_complete().unwrap();
}

#[test]
fn extracted_output_has_no_inherited_return_credit() {
    let mut runner = Runner::fixture(2, 8);
    runner.step(Step::Batch(true)).unwrap();
    runner.step(Step::Batch(true)).unwrap();
    runner.step(Step::Pack(0, 1)).unwrap();
    for resource in 0..2 {
        runner.step(Step::ReturnOwner(resource, 3)).unwrap();
    }
    runner.step(Step::ReturnValue(0)).unwrap();
    runner.step(Step::Leave).unwrap();
    runner.step(Step::Extract(0)).unwrap();
    runner.step(Step::MoveValue(1)).unwrap();
    assert_eq!(runner.oracle.values.used(), 16);
    assert_eq!(runner.step(Step::ReturnValue(1)), Err(Error::Limit));
    runner.step(Step::Destroy(1)).unwrap();
    runner.step(Step::ReturnValue(0)).unwrap();
    for resource in 0..2 {
        runner.step(Step::Reclaim(resource)).unwrap();
    }
    runner.step(Step::Leave).unwrap();
    runner.step(Step::Destroy(0)).unwrap();
    runner.model.check_ownership_complete().unwrap();
}
