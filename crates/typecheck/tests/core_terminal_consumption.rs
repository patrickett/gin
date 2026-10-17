use ast::{prelude::FileSemanticOrigin, ty::PackageInstanceKey};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::transform::{
    PackageTransformOptions, transform_package_with_shared_context_and_package,
};

const LIBRARY_SOURCES: [(&str, &str); 10] = [
    ("ops", include_str!("../../../modules/core/ops/numeric.gin")),
    (
        "ops",
        include_str!("../../../modules/core/ops/operators.gin"),
    ),
    ("ops", include_str!("../../../modules/core/ops/release.gin")),
    ("ops", include_str!("../../../modules/core/ops/copy.gin")),
    (
        "slice",
        include_str!("../../../modules/core/slice/slice.gin"),
    ),
    (
        "alloc",
        include_str!("../../../modules/core/alloc/error.gin"),
    ),
    (
        "alloc",
        include_str!("../../../modules/core/alloc/count.gin"),
    ),
    (
        "alloc",
        include_str!("../../../modules/core/alloc/address.gin"),
    ),
    (
        "alloc",
        include_str!("../../../modules/core/alloc/layout.gin"),
    ),
    ("iter", include_str!("../../../modules/core/iter/iter.gin")),
];

fn client_flaws(source: &str) -> Vec<String> {
    let package = PackageInstanceKey::workspace("core", "1");
    let metadata = include_str!("../../../modules/core/target/emission/emission.gin")
        .split("InstructionWord is")
        .next()
        .unwrap()
        .replace("use core.range Range\n", "");
    let string_views = format!(
        "use core.slice Byte, Length, SliceView\nStringBytes has{}",
        include_str!("../../../modules/core/string/string.gin")
            .split_once("StringBytes has")
            .unwrap()
            .1
            .split("FromUtf8Error")
            .next()
            .unwrap()
    );
    let sources = LIBRARY_SOURCES.into_iter().chain([
        ("target.emission", metadata.as_str()),
        ("string", string_views.as_str()),
        (
            "resource",
            "Resource has\n    value @()\n    release(self): return\n",
        ),
        ("client", source),
    ]);
    let target =
        flask::CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap());
    let mut files = sources
        .map(|(module, source)| {
            let mut parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{module}: {:?}",
                parsed.symptoms
            );
            parsed.ast.semantic_origin = Some(FileSemanticOrigin {
                package: package.clone(),
                module: module.split('.').map(Intern::from_ref).collect(),
            });
            parsed.ast
        })
        .collect::<Vec<_>>();
    let preparation = typecheck::prepare_package_asts(&mut files, &target);
    assert!(preparation.iter().all(Vec::is_empty), "{preparation:?}");
    let outputs = transform_package_with_shared_context_and_package(
        files,
        PackageTransformOptions::FULL.with_compile_target(target),
        package,
    );
    let (client, library) = outputs.typed_asts.split_last().unwrap();
    for file in library {
        for (_, flaw) in file.all_flaws() {
            assert_eq!(
                flaw.code.slug().as_str(),
                "type-return-mismatch",
                "{flaw:?}"
            );
        }
    }
    client
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.to_string())
        .collect()
}

#[test]
fn scalar_metadata_and_borrowed_views_need_no_terminal_operation() {
    let flaws = client_flaws(
        r#"use core.slice Byte, Slice, ArrayView, ArrayMutView, SliceView, SliceMutView
use core.alloc Layout, AllocError
use core.target.emission Signature, Input, MemoryAccess, Code
use core.string StringBytes, StringByte
forget(
    slice Slice(Byte)
    array ArrayView(Byte, 2)
    array_mut ArrayMutView(Byte, 2)
    view SliceView(Byte)
    view_mut SliceMutView(Byte)
    layout Layout(Byte)
    error AllocError
    signature Signature(Byte, Byte)
    input Input(Byte, Byte)
    memory MemoryAccess(Byte, Byte, Byte)
    code Code(Byte, Byte, Byte, Byte, Byte, Byte, Byte, Byte)
    string_bytes StringBytes
    string_byte StringByte
): return
"#,
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn iterator_steps_preserve_resource_payloads() {
    let imports = "use core.iter Step\nuse core.resource Resource\n";
    let flaws = client_flaws(&format!("{imports}forget(step Step(Resource)): return\n"));
    assert_eq!(flaws, ["type-terminal-value-not-consumed"]);
    let flaws = client_flaws(&format!(
        "{imports}finish(step Step(Resource)):\n    when step is Yield(value) then value.release\n    return\n"
    ));
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn emission_records_preserve_resource_fields() {
    let imports =
        "use core.target.emission Code\nuse core.slice Byte\nuse core.resource Resource\n";
    let parameter = "code Code(Byte, Byte, Resource, Byte, Byte, Byte, Byte, Byte)";
    let flaws = client_flaws(&format!("{imports}forget({parameter}): return\n"));
    assert_eq!(flaws, ["type-terminal-field-not-consumed"]);
    let flaws = client_flaws(&format!(
        "{imports}finish({parameter}):\n    code.bytes.release\n    return\n"
    ));
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn generic_copy_results_still_release_owned_payloads() {
    let flaws = client_flaws(
        "use core.ops CopyResult\nuse core.resource Resource\nforget(result CopyResult(Resource)): return\n",
    );
    assert_eq!(flaws, ["type-terminal-value-not-consumed"]);
    let flaws = client_flaws(
        "use core.ops CopyResult\nuse core.resource Resource\nfinish(result CopyResult(Resource)): return result.release\n",
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}
