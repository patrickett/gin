use parser::query::SourceParseExt;
use typecheck::FileId;
use typecheck::transform::transform_file;

const DEFINITIONS: &str = r#"Numeric has
    as_num(self ref) is in Self: return (number is in Self: self)
Count is in 0 to 255
Count has Numeric
    finish(self):
        return
Pointer is @Count
Pointer has
    finish(self):
        return
replace(previous Count, next Count) Count: return next
Region has
    initialized Count
    initialized_count(self ref) is in Count: return self.initialized.as_num
    last(self ref and self.initialized_count > 0) Count: return 0
    finish(self):
        return
reset(address @Count):
    pointer Pointer: address
    pointer.[0]:: replace(pointer.[0], 0)
    return
forward(address @Count): return reset(address)
"#;

const UNPROVEN: &str = "type-parameter-refinement-unproven";

enum Expectation {
    Empty,
    Any(&'static str),
}

const CASES: &[(&str, &[&str], Expectation)] = &[
    (
        "raw_pointer_stores_invalidate_record_proofs",
        &[
            "check(region mut Region and region.initialized_count > 0) Count:\n    pointer Pointer: @region.initialized\n    reset(pointer)\n    return region.last\n",
            "check(region mut Region and region.initialized_count > 0) Count:\n    pointer Pointer: @region.initialized\n    forward(pointer)\n    return region.last\n",
            "check(region mut Region and region.initialized_count > 0) Count:\n    pointer Pointer: @region.initialized\n    pointer.[0]:: replace(pointer.[0], 0)\n    return region.last\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
    (
        "raw_pointer_stores_invalidate_scalar_proofs",
        &[
            "positive(value ref Count and value > 0) Count: return 0\ncheck(value Count and value > 0) Count:\n    pointer Pointer: @value\n    reset(pointer)\n    return positive(value)\n",
            "positive(value ref Count and value > 0) Count: return 0\ncheck(value Count and value > 0) Count:\n    pointer Pointer: @value\n    forward(pointer)\n    return positive(value)\n",
            "positive(value ref Count and value > 0) Count: return 0\ncheck(value Count and value > 0) Count:\n    pointer Pointer: @value\n    pointer.[0]:: replace(pointer.[0], 0)\n    return positive(value)\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
    (
        "raw_pointer_stores_invalidate_scalar_proofs",
        &[
            "positive(value ref Count and value > 0) Count: return 0\ncheck(value Count and value > 0) Count:\n    pointer Pointer: @value\n    \n    return positive(value)\n",
        ],
        Expectation::Empty,
    ),
    (
        "raw_exposure_through_helpers_invalidates_owned_headers",
        &[
            "positive(value ref Count and value > 0) Count: return 0\nexpose(region ref Region) @Count: return @region.initialized\ncheck(unused @Count) Count:\n    region: Region(initialized: 1)\n    pointer: expose(region)\n    reset(pointer)\n    value: region.last\n    region.finish\n    return value\n",
            "positive(value ref Count and value > 0) Count: return 0\nexpose(region ref Region) @Count: return (ref region.initialized) as @Count\ncheck(unused @Count) Count:\n    region: Region(initialized: 1)\n    pointer: expose(region)\n    reset(pointer)\n    value: region.last\n    region.finish\n    return value\n",
            "positive(value ref Count and value > 0) Count: return 0\nexpose(value ref Count) @Count: return @value\ncheck(value Count and value > 0) Count:\n    pointer: expose(value)\n    reset(pointer)\n    return positive(value)\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
    (
        "an_unknown_call_still_invalidates_its_explicitly_mutated_owned_receiver",
        &[
            "change(region mut Region, address @Count):\n    region.initialized:: replace(region.initialized, 0)\n    reset(address)\n    return\ncheck(address @Count) Count:\n    region: Region(initialized: 1)\n    change(region, address)\n    value: region.last\n    region.finish\n    return value\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
    (
        "nested_loops_preserve_unexposed_header_fields_through_raw_helpers",
        &[
            "Buffer has\n    initialized Count\n    capacity Count\n    available(self ref and self.capacity >= 8) Count: return 0\n    clear(self mut):\n        cell Count: 1\n        reset(@cell)\n        self.initialized :: replace(self.initialized, 1)\n        return\n    finish(self): return\ncheck(buffer own Buffer and buffer.capacity >= 8) Count:\n    for outer in 0 to 2\n        buffer.clear\n        for inner in 0 to 2\n            value: buffer.available\n            value.finish\n    buffer.finish\n    return 0\n",
        ],
        Expectation::Empty,
    ),
    (
        "nested_loop_heads_forget_proofs_that_incoming_raw_aliases_can_change",
        &[
            "check(region ref Region and region.initialized_count > 0, address @Count) Count:\n    for outer in 0 to 2\n        for inner in 0 to 2\n            value: region.last\n            value.finish\n            reset(address)\n    return 0\n",
            "check(region ref Region and region.initialized_count > 0, address @Count) Count:\n    for outer in 0 to 2\n        for inner in 0 to 2\n            value: region.last\n            value.finish\n            forward(address)\n    return 0\n",
            "check(region ref Region and region.initialized_count > 0, address @Count) Count:\n    for outer in 0 to 2\n        for inner in 0 to 2\n            value: region.last\n            value.finish\n            address.[0]:: replace(address.[0], 0)\n    return 0\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
    (
        "nested_loop_heads_forget_proofs_about_exposed_owned_fields",
        &[
            "check Count:\n    region: Region(initialized: 1)\n    pointer Pointer: @region.initialized\n    for outer in 0 to 2\n        for inner in 0 to 2\n            value: region.last\n            value.finish\n            reset(pointer)\n    region.finish\n    return 0\n",
            "check Count:\n    region: Region(initialized: 1)\n    pointer Pointer: @region.initialized\n    for outer in 0 to 2\n        for inner in 0 to 2\n            value: region.last\n            value.finish\n            pointer.[0]:: replace(pointer.[0], 0)\n    region.finish\n    return 0\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
    (
        "nested_loops_preserve_only_the_unexposed_field_through_wrappers",
        &[
            "Buffer has\n    initialized Count\n    capacity Count\n    available(self ref and self.capacity >= 8) Count: return 0\n    last(self ref and self.initialized > 0) Count: return 0\n    clear(self mut):\n        reset(@self.initialized)\n        return\n    forward_clear(self mut):\n        self.clear\n        return\n    finish(self): return\ncheck(buffer own Buffer and buffer.capacity >= 8 and buffer.initialized > 0) Count:\n    for outer in 0 to 2\n        buffer.forward_clear\n        for inner in 0 to 2\n            value: buffer.available\n            value.finish\n    buffer.finish\n    return 0\n",
            "Buffer has\n    initialized Count\n    capacity Count\n    available(self ref and self.capacity >= 8) Count: return 0\n    last(self ref and self.initialized > 0) Count: return 0\n    clear(self mut):\n        reset(@self.initialized)\n        return\n    forward_clear(self mut):\n        self.clear\n        return\n    finish(self): return\ncheck(buffer mut Buffer and buffer.capacity >= 8 and buffer.initialized > 0) Count:\n    for outer in 0 to 2\n        buffer.forward_clear\n        for inner in 0 to 2\n            value: buffer.available\n            value.finish\n    \n    return 0\n",
        ],
        Expectation::Empty,
    ),
    (
        "nested_loops_preserve_only_the_unexposed_field_through_wrappers",
        &[
            "Buffer has\n    initialized Count\n    capacity Count\n    available(self ref and self.capacity >= 8) Count: return 0\n    last(self ref and self.initialized > 0) Count: return 0\n    clear(self mut):\n        reset(@self.initialized)\n        return\n    forward_clear(self mut):\n        self.clear\n        return\n    finish(self): return\ncheck(buffer own Buffer and buffer.capacity >= 8 and buffer.initialized > 0) Count:\n    for outer in 0 to 2\n        buffer.forward_clear\n        for inner in 0 to 2\n            value: buffer.last\n            value.finish\n    buffer.finish\n    return 0\n",
            "Buffer has\n    initialized Count\n    capacity Count\n    available(self ref and self.capacity >= 8) Count: return 0\n    last(self ref and self.initialized > 0) Count: return 0\n    clear(self mut):\n        reset(@self.initialized)\n        return\n    forward_clear(self mut):\n        self.clear\n        return\n    finish(self): return\ncheck(buffer mut Buffer and buffer.capacity >= 8 and buffer.initialized > 0) Count:\n    for outer in 0 to 2\n        buffer.forward_clear\n        for inner in 0 to 2\n            value: buffer.last\n            value.finish\n    \n    return 0\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
    (
        "loop_raw_exposure_includes_retargeted_reference_aliases",
        &[
            "expose(region ref Region) @Count: return @region.initialized\ncheck Count:\n    first: Region(initialized: 1)\n    second: Region(initialized: 1)\n    alias: mut first\n    for outer in 0 to 2\n        value: second.last\n        value.finish\n        alias :: mut second\n        for inner in 0 to 2\n            reset(@alias.initialized)\n    first.finish\n    second.finish\n    return 0\n",
            "expose(region ref Region) @Count: return @region.initialized\ncheck Count:\n    first: Region(initialized: 1)\n    second: Region(initialized: 1)\n    alias: mut first\n    for outer in 0 to 2\n        value: second.last\n        value.finish\n        alias :: mut second\n        for inner in 0 to 2\n            reset(expose(alias))\n    first.finish\n    second.finish\n    return 0\n",
        ],
        Expectation::Any(UNPROVEN),
    ),
];

#[test]
fn raw_memory_effect_cases() {
    for &(name, bodies, ref expectation) in CASES {
        for body in bodies {
            let source = format!("{DEFINITIONS}{body}");
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "case `{name}` failed: {:?}",
                parsed.symptoms
            );
            let typed = transform_file(parsed.ast, FileId(0));
            let flaws = typed.all_flaws();
            match expectation {
                Expectation::Empty => assert!(flaws.is_empty(), "case `{name}` failed: {flaws:?}"),
                Expectation::Any(code) => assert!(
                    flaws.iter().any(|(_, flaw)| flaw.code.slug() == *code),
                    "case `{name}` failed: {flaws:?}"
                ),
            }
        }
    }
}

#[test]
fn scoped_pointer_stores_remain_available_to_the_compile_time_heap() {
    let parsed = format!("{}Pointer is @Byte\nPointer has\n    finish(self):\n        return\nreplace(previous Byte, next Byte) Byte: return next\nreset(address @Byte):\n    pointer Pointer: address\n    pointer.[0]:: replace(pointer.[0], 0)\n    return\ncomputed Byte :=\n    value Byte: 15\n    pointer Pointer: @value\n    reset(pointer)\n    return value\n", test_fixtures::preludes::BYTE_PRELUDE).parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target =
        flask::CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap());
    let mut evaluator = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    );
    assert_eq!(
        evaluator
            .call(
                typecheck::DefId::named(internment::Intern::from_ref("computed")),
                &[]
            )
            .unwrap(),
        ast::ConstValue::ZERO
    );
}

#[test]
fn raw_pointer_reads_preserve_record_proofs() {
    let source = format!(
        "{DEFINITIONS}read(address @Count) Count: return address.[0]\ncheck(region mut Region and region.initialized_count > 0) Count:\n    pointer Pointer: @region.initialized\n    value: read(pointer)\n    value.finish\n    return region.last\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let effects =
        &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("read"))].effects;
    assert!(!effects.unknown_memory);
    assert!(effects.writes.is_empty());
}

#[test]
fn rewritten_address_stores_keep_their_summary_and_invalidation() {
    let source = format!(
        "{DEFINITIONS}check(region mut Region and region.initialized_count > 0) Count:\n    pointer Pointer: @region.initialized\n    pointer.[0]:: replace(pointer.[0], 0)\n    return region.last\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut typed = transform_file(parsed.ast, FileId(0));
    let id = typecheck::DefId::named(internment::Intern::from_ref("check"));
    let typecheck::BindBody::Body { exprs, .. } = &typed.defs[&id].body else {
        panic!("check body")
    };
    let store = *exprs
        .iter()
        .find(|expr| {
            matches!(
                typed.exprs.kind_of(**expr),
                Some(typecheck::TypedExprKind::BufSet { .. })
            )
        })
        .expect("source store");
    let typecheck::TypedExprKind::BufSet { buf, value, .. } =
        typed.exprs.kind[store.as_usize()].clone()
    else {
        unreachable!()
    };
    let op = typecheck::intrinsic::IntrinsicOp::AddressStore;
    op.validate_signature(
        &[
            typed.exprs.ty[buf.as_usize()].clone(),
            typed.exprs.ty[value.as_usize()].clone(),
        ],
        &ast::Ty::Unit,
        Some(&typed.type_registry),
    )
    .unwrap();
    typed.exprs.kind[store.as_usize()] = typecheck::TypedExprKind::IntrinsicCall {
        op,
        args: vec![buf, value],
    };
    typed.exprs.ty[store.as_usize()] = ast::Ty::Unit;
    typed.exprs.facts.const_facts.const_value[store.as_usize()] = None;
    typed.declaration_flaws.clear();
    for bind in typed.defs.values_mut() {
        std::sync::Arc::make_mut(&mut bind.flaws).clear();
    }
    for flaws in &mut typed.exprs.facts.semantic_facts.flaws {
        flaws.clear();
    }
    let _ = typecheck::transform::infer_package_full(
        std::slice::from_mut(&mut typed),
        &Default::default(),
    );
    assert!(typed.defs[&id].effects.unknown_memory);
    typecheck::transform::stage_flow(&mut typed, &Default::default());
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-parameter-refinement-unproven"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn raw_writes_preserve_unexposed_owned_headers_and_scalar_copies() {
    for body in [
        "check(address @Count) Count:\n    region: Region(initialized: 1)\n    reset(address)\n    value: region.last\n    region.finish\n    return value\n",
        "check(value Count and value > 0, address @Count) Count:\n    reset(address)\n    return positive(value)\n",
    ] {
        let source =
            format!("{DEFINITIONS}positive(value ref Count and value > 0) Count: return 0\n{body}");
        let typed = transform_file(source.parse_source_full().ast, FileId(0));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }
}

#[test]
fn raw_writes_preserve_borrowed_headers_without_a_raw_alias() {
    let source = format!(
        "{DEFINITIONS}unrelated:\n    cell Count: 1\n    reset(@cell)\n    return\ncheck(region ref Region and region.initialized_count > 0) Count:\n    unrelated\n    return region.last\n"
    );
    let typed = transform_file(source.parse_source_full().ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn a_raw_parameter_can_alias_a_borrowed_header() {
    for (parameter, address) in [
        ("address @Count", "address"),
        ("holder Holder", "holder.address"),
    ] {
        let source = format!(
            "{DEFINITIONS}Holder has\n    address @Count\n    finish(self): return\ncheck(region ref Region and region.initialized_count > 0, {parameter}) Count:\n    reset({address})\n    return region.last\n"
        );
        let typed = transform_file(source.parse_source_full().ast, FileId(0));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-parameter-refinement-unproven"),
            "{parameter}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn bodyless_mutators_cannot_preserve_receiver_proofs() {
    let source = format!(
        "{DEFINITIONS}change(region mut Region) Count\ncheck Count:\n    region: Region(initialized: 1)\n    changed: change(region)\n    changed.finish\n    value: region.last\n    region.finish\n    return value\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    let definition = &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("change"))];
    assert!(definition.effects.unknown_memory);
    assert!(!definition.effects.writes.is_empty());
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-parameter-refinement-unproven"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn nested_loops_preserve_disjoint_fields_through_checked_assembly() {
    let assembly = r#"Len is in 0 to 18446744073709551615
Len has
    finish(self): return
#phantom(input, output)
MemorySignature(input, output) has
    version Len
    finish(self): return
memory_signature MemorySignature([pointer Pointer, length Len], ()): MemorySignature(version: 1)
write_code := [signature: memory_signature, target: #target.triple, bytes: [144], inputs: [pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64], length: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [], clobbers: [], reads: [], writes: [[pointer: 0, length: 1]]]
Buffer has
    initialized Count
    capacity Count
    available(self ref and self.capacity >= 8) Count: return 0
    finish(self): return
"#;
    for (field, allowed) in [("initialized", true), ("capacity", false)] {
        let source = format!(
            "{DEFINITIONS}{assembly}check(buffer own Buffer and buffer.capacity >= 8) Count:\n    for outer in 0 to 2\n        #emit(write_code, [pointer: (@buffer.{field}) as Pointer, length: 1])\n        for inner in 0 to 2\n            value: buffer.available\n            value.finish\n    buffer.finish\n    return 0\n"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{field}: {:?}", parsed.symptoms);
        let mut context = typecheck::transform::TransformCtx::new();
        context.compile_target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
        );
        let typed = typecheck::transform::transform(&parsed.ast, FileId(0), &context);
        let flaws = typed.all_flaws();
        if allowed {
            assert!(flaws.is_empty(), "{field}: {flaws:?}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-parameter-refinement-unproven"),
                "{field}: {flaws:?}"
            );
        }
    }
}
