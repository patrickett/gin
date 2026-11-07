use super::model::{Domain, Effect, Error, Handle, LIMIT, Mode, Model, Place, Policy};
use super::source::{SourceError, write_through_slot, written_modes};

fn allocation(model: &mut Model, extent: u8) -> (Domain, Place, Handle) {
    let domain = model.new_domain().unwrap();
    let place = model.new_place(domain, extent, None).unwrap();
    let handle = model.new_handle(place, Mode::Write).unwrap();
    (domain, place, handle)
}

#[test]
fn source_adapter_does_not_infer_conventions_or_borrowed_types() {
    assert_eq!(
        written_modes("f(x ref Item, y mut Item): return\n"),
        Ok(vec![Mode::Read, Mode::Write])
    );
    assert_eq!(
        written_modes("f(x Item): return\n"),
        Err(SourceError::UnwrittenConvention)
    );
    assert_eq!(
        written_modes("f(x own Item): return\n"),
        Err(SourceError::OwnershipDeferred)
    );
    // These surfaces parse, but the adapter still extracts only the convention.
    // Inner types and group bindings remain explicit semantic-scenario metadata.
    assert_eq!(
        written_modes("f(x ref mut Item): return\n"),
        Ok(vec![Mode::Read])
    );
    assert_eq!(
        written_modes("f(x mut{g} Item): return\n"),
        Ok(vec![Mode::Write])
    );
    assert_eq!(written_modes("Thing has\n"), Err(SourceError::Shape));
}

#[test]
fn parsed_parameters_can_share_one_target_without_duplicating_authority() {
    let modes = written_modes("attack(a mut Item, b mut Item): return\n").unwrap();
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, target, _) = allocation(&mut model, 3);
    let first = model.new_handle(target, modes[0]).unwrap();
    let second = model.new_handle(target, modes[1]).unwrap();
    // Domain/actual-target binding is explicit scenario metadata, not parsed group syntax.
    model.enter_call(&[domain]).unwrap();
    let old_fact = model.remember(first).unwrap();
    model.write(first).unwrap();
    model.write(second).unwrap();
    assert_eq!(model.check_fact(old_fact), Err(Error::ForgottenFact));
    model.read(first).unwrap();
    model.leave_call().unwrap();
    model.assert_invariants();
}

#[test]
fn nested_matrix_compares_policies_without_claiming_nested_syntax_support() {
    for outer in [Mode::Read, Mode::Write] {
        for inner in [Mode::Read, Mode::Write] {
            for policy in [Policy::SharedView, Policy::ReadOnlySlot] {
                check_matrix_case(outer, inner, policy);
            }
        }
    }
}

fn check_matrix_case(outer: Mode, inner: Mode, policy: Policy) {
    let source = match outer {
        Mode::Read => "f(h ref Item): return\n",
        Mode::Write => "f(h mut Item): return\n",
    };
    assert_eq!(written_modes(source).unwrap(), [outer]);
    let mut model = Model::new(policy);
    let (_, target, _) = allocation(&mut model, 3);
    let initial = model.new_handle(target, inner).unwrap();
    let (_, storage, _) = allocation(&mut model, 2);
    model.new_slot(storage, initial, &[target], 2).unwrap();
    let access = model.new_handle(storage, outer).unwrap();
    let lease = model.protect(access, outer).unwrap();
    let view = model.view(lease).unwrap();
    let target_access = model.load_slot(view).unwrap();
    model.read(target_access).unwrap();
    let allowed = inner == Mode::Write && (policy == Policy::ReadOnlySlot || outer == Mode::Write);
    let expected = if allowed {
        Ok(())
    } else {
        Err(Error::ReadOnly)
    };
    let before = model.clone();
    assert_eq!(write_through_slot(&mut model, view), expected);
    if !allowed {
        assert_eq!(model, before);
    }
    let loaded_mode = if allowed { Mode::Write } else { Mode::Read };
    assert_eq!(model.handle_mode(target_access), loaded_mode);
    let expected_retarget = match outer {
        Mode::Read => Err(Error::ReadOnly),
        Mode::Write => Ok(()),
    };
    assert_eq!(model.retarget(view, initial), expected_retarget);
    model.release(lease).unwrap();
    assert_eq!(model.read(view), Err(Error::LeaseExpired));
}

#[test]
fn copied_locators_cannot_forge_or_rename_authority() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, _, handle) = allocation(&mut model, 3);
    let unrelated = model.new_domain().unwrap();
    let snapshot = model.clone();
    assert_eq!(
        model.enter_call(&[domain, domain]),
        Err(Error::DuplicateAuthority)
    );
    assert_eq!(model, snapshot);
    model.enter_call(&[unrelated]).unwrap();
    let copied = model.copy_handle(handle).unwrap();
    assert_eq!(model.write(copied), Err(Error::NoAuthority));
    assert_eq!(model.new_domain(), Err(Error::SetupOnly));
    let returned = model.return_handle(copied, 3).unwrap();
    model.leave_call().unwrap();
    assert_eq!(model.write(copied), Err(Error::InaccessibleHandle));
    model.write(returned).unwrap();
}

#[test]
fn leases_block_reentry_and_exclusive_views_are_not_copyable() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, _, handle) = allocation(&mut model, 3);
    model.enter_call(&[domain]).unwrap();
    let lease = model.protect(handle, Mode::Write).unwrap();
    let view = model.view(lease).unwrap();
    model.write(view).unwrap();
    assert_eq!(model.copy_handle(view), Err(Error::ExclusiveCopy));
    assert_eq!(model.view(lease), Err(Error::ExclusiveCopy));
    assert_eq!(model.read(handle), Err(Error::LeaseConflict));
    assert_eq!(model.enter_call(&[domain]), Err(Error::LeaseConflict));
    assert_eq!(model.leave_call(), Err(Error::OutstandingLease));
    model.enter_call(&[]).unwrap();
    assert_eq!(model.write(handle), Err(Error::NoAuthority));
    model.leave_call().unwrap();
    model.release(lease).unwrap();
    model.enter_call(&[domain]).unwrap();
    model.write(handle).unwrap();
    model.leave_call().unwrap();
    model.leave_call().unwrap();
}

#[test]
fn protected_reads_preserve_facts_and_copies_retain_lease_dependencies() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, _, handle) = allocation(&mut model, 3);
    let fact = model.remember(handle).unwrap();
    let lease = model.protect(handle, Mode::Read).unwrap();
    let view = model.view(lease).unwrap();
    let copied = model.copy_handle(view).unwrap();
    assert_eq!(model.write(handle), Err(Error::LeaseConflict));
    assert_eq!(model.invalidate(handle), Err(Error::LeaseConflict));
    model.check_fact(fact).unwrap();
    assert_eq!(model.check_escape(copied, 1), Err(Error::OutstandingLease));
    model.release(lease).unwrap();
    assert_eq!(model.read(copied), Err(Error::LeaseExpired));
    model.write(handle).unwrap();
    assert_eq!(model.check_fact(fact), Err(Error::ForgottenFact));
}

#[test]
fn invalidation_preserves_transitive_support_and_cannot_resurrect_a_handle() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, backing, backing_handle) = allocation(&mut model, 3);
    let child_domain = model.new_domain().unwrap();
    let child = model.new_place(child_domain, 2, Some(backing)).unwrap();
    let nested = model.new_place(child_domain, 1, Some(child)).unwrap();
    let element = model.new_handle(nested, Mode::Write).unwrap();
    let copied = model.copy_handle(element).unwrap();
    model.write(element).unwrap();
    model.invalidate(backing_handle).unwrap();
    assert_eq!(model.read(copied), Err(Error::Invalidated));
    assert_eq!(
        model.new_handle(nested, Mode::Read),
        Err(Error::Invalidated)
    );
    assert_eq!(model.check_escape(copied, 1), Err(Error::Invalidated));
}

#[test]
fn copied_contents_can_escape_where_a_borrow_of_the_local_slot_cannot() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, target, original) = allocation(&mut model, 3);
    let (_, storage, _) = allocation(&mut model, 1);
    model.new_slot(storage, original, &[target], 3).unwrap();
    let access = model.new_handle(storage, Mode::Read).unwrap();
    let copied = model.load_slot(access).unwrap();
    model.check_escape(copied, 3).unwrap();
    assert_eq!(model.check_escape(access, 3), Err(Error::Validity));
}

#[test]
fn retargeting_enforces_mode_family_and_the_callers_full_validity_bound() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, first, original) = allocation(&mut model, 3);
    let (_, second, replacement) = allocation(&mut model, 3);
    let (_, local, short) = allocation(&mut model, 1);
    let (_, outsider, outside) = allocation(&mut model, 3);
    let (_, storage, _) = allocation(&mut model, 2);
    model
        .new_slot(storage, original, &[first, second, local], 3)
        .unwrap();
    let observer = model.new_handle(storage, Mode::Read).unwrap();
    let writer = model.new_handle(storage, Mode::Write).unwrap();
    let old_copy = model.load_slot(observer).unwrap();
    let read_only = model.new_handle(second, Mode::Read).unwrap();
    assert_eq!(model.retarget(observer, replacement), Err(Error::ReadOnly));
    assert_eq!(model.retarget(writer, short), Err(Error::Validity));
    assert_eq!(model.retarget(writer, outside), Err(Error::OriginFamily));
    assert_eq!(model.retarget(writer, read_only), Err(Error::HandleType));
    model.retarget(writer, replacement).unwrap();
    // Changing the slot does not change the target of an already copied locator.
    model.invalidate(original).unwrap();
    assert_eq!(model.read(old_copy), Err(Error::Invalidated));
    let new_copy = model.load_slot(observer).unwrap();
    model.read(new_copy).unwrap();
    assert_ne!(first, outsider);
}

#[test]
fn supporting_slot_replacement_invalidates_dependent_access_across_domains() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, target, original) = allocation(&mut model, 3);
    let (_, storage, writer) = allocation(&mut model, 2);
    model.new_slot(storage, original, &[target], 2).unwrap();
    let domain = model.new_domain().unwrap();
    let child = model.new_place(domain, 2, Some(storage)).unwrap();
    let dependent = model.new_handle(child, Mode::Read).unwrap();
    let copied = model.copy_handle(dependent).unwrap();
    let lease = model.protect(dependent, Mode::Read).unwrap();
    assert_eq!(model.retarget(writer, original), Err(Error::LeaseConflict));
    assert_eq!(model.invalidate(writer), Err(Error::LeaseConflict));
    model.release(lease).unwrap();
    model.retarget(writer, original).unwrap();
    assert_eq!(model.read(copied), Err(Error::Invalidated));
    assert_eq!(model.new_handle(child, Mode::Read), Err(Error::Invalidated));
}

#[test]
fn opaque_effects_fail_closed_and_failed_summaries_are_atomic() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, _, handle) = allocation(&mut model, 3);
    let snapshot = model.clone();
    assert_eq!(
        model.apply_summary(&[Effect::Write(handle), Effect::Unknown]),
        Err(Error::UnknownEffect)
    );
    assert_eq!(model, snapshot);
    let lease = model.protect(handle, Mode::Read).unwrap();
    let snapshot = model.clone();
    assert_eq!(
        model.apply_summary(&[Effect::Read(handle), Effect::Invalidate(handle)]),
        Err(Error::LeaseConflict)
    );
    assert_eq!(model, snapshot);
    model.release(lease).unwrap();
    model
        .apply_summary(&[Effect::Read(handle), Effect::Write(handle)])
        .unwrap();
    model.apply_summary(&[Effect::Invalidate(handle)]).unwrap();
    assert_eq!(model.read(handle), Err(Error::Invalidated));
}

#[test]
fn model_resources_have_explicit_refusal_bounds() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, target, handle) = allocation(&mut model, 3);
    for _ in 1..LIMIT {
        model.copy_handle(handle).unwrap();
    }
    assert_eq!(model.copy_handle(handle), Err(Error::Limit));
    assert_eq!(model.new_handle(target, Mode::Read), Err(Error::Limit));
    for _ in 0..4 {
        model.enter_call(&[domain]).unwrap();
    }
    assert_eq!(model.enter_call(&[domain]), Err(Error::Limit));
    for _ in 0..4 {
        model.leave_call().unwrap();
    }
    assert_eq!(model.leave_call(), Err(Error::NoCaller));
    model.assert_invariants();
}

#[test]
fn retargeting_through_a_protected_slot_preserves_the_slot_but_not_old_support() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, target, original) = allocation(&mut model, 3);
    let (_, storage, writer) = allocation(&mut model, 2);
    model.new_slot(storage, original, &[target], 2).unwrap();
    let lease = model.protect(writer, Mode::Write).unwrap();
    let view = model.view(lease).unwrap();
    model.retarget(view, original).unwrap();
    let contents = model.load_slot(view).unwrap();
    model.read(contents).unwrap();
    assert_eq!(model.retarget(writer, original), Err(Error::LeaseConflict));
    assert_eq!(model.invalidate(view), Err(Error::LeaseConflict));
    model.release(lease).unwrap();
    model.read(writer).unwrap();
}

#[test]
fn retargeting_cannot_install_a_handle_depending_on_the_replaced_slot_generation() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, target, original) = allocation(&mut model, 3);
    let (_, storage, writer) = allocation(&mut model, 2);
    let domain = model.new_domain().unwrap();
    let dependent = model.new_place(domain, 2, Some(storage)).unwrap();
    let replacement = model.new_handle(dependent, Mode::Write).unwrap();
    model
        .new_slot(storage, original, &[target, dependent], 2)
        .unwrap();
    let before = model.clone();
    assert_eq!(
        model.retarget(writer, replacement),
        Err(Error::SupportConflict)
    );
    assert_eq!(model, before);
    model.read(replacement).unwrap();
}

#[test]
fn every_shared_lease_must_end_before_mutation_and_foreign_frames_cannot_end_it() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, _, handle) = allocation(&mut model, 3);
    let first = model.protect(handle, Mode::Read).unwrap();
    let second = model.protect(handle, Mode::Read).unwrap();
    model.enter_call(&[]).unwrap();
    assert_eq!(model.release(first), Err(Error::NoAuthority));
    model.leave_call().unwrap();
    model.release(first).unwrap();
    assert_eq!(model.release(first), Err(Error::LeaseExpired));
    assert_eq!(model.write(handle), Err(Error::LeaseConflict));
    model.release(second).unwrap();
    model.write(handle).unwrap();
}

#[test]
fn coarse_domain_leases_deliberately_reject_disjoint_slot_and_target_writes() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, target, original) = allocation(&mut model, 3);
    let storage = model.new_place(domain, 2, None).unwrap();
    model.new_slot(storage, original, &[target], 2).unwrap();
    let observer = model.new_handle(storage, Mode::Read).unwrap();
    let lease = model.protect(observer, Mode::Read).unwrap();
    let view = model.view(lease).unwrap();
    let before = model.clone();
    assert_eq!(
        write_through_slot(&mut model, view),
        Err(Error::LeaseConflict)
    );
    assert_eq!(model, before);
    model.release(lease).unwrap();
    write_through_slot(&mut model, observer).unwrap();
}

#[test]
fn place_domain_lease_fact_and_revision_limits_refuse_without_partial_updates() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, _, handle) = allocation(&mut model, 3);
    for _ in 1..LIMIT {
        model.new_domain().unwrap();
        model.new_place(domain, 3, None).unwrap();
    }
    assert_eq!(model.new_domain(), Err(Error::Limit));
    assert_eq!(model.new_place(domain, 3, None), Err(Error::Limit));
    for _ in 0..LIMIT {
        model.remember(handle).unwrap();
        model.protect(handle, Mode::Read).unwrap();
    }
    assert_eq!(model.remember(handle), Err(Error::Limit));
    assert_eq!(model.protect(handle, Mode::Read), Err(Error::Limit));
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, _, handle) = allocation(&mut model, 3);
    for _ in 0..u16::MAX {
        model.write(handle).unwrap();
    }
    let before = model.clone();
    assert_eq!(model.write(handle), Err(Error::Limit));
    assert_eq!(model, before);
}

#[test]
fn declared_stored_value_validity_is_independent_of_the_slots_own_extent() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, long, original) = allocation(&mut model, 3);
    let (_, short, replacement) = allocation(&mut model, 1);
    let (_, storage, access) = allocation(&mut model, 1);
    model
        .new_slot(storage, original, &[long, short], 3)
        .unwrap();
    let before = model.clone();
    assert_eq!(model.retarget(access, replacement), Err(Error::Validity));
    assert_eq!(model, before);
    let contents = model.load_slot(access).unwrap();
    model.check_escape(contents, 3).unwrap();
    let (_, longer_storage, _) = allocation(&mut model, 3);
    let before = model.clone();
    assert_eq!(
        model.new_slot(longer_storage, original, &[long], 1),
        Err(Error::Validity)
    );
    assert_eq!(model, before);
}

#[test]
fn shared_view_runs_cannot_regain_write_mode_through_repeated_loads_or_copies() {
    let mut model = Model::new(Policy::SharedView);
    let (target_domain, target, original) = allocation(&mut model, 3);
    let (slot_domain, storage, _) = allocation(&mut model, 2);
    model.new_slot(storage, original, &[target], 3).unwrap();
    let observer = model.new_handle(storage, Mode::Read).unwrap();
    for _ in 0..3 {
        let loaded = model.load_slot(observer).unwrap();
        let copied = model.copy_handle(loaded).unwrap();
        assert_eq!(model.handle_mode(loaded), Mode::Read);
        assert_eq!(model.write(copied), Err(Error::ReadOnly));
    }
    model.enter_call(&[slot_domain, target_domain]).unwrap();
    let loaded = model.load_slot(observer).unwrap();
    assert_eq!(model.write(loaded), Err(Error::ReadOnly));
    let before = model.clone();
    assert_eq!(
        write_through_slot(&mut model, observer),
        Err(Error::ReadOnly)
    );
    assert_eq!(model, before);
    model.leave_call().unwrap();
}

#[test]
fn leaving_a_call_does_not_implicitly_return_handles_or_revive_them_on_reentry() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, _, handle) = allocation(&mut model, 1);
    model.enter_call(&[domain]).unwrap();
    let local = model.copy_handle(handle).unwrap();
    let before = model.clone();
    assert_eq!(model.return_handle(local, 3), Err(Error::Validity));
    assert_eq!(model, before);
    model.leave_call().unwrap();
    assert_eq!(model.read(local), Err(Error::InaccessibleHandle));
    assert_eq!(model.copy_handle(local), Err(Error::InaccessibleHandle));
    assert_eq!(model.check_escape(local, 1), Err(Error::InaccessibleHandle));
    let before = model.clone();
    assert_eq!(
        model.apply_summary(&[Effect::Read(local)]),
        Err(Error::InaccessibleHandle)
    );
    assert_eq!(model, before);
    model.enter_call(&[domain]).unwrap();
    assert_eq!(model.read(local), Err(Error::InaccessibleHandle));
    assert_eq!(
        model.return_handle(local, 1),
        Err(Error::InaccessibleHandle)
    );
    model.leave_call().unwrap();
}

#[test]
fn checked_returns_promote_a_value_only_to_the_immediate_caller() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (domain, _, original) = allocation(&mut model, 3);
    model.enter_call(&[domain]).unwrap();
    model.enter_call(&[domain]).unwrap();
    let local = model.copy_handle(original).unwrap();
    let parent = model.return_handle(local, 3).unwrap();
    model.leave_call().unwrap();
    assert_eq!(model.read(local), Err(Error::InaccessibleHandle));
    model.read(parent).unwrap();
    let root = model.return_handle(parent, 3).unwrap();
    model.leave_call().unwrap();
    assert_eq!(model.read(parent), Err(Error::InaccessibleHandle));
    model.read(root).unwrap();
    model.read(original).unwrap();
    assert_eq!(model.return_handle(root, 3), Err(Error::NoCaller));
}

#[test]
fn checked_slot_storage_copies_values_without_borrowing_the_callees_variable() {
    let mut model = Model::new(Policy::ReadOnlySlot);
    let (_, first, original) = allocation(&mut model, 3);
    let (_, second, replacement) = allocation(&mut model, 3);
    let (_, local_place, short) = allocation(&mut model, 1);
    let (slot_domain, storage, access) = allocation(&mut model, 2);
    model
        .new_slot(storage, original, &[first, second, local_place], 3)
        .unwrap();
    model.enter_call(&[slot_domain]).unwrap();
    let local = model.copy_handle(replacement).unwrap();
    let local_short = model.copy_handle(short).unwrap();
    let before = model.clone();
    assert_eq!(model.retarget(access, local_short), Err(Error::Validity));
    assert_eq!(model, before);
    model.retarget(access, local).unwrap();
    model.leave_call().unwrap();
    assert_eq!(model.read(local), Err(Error::InaccessibleHandle));
    let loaded = model.load_slot(access).unwrap();
    model.read(loaded).unwrap();
    model.check_escape(loaded, 3).unwrap();
}
