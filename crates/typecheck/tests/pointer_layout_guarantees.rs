mod support;

use parser::query::SourceParseExt;
use typecheck::transform::{TransformCtx, transform};

const MODEL: &str = r#"Word is in 0 to 18446744073709551615
Word has release(self): return
Address is in 0 to 18446744073709551615
Address has release(self): return
Region has
    address own Word
    address_value(self ref) Address: return self.address as Address
    release(self):
        Region(address) := self
        raw: address as @Word
        return
Error is Failed or Empty
Error has release(self): return
"#;

fn flaws(body: &str) -> Vec<String> {
    let source = format!("{MODEL}{body}");
    source_flaws(
        &source,
        flask::CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap()),
    )
}

fn source_flaws(source: &str, target: flask::CompileTarget) -> Vec<String> {
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut context = TransformCtx::new();
    context.compile_target = target;
    let mut asts = vec![parsed.ast];
    let preparation = typecheck::prepare_package_asts(&mut asts, &context.compile_target);
    assert!(preparation.iter().all(Vec::is_empty), "{preparation:?}");
    context.compile_time_eval_ast = std::sync::Arc::new(asts[0].clone());
    let typed = transform(&asts[0], typecheck::FileId(0), &context);
    support::flaw_codes(&typed)
}

#[test]
fn aligned_nonnull_storage_proves_its_success_contract() {
    let codes = flaws(
        "reserve(unused Address) storage Region thus storage.address_value > 0 and storage.address_value % #alignment(Word) = 0 or Error:\n    return Region(address: (4096 as Address) as @Word)\n",
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn null_and_misaligned_success_payloads_are_rejected() {
    for address in [0, 1, 18446744073709551615_u64] {
        let codes = flaws(&format!(
            "reserve(unused Address) storage Region thus storage.address_value > 0 and storage.address_value % #alignment(Word) = 0 or Error:\n    return Region(address: ({address} as Address) as @Word)\n"
        ));
        assert!(
            codes.iter().any(|code| code == "type-postcondition-failed"),
            "{address}: {codes:?}"
        );
    }
}

#[test]
fn address_facts_survive_pointer_casts_and_record_moves() {
    let codes = flaws(
        "reserve(address Address and address > 0 and address % 4096 = 0) storage Region thus storage.address_value > 0 and storage.address_value % #alignment(Word) = 0 or Error:\n    pointer: address as @Word\n    region: Region(address: pointer)\n    moved: region\n    return moved\n",
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn callers_receive_layout_facts_only_for_the_success_variant() {
    let codes = flaws(
        "reserve(unused Address) storage Region thus storage.address_value > 0 and storage.address_value % #alignment(Word) = 0 or Error: return Region(address: (4096 as Address) as @Word)\nconsume(storage Region and storage.address_value > 0 and storage.address_value % 8 = 0):\n    storage.release\n    return\nrun:\n    attempt: reserve(0)\n    when attempt is\n        Region(storage) then consume(storage)\n        Error(error) then error.release\n    return\n",
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn replacing_an_owned_pointer_invalidates_layout_facts() {
    let codes = flaws(
        "replace(previous Region, next Region) Region:\n    previous.release\n    return next\nchange(storage Region) Region:\n    storage.release\n    return Region(address: (1 as Address) as @Word)\nreserve(unused Address) storage Region thus storage.address_value > 0 and storage.address_value % 8 = 0 or Error: return Region(address: (4096 as Address) as @Word)\nconsume(storage Region and storage.address_value % 8 = 0):\n    storage.release\n    return\nrun:\n    attempt: reserve(0)\n    when attempt is\n        Region(storage) then\n            storage :: change(storage)\n            consume(storage)\n        Error(error) then error.release\n    return\n",
    );
    assert!(
        codes
            .iter()
            .any(|code| code == "type-parameter-refinement-failed"
                || code == "type-parameter-refinement-unproven"),
        "{codes:?}"
    );
}

#[test]
fn narrower_pointer_casts_do_not_preserve_the_original_address() {
    let codes = flaws(
        "Small is in 0 to 255\nSmall has release(self): return\ncheck:\n    pointer: (4096 as Address) as @Word\n    truncated: pointer as Small\n    return thus truncated = 4096\n",
    );
    assert!(
        codes.iter().any(
            |code| code == "type-postcondition-failed" || code == "type-postcondition-unproven"
        ),
        "{codes:?}"
    );
}

#[test]
fn alignment_facts_do_not_leak_from_a_conditional_branch() {
    let codes = flaws(
        "replace(previous Region, next Region) Region:\n    previous.release\n    return next\nmake(unused Address) storage Region thus storage.address_value % 8 = 0: return Region(address: (4096 as Address) as @Word)\ncheck(flag Address, storage Region):\n    when flag > 0 then\n        storage :: replace(storage, make(0))\n    return storage thus storage.address_value % 8 = 0\n",
    );
    assert!(
        codes
            .iter()
            .any(|code| code == "type-postcondition-unproven"),
        "{codes:?}"
    );
}

#[test]
fn refined_address_casts_require_a_layout_proof() {
    for (address, valid) in [(4096, true), (1, false)] {
        let codes = flaws(&format!(
            "Aligned is in 0 to 18446744073709551615 and self % 4096 = 0\nAligned has release(self): return\ncheck:\n    pointer: ({address} as Address) as @Word\n    aligned: pointer as Aligned\n    return aligned\n"
        ));
        if valid {
            assert!(codes.is_empty(), "{codes:?}");
        } else {
            assert!(
                codes
                    .iter()
                    .any(|code| code.starts_with("type-integer-narrowing-")),
                "{codes:?}"
            );
        }
    }
}

#[test]
fn target_width_addresses_preserve_layout_in_library_interfaces() {
    let codes = source_flaws(
        r#"Word is in 0 to 18446744073709551615
Word has release(self): return
bits := #index_bits(@())
maximum := (1 << bits) - 1
#bits(bits)
Address is in 0 to maximum
Address has release(self): return
#bits(bits)
Aligned is in 0 to maximum and self % 4096 = 0
Aligned has release(self): return
Region has
    address own Word
    address_value(self ref) Address: return self.address as Address
    release(self):
        Region(address) := self
        raw: address as @Word
        return
reserve(value Aligned and value > 0) storage Region thus storage.address_value > 0 and storage.address_value % #alignment(Word) = 0 and storage.address_value % #alignment(@Word) = 0:
    return Region(address: value as @Word)
"#,
        flask::CompileTarget::Library,
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn pointer_truncation_cannot_establish_a_nonnull_guarantee() {
    let codes = source_flaws(
        &format!(
            "{MODEL}reserve(unused Address) storage Region thus storage.address_value > 0:\n    return Region(address: (4294967296 as Address) as @Word)\n"
        ),
        flask::CompileTarget::Concrete(
            flask::TargetTriple::parse("wasm32-unknown-unknown").unwrap(),
        ),
    );
    assert!(
        codes.iter().any(
            |code| code == "type-postcondition-unproven" || code == "type-postcondition-failed"
        ),
        "{codes:?}"
    );
}

#[test]
fn taking_an_address_does_not_equate_it_with_the_stored_value() {
    let codes = flaws(
        "check(value Address):\n    pointer: @value\n    address: pointer as Address\n    return thus address = value\n",
    );
    assert!(
        codes
            .iter()
            .any(|code| code == "type-postcondition-unproven"),
        "{codes:?}"
    );
}

#[test]
fn alignment_queries_instantiate_generic_type_arguments() {
    let codes = flaws(
        "Holder(element) has\n    value element\n    release(self): return\nreserve(unused Address) storage Region thus storage.address_value % #alignment(Holder(Word)) = 0:\n    return Region(address: (4096 as Address) as @Word)\n",
    );
    assert!(codes.is_empty(), "{codes:?}");
}
