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
        "outer:\n    inner() True or False: True\n    return inner()\n",
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
        "Byte is in 0...255\nCodePoint is in 0...255\n#phantom(unit)\nCount(unit) is in 0...255\naccept(value Count(Byte)) Count(Byte): value\n",
    ]);
    let code_point = transform_with_sources(vec![
        "Byte is in 0...255\nCodePoint is in 0...255\n#phantom(unit)\nCount(unit) is in 0...255\naccept(value Count(CodePoint)) Count(CodePoint): value\n",
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
    let source = "\
expose(value Secret) Secret: value

private

Secret has value Int
";
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
    let source = "\
PublicValue has value Int
expose(value Hidden) PublicValue: value

private

Hidden is PublicValue
";
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
                flaw.code.slug(),
                "interface-public-private-identity" | "type-unknown-symbol"
            )
        }),
        "a folded private constant should leave no private identity: {flaws:?}"
    );
}

fn transform_with_sources(
    sources: impl IntoIterator<Item = &'static str>,
) -> PackageTransformArtifacts {
    transform_with_sources_and_target(sources, PackageTransformOptions::FULL)
}

fn transform_with_sources_and_target(
    sources: impl IntoIterator<Item = &'static str>,
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
            flask::TargetTriple::parse("arm64-unknown-linux").unwrap(),
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
