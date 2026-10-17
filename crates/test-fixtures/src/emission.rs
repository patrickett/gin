pub fn increment_source(target: &str) -> String {
    let (arch, index, bytes) = if target.starts_with("aarch64") {
        ("aarch64", 9, vec![41, 5, 0, 145])
    } else {
        ("x86_64", 0, vec![72, 255, 192])
    };
    let count = bytes.len();
    let bytes = bytes
        .into_iter()
        .map(|byte| format!("{byte} as Byte"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Word is in 0 to 18446744073709551615\nWord has\n    finish(self):\n        return\n{}Inputs has\n    value Word\n    finish(self):\n        return\n#phantom(a, b)\nSignature(a, b) has\n    version Word\n    finish(self):\n        return\nsig Signature(Inputs, Word): Signature(version: 1)\nregister := [architecture: \"{arch}\", bank: \"general\", index: {index}, width: 64]\nflags := [architecture: \"{arch}\", bank: \"flags\", index: 0, width: 4]\ninstruction InstructionBytes(Byte, {count}): [{bytes}]\ncode := [\n    signature: sig\n    target: #target.triple\n    bytes: instruction\n    inputs: [value: register]\n    outputs: [value: register]\n    clobbers: [flags]\n    reads: []\n    writes: []\n]\nincrement(value Word) Word: return #emit(code, Inputs(value: value))\n\n#fixed_array(element, count)\nInstructionBytes(element, count BigInt) has\n    finish(self):\n        return\n",
        crate::preludes::BYTE_PRELUDE
    )
}

pub fn buffered_increment_source(target: &str) -> String {
    let source = increment_source(target);
    let source = source
        .lines()
        .map(|line| {
            if line.starts_with("instruction InstructionBytes") {
                "instruction := assemble\n".to_owned()
            } else {
                format!("{line}\n")
            }
        })
        .collect::<String>();
    let bytes = if target.starts_with("aarch64") {
        vec![41, 5, 0, 145]
    } else {
        vec![72, 255, 192]
    };
    let append = bytes
        .iter()
        .map(|byte| format!("    Buffer.append(buffer, {byte})\n"))
        .collect::<String>();
    let builder = format!(
        r#"
Offset is in 0 to 16
Offset has finish(self): return
Buffer has
    finish(self):
        return
    storage InstructionBytes(Byte, 16)
    length Offset
    append(buffer mut Self and buffer.length < 16, value Byte):
        position is in Offset: buffer.length
        storage: mut buffer.storage
        storage.[position]:: replace_byte(storage.[position], value)
        buffer.length:: next_offset(buffer.length)
        return thus buffer.length = position + 1
    patch(buffer mut Self, position Offset and position < buffer.length, value Byte):
        storage: mut buffer.storage
        storage.[position]:: replace_byte(storage.[position], value)
        return
replace_byte(old Byte, next Byte) Byte: return next
next_offset(old Offset and old < 16) Offset:
    return ((value is in Offset: old) + 1) as Offset
assemble Buffer:
    buffer: Buffer(storage: [0 as Byte; 16], length: 0)
{append}    Buffer.patch(buffer, 0, {first})
    return buffer
"#,
        first = bytes[0]
    );
    source.replace(
        "#fixed_array(element, count)",
        &format!("{builder}\n#fixed_array(element, count)"),
    )
}
