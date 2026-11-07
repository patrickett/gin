//! Composition of owning arguments with the existing call/authority model.

use super::model::{Error, Initialization, LIMIT, Model, Policy};

#[test]
fn one_call_can_receive_two_storage_owners_and_an_initialized_owned_value() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let first = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let second = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let input = model.new_owned_value().unwrap();
    let (owners, values) = model
        .enter_ownership_call(&[first, second], &[input], &[domain])
        .unwrap();
    assert_eq!(owners.len(), 2);
    assert_eq!(values.len(), 1);
    assert_eq!(model.initialization(first, 0), Err(Error::OwnerMoved));
    assert_eq!(model.destroy_value(input), Err(Error::ValueMoved));
    model.initialize_element(owners[0], 0, values[0]).unwrap();
    assert_eq!(
        model.initialization(owners[0], 0),
        Ok(Initialization::Initialized)
    );
    model.destroy_element(owners[0], 0).unwrap();
    model.reclaim_storage(owners[0], allocator).unwrap();
    model.reclaim_storage(owners[1], allocator).unwrap();
    model.leave_call().unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn an_owned_value_can_be_passed_without_a_storage_owner_or_domain_authority() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let original = model.new_owned_value().unwrap();
    let (owners, values) = model.enter_ownership_call(&[], &[original], &[]).unwrap();
    assert!(owners.is_empty());
    assert_eq!(values.len(), 1);
    assert_eq!(model.destroy_value(original), Err(Error::ValueMoved));
    let before = model.clone();
    assert_eq!(model.leave_call(), Err(Error::OutstandingOwnership));
    assert_eq!(model, before);
    let returned = model.return_value(values[0]).unwrap();
    model.leave_call().unwrap();
    model.destroy_value(returned).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn duplicate_owning_arguments_refuse_without_moving_tokens_or_entering_the_call() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let value = model.new_owned_value().unwrap();
    let before = model.clone();
    assert_eq!(
        model.enter_ownership_call(&[owner, owner], &[value], &[domain]),
        Err(Error::DuplicateOwnership)
    );
    assert_eq!(model, before);
    assert_eq!(
        model.enter_ownership_call(&[owner], &[value, value], &[domain]),
        Err(Error::DuplicateOwnership)
    );
    assert_eq!(model, before);
    assert_eq!(
        model.enter_ownership_call(&[owner; LIMIT + 1], &[], &[domain]),
        Err(Error::Limit)
    );
    assert_eq!(model, before);
    model.destroy_value(value).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn midbatch_owner_history_exhaustion_rolls_back_every_earlier_move_and_authority() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let mut first = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let second = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    for _ in 2..LIMIT - 1 {
        first = model.transfer_owner(first).unwrap();
    }
    let before = model.clone();
    assert_eq!(
        model.enter_ownership_call(&[first, second], &[], &[domain]),
        Err(Error::Limit)
    );
    assert_eq!(model, before);
    model.reclaim_storage(first, allocator).unwrap();
    model.reclaim_storage(second, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn failed_value_transfer_rolls_back_the_already_moved_storage_owner() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let mut value = model.new_owned_value().unwrap();
    for _ in 1..LIMIT {
        value = model.transfer_value(value).unwrap();
    }
    let before = model.clone();
    assert_eq!(
        model.enter_ownership_call(&[owner], &[value], &[domain]),
        Err(Error::Limit)
    );
    assert_eq!(model, before);
    model.destroy_value(value).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn batch_entry_cannot_capture_ancestor_scoped_owners_or_values() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let value = model.new_owned_value().unwrap();
    model.enter_call(&[domain]).unwrap();
    let before = model.clone();
    assert_eq!(
        model.enter_ownership_call(&[owner], &[], &[domain]),
        Err(Error::OwnerNotHeld)
    );
    assert_eq!(
        model.enter_ownership_call(&[], &[value], &[]),
        Err(Error::ValueNotHeld)
    );
    assert_eq!(model, before);
    model.leave_call().unwrap();
    model.destroy_value(value).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn callee_created_owners_and_values_can_return_using_unreserved_capacity() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    model.enter_call(&[domain]).unwrap();
    let local = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let value = model.new_owned_value().unwrap();
    let returned = model.return_owner(local, 3).unwrap();
    let value = model.return_value(value).unwrap();
    model.leave_call().unwrap();
    model.reclaim_storage(returned, allocator).unwrap();
    model.destroy_value(value).unwrap();
    model.check_ownership_complete().unwrap();
}
