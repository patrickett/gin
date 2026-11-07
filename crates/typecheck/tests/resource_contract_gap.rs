//! Characterization of the missing resource interpretation, not storage verification.
//! Successful checking below establishes only ordinary descriptor accounting.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const DESCRIPTORS: &str = include_str!("../examples/resource-contract-descriptors.gin");
const FOREIGN: &str = include_str!("../examples/resource-contract-foreign.gin");
const LIBRARY: &str = include_str!("../examples/resource-contract-library.gin");
const FORGED: &str = include_str!("../examples/resource-contract-forged.gin");
const CLIENT: &str = include_str!("../examples/resource-contract-client.gin");
const NOMINAL_LIBRARY: &str = include_str!("../examples/nominal-storage-library.gin");
const NOMINAL_CLIENT: &str = include_str!("../examples/nominal-storage-client.gin");

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("existing Gin syntax");
    let before = source.clone();
    let checked = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    checked
}

#[test]
fn ordinary_descriptor_bodies_and_their_client_check_without_allocating_anything() {
    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let library = check("example.buffers", LIBRARY, &[&descriptors]).unwrap();
    let client = check("example.client", CLIENT, &[&library]).unwrap();
    assert_eq!(library.checked().functions.len(), 3);
    assert_eq!(client.checked().functions.len(), 1);
}

#[test]
fn scalar_record_construction_does_not_currently_require_a_memory_resource() {
    let forged = check("example.forged", FORGED, &[]).unwrap();
    assert_eq!(forged.checked().functions[0].name, "forge");

    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let without_acquisition = LIBRARY.replace(
        "    Extent(base, size) := acquire(bytes)\n",
        "    base: 0\n    size: bytes\n",
    );
    assert_ne!(without_acquisition, LIBRARY);
    check("example.buffers", &without_acquisition, &[&descriptors]).unwrap();
}

#[test]
fn structural_consumption_is_not_a_physical_release_effect() {
    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let without_release = LIBRARY.replace("    relinquish(Extent(base: base, bytes: size))\n", "");
    assert_ne!(without_release, LIBRARY);
    check("example.buffers", &without_release, &[&descriptors]).unwrap();
}

#[test]
fn bodyless_memory_headers_do_not_acquire_trust_from_their_names() {
    for name in ["storage.boundary", "runtime.memory", "unrelated.package"] {
        let error = check(name, FOREIGN, &[]).unwrap_err();
        assert_eq!(error, SourceCheckError::MissingBody);
    }
}

#[test]
fn descriptor_accounting_does_not_depend_on_storage_looking_names() {
    let descriptors = DESCRIPTORS
        .replace("Extent", "Measurement")
        .replace("base", "position")
        .replace("bytes", "length")
        .replace("acquire", "produce")
        .replace("relinquish", "consume");
    let library = LIBRARY
        .replace("storage.boundary", "ordinary.records")
        .replace("Extent", "Measurement")
        .replace("Buffer", "Envelope")
        .replace("base", "position")
        .replace("bytes", "length")
        .replace("acquire", "produce")
        .replace("relinquish", "consume");
    let descriptors = check("ordinary.records", &descriptors, &[]).unwrap();
    let library = check("example.buffers", &library, &[&descriptors]).unwrap();
    let client = check("example.client", CLIENT, &[&library]).unwrap();
    assert!(
        library
            .checked()
            .aggregates
            .iter()
            .any(|aggregate| aggregate.name == "Envelope")
    );
    assert_eq!(client.checked().functions.len(), 1);
}

#[test]
fn an_existing_nominal_child_can_be_packed_taken_and_repacked_across_calls() {
    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let library = check("example.buffers", NOMINAL_LIBRARY, &[&descriptors]).unwrap();
    let client = check("example.client", NOMINAL_CLIENT, &[&library]).unwrap();
    assert_eq!(library.checked().functions.len(), 4);
    assert_eq!(client.checked().functions.len(), 1);
}

#[test]
fn packing_a_nominal_child_does_not_leave_another_owner_at_the_call_site() {
    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let library = check("example.buffers", NOMINAL_LIBRARY, &[&descriptors]).unwrap();
    let client = "use storage.boundary Extent, relinquish
use example.buffers wrap, release
duplicate(storage Extent):
    buffer: wrap(storage)
    release(buffer)
    relinquish(storage)
    return
";
    let error = check("example.client", client, &[&descriptors, &library]).unwrap_err();
    assert_eq!(error, SourceCheckError::UseAfterMove);
}

#[test]
fn a_taken_nominal_child_cannot_be_abandoned_as_a_temporary() {
    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let library = check("example.buffers", NOMINAL_LIBRARY, &[&descriptors]).unwrap();
    let client = "use example.buffers Buffer, take
abandon(buffer Buffer):
    take(buffer)
    return
";
    let error = check("example.client", client, &[&library]).unwrap_err();
    assert_eq!(error, SourceCheckError::Unconsumed);
}

#[test]
fn putting_a_fabricated_descriptor_in_an_owned_field_does_not_prove_allocation() {
    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let library = check("example.buffers", NOMINAL_LIBRARY, &[&descriptors]).unwrap();
    let client = "use storage.boundary Extent
use example.buffers wrap, release
fabricate(base in 0 to 65535, bytes in 1 to 64):
    buffer: wrap(Extent(base: base, bytes: bytes))
    release(buffer)
    return
";
    // This provider consumes a record; it is not a native deallocator.
    let client = check("example.client", client, &[&descriptors, &library]).unwrap();
    assert_eq!(client.checked().functions.len(), 1);
}

#[test]
fn structural_unpack_and_rebuild_prove_only_descriptor_accounting() {
    let descriptors = check("storage.boundary", DESCRIPTORS, &[]).unwrap();
    let client = "use storage.boundary Extent
strip(storage Extent):
    Extent(base, bytes) := storage
    return
rebuild(storage Extent) Extent:
    Extent(base, bytes) := storage
    return Extent(base: base, bytes: bytes)
";
    // Neither accepted body describes ending or reattaching physical backing.
    let client = check("example.client", client, &[&descriptors]).unwrap();
    assert_eq!(client.checked().functions.len(), 2);
}
