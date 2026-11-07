//! Owning storage participates in the same locator, lease, and support machinery.

use super::model::{Error, Mode, Model, Policy};

#[test]
fn storage_locators_do_not_prove_initialized_values_or_allow_generic_destruction() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let storage = model.borrow_storage(owner, Mode::Write).unwrap();
    let copy = model.copy_handle(storage).unwrap();
    model.read(storage).unwrap(); // Storage identity, not a read of an element T.
    assert_eq!(
        model.borrow_element(owner, 0, Mode::Read),
        Err(Error::Uninitialized)
    );
    let before = model.clone();
    assert_eq!(model.write(storage), Err(Error::OwnershipRequired));
    assert_eq!(model.invalidate(storage), Err(Error::OwnershipRequired));
    assert_eq!(model, before);
    let lease = model.protect(storage, Mode::Read).unwrap();
    let before = model.clone();
    assert_eq!(
        model.reclaim_storage(owner, allocator),
        Err(Error::LeaseConflict)
    );
    assert_eq!(model, before);
    model.release(lease).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    assert_eq!(model.read(storage), Err(Error::Invalidated));
    assert_eq!(model.read(copy), Err(Error::Invalidated));
    model.check_ownership_complete().unwrap();
}

#[test]
fn slots_can_store_managed_element_views_without_owning_their_targets() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let slot_domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let value = model.new_owned_value().unwrap();
    model.initialize_element(owner, 0, value).unwrap();
    let element = model.borrow_element(owner, 0, Mode::Write).unwrap();
    let storage = model.new_place(slot_domain, 1, None).unwrap();
    let slot = model.new_handle(storage, Mode::Write).unwrap();
    model
        .new_slot_from_handles(slot, element, &[element], 3)
        .unwrap();
    let loaded = model.load_slot(slot).unwrap();
    model.write(loaded).unwrap();
    let before = model.clone();
    assert_eq!(model.invalidate(loaded), Err(Error::OwnershipRequired));
    assert_eq!(model, before);
    model.destroy_element(owner, 0).unwrap();
    assert_eq!(model.read(loaded), Err(Error::Invalidated));
    let before = model.clone();
    assert_eq!(model.load_slot(slot), Err(Error::Invalidated));
    assert_eq!(model, before);
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}

#[test]
fn slot_setup_cannot_overwrite_managed_places_or_bypass_write_and_validity_checks() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let domain = model.new_domain().unwrap();
    let allocator = model.new_allocator().unwrap();
    let owner = model.reserve_storage(domain, allocator, 1, 3).unwrap();
    let value = model.new_owned_value().unwrap();
    model.initialize_element(owner, 0, value).unwrap();
    let element = model.borrow_element(owner, 0, Mode::Write).unwrap();
    let backing = model.borrow_storage(owner, Mode::Write).unwrap();
    let storage = model.new_place(domain, 1, None).unwrap();
    let slot = model.new_handle(storage, Mode::Write).unwrap();
    let read_only = model.new_handle(storage, Mode::Read).unwrap();
    let dead = model.new_place(domain, 3, None).unwrap();
    let dead = model.new_handle(dead, Mode::Write).unwrap();
    model.invalidate(dead).unwrap();
    let before = model.clone();
    for managed in [backing, element] {
        assert_eq!(
            model.new_slot_from_handles(managed, element, &[element], 3),
            Err(Error::OwnershipRequired)
        );
        assert_eq!(model, before);
    }
    assert_eq!(
        model.new_slot_from_handles(read_only, element, &[element], 3),
        Err(Error::ReadOnly)
    );
    assert_eq!(
        model.new_slot_from_handles(slot, dead, &[element], 3),
        Err(Error::Invalidated)
    );
    assert_eq!(
        model.new_slot_from_handles(slot, element, &[element, dead], 3),
        Err(Error::Invalidated)
    );
    assert_eq!(model, before);
    model
        .new_slot_from_handles(slot, element, &[element], 3)
        .unwrap();
    model.destroy_element(owner, 0).unwrap();
    model.reclaim_storage(owner, allocator).unwrap();
    model.check_ownership_complete().unwrap();
}
