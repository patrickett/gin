//! Ownership/initialization experiments, not lowering from parsed `own` syntax.

use super::model::{
    Construction, Effect, Error, Initialization, LIMIT, Mode, Model, OWNED_CAPACITY_MAX,
    OwnershipObligations, Policy,
};

#[test]
fn owning_uninitialized_storage_does_not_allow_element_views() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 2, 3).unwrap();
    assert_eq!(
        model.initialization(owner, 0),
        Ok(Initialization::Uninitialized)
    );
    assert_eq!(
        model.borrow_element(owner, 0, Mode::Read),
        Err(Error::Uninitialized)
    );
    assert_eq!(
        model.check_ownership_complete(),
        Err(Error::OutstandingOwnership)
    );
    model.reclaim_storage(owner, allocator).unwrap();
    assert_eq!(model.initialization(owner, 0), Err(Error::OwnerMoved));
    model.check_ownership_complete().unwrap();
    model.assert_invariants();
}

#[test]
fn owner_moves_do_not_copy_obligations_or_relocate_borrowed_elements() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let original = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    model
        .construct_element(original, 0, Construction::Succeed)
        .unwrap();
    let view = model.borrow_element(original, 0, Mode::Write).unwrap();
    let copy = model.copy_handle(view).unwrap();
    let owner = model.transfer_owner(original).unwrap();
    assert_eq!(model.initialization(original, 0), Err(Error::OwnerMoved));
    assert_eq!(
        model.ownership_obligations(),
        OwnershipObligations {
            storage: 1,
            values: 1
        }
    );
    model.write(copy).unwrap();
    model.destroy_element(owner, 0).unwrap();
    assert_eq!(model.read(view), Err(Error::Invalidated));
    assert_eq!(model.read(copy), Err(Error::Invalidated));
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn element_initialization_consumes_a_value_not_its_lifecycle_obligation() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let original = model.new_owned_value().unwrap();
    let value = model.transfer_value(original).unwrap();
    assert_eq!(model.destroy_value(original), Err(Error::ValueMoved));
    model.initialize_element(owner, 0, value).unwrap();
    assert_eq!(model.destroy_value(value), Err(Error::ValueMoved));
    let replacement = model.new_owned_value().unwrap();
    let before = model.clone();
    assert_eq!(
        model.initialize_element(owner, 0, replacement),
        Err(Error::AlreadyInitialized)
    );
    assert_eq!(model, before);
    assert_eq!(
        model.ownership_obligations(),
        OwnershipObligations {
            storage: 1,
            values: 2
        }
    );
    model.destroy_element(owner, 0).unwrap();
    model.destroy_value(replacement).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn borrowed_writes_cannot_destroy_the_owned_element_behind_the_ledger() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    model
        .construct_element(owner, 0, Construction::Succeed)
        .unwrap();
    let view = model.borrow_element(owner, 0, Mode::Write).unwrap();
    model.write(view).unwrap();
    let before = model.clone();
    assert_eq!(model.invalidate(view), Err(Error::OwnershipRequired));
    assert_eq!(model, before);
    assert_eq!(
        model.apply_summary(&[Effect::Write(view), Effect::Invalidate(view)]),
        Err(Error::OwnershipRequired)
    );
    assert_eq!(model, before);
    assert_eq!(
        model.reclaim_storage(owner, allocator),
        Err(Error::LiveElements)
    );
    assert_eq!(model, before);
    model.destroy_element(owner, 0).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
}

#[test]
fn protected_views_block_destructive_lifecycle_operations_atomically() {
    for mode in [Mode::Read, Mode::Write] {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let domain = model.new_domain().unwrap();
        let allocator = model.new_allocator().unwrap();
        let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
        model
            .construct_element(owner, 0, Construction::Succeed)
            .unwrap();
        let handle = model.borrow_element(owner, 0, Mode::Write).unwrap();
        let lease = model.protect(handle, mode).unwrap();
        let view = model.view(lease).unwrap();
        let before = model.clone();
        assert_eq!(model.extract_element(owner, 0), Err(Error::LeaseConflict));
        assert_eq!(model, before);
        assert_eq!(model.destroy_element(owner, 0), Err(Error::LeaseConflict));
        assert_eq!(model, before);
        assert_eq!(
            model.reclaim_storage(owner, allocator),
            Err(Error::LeaseConflict)
        );
        assert_eq!(model, before);
        model.read(view).unwrap();
        model.release(lease).unwrap();
        assert_eq!(model.read(view), Err(Error::LeaseExpired));
        model.destroy_element(owner, 0).unwrap();
        assert_eq!(model.read(view), Err(Error::Invalidated));
        assert_eq!(model.read(handle), Err(Error::Invalidated));
        model.reclaim_storage(owner, allocator).unwrap();
        model.check_ownership_complete().unwrap();
    }
}

#[test]
fn partial_construction_failure_preserves_committed_prefix_and_cleanup_duties() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model
        .reserve_storage(domain, allocator, OWNED_CAPACITY_MAX, 3)
        .unwrap();
    model
        .construct_element(owner, 0, Construction::Succeed)
        .unwrap();
    let before = model.clone();
    assert_eq!(
        model.construct_element(owner, 1, Construction::FailBeforeCommit),
        Err(Error::ConstructionFailed)
    );
    assert_eq!(model, before);
    assert_eq!(
        model.initialization(owner, 0),
        Ok(Initialization::Initialized)
    );
    assert_eq!(
        model.initialization(owner, 1),
        Ok(Initialization::Uninitialized)
    );
    assert_eq!(
        model.ownership_obligations(),
        OwnershipObligations {
            storage: 1,
            values: 1
        }
    );
    model.destroy_element(owner, 0).unwrap();
    assert_eq!(model.destroy_element(owner, 0), Err(Error::Uninitialized));
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn extraction_conserves_the_value_and_reinitialization_never_revives_old_views() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    model
        .construct_element(owner, 0, Construction::Succeed)
        .unwrap();
    let old = model.borrow_element(owner, 0, Mode::Read).unwrap();
    let extracted = model.extract_element(owner, 0).unwrap();
    assert_eq!(model.read(old), Err(Error::Invalidated));
    model
        .construct_element(owner, 0, Construction::Succeed)
        .unwrap();
    let fresh = model.borrow_element(owner, 0, Mode::Read).unwrap();
    model.read(fresh).unwrap();
    assert_eq!(model.read(old), Err(Error::Invalidated));
    model.destroy_element(owner, 0).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    assert_eq!(
        model.ownership_obligations(),
        OwnershipObligations {
            storage: 0,
            values: 1
        }
    );
    assert_eq!(
        model.check_ownership_complete(),
        Err(Error::OutstandingOwnership)
    );
    model.destroy_value(extracted).unwrap();
    assert_eq!(model.destroy_value(extracted), Err(Error::ValueMoved));
    model.check_ownership_complete().unwrap();
}

#[test]
fn unknown_initialization_cannot_erase_hidden_obligations_or_grant_permissions() {
    for initialized in [false, true] {
        let mut model = Model::new(Policy::ReadOnlySlot);
        let domain = model.new_domain().unwrap();
        let allocator = model.new_allocator().unwrap();
        let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
        let old_view = if initialized {
            model
                .construct_element(owner, 0, Construction::Succeed)
                .unwrap();
            Some(model.borrow_element(owner, 0, Mode::Read).unwrap())
        } else {
            None
        };
        let obligations = model.ownership_obligations();
        model.forget_initialization(owner, 0).unwrap();
        let owner = model.transfer_owner(owner).unwrap();
        assert_eq!(model.initialization(owner, 0), Ok(Initialization::Unknown));
        assert_eq!(model.ownership_obligations(), obligations);
        let value = model.new_owned_value().unwrap();
        let before = model.clone();
        assert_eq!(
            model.initialize_element(owner, 0, value),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            model.borrow_element(owner, 0, Mode::Read),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            model.extract_element(owner, 0),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            model.destroy_element(owner, 0),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(
            model.reclaim_storage(owner, allocator),
            Err(Error::InitializationUnknown)
        );
        assert_eq!(model, before);
        if let Some(view) = old_view {
            model.read(view).unwrap();
        }
        model.destroy_value(value).unwrap();
        assert_eq!(model.ownership_obligations(), obligations);
        assert_eq!(
            model.check_ownership_complete(),
            Err(Error::OutstandingOwnership)
        );
        model.assert_invariants();
    }
}

#[test]
fn captured_tokens_are_not_implicitly_transferred_to_a_callee() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    model
        .construct_element(owner, 0, Construction::Succeed)
        .unwrap();
    let view = model.borrow_element(owner, 0, Mode::Read).unwrap();
    let value = model.new_owned_value().unwrap();
    model.enter_call(&[domain]).unwrap();
    model.read(view).unwrap();
    let before = model.clone();
    assert_eq!(model.destroy_element(owner, 0), Err(Error::OwnerNotHeld));
    assert_eq!(model.transfer_owner(owner), Err(Error::OwnerNotHeld));
    assert_eq!(model.destroy_value(value), Err(Error::ValueNotHeld));
    assert_eq!(model, before);
    model.leave_call().unwrap();
    model.destroy_element(owner, 0).unwrap();
    model.destroy_value(value).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn owning_responsibility_does_not_grant_the_callees_missing_domain_authority() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let original = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let callee = model.enter_owner_call(original, &[]).unwrap();
    let before = model.clone();
    assert_eq!(
        model.construct_element(callee, 0, Construction::Succeed),
        Err(Error::NoAuthority)
    );
    assert_eq!(
        model.reclaim_storage(callee, allocator),
        Err(Error::NoAuthority)
    );
    assert_eq!(model.leave_call(), Err(Error::OutstandingOwnership));
    assert_eq!(model, before);
    let returned = model.return_owner(callee, 3).unwrap();
    assert_eq!(model.initialization(returned, 0), Err(Error::OwnerNotHeld));
    model.leave_call().unwrap();
    assert_eq!(model.initialization(original, 0), Err(Error::OwnerMoved));
    model.reclaim_storage(returned, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn storage_return_does_not_discharge_an_extracted_value_in_the_callee() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let original = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let owner = model.enter_owner_call(original, &[domain]).unwrap();
    model
        .construct_element(owner, 0, Construction::Succeed)
        .unwrap();
    let value = model.extract_element(owner, 0).unwrap();
    let owner = model.return_owner(owner, 3).unwrap();
    let before = model.clone();
    assert_eq!(model.leave_call(), Err(Error::OutstandingOwnership));
    assert_eq!(model, before);
    let returned_value = model.return_value(value).unwrap();
    assert_eq!(model.destroy_value(value), Err(Error::ValueMoved));
    assert_eq!(
        model.destroy_value(returned_value),
        Err(Error::ValueNotHeld)
    );
    model.leave_call().unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    assert_eq!(
        model.check_ownership_complete(),
        Err(Error::OutstandingOwnership)
    );
    model.destroy_value(returned_value).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn owner_return_checks_storage_validity_without_consuming_the_rejected_owner() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let original = model.reserve_storage(domain, allocator, 1, 1).unwrap();
    let callee = model.enter_owner_call(original, &[domain]).unwrap();
    let before = model.clone();
    assert_eq!(model.return_owner(callee, 3), Err(Error::Validity));
    assert_eq!(model, before);
    let owner = model.return_owner(callee, 1).unwrap();
    model.leave_call().unwrap();
    model.enter_call(&[domain]).unwrap();
    assert_eq!(model.initialization(callee, 0), Err(Error::OwnerMoved));
    assert_eq!(model.initialization(owner, 0), Err(Error::OwnerNotHeld));
    model.leave_call().unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
}

#[test]
fn allocator_identity_capacity_and_element_bounds_are_checked_atomically() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let wrong = model.new_allocator().unwrap();
    let before = model.clone();
    assert_eq!(
        model.reserve_storage(domain, allocator, OWNED_CAPACITY_MAX + 1, 3),
        Err(Error::Limit)
    );
    assert_eq!(model, before);
    let owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    let before = model.clone();
    assert_eq!(model.initialization(owner, 0), Err(Error::ElementBounds));
    assert_eq!(
        model.construct_element(owner, 0, Construction::Succeed),
        Err(Error::ElementBounds)
    );
    assert_eq!(
        model.reclaim_storage(owner, wrong),
        Err(Error::WrongAllocator)
    );
    assert_eq!(model, before);
    model.reclaim_storage(owner, allocator).unwrap();
    assert_eq!(
        model.reclaim_storage(owner, allocator),
        Err(Error::OwnerMoved)
    );
    model.check_ownership_complete().unwrap();
}

#[test]
fn exhausted_token_history_cannot_partially_move_or_prevent_final_cleanup() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let mut owner = model.reserve_storage(domain, allocator, 0, 3).unwrap();
    for _ in 1..LIMIT {
        owner = model.transfer_owner(owner).unwrap();
    }
    let before = model.clone();
    assert_eq!(model.transfer_owner(owner), Err(Error::Limit));
    assert_eq!(model.enter_owner_call(owner, &[domain]), Err(Error::Limit));
    assert_eq!(model, before);
    model.reclaim_storage(owner, allocator).unwrap();
    let mut value = model.new_owned_value().unwrap();
    for _ in 1..LIMIT {
        value = model.transfer_value(value).unwrap();
    }
    let before = model.clone();
    assert_eq!(model.transfer_value(value), Err(Error::Limit));
    assert_eq!(model, before);
    model.destroy_value(value).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn three_cell_cleanup_commits_each_step_without_rolling_back_earlier_destruction() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model
        .reserve_storage(domain, allocator, OWNED_CAPACITY_MAX, 3)
        .unwrap();
    for index in 0..OWNED_CAPACITY_MAX {
        model
            .construct_element(owner, index, Construction::Succeed)
            .unwrap();
    }
    let first = model.borrow_element(owner, 0, Mode::Read).unwrap();
    model.destroy_element(owner, 0).unwrap();
    let before = model.clone();
    assert_eq!(
        model.reclaim_storage(owner, allocator),
        Err(Error::LiveElements)
    );
    assert_eq!(model, before);
    assert_eq!(model.read(first), Err(Error::Invalidated));
    assert_eq!(
        model.ownership_obligations(),
        OwnershipObligations {
            storage: 1,
            values: 2
        }
    );
    for index in 1..OWNED_CAPACITY_MAX {
        model.destroy_element(owner, index).unwrap();
    }
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn logical_place_exhaustion_preserves_the_incoming_value_and_empty_destination() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    for _ in 1..LIMIT {
        model.new_place(domain, 3, None).unwrap();
    }
    let value = model.new_owned_value().unwrap();
    let before = model.clone();
    assert_eq!(model.initialize_element(owner, 0, value), Err(Error::Limit));
    assert_eq!(model, before);
    assert_eq!(
        model.initialization(owner, 0),
        Ok(Initialization::Uninitialized)
    );
    model.destroy_value(value).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn returned_views_do_not_acquire_ownership_and_follow_the_elements_lifecycle() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let original = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let callee = model.enter_owner_call(original, &[domain]).unwrap();
    model
        .construct_element(callee, 0, Construction::Succeed)
        .unwrap();
    let local = model.borrow_element(callee, 0, Mode::Read).unwrap();
    let view = model.return_handle(local, 3).unwrap();
    let owner = model.return_owner(callee, 3).unwrap();
    model.leave_call().unwrap();
    model.read(view).unwrap();
    assert_eq!(model.read(local), Err(Error::InaccessibleHandle));
    model.destroy_element(owner, 0).unwrap();
    assert_eq!(model.read(view), Err(Error::Invalidated));
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}
