use ast::FileAst;
use ast::ty::PackageInstanceKey;
use flask::CompileTarget;
use parser::query::SourceParseExt;
use typecheck::transform::{
    PackageTransformArtifacts, PackageTransformOptions,
    transform_package_with_shared_context_and_package,
};

#[test]
fn public_interface_is_deterministic_under_file_order() {
    let source_a = "a:\n    return 1\n";
    let source_b = "b:\n    return 2\n\nc:\n    return 3\n";

    let package_a_first = transform_with_sources(vec![source_a, source_b]);
    let package_b_first = transform_with_sources(vec![source_b, source_a]);

    let bytes_a = package_a_first.public_interface.canonical_bytes();
    let bytes_b = package_b_first.public_interface.canonical_bytes();

    assert_eq!(bytes_a, bytes_b);
}

#[test]
fn public_local_result_family_blocks_authoritative_publication() {
    let package = transform_with_sources(vec![
        "outer:\n    inner() True or False: return True\n    return inner()\n",
    ]);
    let flaws: Vec<_> = package
        .typed_asts
        .iter()
        .flat_map(|typed| typed.all_flaws())
        .collect();

    assert!(
        flaws
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "interface-local-result-family-escapes" })
    );
    assert_eq!(
        package.interface_publication,
        interface::InterfacePublication::Unavailable
    );
}

#[test]
fn clean_package_publishes_authoritative_interface() {
    let package = transform_with_sources(vec!["answer: 42\n"]);

    assert!(matches!(
        package.interface_publication,
        interface::InterfacePublication::Published(_)
    ));
}

#[test]
fn parameter_constraints_affect_public_interface_fingerprint() {
    let first = transform_with_sources(vec![
        "A has run(self ref)\nB has run(self ref)\naccept(value ref has A):\n    return\n",
    ]);
    let second = transform_with_sources(vec![
        "A has run(self ref)\nB has run(self ref)\naccept(value ref has B):\n    return\n",
    ]);
    for package in [&first, &second] {
        let flaws: Vec<_> = package
            .typed_asts
            .iter()
            .flat_map(|typed| typed.all_flaws())
            .collect();
        assert!(flaws.is_empty(), "{flaws:?}");
    }
    assert_ne!(
        first.public_interface.semantic_surface_fingerprint,
        second.public_interface.semantic_surface_fingerprint
    );
}

#[test]
fn named_result_changes_affect_public_interface_fingerprint() {
    let first = transform_with_sources(vec!["pair() first Int, second Int: return 1, 2\n"]);
    let second = transform_with_sources(vec!["pair() left Int, right Int: return 1, 2\n"]);

    assert_ne!(
        first.public_interface.semantic_surface_fingerprint,
        second.public_interface.semantic_surface_fingerprint
    );
}

#[test]
fn named_result_projection_resolves_across_package_files() {
    let package = transform_with_sources(vec![
        "pair() first Int, second Int: return 1, 2\n",
        "main:\n    result: pair()\n    return result.second\n",
    ]);
    let flaws: Vec<_> = package
        .typed_asts
        .iter()
        .flat_map(|typed| typed.all_flaws())
        .collect();

    assert!(
        flaws
            .iter()
            .all(|(_, flaw)| flaw.code.slug() != "type-unknown-result-projection"),
        "named result should remain available across package files: {flaws:?}"
    );
}

#[test]
fn declaration_rewrite_order_does_not_change_fingerprint() {
    let source_one = "a:\n    return 1\n\nb:\n    return 2\n";
    let source_two = "b:\n    return 2\n\na:\n    return 1\n";

    let package_one = transform_with_sources(vec![source_one]);
    let package_two = transform_with_sources(vec![source_two]);

    assert_eq!(
        package_one.public_interface.semantic_surface_fingerprint,
        package_two.public_interface.semantic_surface_fingerprint
    );
    assert_eq!(
        package_one.public_interface.public_closure_fingerprint,
        package_two.public_interface.public_closure_fingerprint
    );
    assert_eq!(
        package_one.public_interface.canonical_bytes(),
        package_two.public_interface.canonical_bytes()
    );
}

#[test]
fn source_perturbation_does_not_change_interface_output() {
    let source_one = "a:\n    return 1\n";
    let source_two = "a:\n\n    \n    return 1\n\n";

    let package_one = transform_with_sources(vec![source_one]);
    let package_two = transform_with_sources(vec![source_two]);

    assert_eq!(package_one.public_interface, package_two.public_interface);
}

#[test]
fn applied_nominal_arguments_participate_in_public_identity() {
    let byte = transform_with_sources(vec![
        "Byte is in 0 to 255\nCodePoint is in 0 to 255\n#phantom(unit)\nCount(unit) is in 0 to 255\naccept(value Count(Byte)) Count(Byte): return value\n",
    ]);
    let code_point = transform_with_sources(vec![
        "Byte is in 0 to 255\nCodePoint is in 0 to 255\n#phantom(unit)\nCount(unit) is in 0 to 255\naccept(value Count(CodePoint)) Count(CodePoint): return value\n",
    ]);

    let fingerprint_for = |package: &PackageTransformArtifacts, name: &str| {
        package
            .public_interface
            .declarations
            .iter()
            .find(|declaration| match &declaration.reference {
                interface::DeclarationRef::Subject {
                    declaration_path, ..
                }
                | interface::DeclarationRef::External {
                    declaration_path, ..
                } => declaration_path.rsplit("::").next() == Some(name),
            })
            .expect("public declaration")
            .fingerprint
    };

    assert_ne!(
        fingerprint_for(&byte, "accept"),
        fingerprint_for(&code_point, "accept")
    );
}

#[test]
fn checked_assembly_protocol_has_separate_implementation_fingerprint() {
    let options = PackageTransformOptions::FULL.with_compile_target(CompileTarget::Concrete(
        flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
    ));
    let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu")
        .replace("increment", "write");
    let changed = source.replace(
        "72 as Byte, 255 as Byte, 192 as Byte",
        "72 as Byte, 255 as Byte, 200 as Byte",
    );
    let first = transform_with_sources_and_target(vec![source.as_str()], options.clone());
    let second = transform_with_sources_and_target(vec![changed.as_str()], options);
    let fingerprint_for = |package: &PackageTransformArtifacts| {
        package
            .public_interface
            .declarations
            .iter()
            .find(|declaration| match &declaration.reference {
                interface::DeclarationRef::Subject {
                    declaration_path, ..
                }
                | interface::DeclarationRef::External {
                    declaration_path, ..
                } => declaration_path.ends_with("::write"),
            })
            .expect("public assembly declaration")
            .fingerprint
    };

    assert_eq!(fingerprint_for(&first), fingerprint_for(&second));
    assert_ne!(
        first.implementation_fingerprint,
        second.implementation_fingerprint
    );
    assert_ne!(
        first.public_interface.target_realizations[0].semantic_surface_fingerprint,
        second.public_interface.target_realizations[0].semantic_surface_fingerprint
    );
}

#[test]
fn private_defs_are_omitted_from_public_interface() {
    let source = "a:
    return 1

private

private_value:
    return 2
";

    let package = transform_with_sources(std::iter::once(source));
    let names = package
        .public_interface
        .declarations
        .iter()
        .map(|declaration| match &declaration.reference {
            interface::DeclarationRef::Subject {
                declaration_path, ..
            }
            | interface::DeclarationRef::External {
                declaration_path, ..
            } => declaration_path.as_str(),
        })
        .collect::<Vec<_>>();

    assert!(
        names
            .iter()
            .any(|name| name.rsplit("::").next() == Some("a")),
        "expected public declaration to be exported"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.rsplit("::").next() == Some("private_value")),
        "private bind should not be exported"
    );
}

#[test]
fn public_surface_rejects_residual_private_nominal_identity() {
    let source = "expose(value Secret) Secret: return value\n\nprivate\n\nSecret has value Int\n";
    let package = transform_with_sources(std::iter::once(source));
    let flaws: Vec<_> = package
        .typed_asts
        .iter()
        .flat_map(|typed| typed.all_flaws())
        .collect();

    assert!(
        flaws
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "interface-public-private-identity" }),
        "public definitions: {:?}; private tags: {:?}",
        package.typed_asts[0].defs,
        package.typed_asts[0].private_tags,
    );
    assert_eq!(
        package.interface_publication,
        interface::InterfacePublication::Unavailable
    );
}

#[test]
fn public_surface_erases_a_fully_resolved_private_alias() {
    let source = "PublicValue has value Int\nexpose(value Hidden) PublicValue: return value\n\nprivate\n\nHidden is PublicValue\n";
    let package = transform_with_sources(std::iter::once(source));
    let flaws: Vec<_> = package
        .typed_asts
        .iter()
        .flat_map(|typed| typed.all_flaws())
        .collect();

    assert!(
        flaws
            .iter()
            .all(|(_, flaw)| { flaw.code.slug() != "interface-public-private-identity" }),
        "resolved private aliases should erase to their public target: {flaws:?}"
    );
}

#[test]
fn public_surface_accepts_folded_private_constant_without_identity() {
    let source = "\
expose: hidden

private

hidden := 42
";
    let package = transform_with_sources(std::iter::once(source));
    let flaws: Vec<_> = package
        .typed_asts
        .iter()
        .flat_map(|typed| typed.all_flaws())
        .collect();

    assert!(
        flaws.iter().all(|(_, flaw)| {
            !matches!(
                flaw.code.slug().as_str(),
                "interface-public-private-identity" | "type-unknown-symbol"
            )
        }),
        "a folded private constant should leave no private identity: {flaws:?}"
    );
}

fn transform_with_sources<'a>(
    sources: impl IntoIterator<Item = &'a str>,
) -> PackageTransformArtifacts {
    transform_with_sources_and_target(sources, PackageTransformOptions::FULL)
}

fn transform_with_sources_and_target<'a>(
    sources: impl IntoIterator<Item = &'a str>,
    options: PackageTransformOptions,
) -> PackageTransformArtifacts {
    let file_asts: Vec<FileAst> = sources
        .into_iter()
        .map(|source| source.parse_source_full().ast)
        .collect();

    transform_package_with_shared_context_and_package(
        file_asts,
        options,
        PackageInstanceKey::workspace("pkg", "1"),
    )
}

#[test]
fn concrete_target_populates_realization_and_image_fingerprints() {
    let source = "a:\n    return 1\n";
    let interface = transform_with_sources_and_target(
        vec![source],
        PackageTransformOptions::FULL.with_compile_target(CompileTarget::Concrete(
            flask::TargetTriple::parse("x86_64-unknown-linux").unwrap(),
        )),
    )
    .public_interface;

    assert_eq!(interface.target_profiles.len(), 1);
    assert_eq!(interface.target_profiles[0].target, "x86_64-unknown-linux");
    assert_eq!(interface.target_realizations.len(), 1);
    assert_eq!(
        interface.target_realizations[0].target,
        "x86_64-unknown-linux"
    );
}

#[test]
fn target_realization_fingerprints_differ_for_same_base() {
    let source = "a:\n    return 1\n";
    let package_x86 = transform_with_sources_and_target(
        vec![source],
        PackageTransformOptions::FULL.with_compile_target(CompileTarget::Concrete(
            flask::TargetTriple::parse("x86_64-unknown-linux").unwrap(),
        )),
    );
    let package_arm = transform_with_sources_and_target(
        vec![source],
        PackageTransformOptions::FULL.with_compile_target(CompileTarget::Concrete(
            flask::TargetTriple::parse("aarch64-unknown-linux").unwrap(),
        )),
    );

    assert_eq!(
        package_x86.public_interface.base_fingerprint(),
        package_arm.public_interface.base_fingerprint(),
    );
    assert_ne!(
        package_x86.public_interface.target_realizations[0].semantic_surface_fingerprint,
        package_arm.public_interface.target_realizations[0].semantic_surface_fingerprint,
    );
    assert_ne!(
        package_x86.public_interface.target_realizations[0].public_closure_fingerprint,
        package_arm.public_interface.target_realizations[0].public_closure_fingerprint,
    );
}

#[test]
fn every_same_named_callable_contributes_to_the_interface() {
    let first = "select(value True) True: return value\n";
    let second = "select(value False) False: return value\n";
    let one = transform_with_sources(vec![first]);
    let both = transform_with_sources(vec![first, second]);
    let reversed = transform_with_sources(vec![second, first]);
    assert_eq!(both.public_interface, reversed.public_interface);
    assert_eq!(
        interface::classify_interfaces(&one.public_interface, &both.public_interface),
        interface::Compatibility::Breaking
    );
}

#[test]
fn generic_trait_method_types_resolve_in_either_file_order() {
    let declarations = r#"
Parcel(element) has
    value element
    finish(self):
        return
Attempt(element) is Parcel(element) or Failed
Attempt(element) has
    finish(self):
        return
Store has
    wrap(self, element, value element) Attempt(element)
    unwrap(self, element, p Parcel(element)) element
"#;
    let implementation = r#"
use Parcel
use Attempt
use Store
Storage has Store
    wrap(self, element, value element) Attempt(element):
        return Parcel(value: value)
    unwrap(self, element, p Parcel(element)) element:
        Parcel(value: value) := p
        return value
"#;
    for sources in [
        [declarations, implementation],
        [implementation, declarations],
    ] {
        let package = transform_with_sources(sources);
        let flaws: Vec<_> = package
            .typed_asts
            .iter()
            .flat_map(|typed| typed.all_flaws())
            .collect();
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn private_field_layout_and_grants_affect_the_public_fingerprint() {
    let source = "Byte is in 0 to 255\nRecord has\n    #private\n    value Byte\n";
    let baseline = transform_with_sources(vec![source]);
    for changed in [
        source.replace("#private", "#private(client)"),
        source.replace("value Byte", "value [Byte; 2]"),
    ] {
        let updated = transform_with_sources(vec![changed.as_str()]);
        assert_ne!(
            baseline.public_interface.semantic_surface_fingerprint,
            updated.public_interface.semantic_surface_fingerprint
        );
    }
}

#[test]
fn private_field_constraints_affect_the_public_fingerprint() {
    let source = "Count is in 0 to 255\nRegion has\n    #private\n    capacity Count\n    #private\n    initialized Count and <= capacity\n";
    let baseline = transform_with_sources(vec![source]);
    let changed = source.replace("<= capacity", "= 0");
    let updated = transform_with_sources(vec![changed.as_str()]);
    assert_ne!(
        baseline.public_interface.semantic_surface_fingerprint,
        updated.public_interface.semantic_surface_fingerprint
    );
}

#[test]
fn inferred_scalar_proofs_affect_the_public_fingerprint() {
    let baseline = transform_with_sources(vec!["Count is in 0 to 255\nvalue Count: 1\n"]);
    let updated = transform_with_sources(vec!["Count is in 0 to 255\nvalue Count: 2\n"]);
    assert_ne!(
        baseline.public_interface.semantic_surface_fingerprint,
        updated.public_interface.semantic_surface_fingerprint
    );
}

#[test]
fn private_methods_are_excluded_from_the_public_interface() {
    let package = transform_with_sources(vec![
        "Record has\n    #private\n    hidden(self ref):\n        return\n    visible(self ref):\n        return\n",
    ]);
    let declarations = format!("{:?}", package.public_interface.declarations);
    assert!(!declarations.contains("Record.hidden"), "{declarations}");
    assert!(declarations.contains("Record.visible"), "{declarations}");
}

fn generic_surface_source(argument: &str, private_declaration: &str) -> String {
    format!(
        "{}#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\nexpose(value {argument}) {argument}: return value\n\nprivate\n\n{private_declaration}\n",
        test_fixtures::preludes::WORD_PRELUDE
    )
}

#[test]
fn public_generic_arguments_reject_private_nominal_identity() {
    let source = generic_surface_source(
        "Count(Secret)",
        "Secret has\n    value Word\n    finish(self):\n        return",
    );
    let package = transform_with_sources(std::iter::once(source.as_str()));
    let expose = &package.typed_asts[0].defs
        [&typecheck::DefId::named(internment::Intern::from_ref("expose"))];
    assert_eq!(
        expose.params[0]
            .1
            .named_instance()
            .unwrap()
            .declaration
            .name
            .as_str(),
        "Count",
        "{:?}",
        expose.params[0].1
    );
    let flaws = &expose.flaws;
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "interface-public-private-identity"),
        "{flaws:?}"
    );
    assert_eq!(
        package.interface_publication,
        interface::InterfacePublication::Unavailable
    );
}

#[test]
fn nested_public_generic_arguments_reject_private_nominal_identity() {
    let source = generic_surface_source(
        "Count(Count(Secret))",
        "Secret has\n    value Word\n    finish(self):\n        return",
    );
    let package = transform_with_sources(std::iter::once(source.as_str()));
    let expose = &package.typed_asts[0].defs
        [&typecheck::DefId::named(internment::Intern::from_ref("expose"))];
    let flaws = &expose.flaws;
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "interface-public-private-identity"),
        "{flaws:?}"
    );
    assert_eq!(
        package.interface_publication,
        interface::InterfacePublication::Unavailable
    );
}

#[test]
fn public_generic_arguments_erase_private_aliases_to_public_types() {
    let source = generic_surface_source("Count(Count(Hidden))", "Hidden is Word");
    let package = transform_with_sources(std::iter::once(source.as_str()));
    let flaws = package.typed_asts[0].all_flaws();
    assert!(flaws.is_empty(), "{flaws:?}");
    assert_ne!(
        package.interface_publication,
        interface::InterfacePublication::Unavailable
    );
}
