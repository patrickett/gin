mod support;

use parser::query::SourceParseExt;
use support::package;
use typecheck::FileId;

#[test]
fn recursive_effect_inference_terminates() {
    const CASE: &str = "GIN_TEST_RECURSIVE_EFFECT_CASE";
    if let Ok(case) = std::env::var(CASE) {
        if case.starts_with("cross-module") {
            let mut files = vec![
                (
                    "model",
                    "Node has\n    next ref Node\n    finish(self):\n        return\n",
                ),
                (
                    "left",
                    "use sample.model Node\nuse sample.right bounce\nwalk(value ref Node):\n    bounce(value.next)\n    return\n",
                ),
                (
                    "right",
                    "use sample.model Node\nuse sample.left walk\nbounce(value ref Node):\n    walk(value)\n    return\n",
                ),
            ];
            if case.ends_with("reversed") {
                files.reverse();
            }
            let typed_asts = package("sample", &files);
            let mut checked = 0;
            for typed in &typed_asts {
                assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
                for (id, bind) in &typed.defs {
                    if matches!(id.0.as_str(), "walk" | "bounce")
                        || id.0.as_str().ends_with(".walk")
                        || id.0.as_str().ends_with(".bounce")
                    {
                        checked += 1;
                        assert!(bind.effects.reads.iter().any(|target| matches!(
                            target,
                            typecheck::EffectTarget::Descendants(_)
                        )));
                    }
                }
            }
            assert_eq!(
                checked,
                2,
                "{:?}",
                typed_asts
                    .iter()
                    .map(|typed| typed.defs.keys().collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            );
            return;
        }
        let package_mode = case.ends_with("-package");
        let case = case.strip_suffix("-package").unwrap_or(&case);
        let prefix = "Node has\n    next ref Node\n    finish(self):\n        return\n";
        let source = match case {
            "identity" => format!("{prefix}walk(value ref Node):\n    walk(value)\n    return\n"),
            "projected" => {
                format!("{prefix}walk(value ref Node):\n    walk(value.next)\n    return\n")
            }
            "mutual" => format!(
                "{prefix}walk(value ref Node):\n    bounce(value.next)\n    return\nbounce(value ref Node):\n    walk(value)\n    return\n"
            ),
            "writes" => String::from(format!(
                "{}replace(previous Byte, next Byte) Byte: return next\nNode has\n    next mut Node\n    value Byte\n    finish(self):\n        return\nwalk(value mut Node):\n    value.value:: replace(value.value, 1)\n    walk(value.next)\n    return\n",
                test_fixtures::preludes::BYTE_PRELUDE
            )),
            "assembly" | "assembly-write" => String::from(
                r#"Len is in 0 to 18446744073709551615
Len has
    finish(self):
        return
Word is in 0 to 255
Word has
    finish(self):
        return
#phantom(input, output)
MemorySignature(input, output) has
    version Len
    finish(self):
        return
memory_signature MemorySignature([pointer ref Word, length Len], ()): MemorySignature(version: 1)
read_code := [signature: memory_signature, target: #target.triple, bytes: [144], inputs: [pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64], length: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [], clobbers: [], reads: [[pointer: 0, length: 1]], writes: []]
read(pointer ref Word, length Len): return #emit(read_code, [pointer: pointer, length: length])
walk(pointer ref Word, length Len):
    read(pointer, 1)
    walk(pointer, length)
    return
"#,
            ),
            _ => panic!("unknown case: {case}"),
        };
        let source = if case == "assembly-write" {
            source.replace(
                "reads: [[pointer: 0, length: 1]], writes: []",
                "reads: [], writes: [[pointer: 0, length: 1]]",
            )
        } else {
            source
        };
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut context = typecheck::transform::TransformCtx::new();
        context.compile_target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
        );
        let typed = if package_mode {
            typecheck::transform::transform_package_with_shared_context(
                vec![parsed.ast],
                typecheck::transform::PackageTransformOptions::FULL
                    .with_compile_target(context.compile_target),
            )
            .typed_asts
            .remove(0)
        } else {
            typecheck::transform::transform(&parsed.ast, FileId(0), &context)
        };
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let effects =
            &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("walk"))].effects;
        if case == "assembly" {
            assert!(!effects.unknown_memory);
            assert_eq!(effects.assembly_reads.len(), 1);
            assert!(effects.assembly_writes.is_empty());
        } else if case == "assembly-write" {
            assert!(effects.unknown_memory);
            assert!(effects.assembly_reads.is_empty());
            assert_eq!(effects.assembly_writes.len(), 1);
        } else {
            assert!(!effects.unknown_memory);
            if case == "writes" {
                assert!(
                    effects
                        .writes
                        .iter()
                        .any(|target| matches!(target, typecheck::EffectTarget::Descendants(_)))
                );
            } else {
                assert!(effects.writes.is_empty());
            }
            assert!(
                effects
                    .reads
                    .iter()
                    .any(|target| matches!(target, typecheck::EffectTarget::Descendants(_)))
            );
        }
        return;
    }
    let thread = std::thread::current();
    let mut failures = Vec::new();
    for case in [
        "identity",
        "projected",
        "mutual",
        "assembly",
        "assembly-write",
        "writes",
        "identity-package",
        "projected-package",
        "mutual-package",
        "assembly-package",
        "assembly-write-package",
        "writes-package",
        "cross-module",
        "cross-module-reversed",
    ] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", thread.name().unwrap(), "--nocapture"])
            .env(CASE, case)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        if !output.status.success() {
            failures.push(format!(
                "{case}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn acyclic_effect_inference_preserves_projected_targets() {
    let parsed = "Node has\n    next ref Node\n    finish(self):\n        return\nobserve(value ref Node) ref Node: return value.next\nwalk(value ref Node) ref Node: return observe(value.next)\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let effects =
        &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("walk"))].effects;
    assert!(!effects.unknown_memory);
    assert!(
        effects.reads.iter().any(
            |target| matches!(target, typecheck::EffectTarget::Deref(base)
                if matches!(base.as_ref(), typecheck::EffectTarget::Field { .. }))
        ),
        "{effects:?}"
    );
    assert!(
        !effects
            .reads
            .iter()
            .any(|target| matches!(target, typecheck::EffectTarget::Descendants(_)))
    );
}
