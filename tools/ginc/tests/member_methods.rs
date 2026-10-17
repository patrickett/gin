mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn imported_generic_members_resolve_from_nested_receivers() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("member_calls");
        package.write_flask("member_calls");
        package.write(
            "storage/storage.gin",
            r#"Buffer(element) has
    finish(self):
        return
    value element

    read(self ref) element: return self.value

    write(self mut, next element):
        self.value:: element.replace(self.value, next)
        return

    choose(self ref, other ref Self) element: return other.value

    take(self own) element:
        Buffer(value: value) := self
        return value

    release(self own):
        Buffer(value: value) := self
        element.discard(value)
        return
"#,
        );
        let input = package.write(
            "main.gin",
            &format!("use member_calls.storage Buffer\n{}    replace(previous Self, next Self) Self: return next\n    discard(value Self):\n        return\n\nEnvelope has\n    finish(self):\n        self.bytes.release\n        return\n    bytes Buffer(Word)\n\n    read(self ref) Word: return self.bytes.read\n    write(self mut, next Word):\n        self.bytes.write(next)\n        return\n    choose(self ref, other ref Self) Word: return self.bytes.choose(other.bytes)\n\nReferenceEnvelope has\n    finish(self):\n        return\n    bytes ref Buffer(Word)\n    read(self ref) Word: return self.bytes.read\n\nWrapper(element) has\n    finish(self):\n        return\n    value element\n    take(self own) element:\n        Wrapper(value: value) := self\n        return value\n\n#entry\nmain:\n    value Envelope: Envelope(bytes: Buffer(value: 3 as Word))\n    value.write(7 as Word)\n    peer: Envelope(bytes: Buffer(value: 13 as Word))\n    selected is in Word: value.choose(peer)\n    Envelope(bytes: peer_bytes) := peer\n    peer_bytes.release()\n    reference: ReferenceEnvelope(bytes: value.bytes)\n    result is in Word: reference.read\n    direct is in Word: value.read\n    nested Wrapper(Buffer(Word)): Wrapper(value: Buffer(value: 5 as Word))\n    taken is in Word: nested.take.take\n    Envelope(bytes: bytes) := value\n    bytes.release\n    return when result = 7 and direct = 7 and taken = 5 and selected = 13 then 7 else 1\n", test_fixtures::preludes::WORD_PRELUDE),
        );
        let executable = package.join("program");
        let output = compile_and_run_exe(input, &executable, profile, "", "run member calls");
        assert_eq!(output.status.code(), Some(7), "{profile:?}");
    }
}
