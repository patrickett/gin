//! Primitive feasibility witnesses, not verification of a Gin library declaration.
//! The model already knows storage lifecycle operations; no Region type is added here.
//! Reading initialization below is a harness observation, not a source proof/refinement.

use super::model::{
    Construction, Error, Initialization, Mode, Model, OWNED_CAPACITY_MAX, OwnershipObligations,
    Policy, StorageOwner,
};

fn footprint(model: &Model, owner: StorageOwner, capacity: u8) -> u8 {
    assert!(capacity <= OWNED_CAPACITY_MAX);
    let mut occupied = 0;
    for index in 0..capacity {
        match model.initialization(owner, index).unwrap() {
            Initialization::Initialized => occupied |= 1 << index,
            Initialization::Uninitialized => {}
            Initialization::Unknown => panic!("this corpus supplies only known footprints"),
        }
    }
    assert!(occupied < (1 << capacity));
    occupied
}

#[test]
fn primitive_storage_supports_holes_and_count_alone_does_not_establish_a_prefix() {
    let mut cases = 0;
    let mut prefixes = 0;
    for capacity in 0..=OWNED_CAPACITY_MAX {
        for occupied in 0u8..(1 << capacity) {
            let mut model = Model::new(Policy::ReadOnlySlot);
            let domain = model.new_domain().unwrap();
            let allocator = model.new_allocator().unwrap();
            let owner = model
                .reserve_storage(domain, allocator, capacity, 3)
                .unwrap();
            for index in 0..capacity {
                if occupied & (1 << index) != 0 {
                    model
                        .construct_element(owner, index, Construction::Succeed)
                        .unwrap();
                }
            }
            assert_eq!(footprint(&model, owner, capacity), occupied);
            let count = occupied.count_ones() as u8;
            assert_eq!(
                model.ownership_obligations(),
                OwnershipObligations {
                    storage: 1,
                    values: count
                }
            );
            // This is a predicate over observed state, not a contract accepted by a checker.
            prefixes += usize::from(occupied == (1 << count) - 1);
            for index in 0..capacity {
                if occupied & (1 << index) != 0 {
                    let view = model.borrow_element(owner, index, Mode::Read).unwrap();
                    model.read(view).unwrap();
                    model.destroy_element(owner, index).unwrap();
                    assert_eq!(model.read(view), Err(Error::Invalidated));
                } else {
                    let before = model.clone();
                    assert_eq!(
                        model.borrow_element(owner, index, Mode::Read),
                        Err(Error::Uninitialized)
                    );
                    assert_eq!(model, before);
                }
            }
            model.reclaim_storage(owner, allocator).unwrap();
            model.check_ownership_complete().unwrap();
            cases += 1;
        }
    }
    assert_eq!((cases, prefixes, cases - prefixes), (15, 10, 5));
}

#[test]
fn bounded_prefix_algorithm_preserves_observed_state_across_failure_and_extraction() {
    for length in 0..=OWNED_CAPACITY_MAX {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let domain = model.new_domain().unwrap();
        let allocator = model.new_allocator().unwrap();
        let owner = model
            .reserve_storage(domain, allocator, OWNED_CAPACITY_MAX, 3)
            .unwrap();
        for index in 0..length {
            model
                .construct_element(owner, index, Construction::Succeed)
                .unwrap();
            assert_eq!(
                footprint(&model, owner, OWNED_CAPACITY_MAX),
                (1 << (index + 1)) - 1
            );
        }
        if length < OWNED_CAPACITY_MAX {
            let before = model.clone();
            assert_eq!(
                model.construct_element(owner, length, Construction::FailBeforeCommit),
                Err(Error::ConstructionFailed)
            );
            assert_eq!(model, before);
        }
        for index in (0..length).rev() {
            let value = model.extract_element(owner, index).unwrap();
            assert_eq!(
                footprint(&model, owner, OWNED_CAPACITY_MAX),
                (1 << index) - 1
            );
            assert_eq!(
                model.ownership_obligations(),
                OwnershipObligations {
                    storage: 1,
                    values: index + 1
                }
            );
            model.destroy_value(value).unwrap();
        }
        model.reclaim_storage(owner, allocator).unwrap();
        model.check_ownership_complete().unwrap();
    }
}

#[test]
fn allocation_origin_and_coordinated_aliases_are_independent_requirements() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let other_allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 2, 3).unwrap();
    model
        .construct_element(owner, 1, Construction::Succeed)
        .unwrap();
    let first = model.borrow_element(owner, 1, Mode::Write).unwrap();
    let second = model.copy_handle(first).unwrap();
    model.write(first).unwrap();
    model.write(second).unwrap();
    assert_eq!(
        model.ownership_obligations(),
        OwnershipObligations {
            storage: 1,
            values: 1
        }
    );
    let lease = model.protect(first, Mode::Read).unwrap();
    let before = model.clone();
    assert_eq!(model.write(second), Err(Error::LeaseConflict));
    assert_eq!(model.destroy_element(owner, 1), Err(Error::LeaseConflict));
    assert_eq!(
        model.reclaim_storage(owner, other_allocator),
        Err(Error::WrongAllocator)
    );
    assert_eq!(model, before);
    model.release(lease).unwrap();
    model.destroy_element(owner, 1).unwrap();
    assert_eq!(model.read(first), Err(Error::Invalidated));
    assert_eq!(model.read(second), Err(Error::Invalidated));
    let before = model.clone();
    assert_eq!(
        model.reclaim_storage(owner, other_allocator),
        Err(Error::WrongAllocator)
    );
    assert_eq!(model, before);
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}
