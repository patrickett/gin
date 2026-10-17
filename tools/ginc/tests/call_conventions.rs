mod common;
use common::run_in_profiles;
use test_fixtures::TempPackage;

#[test]
fn ordinary_arguments_borrow_and_mutate_at_runtime() {
    let package = TempPackage::new("call_conventions");
    let input = package.write(
        "main.gin",
        &format!("{}replace(previous Byte, next Byte) Byte: return next\nBox has\n    finish(self):\n        return\n    value Byte\n    write(self mut, value Byte):\n        self.value:: replace(self.value, value)\n        return\n    read(self ref) Byte: return self.value\nread(value ref Byte) Byte: return deref value\nwrite(value mut Byte, replacement Byte):\n    value:: replacement\n    return\nforward(value mut Byte) Byte:\n    write(value, 11)\n    return read(value)\n#entry\nmain:\n    value: 7 as Byte\n    first is in Byte: read(value)\n    second is in Byte: forward(value)\n    write(replacement: 19, value: value)\n    box: Box(value: 1)\n    Box.write(box, 23)\n    box.write(29)\n    third is in Byte: Box.read(box)\n    last is in Byte: value\n    return when first = 7 and second = 11 and third = 29 and last = 19 then 0 else 1\n", test_fixtures::preludes::BYTE_PRELUDE),
    );
    run_in_profiles(&package, input, "run arguments");
}
