mod support;

use support::{check, flaw_codes, package_flaws};

const TYPES: &str = "Byte is in 0 to 255\nByte.finish(self): return\nTransaction has\n    state Byte\n    commit(self): return\nPair has\n    first Transaction\n    second Transaction\nMaybe is Present(Transaction) or Absent\n";

fn foreign_codes(body: &str) -> Vec<String> {
    let client = format!(
        "use review.resources Byte, Transaction\nPair has\n    first Transaction\n    second Transaction\nMaybe is Present(Transaction) or Absent\n{body}"
    );
    package_flaws(
        "review",
        &[
            (
                "resources",
                "Byte is in 0 to 255\nByte.finish(self): return\nTransaction has\n    state Byte\n    commit(self): return\n",
            ),
            ("client", &client),
        ],
    )
}

#[test]
fn structural_wrappers_cannot_hide_terminal_obligations_in_arrays() {
    for body in [
        "forget(values [Pair; 2]):\n    values.[0].first.commit\n    values.[0].second.commit\n    values.[1].first.commit\n",
        "forget(values [Pair; 2]):\n    values.[0].first.commit\n    values.[0].second.commit\n    values.[1].first.commit\n    return\n",
    ] {
        let codes = foreign_codes(body);
        assert!(
            codes.iter().any(|code| code.starts_with("type-terminal-")),
            "{body}: {codes:?}"
        );
    }
}

#[test]
fn guarded_union_matches_do_not_discharge_rejected_payloads() {
    for body in [
        "forget(value Maybe, flag ref Byte):\n    when value is Present(tx) and flag is 0 then tx.commit\n",
        "forget(value Maybe, flag ref Byte):\n    when value is Present(tx) and flag is 0 then\n        tx.commit\n        return\n    return\n",
    ] {
        let codes = foreign_codes(body);
        assert!(
            codes.iter().any(|code| code.starts_with("type-terminal-")),
            "{body}: {codes:?}"
        );
    }
}

#[test]
fn partially_consumed_wrappers_cannot_be_returned_or_borrowed_whole() {
    for body in [
        "bad(pair Pair) Pair:\n    pair.first.commit\n    return pair\n",
        "inspect(pair ref Pair): return\nbad(pair Pair):\n    pair.first.commit\n    inspect(pair)\n    pair.second.commit\n",
    ] {
        let typed = check(&format!("{TYPES}{body}"));
        assert!(
            flaw_codes(&typed).iter().any(|code| matches!(
                code.as_str(),
                "type-use-of-moved-value" | "type-use-of-maybe-moved-value"
            )),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn conditional_empty_variants_do_not_erase_another_paths_payload() {
    let codes = foreign_codes(
        "bad(value Maybe, flag ref Byte):\n    when flag is 0 then\n        when value is Present(tx) then tx.commit\n    return\n",
    );
    assert!(
        codes.iter().any(|code| code.starts_with("type-terminal-")),
        "{codes:?}"
    );
}

#[test]
fn replacing_an_array_element_invalidates_borrows_of_its_fields() {
    let typed = check(&format!(
        "{TYPES}Item has value Byte\nreplace(previous Item, next Item) Item: return next\nread(value ref Byte) Byte: return deref value\nbad(values mut [Item; 2]) Byte:\n    ref selected Byte: values.[0].value\n    values.[0]:: replace(values.[0], Item(value: 5))\n    return read(selected)\n"
    ));
    assert!(
        flaw_codes(&typed).contains(&"type-use-of-invalidated-reference".to_owned()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn replacing_an_array_element_field_invalidates_its_record_borrow() {
    let typed = check(&format!(
        "{TYPES}Item has value Byte\nreplace(previous Byte, next Byte) Byte: return next\nread(value ref Item) Byte: return value.value\nbad(values mut [Item; 2]) Byte:\n    ref selected Item: values.[0]\n    values.[0].value:: replace(values.[0].value, 5)\n    return read(selected)\n"
    ));
    assert!(
        flaw_codes(&typed).contains(&"type-use-of-invalidated-reference".to_owned()),
        "{:?}",
        typed.all_flaws()
    );
}
