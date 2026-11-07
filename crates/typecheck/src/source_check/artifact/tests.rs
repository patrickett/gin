use super::*;
use interface_codec::{Layer, decode_artifact};

const BASE: &str = "\
limit := 9
#unrestricted
Digit is in 0 to limit
Width has
    width(self) in 0 to 9
Digit has Width
    width(self) in 0 to 9: return 4
#unrestricted
Level is Low = 1 or High = 2
#unrestricted
Shape is Dot or Line(Digit)
Box(element) has
    #private
    item element
wrap(element Type, x element) Box(element): return Box(item: x)
pick(element Type, x element) element: return x
double(d Digit) in 0 to 18: return d + d
";

const APP: &str = "\
use base Digit, Level, Shape, Box, wrap, pick, double, limit, Width
total(d Digit) in 0 to 18: return double(d)
same(d Digit) Digit: return pick(Digit, d)
boxed(d Digit) Box(Digit): return wrap(Digit, d)
measure(d Digit) in 0 to 9: return d.width()
#unrestricted
Count is in 0 to 9
Count has Width
    width(self) in 0 to 9: return 2
";

fn id(package: &str, module: &str, source: u8) -> ModuleId {
    ModuleId {
        package: package.into(),
        version: "1.0.0".into(),
        source: [source; 32],
        module: module.into(),
    }
}

fn identity(module: ModuleId, dependencies: &[ModuleId]) -> ArtifactIdentity {
    ArtifactIdentity {
        module,
        dependencies: dependencies.to_vec(),
        target: "test-target".into(),
        compiler: "ginc test".into(),
    }
}

fn expectation(identity: &ArtifactIdentity) -> Expectation {
    Expectation {
        module: identity.module.clone(),
        target: identity.target.clone(),
        compiler: identity.compiler.clone(),
    }
}

fn checked(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> std::result::Result<CheckedSourceModule, SourceCheckError> {
    let source =
        parser::parse_strict(text).unwrap_or_else(|error| panic!("{text}\n{error:?}"));
    check_module(name, &source, dependencies, crate::Limits::default())
}

fn module(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    checked(name, text, dependencies).unwrap_or_else(|error| panic!("{text}\n{error:?}"))
}

fn base_id() -> ModuleId {
    id("base-pkg", "base", 1)
}

fn sig_hash(module: &CheckedSourceModule, identity: &ArtifactIdentity) -> [u8; 32] {
    decode_artifact(&write_artifact(module, identity).unwrap())
        .unwrap()
        .signature
        .hash
}

fn base_sig(text: &str) -> [u8; 32] {
    sig_hash(&module("base", text, &[]), &identity(base_id(), &[]))
}

#[test]
fn hidden_field_counts_cannot_allocate_unrepresented_slots() {
    let base = module("base", "Box has\n    #private\n    item in 0 to 9\n", &[]);
    let identity = identity(base_id(), &[]);
    let bytes = write_artifact(&base, &identity).unwrap();
    let mut artifact = decode_artifact(&bytes).unwrap();
    let Node::Record(_, signature) = &mut artifact.signature.body else {
        unreachable!()
    };
    let Node::List(layouts) = &mut signature[1] else {
        unreachable!()
    };
    let Node::Record(_, layout) = &mut layouts[0] else {
        unreachable!()
    };
    layout[4] = Node::Uint(u64::MAX);
    layout[5] = Node::Uint(u64::MAX);
    let forged = interface_codec::encode_artifact(
        artifact.header(),
        &artifact.signature.body,
        &artifact.instantiation.body,
    );
    assert!(matches!(
        ArtifactReader::new().read(&forged, &expectation(&identity)),
        Err(ArtifactError::Shape(_))
    ));
}

#[test]
fn summary_dispositions_are_parallel_to_inputs() {
    let base = module(
        "base",
        "double(x in 0 to 9) in 0 to 18: return x + x\n",
        &[],
    );
    let identity = identity(base_id(), &[]);
    let mut artifact = decode_artifact(&write_artifact(&base, &identity).unwrap()).unwrap();
    let Node::Record(_, signature) = &mut artifact.signature.body else {
        unreachable!()
    };
    let Node::List(functions) = &mut signature[2] else {
        unreachable!()
    };
    let Node::Record(_, function) = &mut functions[0] else {
        unreachable!()
    };
    let Node::List(summaries) = &mut function[9] else {
        unreachable!()
    };
    let Node::Record(_, summary) = &mut summaries[0] else {
        unreachable!()
    };
    summary[3] = Node::List(Vec::new());
    let bytes = interface_codec::encode_artifact(
        artifact.header(),
        &artifact.signature.body,
        &artifact.instantiation.body,
    );
    assert!(matches!(
        ArtifactReader::new().read(&bytes, &expectation(&identity)),
        Err(ArtifactError::Shape(_))
    ));
}

fn body_nodes(instantiation: &mut Node) -> &mut Vec<Node> {
    let Node::Record(_, fields) = instantiation else {
        unreachable!()
    };
    let Node::List(payloads) = &mut fields[3] else {
        unreachable!()
    };
    let Node::Record(_, payload) = &mut payloads[0] else {
        unreachable!()
    };
    let Node::Record(_, body) = &mut payload[3] else {
        unreachable!()
    };
    let Node::List(nodes) = &mut body[3] else {
        unreachable!()
    };
    nodes
}

#[test]
fn body_references_must_precede_their_consumer() {
    let base = module(
        "base",
        "double(x in 0 to 9) in 0 to 18: return x + x\n",
        &[],
    );
    let identity = identity(base_id(), &[]);
    let (header, signature, mut instantiation) = artifact_nodes(&base, &identity).unwrap();
    let nodes = body_nodes(&mut instantiation);
    let mut changed = false;
    for (at, node) in nodes.iter_mut().enumerate() {
        let Node::Record(_, expression) = node else {
            unreachable!()
        };
        if let Node::Record(K_RETURN, fields) = &mut expression[1] {
            fields[0] = Node::Uint(at as u64);
            changed = true;
        }
    }
    assert!(changed);
    let bytes = interface_codec::encode_artifact(&header, &signature, &instantiation);
    assert!(matches!(
        ArtifactReader::new().read(&bytes, &expectation(&identity)),
        Err(ArtifactError::Shape(_))
    ));
}

#[test]
fn select_regions_cannot_escape_their_expression() {
    let base = module(
        "base",
        "pick(x in 0 to 9) in 0 to 9: return when x < 5 then x else 0\n",
        &[],
    );
    let identity = identity(base_id(), &[]);
    let (header, signature, mut instantiation) = artifact_nodes(&base, &identity).unwrap();
    let mut changed = false;
    for node in body_nodes(&mut instantiation) {
        let Node::Record(_, expression) = node else {
            unreachable!()
        };
        if let Node::Record(K_SELECT, fields) = &mut expression[1] {
            fields[5] = Node::record(
                SELECT_REGION,
                vec![Node::Uint(0), Node::Uint(u32::MAX.into()), Node::Bool(true)],
            );
            changed = true;
        }
    }
    assert!(changed);
    let bytes = interface_codec::encode_artifact(&header, &signature, &instantiation);
    assert!(matches!(
        ArtifactReader::new().read(&bytes, &expectation(&identity)),
        Err(ArtifactError::Shape(_))
    ));
}

#[test]
fn summary_roots_match_the_function_value_arity() {
    let base = module(
        "base",
        "double(x in 0 to 9) in 0 to 18: return x + x\n",
        &[],
    );
    let identity = identity(base_id(), &[]);
    let (header, mut signature, instantiation) = artifact_nodes(&base, &identity).unwrap();
    let Node::Record(_, fields) = &mut signature else {
        unreachable!()
    };
    let Node::List(functions) = &mut fields[2] else {
        unreachable!()
    };
    let Node::Record(_, function) = &mut functions[0] else {
        unreachable!()
    };
    let Node::List(summaries) = &mut function[9] else {
        unreachable!()
    };
    let Node::Record(_, summary) = &mut summaries[0] else {
        unreachable!()
    };
    summary[0] = Node::List(Vec::new());
    summary[1] = Node::List(Vec::new());
    summary[2] = Node::List(Vec::new());
    summary[3] = Node::List(Vec::new());
    summary[5] = Node::List(Vec::new());
    summary[6] = Node::List(Vec::new());
    let bytes = interface_codec::encode_artifact(&header, &signature, &instantiation);
    assert!(matches!(
        ArtifactReader::new().read(&bytes, &expectation(&identity)),
        Err(ArtifactError::Shape(_))
    ));
}

#[test]
fn dependency_callees_survive_read_write_read() {
    let base = module("base", BASE, &[]);
    let app = module("app", APP, &[&base]);
    let base_identity = identity(base_id(), &[]);
    let app_identity = identity(id("app-pkg", "app", 3), &[base_id()]);
    let base_bytes = write_artifact(&base, &base_identity).unwrap();
    let app_bytes = write_artifact(&app, &app_identity).unwrap();
    let mut reader = ArtifactReader::new();
    reader
        .read(&base_bytes, &expectation(&base_identity))
        .unwrap();
    let read = reader
        .read(&app_bytes, &expectation(&app_identity))
        .unwrap();
    let rewritten = write_artifact(&read, &app_identity).unwrap();
    assert_eq!(rewritten, app_bytes);
    let again = reader
        .read(&rewritten, &expectation(&app_identity))
        .unwrap();
    assert_eq!(write_artifact(&again, &app_identity).unwrap(), app_bytes);
    let top = "use base Digit\nuse app total\nf(d Digit) in 0 to 18: return total(d)\n";
    let base_read = reader
        .read(&base_bytes, &expectation(&base_identity))
        .unwrap();
    assert!(checked("top", top, &[&again, &base_read]).is_ok());
}

#[test]
fn the_same_module_gives_the_same_bytes() {
    let base = module("base", BASE, &[]);
    let identity = identity(base_id(), &[]);
    let first = write_artifact(&base, &identity).unwrap();
    assert_eq!(first, write_artifact(&base, &identity).unwrap());
    assert_eq!(
        first,
        write_artifact(&module("base", BASE, &[]), &identity).unwrap()
    );
    let dump = dump_artifact(&first).unwrap();
    assert_eq!(dump, dump_artifact(&first).unwrap());
    for name in [
        "\"Digit\"",
        "\"Shape.Line\"",
        "\"pick\"",
        "discriminant-integer",
        "\"Width\"",
        "\"limit\"",
    ] {
        assert!(dump.contains(name), "{name} missing from\n{dump}");
    }
}

#[test]
fn reordering_declarations_keeps_the_signature_hash() {
    // Same declarations, other order (a type's `has` block stays after the type).
    let reordered = "\
double(d Digit) in 0 to 18: return d + d
pick(element Type, x element) element: return x
wrap(element Type, x element) Box(element): return Box(item: x)
Box(element) has
    #private
    item element
#unrestricted
Shape is Dot or Line(Digit)
#unrestricted
Level is Low = 1 or High = 2
Width has
    width(self) in 0 to 9
#unrestricted
Digit is in 0 to limit
Digit has Width
    width(self) in 0 to 9: return 4
limit := 9
";
    let reordered = module("base", reordered, &[]);
    let identity = identity(base_id(), &[]);
    assert_eq!(sig_hash(&reordered, &identity), base_sig(BASE));
    // Negative control: reordering a union's variants is a change (order is the tag).
    let swapped = BASE.replace("Shape is Dot or Line(Digit)", "Shape is Line(Digit) or Dot");
    assert_ne!(base_sig(&swapped), base_sig(BASE));
}

#[test]
fn a_private_field_or_private_method_body_does_not_change_the_signature() {
    let with_private = "#unrestricted\nCounter has\n    #private\n    value in 0 to 9\n    #private\n    peek(self) in 0 to 9: return 1\n";
    let changed = with_private.replace("return 1", "return 2");
    let a = module("base", with_private, &[]);
    let b = module("base", &changed, &[]);
    let identity = identity(base_id(), &[]);
    let (x, y) = (
        write_artifact(&a, &identity).unwrap(),
        write_artifact(&b, &identity).unwrap(),
    );
    let (x, y) = (decode_artifact(&x).unwrap(), decode_artifact(&y).unwrap());
    assert_eq!(x.signature.hash, y.signature.hash);
    // Negative control: a private field's type is in the other layer.
    let wider = with_private.replace("value in 0 to 9", "value in 0 to 99");
    let z =
        decode_artifact(&write_artifact(&module("base", &wider, &[]), &identity).unwrap()).unwrap();
    assert_eq!(x.signature.hash, z.signature.hash);
    assert_ne!(x.instantiation.hash, z.instantiation.hash);
}

#[test]
fn public_changes_change_the_signature_hash() {
    let original = base_sig(BASE);
    let changes = [
        // A public signature.
        ("double(d Digit) in 0 to 18", "double(d Digit) in 0 to 19"),
        // A domain bound, through the constant and directly.
        ("limit := 9", "limit := 8"),
        ("Digit is in 0 to limit", "Digit is in 0 to 8"),
        // A union's variant set.
        (
            "Shape is Dot or Line(Digit)",
            "Shape is Dot or Line(Digit) or Arc",
        ),
        // A discriminant.
        ("Low = 1 or High = 2", "Low = 1 or High = 3"),
        // A claim.
        (
            "double(d Digit) in 0 to 18: return d + d",
            "double(d Digit and < 5) in 0 to 18: return d + d",
        ),
    ];
    for (from, to) in changes {
        assert!(BASE.contains(from), "{from}");
        let text = BASE.replace(from, to);
        let changed =
            checked("base", &text, &[]).unwrap_or_else(|error| panic!("{from} -> {to}: {error:?}"));
        assert_ne!(
            sig_hash(&changed, &identity(base_id(), &[])),
            original,
            "{from} -> {to}"
        );
    }
}

#[test]
fn version_magic_hash_truncation_and_corruption_are_refused() {
    let base = module("base", BASE, &[]);
    let identity = identity(base_id(), &[]);
    let bytes = write_artifact(&base, &identity).unwrap();
    let expected = expectation(&identity);
    assert!(ArtifactReader::new().read(&bytes, &expected).is_ok());
    let mut wrong = bytes.clone();
    wrong[0] ^= 1;
    assert_eq!(
        ArtifactReader::new().read(&wrong, &expected).err(),
        Some(ArtifactError::Codec(CodecError::Magic))
    );
    let mut wrong = bytes.clone();
    wrong[6] = 9;
    assert_eq!(
        ArtifactReader::new().read(&wrong, &expected).err(),
        Some(ArtifactError::Codec(CodecError::Version(9)))
    );
    let mut wrong = bytes.clone();
    let middle = bytes.len() / 3;
    wrong[middle] ^= 0x40;
    assert!(matches!(
        ArtifactReader::new().read(&wrong, &expected),
        Err(ArtifactError::Codec(_))
    ));
    for cut in (0..bytes.len()).step_by(7) {
        assert!(
            ArtifactReader::new()
                .read(&bytes[..cut], &expected)
                .is_err()
        );
    }
    // A raw artifact made from other source is refused, however intact.
    let mut other = expected.clone();
    other.module.source = [2; 32];
    assert_eq!(
        ArtifactReader::new().read(&bytes, &other).err(),
        Some(ArtifactError::Codec(CodecError::Provenance(
            "source digest"
        )))
    );
    let mut other = expected;
    other.compiler = "ginc other".into();
    assert!(ArtifactReader::new().read(&bytes, &other).is_err());
}

#[test]
fn well_formed_bytes_with_a_forged_body_are_refused_cleanly() {
    let base = module("base", BASE, &[]);
    let identity = identity(base_id(), &[]);
    let (header, signature, instantiation) = artifact_nodes(&base, &identity).unwrap();
    let expected = expectation(&identity);
    let forge =
        |signature: &Node| interface_codec::encode_artifact(&header, signature, &instantiation);
    let Node::Record(tag, mut fields) = signature.clone() else {
        unreachable!()
    };
    // A type number past the table.
    fields[0] = Node::List(Vec::new());
    let bytes = forge(&Node::Record(tag, fields));
    assert!(matches!(
        ArtifactReader::new().read(&bytes, &expected),
        Err(ArtifactError::Shape(_) | ArtifactError::UnknownItem)
    ));
    // A body of the wrong shape.
    let bytes = forge(&Node::Uint(3));
    assert!(matches!(
        ArtifactReader::new().read(&bytes, &expected),
        Err(ArtifactError::Codec(_) | ArtifactError::Shape(_))
    ));
}

#[test]
fn two_packages_with_a_same_named_type_do_not_collide() {
    let text = "#unrestricted\nBuffer is in 0 to 9\nmake() Buffer: return 1\n";
    let one = module("buffers", text, &[]);
    let one_id = id("first", "buffers", 1);
    let two_id = id("second", "buffers", 2);
    let one_identity = identity(one_id.clone(), &[]);
    let two_identity = identity(two_id.clone(), &[]);
    let first = write_artifact(&one, &one_identity).unwrap();
    let second = write_artifact(&one, &two_identity).unwrap();
    assert_ne!(
        decode_artifact(&first).unwrap().signature.hash,
        decode_artifact(&second).unwrap().signature.hash
    );
    let mut reader = ArtifactReader::new();
    let a = reader.read(&first, &expectation(&one_identity)).unwrap();
    let b = reader.read(&second, &expectation(&two_identity)).unwrap();
    assert!(a.layouts[0].origin != b.layouts[0].origin);
    // The same identity read again is the same module: one origin.
    let again = reader.read(&first, &expectation(&one_identity)).unwrap();
    assert!(a.layouts[0].origin == again.layouts[0].origin);
}

/// A dependent checked against the capsule read back from the artifact gets exactly the
/// signature it gets against the capsule checked from source, and fails where it fails.
#[test]
fn a_dependent_checks_the_same_against_the_artifact() {
    let base = module("base", BASE, &[]);
    let base_identity = identity(base_id(), &[]);
    let bytes = write_artifact(&base, &base_identity).unwrap();
    let read = ArtifactReader::new()
        .read(&bytes, &expectation(&base_identity))
        .unwrap();
    // The capsule read back writes the same signature layer.
    assert_eq!(
        decode_artifact(&write_artifact(&read, &base_identity).unwrap())
            .unwrap()
            .signature,
        decode_artifact(&bytes).unwrap().signature
    );
    let app_identity = identity(id("app-pkg", "app", 3), &[base_id()]);
    let from_source = module("app", APP, &[&base]);
    let from_artifact = module("app", APP, &[&read]);
    let source_bytes = write_artifact(&from_source, &app_identity).unwrap();
    let artifact_bytes = write_artifact(&from_artifact, &app_identity).unwrap();
    assert_eq!(
        dump_artifact(&source_bytes).unwrap(),
        dump_artifact(&artifact_bytes).unwrap()
    );
    assert_eq!(source_bytes, artifact_bytes);
    let dump = dump_artifact(&source_bytes).unwrap();
    assert!(
        dump.contains("\"base-pkg\"") && dump.contains("\"Box\"") && dump.contains("\"Digit\""),
        "{dump}"
    );
    // Negative controls: what fails against the source capsule fails against the artifact.
    for wrong in [
        APP.replace("total(d Digit) in 0 to 18", "total(d Digit) in 0 to 17"),
        APP.replace("measure(d Digit) in 0 to 9", "measure(d Digit) in 0 to 3"),
        format!("{APP}bad(s Shape) in 0 to 9: return s\n"),
        format!("{APP}level() Level: return Level.Middle\n"),
    ] {
        let a = checked("app", &wrong, &[&base]).err();
        let b = checked("app", &wrong, &[&read]).err();
        assert!(a.is_some(), "{wrong}");
        assert_eq!(a, b, "{wrong}");
    }
}

#[test]
fn every_tag_has_a_name_in_the_dump() {
    let base = module("base", BASE, &[]);
    let bytes = write_artifact(&base, &identity(base_id(), &[])).unwrap();
    let artifact = decode_artifact(&bytes).unwrap();
    let dump = interface_codec::dump(&artifact, &|layer, tag| {
        let _ = matches!(layer, Layer::Signature);
        artifact_tag_name(tag)
    });
    assert!(
        !dump.lines().any(|line| line.trim_start().starts_with('#')),
        "{dump}"
    );
}

/// Three levels: a module whose capsule carries its dependency's types is written and read
/// back, and a module using both checks as it does against the source capsules.
#[test]
fn a_diamond_through_artifacts_checks_the_same() {
    const MID: &str =
        "use base Digit, Box, wrap\nlift(d Digit) Box(Digit): return wrap(Digit, d)\n";
    const TOP: &str =
        "use base Digit, Box\nuse mid lift\ntop(d Digit) Box(Digit): return lift(d)\n";
    let base = module("base", BASE, &[]);
    let mid = module("mid", MID, &[&base]);
    let top = module("top", TOP, &[&mid, &base]);
    let base_identity = identity(base_id(), &[]);
    let mid_identity = identity(id("mid-pkg", "mid", 4), &[base_id()]);
    let top_identity = identity(
        id("top-pkg", "top", 5),
        &[base_id(), mid_identity.module.clone()],
    );
    let mut reader = ArtifactReader::new();
    let base_read = reader
        .read(
            &write_artifact(&base, &base_identity).unwrap(),
            &expectation(&base_identity),
        )
        .unwrap();
    let mid_read = reader
        .read(
            &write_artifact(&mid, &mid_identity).unwrap(),
            &expectation(&mid_identity),
        )
        .unwrap();
    let top_read = module("top", TOP, &[&mid_read, &base_read]);
    assert_eq!(
        write_artifact(&top, &top_identity).unwrap(),
        write_artifact(&top_read, &top_identity).unwrap()
    );
    // Negative control: two readers do not share origins, so the same `Box` from two readers is
    // two types and the call no longer checks.
    let mid_bytes = write_artifact(&mid, &mid_identity).unwrap();
    let mut second = ArtifactReader::new();
    // A body of `mid` calls into `base`: a reader that has not read `base` cannot resolve it.
    assert_eq!(
        second.read(&mid_bytes, &expectation(&mid_identity)).err(),
        Some(ArtifactError::UnknownItem)
    );
    second
        .read(
            &write_artifact(&base, &base_identity).unwrap(),
            &expectation(&base_identity),
        )
        .unwrap();
    let other = second
        .read(&mid_bytes, &expectation(&mid_identity))
        .unwrap();
    assert!(checked("top", TOP, &[&other, &base_read]).is_err());
}

/// A module with no type of its own makes an anonymous record and an inline result sum. They
/// get that module's origin, not its dependency's, so a third module importing both keeps
/// every type apart.
#[test]
fn layouts_the_checker_makes_have_their_own_module_origin() {
    let a = module("a", "#unrestricted\nCode is in 0 to 9\n", &[]);
    let b = module(
        "b",
        concat!(
            "use a Code\n",
            "split(c Code) first in 0 to 9, second in 0 to 9: return c, c\n",
            "pick(n in 0 to 9) Low(Code) or\n  High(Code):\n    return Low(n as Code)\n",
        ),
        &[&a],
    );
    let code = &a.layouts[0];
    let made: Vec<&Layout> = b
        .layouts
        .iter()
        .filter(|layout| layout.key.is_some())
        .collect();
    assert!(
        made.len() >= 3,
        "{:?}",
        b.layouts.iter().map(|l| &l.schema.name).collect::<Vec<_>>()
    );
    for layout in &made {
        assert!(layout.origin != code.origin, "{}", layout.schema.name);
        assert!(
            !Arc::ptr_eq(&layout.origin.module, &code.origin.module),
            "{}",
            layout.schema.name
        );
    }
    let keys: Vec<&str> = made
        .iter()
        .map(|layout| layout.key.as_deref().unwrap())
        .collect();
    assert!(
        keys.contains(&"split#record")
            && keys.contains(&"pick#result")
            && keys.contains(&"pick#result.Low"),
        "{keys:?}"
    );
    let c = module(
        "c",
        "use a Code\nuse b split, pick\nfirst(x Code) in 0 to 9: return split(x).first\nuse_code(x Code) Code: return x\n",
        &[&b, &a],
    );
    // `Code` and each layout `b` made stay distinct in the importer.
    let names: Vec<&str> = c
        .layouts
        .iter()
        .map(|layout| layout.schema.name.as_str())
        .collect();
    assert!(
        names.contains(&"Code") && names.iter().filter(|name| **name == "Code").count() == 1,
        "{names:?}"
    );
    assert!(c.layouts.len() >= 1 + made.len(), "{names:?}");
    // `b`'s artifact names them by their recorded keys.
    let bytes = write_artifact(&b, &identity(id("b-pkg", "b", 7), &[id("a-pkg", "a", 6)])).unwrap();
    let dump = dump_artifact(&bytes).unwrap();
    for key in [
        "\"split#record\"",
        "\"pick#result\"",
        "\"pick#result.High\"",
    ] {
        assert!(dump.contains(key), "{key}");
    }
}

/// No layout at all, no import: making an anonymous record must not panic.
#[test]
fn a_module_with_no_layouts_makes_a_record() {
    let alone = module(
        "alone",
        "split(n in 0 to 9) first in 0 to 9, second in 0 to 9: return n, n\n",
        &[],
    );
    assert_eq!(alone.layouts.len(), 1);
    assert_eq!(alone.layouts[0].key.as_deref(), Some("split#record"));
}

/// A package with a private helper that a public body calls, a folded call, a template and an
/// entry-free library: what the payload layer must carry.
const LIB: &str = "\
#unrestricted
Digit is in 0 to 9
#unrestricted
Pair has
    value Digit
    #private
    helper(self) in 0 to 18: return self.value + self.value
    sum(self) in 0 to 18: return self.helper()
double(d Digit) in 0 to 18: return d + d
pick(element Type, x element) element: return x
seven() Digit: return 7
";

const USER: &str = "\
use lib Digit, Pair, double, pick, seven
twice() in 0 to 18: return double(seven())
same(d Digit) Digit: return pick(Digit, d)
total(d Digit) in 0 to 18: return double(d)
paired(p Pair) in 0 to 18: return p.sum()
";

fn lib_id() -> ModuleId {
    id("lib-pkg", "lib", 8)
}

/// Remove every body a capsule carries: what the artifact read back was before the payload.
fn without_bodies(module: &mut CheckedSourceModule) {
    for function in &mut module.functions {
        function.body = None;
    }
    for body in &mut module.checked.bodies {
        *body = None;
    }
}

#[test]
fn the_payload_round_trips_and_carries_private_callees() {
    let lib = module("lib", LIB, &[]);
    let identity = identity(lib_id(), &[]);
    let bytes = write_artifact(&lib, &identity).unwrap();
    let read = ArtifactReader::new()
        .read(&bytes, &expectation(&identity))
        .unwrap();
    // Both layers, written again from the capsule read back, are the same bytes.
    assert_eq!(write_artifact(&read, &identity).unwrap(), bytes);
    // Every own function is there at its own index, with its body, the private one included.
    assert_eq!(read.checked.functions.len(), lib.checked.functions.len());
    for (a, b) in read.checked.functions.iter().zip(&lib.checked.functions) {
        assert_eq!(a.name, b.name);
    }
    let helper = read
        .functions
        .iter()
        .position(|f| f.signature.name == "Pair.helper")
        .unwrap();
    assert!(read.functions[helper].signature.private && read.functions[helper].body.is_some());
    assert!(!read.exports.iter().any(|(name, _)| name == "Pair.helper"));
    assert_eq!(
        read.checked.bodies.iter().filter(|b| b.is_some()).count(),
        lib.checked.bodies.iter().filter(|b| b.is_some()).count()
    );
    let dump = dump_artifact(&bytes).unwrap();
    assert!(
        dump.contains("\"Pair.helper\"") && dump.contains("payload") && dump.contains("local"),
        "{dump}"
    );
    // A private helper's body is in `inst` only.
    let changed = module(
        "lib",
        &LIB.replace("return self.value + self.value", "return self.value * 2"),
        &[],
    );
    let (x, y) = (
        decode_artifact(&bytes).unwrap(),
        decode_artifact(&write_artifact(&changed, &identity).unwrap()).unwrap(),
    );
    assert_eq!(x.signature.hash, y.signature.hash);
    assert_ne!(x.instantiation.hash, y.instantiation.hash);
}

/// An importer folds a call of an imported function with known arguments, and calls a
/// template: against the artifact it records exactly what it records against the source.
#[test]
fn an_importer_folds_and_instantiates_the_same_through_the_artifact() {
    let lib = module("lib", LIB, &[]);
    let lib_identity = identity(lib_id(), &[]);
    let read = ArtifactReader::new()
        .read(
            &write_artifact(&lib, &lib_identity).unwrap(),
            &expectation(&lib_identity),
        )
        .unwrap();
    let user_identity = identity(id("user-pkg", "user", 9), &[lib_id()]);
    let from_source = module("user", USER, &[&lib]);
    let from_artifact = module("user", USER, &[&read]);
    let twice = |m: &CheckedSourceModule| {
        m.functions
            .iter()
            .find(|f| f.signature.name == "twice")
            .unwrap()
            .folds
            .len()
    };
    assert!(twice(&from_source) > 0, "the call is folded from source");
    assert_eq!(twice(&from_source), twice(&from_artifact));
    let a = write_artifact(&from_source, &user_identity).unwrap();
    let b = write_artifact(&from_artifact, &user_identity).unwrap();
    assert_eq!(dump_artifact(&a).unwrap(), dump_artifact(&b).unwrap());
    assert_eq!(a, b);
    // Negative control: without the bodies, the importer cannot fold, and its payload differs.
    let mut bare = ArtifactReader::new()
        .read(
            &write_artifact(&lib, &lib_identity).unwrap(),
            &expectation(&lib_identity),
        )
        .unwrap();
    without_bodies(&mut bare);
    let from_bare = module("user", USER, &[&bare]);
    assert_eq!(twice(&from_bare), 0);
    let c = write_artifact(&from_bare, &user_identity).unwrap();
    assert_eq!(
        decode_artifact(&a).unwrap().signature,
        decode_artifact(&c).unwrap().signature
    );
    assert_ne!(
        decode_artifact(&a).unwrap().instantiation,
        decode_artifact(&c).unwrap().instantiation
    );
}

#[test]
fn a_forged_body_node_id_is_refused() {
    let lib = module("lib", LIB, &[]);
    let identity = identity(lib_id(), &[]);
    let (header, signature, instantiation) = artifact_nodes(&lib, &identity).unwrap();
    let expected = expectation(&identity);
    // Point every `return` past the end of its body.
    fn forge(node: &mut Node) {
        match node {
            Node::Record(K_RETURN, fields) => fields[0] = Node::Uint(1 << 20),
            Node::Record(_, fields) | Node::List(fields) => fields.iter_mut().for_each(forge),
            _ => {}
        }
    }
    let mut forged = instantiation.clone();
    forge(&mut forged);
    assert_ne!(forged, instantiation);
    let bytes = interface_codec::encode_artifact(&header, &signature, &forged);
    assert_eq!(
        ArtifactReader::new().read(&bytes, &expected).err(),
        Some(ArtifactError::Shape("body node"))
    );
    // A local callee naming no function of the module.
    fn rename(node: &mut Node) {
        match node {
            Node::Record(CALLEE_LOCAL, fields) => fields[0] = Node::text("nowhere"),
            Node::Record(_, fields) | Node::List(fields) => fields.iter_mut().for_each(rename),
            _ => {}
        }
    }
    let mut forged = instantiation;
    rename(&mut forged);
    let bytes = interface_codec::encode_artifact(&header, &signature, &forged);
    assert_eq!(
        ArtifactReader::new().read(&bytes, &expected).err(),
        Some(ArtifactError::UnknownItem)
    );
}
