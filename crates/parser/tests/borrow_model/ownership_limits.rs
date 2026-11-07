//! Accepted ownership entry must not consume the identity capacity needed to unwind.

use super::model::{Error, Initialization, LIMIT, Model, Policy};

#[test]
fn last_owner_token_slot_cannot_be_consumed_by_an_authority_free_call() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let mut owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    for _ in 1..LIMIT - 1 {
        owner = model.transfer_owner(owner).unwrap();
    }
    let before = model.clone();
    assert_eq!(model.enter_owner_call(owner, &[]), Err(Error::Limit));
    assert_eq!(model, before);
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn last_value_token_slot_cannot_be_consumed_by_call_entry() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let mut value = model.new_owned_value().unwrap();
    for _ in 1..LIMIT - 1 {
        value = model.transfer_value(value).unwrap();
    }
    let before = model.clone();
    assert_eq!(
        model.enter_ownership_call(&[], &[value], &[]),
        Err(Error::Limit)
    );
    assert_eq!(model, before);
    model.destroy_value(value).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn ordinary_moves_and_new_values_cannot_consume_reserved_return_capacity() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let mut owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let mut value = model.new_owned_value().unwrap();
    for _ in 1..LIMIT - 2 {
        owner = model.transfer_owner(owner).unwrap();
        value = model.transfer_value(value).unwrap();
    }
    let (owners, values) = model.enter_ownership_call(&[owner], &[value], &[]).unwrap();
    let before = model.clone();
    assert_eq!(model.transfer_owner(owners[0]), Err(Error::Limit));
    assert_eq!(model.transfer_value(values[0]), Err(Error::Limit));
    assert_eq!(model.new_owned_value(), Err(Error::Limit));
    assert_eq!(model.return_owner(owners[0], 4), Err(Error::Validity));
    assert_eq!(model, before);
    let owner = model.return_owner(owners[0], 3).unwrap();
    let value = model.return_value(values[0]).unwrap();
    model.leave_call().unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.destroy_value(value).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn nested_calls_protect_the_entire_owner_and_value_unwind_path() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let mut owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let mut value = model.new_owned_value().unwrap();
    for _ in 1..LIMIT - 4 {
        owner = model.transfer_owner(owner).unwrap();
        value = model.transfer_value(value).unwrap();
    }
    let (outer_owners, outer_values) = model.enter_ownership_call(&[owner], &[value], &[]).unwrap();
    let (inner_owners, inner_values) = model
        .enter_ownership_call(&outer_owners, &outer_values, &[])
        .unwrap();
    let before = model.clone();
    assert_eq!(
        model.enter_ownership_call(&inner_owners, &inner_values, &[]),
        Err(Error::Limit)
    );
    assert_eq!(model, before);
    let owner = model.return_owner(inner_owners[0], 3).unwrap();
    let value = model.return_value(inner_values[0]).unwrap();
    model.leave_call().unwrap();
    let owner = model.return_owner(owner, 3).unwrap();
    let value = model.return_value(value).unwrap();
    model.leave_call().unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.destroy_value(value).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn unknown_storage_remains_returnable_even_when_local_cleanup_is_forbidden() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let mut owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    model.forget_initialization(owner, 0).unwrap();
    for _ in 1..LIMIT - 2 {
        owner = model.transfer_owner(owner).unwrap();
    }
    let callee = model.enter_owner_call(owner, &[]).unwrap();
    let returned = model.return_owner(callee, 3).unwrap();
    model.leave_call().unwrap();
    assert_eq!(
        model.initialization(returned, 0),
        Ok(Initialization::Unknown)
    );
    assert_eq!(
        model.reclaim_storage(returned, allocator),
        Err(Error::InitializationUnknown)
    );
    model.assert_invariants();
}

#[test]
fn packing_consumes_detached_return_capacity_but_preserves_the_value_obligation() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let mut value = model.new_owned_value().unwrap();
    for _ in 1..LIMIT - 2 {
        value = model.transfer_value(value).unwrap();
    }
    let (owners, values) = model
        .enter_ownership_call(&[owner], &[value], &[domain])
        .unwrap();
    model.initialize_element(owners[0], 0, values[0]).unwrap();
    let extracted = model.extract_element(owners[0], 0).unwrap();
    let before = model.clone();
    // Extraction issues a new standalone output, not the consumed input token.
    assert_eq!(model.return_value(extracted), Err(Error::Limit));
    assert_eq!(model, before);
    model.destroy_value(extracted).unwrap();
    let owner = model.return_owner(owners[0], 3).unwrap();
    model.leave_call().unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}
