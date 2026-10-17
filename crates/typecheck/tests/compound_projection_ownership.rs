mod support;

use support::check_lexed;
use typecheck::TypedFileAst;

const TYPES: &str = "Byte is in 0 to 255\nByte has\n    copy(self ref) Self: return deref self\n    finish(self):\n        return\n#fixed_array(element, count)\nBuffer(element, count BigInt) has\n    finish(self):\n        return\nBox has\n    value Byte\n    finish(self):\n        return\n\nadd_bits(a Byte, b Byte) Byte:\n    left is in 0 to 255: a\n    right is in 0 to 255: b\n    return ((left + right) & 255) as Byte\n#infix('+')\nbyte_add(a Byte, b Byte) Byte: return add_bits(a, b)\n";

fn check_update(ty: &str, assignment: &str) -> TypedFileAst {
    check_lexed(&format!(
        "{TYPES}update(value mut {ty}, replacement Byte) Byte:\n    value{assignment}\n    return replacement\n"
    ))
}

#[test]
fn consuming_compound_operators_transfer_previous_projected_values() {
    for (ty, projection) in [("Buffer(Byte, 4)", ".[0]"), ("Box", ".value")] {
        let typed = check_update(ty, &format!("{projection}+: Byte.copy(replacement)"));
        assert!(
            typed.all_flaws().is_empty(),
            "{ty}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn ordinary_projected_replacement_still_requires_consuming_previous_value() {
    for (ty, projection) in [("Buffer(Byte, 4)", ".[0]"), ("Box", ".value")] {
        let typed = check_update(ty, &format!("{projection}:: Byte.copy(replacement)"));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, d)| d.code.slug() == "type-rebind-would-discard-value"),
            "{ty}: {:?}",
            typed.all_flaws()
        );
    }
}
