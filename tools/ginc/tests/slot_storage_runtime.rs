mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn imported_slot_transfers_values_and_can_be_initialized_again() {
    let package = TempPackage::new("slot_storage");
    package.write_flask("slot_storage");
    package.write(
        "slot/slot.gin",
        include_str!("../../../modules/core/alloc/slot.gin"),
    );
    package.write(
        "value/value.gin",
        &format!("{}Resource has\n    value Byte\n    read(self ref) Byte: return self.value\n    release(self):\n        return\nmake(value Byte) Resource: return Resource(value)\n", test_fixtures::preludes::BYTE_PRELUDE),
    );
    let input = package.write(
        "main.gin",
        include_str!("../../../crates/test-fixtures/fixtures/slot_storage_runtime_main.gin"),
    );
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("program-{profile:?}"));
        let run = compile_and_run_exe(input.clone(), &executable, profile, "", "run slot storage");
        assert_eq!(run.status.code(), Some(0), "{profile:?}: {run:?}");
    }
}
