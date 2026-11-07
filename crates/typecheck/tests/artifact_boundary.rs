//! Adversarial canonical bytes exercise the public reader, then the importer on valid controls.
use interface_codec::{Expectation, ModuleId, Node, decode_artifact, encode_artifact};
use typecheck::{
    ArtifactError, ArtifactIdentity, ArtifactReader, CheckedSourceModule, Limits, check_module,
    write_artifact,
};

fn module(text: &str) -> CheckedSourceModule {
    let source = parser::parse_strict(text).unwrap();
    check_module("base", &source, &[], Limits::default()).unwrap()
}

fn identity() -> ArtifactIdentity {
    ArtifactIdentity {
        module: ModuleId {
            package: "boundary".into(),
            version: "1.0.0".into(),
            source: [1; 32],
            module: "base".into(),
        },
        dependencies: Vec::new(),
        target: "test-target".into(),
        compiler: "ginc test".into(),
    }
}

fn fields(node: &mut Node) -> &mut Vec<Node> {
    match node {
        Node::Record(_, fields) | Node::List(fields) => fields,
        _ => panic!("fixture has a record or list"),
    }
}

fn body_nodes(instantiation: &mut Node) -> &mut Vec<Node> {
    let payloads = fields(&mut fields(instantiation)[3]);
    let body = &mut fields(&mut payloads[0])[3];
    fields(&mut fields(body)[3])
}

fn forged(text: &str, tag: u64, mutate: impl Fn(&mut Vec<Node>)) -> Vec<u8> {
    let bytes = write_artifact(&module(text), &identity()).unwrap();
    let mut artifact = decode_artifact(&bytes).unwrap();
    let mut changed = 0;
    for expression in body_nodes(&mut artifact.instantiation.body) {
        match &mut fields(expression)[1] {
            Node::Record(kind, parts) if *kind == tag => {
                mutate(parts);
                changed += 1;
            }
            _ => {}
        }
    }
    assert!(changed > 0, "fixture must exercise tag {tag}");
    encode_artifact(
        artifact.header(),
        &artifact.signature.body,
        &artifact.instantiation.body,
    )
}

fn read(bytes: &[u8]) -> Result<CheckedSourceModule, ArtifactError> {
    let identity = identity();
    ArtifactReader::new().read(
        bytes,
        &Expectation {
            module: identity.module,
            compiler: identity.compiler,
            target: identity.target,
        },
    )
}

#[test]
fn parameter_indexes_are_checked_against_the_signature() {
    let text = "double(x in 0 to 9) in 0 to 18: return x + x\n";
    let bytes = forged(text, 101, |parts| parts[0] = Node::Uint(u32::MAX.into()));
    assert!(matches!(read(&bytes), Err(ArtifactError::Shape(_))));
    let valid = read(&write_artifact(&module(text), &identity()).unwrap()).unwrap();
    let importer =
        parser::parse_strict("use base double\nanswer() in 0 to 18: return double(4)\n")
            .unwrap();
    assert!(check_module("app", &importer, &[&valid], Limits::default()).is_ok());
}

#[test]
fn loop_regions_carries_and_results_are_bounded() {
    let text = "f(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\n";
    let valid = write_artifact(&module(text), &identity()).unwrap();
    assert!(read(&valid).is_ok());
    // Loop fields: start, end, counter, body_start, carried.
    for field in [2, 3] {
        let bytes = forged(text, 119, |parts| {
            parts[field] = Node::Uint(u32::MAX.into())
        });
        assert!(
            matches!(read(&bytes), Err(ArtifactError::Shape(_))),
            "loop field {field}"
        );
    }
    for field in 0..3 {
        let bytes = forged(text, 119, |parts| {
            let carry = &mut fields(&mut parts[4])[0];
            fields(carry)[field] = Node::Uint(u32::MAX.into());
        });
        assert!(
            matches!(read(&bytes), Err(ArtifactError::Shape(_))),
            "carry field {field}"
        );
    }
    let bytes = forged(text, 128, |parts| parts[1] = Node::Uint(u32::MAX.into()));
    assert!(matches!(read(&bytes), Err(ArtifactError::Shape(_))));
}

#[test]
fn while_split_and_exit_indexes_are_not_expression_ids() {
    let text =
        "f() in 0 to 10:\n  i in 0 to 10: 0\n  while i < 10 do\n    i :: i + 1\n  return i\n";
    assert!(read(&write_artifact(&module(text), &identity()).unwrap()).is_ok());
    for field in [4, 5, 7] {
        let bytes = forged(text, 120, |parts| {
            parts[field] = Node::Uint(u32::MAX.into())
        });
        assert!(
            matches!(read(&bytes), Err(ArtifactError::Shape(_))),
            "while field {field}"
        );
    }
    // An in-arena id is still not a valid carry index.
    let bytes = forged(text, 120, |parts| {
        parts[7] = Node::Uint(fields(&mut parts[6]).len() as u64);
    });
    assert!(matches!(read(&bytes), Err(ArtifactError::Shape(_))));
}

#[test]
fn match_and_select_regions_are_ordered_and_bounded() {
    let select = "f(x in 0 to 9) in 0 to 9: return when x < 5 then x else 0\n";
    let matching = "#unrestricted\nS is A or B\nf(s S) in 0 to 1:\n  when s is\n    A then return 0\n    B then return 1\n";
    for (text, tag, region_field, starts) in [(select, 114, 5, [0, 1]), (matching, 109, 2, [1, 2])]
    {
        assert!(read(&write_artifact(&module(text), &identity()).unwrap()).is_ok());
        for start in starts {
            let bytes = forged(text, tag, |parts| {
                fields(&mut parts[region_field])[start] = Node::Uint(u32::MAX.into());
            });
            assert!(
                matches!(read(&bytes), Err(ArtifactError::Shape(_))),
                "tag {tag} start {start}"
            );
        }
        let bytes = forged(text, tag, |parts| {
            let region = fields(&mut parts[region_field]);
            region[starts[0]] = Node::Uint(1);
            region[starts[1]] = Node::Uint(0);
        });
        assert!(
            matches!(read(&bytes), Err(ArtifactError::Shape(_))),
            "reversed tag {tag}"
        );
    }
}

#[test]
fn body_calls_match_the_callee_arity() {
    let text = "f(x in 0 to 9) in 0 to 9: return g(x)\ng(x in 0 to 9) in 0 to 9: return x\n";
    let bytes = forged(text, 112, |parts| parts[1] = Node::List(Vec::new()));
    assert!(matches!(read(&bytes), Err(ArtifactError::Shape(_))));
}

#[test]
fn type_instance_arguments_match_their_heads() {
    let text = "#unrestricted\nDigit is in 0 to 9\nf(x [Digit; 2]) [Digit; 2]: return x\n";
    let bytes = write_artifact(&module(text), &identity()).unwrap();
    assert!(read(&bytes).is_ok());
    let mut artifact = decode_artifact(&bytes).unwrap();
    let table = fields(&mut fields(&mut artifact.signature.body)[0]);
    assert!(!table.is_empty());
    let array = table.iter().position(|instance| {
        matches!(instance, Node::Record(_, fields) if matches!(&fields[0], Node::Record(41, _)))
    }).unwrap();
    fields(&mut table[array])[1] = Node::List(Vec::new());
    let bytes = encode_artifact(
        artifact.header(),
        &artifact.signature.body,
        &artifact.instantiation.body,
    );
    assert!(matches!(read(&bytes), Err(ArtifactError::Shape(_))));
}

#[test]
fn ownership_summaries_replay_after_artifact_roundtrip() {
    let library = "\
Slot(element) is Empty or Full(element)
Box(element) has value element
identity(element Type, value element) element: return value
pack(element Type, value element) Box(element): return Box(value: value)
unpack(element Type, boxed Box(element)) element:
    Box(value) := boxed
    return value
swap(element Type, values [element; 2]) [element; 2]:
    first: values.[0]
    second: values.[1]
    return [second, first]
rebuild(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
";
    let bytes = write_artifact(&module(library), &identity()).unwrap();
    let library = read(&bytes).unwrap();
    assert_eq!(write_artifact(&library, &identity()).unwrap(), bytes);
    let importer = parser::parse_strict(
        "\
use base identity, pack, unpack, swap
Number has value in 0 to 16
observe(first in 0 to 16, second in 0 to 16) in 0 to 16 and = second:
    values: swap(Number, [Number(value: first), Number(value: second)])
    boxed: pack(Number, values.[0])
    remaining: values.[1]
    Number(selected) := identity(Number, unpack(Number, boxed))
    Number(other) := remaining
    return selected
",
    )
    .unwrap();
    assert!(check_module("app", &importer, &[&library], Limits::default()).is_ok());
}

#[test]
fn materialized_summary_outputs_have_the_layouts_dense_fields() {
    let text = "#unrestricted\nPair has\n  a in 0 to 9\n  b in 0 to 9\nf() Pair: return Pair(a: 1, b: 2)\n";
    let bytes = write_artifact(&module(text), &identity()).unwrap();
    let valid = read(&bytes).unwrap();
    let importer =
        parser::parse_strict("use base f\ng() in 0 to 9: return f().b\n").unwrap();
    assert!(check_module("app", &importer, &[&valid], Limits::default()).is_ok());
    for children in [vec![], vec![Node::Uint(1), Node::Uint(1), Node::Uint(2)]] {
        let mut artifact = decode_artifact(&bytes).unwrap();
        let functions = fields(&mut fields(&mut artifact.signature.body)[2]);
        let summaries = fields(&mut fields(&mut functions[0])[9]);
        let outputs = fields(&mut fields(&mut summaries[0])[4]);
        fields(&mut outputs[0])[4] = Node::List(children);
        let forged = encode_artifact(
            artifact.header(),
            &artifact.signature.body,
            &artifact.instantiation.body,
        );
        assert!(matches!(read(&forged), Err(ArtifactError::Shape(_))));
    }
}

#[test]
fn summary_sum_metadata_names_a_nominal_sum() {
    let text = "#unrestricted\nDigit is in 0 to 9\nf(x [Digit; 2]) [Digit; 2]: return x\n";
    let bytes = write_artifact(&module(text), &identity()).unwrap();
    let mut artifact = decode_artifact(&bytes).unwrap();
    let signature = fields(&mut artifact.signature.body);
    let table = fields(&mut signature[0]);
    let array = table.iter().position(|instance| {
        matches!(instance, Node::Record(_, fields) if matches!(&fields[0], Node::Record(41, _)))
    }).unwrap();
    let functions = fields(&mut signature[2]);
    let summaries = fields(&mut fields(&mut functions[0])[9]);
    let outputs = fields(&mut fields(&mut summaries[0])[4]);
    fields(&mut outputs[0])[6] = Node::Uint(array as u64);
    let forged = encode_artifact(
        artifact.header(),
        &artifact.signature.body,
        &artifact.instantiation.body,
    );
    assert!(matches!(read(&forged), Err(ArtifactError::Shape(_))));
}
