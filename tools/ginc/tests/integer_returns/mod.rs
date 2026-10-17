use crate::integer_cases::{Integer, domains};
use ginc::cli::Profile;

#[test]
fn generated_inferred_integer_returns_preserve_all_exit_values() {
    let mut values = domains()
        .flat_map(|domain| domain.boundary_values())
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.compare(*right));
    values.dedup();
    let mut checked = 0;
    for value in values {
        let mut source = format!(
            "forward(flag is in 0 to 1): return direct(flag)\ndirect(flag is in 0 to 1):\n    when flag = 1 then\n        return {value}\n    return 0\nnested(flag is in 0 to 1):\n    when flag = 1 then\n        when flag = 1 then\n            return {value}\n        return 0\n    return 0\nfor_exit(flag is in 0 to 1):\n    for index in 0 to 1\n        when index = 1 and flag = 1 then\n            return {value}\n    return 0\nwhile_exit(flag is in 0 to 1):\n    while flag = 1 do\n        when flag = 1 then\n            return {value}\n    return 0\nwhen_exit(flag is in 0 to 1): return when flag = 1 then {value} else 0\n#entry\nmain:\n"
        );
        for function in [
            "direct",
            "nested",
            "for_exit",
            "while_exit",
            "when_exit",
            "forward",
        ] {
            for flag in [0, 1] {
                let expected = if flag == 0 {
                    Integer::Nonnegative(0)
                } else {
                    value
                };
                source.push_str(&format!(
                    "    when not ({function}({flag}) = {expected}) then\n        return 1\n"
                ));
            }
        }
        source.push_str("    return 0\n");
        for profile in [Profile::Debug, Profile::Release] {
            crate::assert_program(&source, "inferred_return_matrix", profile);
            checked += 12;
        }
    }
    assert_eq!(checked, 1_056);
}

#[test]
fn inferred_early_integer_returns_preserve_values_at_runtime() {
    let source = "choose(flag is in 0 to 1):\n    when flag = 1 then\n        return 256\n    return 0\n#entry\nmain:\n    when not (choose(1) = 256) then\n        return 1\n    when not (choose(0) = 0) then\n        return 1\n    return 0\n";
    for profile in [Profile::Debug, Profile::Release] {
        crate::assert_program(source, "inferred_early_integer_return", profile);
    }
}

#[test]
fn inferred_returns_preserve_rebound_and_named_integer_values_at_runtime() {
    let source = "Byte is in 0 to 255\nByte has finish(self): return\nSignedByte is in -128 to 127\nSignedByte has finish(self): return\nraw_byte(value Byte) is in 0 to 255: return value\nraw_signed(value SignedByte) is in -128 to 127: return value\nnamed(flag is in 0 to 1, value Byte):\n    when flag = 1 then\n        return value\n    value.finish\n    return 0\nsigned(flag is in 0 to 1, value SignedByte):\n    when flag = 1 then\n        return value\n    value.finish\n    return 1\nrebound(flag is in 0 to 1):\n    value: 0\n    when flag = 1 then\n        return value\n    value:: 65536\n    return value\n#entry\nmain:\n    when not (raw_byte(named(1, 254)) = 254) then\n        return 1\n    when not (raw_byte(named(0, 254)) = 0) then\n        return 1\n    when not (raw_signed(signed(1, -128)) = -128) then\n        return 1\n    when not (raw_signed(signed(0, -128)) = 1) then\n        return 1\n    when not (rebound(0) = 65536) then\n        return 1\n    when not (rebound(1) = 0) then\n        return 1\n    return 0\n";
    for profile in [Profile::Debug, Profile::Release] {
        crate::assert_program(source, "inferred_mixed_integer_returns", profile);
    }
}

#[test]
fn inferred_return_rows_preserve_each_column_at_runtime() {
    let source = "choose(flag is in 0 to 1):\n    when flag = 1 then\n        return 256, -129\n    return 0, 0\n#entry\nmain:\n    early: choose(1)\n    final: choose(0)\n    when not (early.[0] = 256) then\n        return 1\n    when not (early.[1] = -129) then\n        return 1\n    when not (final.[0] = 0) then\n        return 1\n    when not (final.[1] = 0) then\n        return 1\n    return 0\n";
    for profile in [Profile::Debug, Profile::Release] {
        crate::assert_program(source, "inferred_return_rows", profile);
    }
}
