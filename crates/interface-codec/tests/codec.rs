use i256::I256;
use interface_codec::*;

fn header() -> Header {
    Header {
        module: ModuleId {
            package: "pkg".into(),
            version: "1.0.0".into(),
            source: [7; 32],
            module: "lib".into(),
        },
        target: "aarch64-apple-darwin".into(),
        compiler: "ginc 0".into(),
    }
}

fn body() -> Node {
    Node::record(
        3,
        vec![
            Node::Absent,
            Node::Bool(true),
            Node::Uint(300),
            Node::Int(I256::from_i128(-5)),
            Node::text("x\"y"),
            Node::Bytes(vec![1, 2]),
            Node::List(vec![Node::Uint(u64::MAX), Node::List(Vec::new())]),
        ],
    )
}

fn artifact() -> Vec<u8> {
    encode_artifact(&header(), &body(), &Node::List(Vec::new()))
}

#[test]
fn round_trip_and_deterministic() {
    let bytes = artifact();
    assert_eq!(bytes, artifact());
    let decoded = decode_artifact(&bytes).unwrap();
    assert_eq!(decoded.signature.body, body());
    assert_eq!(decoded.header(), &header());
    assert_eq!(
        decoded.signature.hash,
        hash_layer(Layer::Signature, &header(), &body())
    );
    let text = dump(&decoded, &|_, tag| (tag == 3).then_some("thing"));
    assert_eq!(
        text,
        dump(&decode_artifact(&artifact()).unwrap(), &|_, tag| (tag == 3)
            .then_some("thing"))
    );
    assert!(text.contains("thing") && text.contains("i-5") && text.contains("\"x\\\"y\""));
}

#[test]
fn node_bytes_are_standard_cbor_with_explicit_tags() {
    let node = Node::record(42, vec![Node::Uint(1)]);
    let mut bytes = Vec::new();
    encode_node(&mut bytes, &node);
    assert_eq!(bytes, [0x83, 0x08, 0x18, 0x2a, 0x81, 0x82, 0x03, 0x01]);
    let mut decoder = minicbor::Decoder::new(&bytes);
    assert_eq!(decoder.array().unwrap(), Some(3));
    assert_eq!(decoder.u8().unwrap(), 8);
    assert_eq!(decoder.u64().unwrap(), 42);
    assert_eq!(decoder.array().unwrap(), Some(1));
    assert_eq!(decoder.array().unwrap(), Some(2));
    assert_eq!(decoder.u8().unwrap(), 3);
    assert_eq!(decoder.u64().unwrap(), 1);
    assert_eq!(decoder.position(), bytes.len());
}

#[test]
fn every_node_kind_and_full_width_integer_round_trips() {
    for node in [
        Node::Absent,
        Node::Bool(false),
        Node::Bool(true),
        Node::Uint(0),
        Node::Uint(23),
        Node::Uint(24),
        Node::Uint(u64::MAX),
        Node::Int(I256::MIN),
        Node::Int(I256::MAX),
        Node::Int(I256::from(-1)),
        Node::Int(I256::from(0)),
        Node::Int(I256::from(1)),
        Node::text("hello 💣\0"),
        Node::Text(String::new()),
        Node::Bytes((0..=255).collect()),
        Node::List(Vec::new()),
        Node::record(u64::MAX, vec![body()]),
    ] {
        let mut bytes = Vec::new();
        encode_node(&mut bytes, &node);
        assert_eq!(decode_node(&bytes), Ok(node));
    }
    let mut integer = Vec::new();
    let mut unsigned = Vec::new();
    encode_node(&mut integer, &Node::Int(I256::from(1)));
    encode_node(&mut unsigned, &Node::Uint(1));
    assert_ne!(integer, unsigned);
}

fn frame<'a>(decoder: &mut minicbor::Decoder<'a>) -> (&'a [u8], &'a [u8]) {
    assert_eq!(decoder.array().unwrap(), Some(2));
    (decoder.bytes().unwrap(), decoder.bytes().unwrap())
}

fn frames(bytes: &[u8]) -> [(&[u8], &[u8]); 2] {
    let mut decoder = minicbor::Decoder::new(&bytes[MAGIC.len() + 4..]);
    assert_eq!(decoder.array().unwrap(), Some(2));
    let frames = [frame(&mut decoder), frame(&mut decoder)];
    assert_eq!(decoder.position(), bytes.len() - MAGIC.len() - 4);
    frames
}

fn with_frames(frames: [(&[u8], &[u8]); 2]) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    let mut encoder = minicbor::Encoder::new(&mut bytes);
    encoder.array(2).unwrap();
    for (payload, hash) in frames {
        encoder
            .array(2)
            .unwrap()
            .bytes(payload)
            .unwrap()
            .bytes(hash)
            .unwrap();
    }
    bytes
}

#[test]
fn artifact_frames_and_payloads_are_definite_cbor() {
    let bytes = artifact();
    for (index, (payload, hash)) in frames(&bytes).into_iter().enumerate() {
        assert_eq!(hash.len(), 32);
        let mut decoder = minicbor::Decoder::new(payload);
        assert_eq!(decoder.array().unwrap(), Some(3));
        assert_eq!(decoder.u64().unwrap(), index as u64 + 1);
        decoder.skip().unwrap();
        decoder.skip().unwrap();
        assert_eq!(decoder.position(), payload.len());
    }
}

#[test]
fn layer_kinds_headers_and_outer_canonical_encoding_are_checked() {
    let bytes = artifact();
    let layers = frames(&bytes);
    assert_eq!(
        decode_artifact(&with_frames([layers[1], layers[0]])),
        Err(CodecError::LayerKind(2)),
    );
    let mut other_header = header();
    other_header.target = "other-target".into();
    let other = encode_artifact(&other_header, &body(), &Node::List(Vec::new()));
    assert_eq!(
        decode_artifact(&with_frames([layers[0], frames(&other)[1]])),
        Err(CodecError::HeaderMismatch),
    );
    let mut noncanonical = bytes.clone();
    // Spell the outer array's length with a redundant additional byte.
    noncanonical.splice(MAGIC.len() + 4..MAGIC.len() + 5, [0x98, 0x02]);
    assert_eq!(
        decode_artifact(&noncanonical),
        Err(CodecError::NonCanonical)
    );
    assert_ne!(
        hash_layer(Layer::Signature, &header(), &body()),
        hash_layer(Layer::Instantiation, &header(), &body()),
    );
}

#[test]
fn malformed_arities_integers_and_indefinite_items_are_refused() {
    for bytes in [
        vec![0x80],
        vec![0x82, 0x00, 0x00],
        vec![0x81, 0x03],
        vec![0x82, 0x03, 0x20],
        vec![0x82, 0x06, 0x5f, 0x41, 0x01, 0xff],
        vec![0x82, 0x07, 0x9f, 0x81, 0x00, 0xff],
        vec![0x82, 0x05, 0x7f, 0x61, b'a', 0xff],
    ] {
        assert!(decode_node(&bytes).is_err(), "{bytes:?}");
    }
    for length in [0, 31, 33] {
        let mut bytes = Vec::new();
        minicbor::Encoder::new(&mut bytes)
            .array(2)
            .unwrap()
            .u8(4)
            .unwrap()
            .bytes(&vec![0; length])
            .unwrap();
        assert_eq!(
            decode_node(&bytes),
            Err(CodecError::Shape("256-bit integer"))
        );
    }
}

#[test]
fn depth_limit_accepts_the_last_valid_level() {
    let mut bytes = [0x82, 0x07, 0x81].repeat(DEPTH_MAX as usize);
    bytes.extend_from_slice(&[0x81, 0x00]);
    assert!(decode_node(&bytes).is_ok());
    bytes.splice(0..0, [0x82, 0x07, 0x81]);
    assert_eq!(decode_node(&bytes), Err(CodecError::TooDeep));
}

#[test]
fn declared_counts_are_checked_before_allocating_child_vectors() {
    let mut bytes = Vec::new();
    minicbor::Encoder::new(&mut bytes)
        .array(2)
        .unwrap()
        .u8(7)
        .unwrap()
        .array(u64::MAX)
        .unwrap();
    assert_eq!(decode_node(&bytes), Err(CodecError::Truncated));
    let mut bytes = Vec::new();
    minicbor::Encoder::new(&mut bytes)
        .array(2)
        .unwrap()
        .u8(7)
        .unwrap()
        .array(NODES_MAX as u64)
        .unwrap();
    bytes.extend([0x81, 0x00].repeat(NODES_MAX));
    assert_eq!(decode_node(&bytes), Err(CodecError::TooLarge));
}

#[test]
fn nested_collections_share_one_node_slot_budget() {
    let count = NODES_MAX / 2;
    let mut bytes = Vec::new();
    minicbor::Encoder::new(&mut bytes)
        .array(2)
        .unwrap()
        .u8(7)
        .unwrap()
        .array(count as u64)
        .unwrap()
        .array(2)
        .unwrap()
        .u8(7)
        .unwrap()
        .array(count as u64)
        .unwrap();
    bytes.extend([0x81, 0x00].repeat(count * 2 - 1));
    assert_eq!(decode_node(&bytes), Err(CodecError::TooLarge));
}

#[test]
fn the_layers_hash_independently() {
    let a = decode_artifact(&encode_artifact(&header(), &body(), &Node::Uint(1))).unwrap();
    let b = decode_artifact(&encode_artifact(&header(), &body(), &Node::Uint(2))).unwrap();
    assert_eq!(a.signature.hash, b.signature.hash);
    assert_ne!(a.instantiation.hash, b.instantiation.hash);
}

#[test]
fn magic_and_version_are_checked() {
    let mut bytes = artifact();
    bytes[0] = b'X';
    assert_eq!(decode_artifact(&bytes), Err(CodecError::Magic));
    let mut bytes = artifact();
    bytes[6] = 1;
    assert_eq!(decode_artifact(&bytes), Err(CodecError::Version(1)));
    assert_eq!(decode_artifact(b"GIN"), Err(CodecError::Magic));
    assert_eq!(decode_artifact(b""), Err(CodecError::Magic));
}

#[test]
fn every_truncation_and_every_flipped_byte_is_refused_without_panic() {
    let bytes = artifact();
    for length in 0..bytes.len() {
        assert!(
            decode_artifact(&bytes[..length]).is_err(),
            "prefix {length}"
        );
    }
    for at in 0..bytes.len() {
        for bit in 0..8 {
            let mut changed = bytes.clone();
            changed[at] ^= 1 << bit;
            assert!(decode_artifact(&changed).is_err(), "byte {at} bit {bit}");
        }
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(decode_artifact(&longer), Err(CodecError::Trailing));
}

#[test]
fn a_corrupted_layer_is_a_hash_error() {
    let mut bytes = artifact();
    // A byte inside the signature payload, not its surrounding CBOR framing.
    let at = 30;
    bytes[at] ^= 0x10;
    assert_eq!(decode_artifact(&bytes), Err(CodecError::Hash("sig")));
}

#[test]
fn non_canonical_nodes_are_refused() {
    // A CBOR uint spelled with a longer-than-preferred representation.
    assert_eq!(
        decode_node(&[0x82, 0x03, 0x18, 0x01]),
        Err(CodecError::NonCanonical)
    );
    assert_eq!(decode_node(&[0x82, 0x03, 0x01]), Ok(Node::Uint(1)));
    assert_eq!(decode_node(&[0x81, 0x09]), Err(CodecError::NodeTag(9)));
    assert_eq!(
        decode_node(&[0x82, 0x05, 0x61, 0xff]),
        Err(CodecError::Utf8)
    );
    assert_eq!(
        decode_node(&[0x82, 0x07, 0x98, 0x7f]),
        Err(CodecError::Truncated)
    );
    let mut deep = [0x82, 0x07, 0x81].repeat(200);
    deep.extend_from_slice(&[0x81, 0x00]);
    assert_eq!(decode_node(&deep), Err(CodecError::TooDeep));
    assert_eq!(
        decode_node(&[0x98, 0x01, 0x00]),
        Err(CodecError::NonCanonical)
    );
    assert_eq!(
        decode_node(&[0x82, 0x05, 0x78, 0x01, b'a']),
        Err(CodecError::NonCanonical)
    );
    assert_eq!(
        decode_node(&[0x9f, 0x00, 0xff]),
        Err(CodecError::NonCanonical)
    );
}

#[test]
fn oversized_byte_and_text_lengths_do_not_overflow_cursor_arithmetic() {
    for (kind, header) in [(6, 0x5b), (5, 0x7b)] {
        let mut bytes = vec![0x82, kind, header];
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(decode_node(&bytes), Err(CodecError::Truncated));
    }
}

#[test]
fn provenance_is_refused_when_it_differs() {
    let decoded = decode_artifact(&artifact()).unwrap();
    let expected = Expectation {
        module: header().module,
        target: header().target,
        compiler: header().compiler,
    };
    assert_eq!(decoded.verify(&expected), Ok(()));
    let mut other = expected.clone();
    other.module.source = [8; 32];
    assert_eq!(
        decoded.verify(&other),
        Err(CodecError::Provenance("source digest"))
    );
    let mut other = expected.clone();
    other.compiler = "ginc 1".into();
    assert_eq!(
        decoded.verify(&other),
        Err(CodecError::Provenance("compiler"))
    );
    let mut other = expected;
    other.target = "x86_64-unknown-linux-gnu".into();
    assert_eq!(
        decoded.verify(&other),
        Err(CodecError::Provenance("target"))
    );
}
