use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};
use typecheck::{DefId, TagId};

const SIZES: &str = "width := #index_bits(@())\nmaximum := (1 << width) - 1\n#bits(width)\nExtent is in 0 to maximum\nExtent has\n    finish(self):\n        return\n";

fn transform(extra: &str, target: flask::CompileTarget) -> typecheck::TypedFileAst {
    let source = format!("{SIZES}{extra}");
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut asts = vec![parsed.ast.qualify_module_defs("sample.sizes")];
    let flaws = typecheck::prepare_package_asts(&mut asts, &target);
    assert!(flaws.iter().all(Vec::is_empty), "{flaws:?}");
    transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(target),
    )
    .typed_asts
    .remove(0)
}

#[test]
fn library_integer_bounds_preserve_symbolic_layout_and_typed_bodies() {
    let typed = transform(
        "Holder has\n    length Extent\n    finish(self):\n        return\nzero Extent: 0\nidentity(value Extent) Extent: return value\n",
        flask::CompileTarget::Library,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let ty = &typed.tags[&TagId(Intern::from_ref("Extent"))].resolved_ty;
    assert_eq!(typed.type_registry.integer_width_for_type(ty), None);
    assert!(typed.target_layout.is_none());
    for name in ["sample.sizes.zero", "sample.sizes.identity"] {
        assert!(!matches!(
            typed.defs[&DefId::named(Intern::from_ref(name))].body,
            typecheck::typed::BindBody::Unlowered
        ));
    }
}

#[test]
fn library_range_projection_accepts_symbolic_closed_bounds() {
    let typed = transform(
        "Interval(lo to hi) is lo to hi\nInterval(lo to hi) has\n    min: lo\n    max: hi\nlimit Extent: Interval(domain of Extent).max\n",
        flask::CompileTarget::Library,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn library_range_bounds_support_cross_file_arithmetic_and_domain_annotations() {
    let sources = [
        SIZES,
        "Interval(lo to hi) is lo to hi\nInterval(lo to hi) has\n    max: hi\nremaining(value Extent):\n    count is in Extent: value\n    limit is in Extent: Interval(domain of Extent).max\n    return Interval(domain of Extent).max - count\n",
    ];
    for sources in [sources, [sources[1], sources[0]]] {
        let mut asts = sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let parsed = source.parse_source_full();
                assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
                parsed
                    .ast
                    .qualify_module_defs(&format!("sample.module{index}"))
            })
            .collect::<Vec<_>>();
        let target = flask::CompileTarget::Library;
        assert!(
            typecheck::prepare_package_asts(&mut asts, &target)
                .iter()
                .all(Vec::is_empty)
        );
        let package = transform_package_with_shared_context(asts, PackageTransformOptions::FULL);
        let flaws = package
            .typed_asts
            .iter()
            .flat_map(typecheck::TypedFileAst::all_flaws)
            .collect::<Vec<_>>();
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn library_rejects_values_outside_known_bounds() {
    for source in ["invalid Extent: -1\n", "invalid Extent: (-1) as Extent\n"] {
        let typed = transform(source, flask::CompileTarget::Library);
        assert!(
            typed.all_flaws().iter().any(|(_, flaw)| matches!(
                flaw.code.slug().as_str(),
                "type-integer-literal-out-of-range" | "type-integer-narrowing-failed"
            )),
            "{source}: {:?}; {:?}",
            typed.all_flaws(),
            typed.type_registry.integer_validity_for_type(
                &typed.tags[&TagId(Intern::from_ref("Extent"))].resolved_ty
            ),
        );
    }
}

#[test]
fn concrete_targets_validate_deferred_integer_literals() {
    let source = "value Extent: 4294967296\n";
    for (target, accepted) in [
        (flask::CompileTarget::Library, true),
        (
            flask::CompileTarget::Concrete(
                flask::TargetTriple::parse("wasm32-unknown-unknown").unwrap(),
            ),
            false,
        ),
        (
            flask::CompileTarget::Concrete(
                flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
            ),
            true,
        ),
        (
            flask::CompileTarget::Concrete(
                flask::TargetTriple::parse("aarch64-unknown-linux-gnu").unwrap(),
            ),
            true,
        ),
    ] {
        let typed = transform(source, target.clone());
        assert_eq!(
            typed.all_flaws().is_empty(),
            accepted,
            "{target:?}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn target_dependent_declarations_do_not_hide_unrelated_invalid_types() {
    let typed = transform(
        "#bits(2)\nInvalid is in 0 to 255\n",
        flask::CompileTarget::Library,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-integer-representation-too-small"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn defaulted_generic_records_preserve_symbolic_integer_fields() {
    let typed = transform(
        &format!(
            "{}DefaultStore has\n    marker Word\n    finish(self):\n        return\n#phantom(unit)\n#bits(width)\nCount(unit) is in 0 to maximum\nCount(unit) has\n    finish(self):\n        return\nBlock(element) has\n    count Count(element)\n    finish(self):\n        return\nBuffer(element, store: DefaultStore) has\n    block Block(element), allocator store\n    finish(self):\n        self.block.finish\n        self.allocator.finish\n        return\nText(store: DefaultStore) has\n    bytes Buffer(Word, store)\n    finish(self):\n        self.bytes.finish\n        return\nHolder has\n    text Text\n    finish(self) Text: return self.text\n",
            test_fixtures::preludes::WORD_PRELUDE
        ),
        flask::CompileTarget::Library,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn cross_file_defaulted_records_preserve_symbolic_integer_fields() {
    let sources = [
        SIZES.to_owned() + &format!("{}#phantom(unit)\n#bits(width)\nCount(unit) is in 0 to maximum\nCount(unit) has\n    finish(self):\n        return\n", test_fixtures::preludes::WORD_PRELUDE),
        "DefaultStore is ()\nBlock(element) has\n    finish(self):\n        return\n    pointer @element\n    count Count(element)\n".to_owned(),
        "Buffer(element, store: DefaultStore) has\n    block Block(element), allocator store, length Extent\n    finish(self):\n        self.block.finish\n        return\n".to_owned(),
        "Text(store: DefaultStore) has\n    bytes Buffer(Word, store)\n    finish(self):\n        self.bytes.finish\n        return\nHolder has\n    text Text\n    finish(self) Text: return self.text\n".to_owned(),
    ];
    for reverse in [false, true] {
        let mut asts = sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let parsed = source.parse_source_full();
                assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
                parsed
                    .ast
                    .qualify_module_defs(&format!("sample.module{index}"))
            })
            .collect::<Vec<_>>();
        if reverse {
            asts.reverse();
        }
        let target = flask::CompileTarget::Library;
        assert!(
            typecheck::prepare_package_asts(&mut asts, &target)
                .iter()
                .all(Vec::is_empty)
        );
        let package = transform_package_with_shared_context(asts, PackageTransformOptions::FULL);
        let flaws = package
            .typed_asts
            .iter()
            .flat_map(typecheck::TypedFileAst::all_flaws)
            .collect::<Vec<_>>();
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn inherited_numeric_methods_resolve_for_destructured_cross_file_counts() {
    let sources = [
        "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\n    to_num(self) is in Self: return (number is in Self: self)\n".to_owned(),
        "use sample.module0 Numeric\n".to_owned() + SIZES + &format!("{}#phantom(unit)\n#bits(width)\nCount(unit) is in 0 to maximum\nCount(unit) has Numeric\n    finish(self):\n        return\n", test_fixtures::preludes::BYTE_PRELUDE),
        "Block(element) has\n    capacity Count(element)\n    finish(self):\n        return\n\nreclaim(block Block(Byte)) is in Count(Byte):\n    Block(capacity) := block\n    return capacity.to_num\n".to_owned(),
    ];
    let mut asts = sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let parsed = source.parse_source_full();
            assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
            parsed
                .ast
                .with_semantic_origin(ast::prelude::FileSemanticOrigin {
                    package: ast::ty::PackageInstanceKey::workspace("sample", "0"),
                    module: vec![Intern::new(format!("module{index}"))],
                })
                .qualify_module_defs(&format!("sample.module{index}"))
        })
        .collect::<Vec<_>>();
    let target = flask::CompileTarget::Library;
    assert!(
        typecheck::prepare_package_asts(&mut asts, &target)
            .iter()
            .all(Vec::is_empty)
    );
    let package = transform_package_with_shared_context(asts, PackageTransformOptions::FULL);
    let flaws = package
        .typed_asts
        .iter()
        .flat_map(typecheck::TypedFileAst::all_flaws)
        .collect::<Vec<_>>();
    assert!(flaws.is_empty(), "{flaws:?}");
}
