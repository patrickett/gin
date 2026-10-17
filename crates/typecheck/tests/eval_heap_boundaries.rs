use ast::ConstValue;
use typecheck::eval::EvalHeap;

#[test]
fn reference_allocations_reject_nonzero_offsets() {
    let mut heap = EvalHeap::default();
    let value = heap.allocate(ConstValue::ONE);
    let reference = heap.allocate_reference(value);
    assert_eq!(heap.reference(reference), Some(value));
    assert_eq!(heap.reference(reference.offset(1).unwrap()), None);
    assert_eq!(heap.reference(reference.offset(isize::MAX).unwrap()), None);
}

#[test]
fn aggregate_allocations_reject_nonzero_base_offsets() {
    let mut heap = EvalHeap::default();
    let first = heap.allocate(ConstValue::ZERO);
    let second = heap.allocate(ConstValue::ONE);
    let aggregate = heap.allocate_aggregate(vec![first, second]);
    assert_eq!(heap.project(aggregate, 0), Ok(first));
    assert_eq!(heap.project(aggregate, 1), Ok(second));
    assert!(heap.project(aggregate, 2).is_err());
    assert!(heap.project(aggregate.offset(1).unwrap(), 0).is_err());
    assert_eq!(heap.freeze(first), Ok(ConstValue::ZERO));
    assert_eq!(heap.freeze(second), Ok(ConstValue::ONE));
}

#[test]
fn projection_offsets_still_select_neighboring_fields() {
    let mut heap = EvalHeap::default();
    let aggregate = heap.allocate(ConstValue::FixedArray(
        vec![ConstValue::ZERO, ConstValue::ONE].into(),
    ));
    let first = heap.project(aggregate, 0).unwrap();
    let second = first.offset(1).unwrap();
    assert_eq!(heap.freeze(second), Ok(ConstValue::ONE));
    heap.store(second, ConstValue::ZERO).unwrap();
    assert_eq!(heap.freeze(second), Ok(ConstValue::ZERO));
    assert!(heap.freeze(second.offset(1).unwrap()).is_err());
}

#[test]
fn cloned_heaps_isolate_writes_through_existing_aliases() {
    let mut accepted = EvalHeap::default();
    let scalar = accepted.allocate(ConstValue::ONE);
    let record = accepted.allocate(ConstValue::FixedArray(
        vec![ConstValue::ZERO, ConstValue::ONE].into(),
    ));
    let projected = accepted.project(record, 1).unwrap();
    let reference = accepted.allocate_reference(projected);
    let aggregate = accepted.allocate_aggregate(vec![scalar, projected]);
    let mut candidate = accepted.clone();
    let scalar_alias = candidate.project(aggregate, 0).unwrap();
    candidate.store(scalar_alias, ConstValue::ZERO).unwrap();
    candidate
        .store(candidate.reference(reference).unwrap(), ConstValue::ZERO)
        .unwrap();
    assert_eq!(candidate.freeze(scalar), Ok(ConstValue::ZERO));
    assert_eq!(candidate.freeze(projected), Ok(ConstValue::ZERO));
    assert_eq!(accepted.freeze(scalar), Ok(ConstValue::ONE));
    assert_eq!(accepted.freeze(projected), Ok(ConstValue::ONE));
    assert_eq!(
        accepted.freeze(record),
        Ok(ConstValue::FixedArray(
            vec![ConstValue::ZERO, ConstValue::ONE].into(),
        ))
    );
}
