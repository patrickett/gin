//! Canonical binary encoding for signature and instantiation artifacts.
//!
//! Payload record tags are chosen by the writer, never by Rust `derive` order. This module
//! owns framing and build identity; semantic capsule validation belongs to the reader's caller.
//! Each tree has exactly one accepted byte form, so its bytes can be hashed:
//!
//! ```text
//! artifact = MAGIC  schema_version:u32le  cbor([layer(sig), layer(inst)])
//! layer    = [payload:bytes, sha256(MAGIC || schema_version:u32le || payload):bytes32]
//! payload  = cbor([kind:uint, header:node, body:node])
//! node     = [0] | [1] | [2]            absent | false | true
//!          | [3, uint]                 unsigned
//!          | [4, bytes32]              signed 256-bit, big-endian two's complement
//!          | [5, text] | [6, bytes]
//!          | [7, [node...]]            list
//!          | [8, tag:uint, [node...]]  record
//! ```
//!
//! All arrays and strings have definite lengths; integers use preferred CBOR encodings.
//! Explicit node/record tags and field positions are independent of Rust declaration order.
//! A decoder accepts only bytes that re-encode to themselves, so two
//! equal trees always have equal bytes and equal hashes, and no other bytes decode at all.
//! Nothing in the format is a span, a path, a pointer or debug text.
use i256::I256;
use minicbor::{Decoder, Encoder};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// The first bytes of every artifact. Distinct from the old `GINIFACE`.
pub const MAGIC: &[u8; 6] = b"GINART";
/// The format version. A reader refuses any other.
pub const SCHEMA_VERSION: u32 = 2;
/// The deepest nesting a decoder follows.
pub const DEPTH_MAX: u32 = 128;
/// The largest artifact a decoder reads.
pub const BYTES_MAX: usize = 1 << 30;
/// Node slots reserved by a decoded tree, including every collection's declared children.
pub const NODES_MAX: usize = 1 << 20;

const NODE_ABSENT: u8 = 0x00;
const NODE_FALSE: u8 = 0x01;
const NODE_TRUE: u8 = 0x02;
const NODE_UINT: u8 = 0x03;
const NODE_INT: u8 = 0x04;
const NODE_TEXT: u8 = 0x05;
const NODE_BYTES: u8 = 0x06;
const NODE_LIST: u8 = 0x07;
const NODE_RECORD: u8 = 0x08;

const HEADER_TAG: u64 = 1;

/// A value in the artifact tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Absent,
    Bool(bool),
    Uint(u64),
    Int(I256),
    Text(String),
    Bytes(Vec<u8>),
    List(Vec<Node>),
    /// A constructor with a stable tag chosen by the writer, and its fields in declared order.
    Record(u64, Vec<Node>),
}

impl Node {
    pub fn record(tag: u64, fields: Vec<Node>) -> Self {
        Self::Record(tag, fields)
    }

    pub fn text(text: &str) -> Self {
        Self::Text(text.to_owned())
    }

    pub fn option<T>(value: Option<T>, f: impl FnOnce(T) -> Node) -> Self {
        value.map_or(Node::Absent, f)
    }
}

/// Which of the two layers a byte string is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// What a dependent's checker may rely on: the semver surface.
    Signature,
    /// What a dependent needs to lower or evaluate, and nothing it may call.
    Instantiation,
}

impl Layer {
    fn tag(self) -> u64 {
        match self {
            Self::Signature => 1,
            Self::Instantiation => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Signature => "sig",
            Self::Instantiation => "inst",
        }
    }
}

/// `(package name, package version, package source digest, module path)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId {
    pub package: String,
    pub version: String,
    /// SHA-256 of the package archive, or of its sorted source files.
    pub source: [u8; 32],
    /// The module's path, as the checker names it (`lib.ops`).
    pub module: String,
}

impl ModuleId {
    pub fn to_node(&self) -> Node {
        Node::record(
            HEADER_TAG + 1,
            vec![
                Node::text(&self.package),
                Node::text(&self.version),
                Node::Bytes(self.source.to_vec()),
                Node::text(&self.module),
            ],
        )
    }

    pub fn from_node(node: &Node) -> Result<Self, CodecError> {
        let fields = record(node, HEADER_TAG + 1, 4)?;
        let source = match &fields[2] {
            Node::Bytes(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| CodecError::Shape("module source digest"))?,
            _ => return Err(CodecError::Shape("module source digest")),
        };
        Ok(Self {
            package: text(&fields[0], "module package")?.to_owned(),
            version: text(&fields[1], "module version")?.to_owned(),
            source,
            module: text(&fields[3], "module path")?.to_owned(),
        })
    }
}

/// What a layer was built from and for. Both layers of one artifact carry the same header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub module: ModuleId,
    /// The target triple: `#target` facts are check-time facts, so each layer is per target.
    pub target: String,
    /// The compiler that built the artifact. An artifact is a cache of that compiler's work.
    pub compiler: String,
}

impl Header {
    fn to_node(&self) -> Node {
        Node::record(
            HEADER_TAG,
            vec![
                self.module.to_node(),
                Node::text(&self.target),
                Node::text(&self.compiler),
            ],
        )
    }

    fn from_node(node: &Node) -> Result<Self, CodecError> {
        let fields = record(node, HEADER_TAG, 3)?;
        Ok(Self {
            module: ModuleId::from_node(&fields[0])?,
            target: text(&fields[1], "header target")?.to_owned(),
            compiler: text(&fields[2], "header compiler")?.to_owned(),
        })
    }
}

/// One decoded layer and the hash of its canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerData {
    pub header: Header,
    pub body: Node,
    pub hash: [u8; 32],
}

/// A decoded artifact whose magic, version, canonical form and hashes were all checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub signature: LayerData,
    pub instantiation: LayerData,
}

/// What the local build expects an artifact to be: the digest of the source it has, the
/// compiler it runs and the target it checks for. An artifact made from anything else is not
/// a cache of this build's work, and is refused. Matching these fields does not authenticate
/// the producer or prove its semantic claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expectation {
    pub module: ModuleId,
    pub target: String,
    pub compiler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("not an interface artifact (bad magic)")]
    Magic,
    #[error("unsupported artifact schema version {0}")]
    Version(u32),
    #[error("artifact ends early")]
    Truncated,
    #[error("artifact has bytes after its end")]
    Trailing,
    #[error("artifact is larger than the reader accepts")]
    TooLarge,
    #[error("artifact nests deeper than the reader follows")]
    TooDeep,
    #[error("unknown node tag {0:#04x}")]
    NodeTag(u8),
    #[error("integer does not fit")]
    Overflow,
    #[error("non-canonical encoding")]
    NonCanonical,
    #[error("text is not UTF-8")]
    Utf8,
    #[error("invalid CBOR: {0}")]
    Cbor(String),
    #[error("the {0} layer does not match its hash")]
    Hash(&'static str),
    #[error("layer kind {0} where another was expected")]
    LayerKind(u64),
    #[error("the two layers have different headers")]
    HeaderMismatch,
    #[error("malformed {0}")]
    Shape(&'static str),
    #[error("artifact was not built from this source, compiler and target: {0} differs")]
    Provenance(&'static str),
}

/// Canonical bytes of one node.
/// Writers supply at most [`NODES_MAX`] nodes; traversal does not recurse.
pub fn encode_node(out: &mut Vec<u8>, node: &Node) {
    let mut encoder = Encoder::new(out);
    let mut pending = vec![std::slice::from_ref(node)];
    for count in 0..=NODES_MAX {
        let Some(nodes) = pending.pop() else { return };
        let (node, rest) = nodes.split_first().expect("pending slices are nonempty");
        assert!(count < NODES_MAX, "writer node limit exceeded");
        if !rest.is_empty() {
            pending.push(rest);
        }
        if let Some(children) = encode_head(&mut encoder, node).expect("Vec encoding is infallible")
            && !children.is_empty()
        {
            pending.push(children);
        }
    }
    unreachable!("a writer either finishes or exceeds its node limit");
}

type EncodeError = minicbor::encode::Error<std::convert::Infallible>;

fn encode_head<'a>(
    encoder: &mut Encoder<&mut Vec<u8>>,
    node: &'a Node,
) -> Result<Option<&'a [Node]>, EncodeError> {
    match node {
        Node::Absent => {
            encoder.array(1)?.u8(NODE_ABSENT)?;
        }
        Node::Bool(false) => {
            encoder.array(1)?.u8(NODE_FALSE)?;
        }
        Node::Bool(true) => {
            encoder.array(1)?.u8(NODE_TRUE)?;
        }
        Node::Uint(value) => {
            encoder.array(2)?.u8(NODE_UINT)?.u64(*value)?;
        }
        Node::Int(value) => {
            encoder
                .array(2)?
                .u8(NODE_INT)?
                .bytes(&value.to_be_bytes())?;
        }
        Node::Text(text) => {
            encoder.array(2)?.u8(NODE_TEXT)?.str(text)?;
        }
        Node::Bytes(bytes) => {
            encoder.array(2)?.u8(NODE_BYTES)?.bytes(bytes)?;
        }
        Node::List(items) => {
            encoder.array(2)?.u8(NODE_LIST)?.array(items.len() as u64)?;
            return Ok(Some(items));
        }
        Node::Record(tag, fields) => {
            encoder
                .array(3)?
                .u8(NODE_RECORD)?
                .u64(*tag)?
                .array(fields.len() as u64)?;
            return Ok(Some(fields));
        }
    }
    Ok(None)
}

fn layer_bytes(layer: Layer, header: &Header, body: &Node) -> Vec<u8> {
    let mut out = Vec::new();
    Encoder::new(&mut out)
        .array(3)
        .and_then(|e| e.u64(layer.tag()))
        .expect("Vec encoding is infallible");
    encode_node(&mut out, &header.to_node());
    encode_node(&mut out, body);
    out
}

fn layer_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(MAGIC);
    hasher.update(SCHEMA_VERSION.to_le_bytes());
    hasher.update(bytes);
    hasher.finalize().into()
}

/// The hash a layer with this header and body has.
pub fn hash_layer(layer: Layer, header: &Header, body: &Node) -> [u8; 32] {
    layer_hash(&layer_bytes(layer, header, body))
}

/// The canonical bytes of an artifact with these two layer bodies.
pub fn encode_artifact(header: &Header, signature: &Node, instantiation: &Node) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    Encoder::new(&mut out)
        .array(2)
        .expect("Vec encoding is infallible");
    for (layer, body) in [
        (Layer::Signature, signature),
        (Layer::Instantiation, instantiation),
    ] {
        let bytes = layer_bytes(layer, header, body);
        Encoder::new(&mut out)
            .array(2)
            .and_then(|e| e.bytes(&bytes))
            .and_then(|e| e.bytes(&layer_hash(&bytes)))
            .expect("Vec encoding is infallible");
    }
    out
}

struct Reader<'a> {
    decoder: Decoder<'a>,
    nodes_left: usize,
}

enum NodeHead {
    Value(Node),
    Children { tag: Option<u64>, count: usize },
}

struct NodeFrame {
    tag: Option<u64>,
    remaining: usize,
    children: Vec<Node>,
}

impl NodeFrame {
    fn finish(self) -> Node {
        assert_eq!(self.remaining, 0);
        match self.tag {
            Some(tag) => Node::Record(tag, self.children),
            None => Node::List(self.children),
        }
    }
}

fn cbor_error(error: minicbor::decode::Error) -> CodecError {
    if error.is_end_of_input() {
        CodecError::Truncated
    } else if std::error::Error::source(&error)
        .is_some_and(|source| source.is::<std::str::Utf8Error>())
    {
        CodecError::Utf8
    } else {
        CodecError::Cbor(error.to_string())
    }
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            decoder: Decoder::new(bytes),
            nodes_left: NODES_MAX,
        }
    }

    fn remaining(&self) -> usize {
        assert!(self.decoder.position() <= self.decoder.input().len());
        self.decoder.input().len() - self.decoder.position()
    }

    fn array(&mut self) -> Result<usize, CodecError> {
        let count = self
            .decoder
            .array()
            .map_err(cbor_error)?
            .ok_or(CodecError::NonCanonical)?;
        usize::try_from(count).map_err(|_| CodecError::Overflow)
    }

    fn require_array(&mut self, fields: usize) -> Result<(), CodecError> {
        if self.array()? == fields {
            Ok(())
        } else {
            Err(CodecError::Shape("CBOR array arity"))
        }
    }

    /// Reserve all declared slots before allocating. Nested frames share this budget, so
    /// each ancestor cannot reserve the full budget independently.
    fn child_count(&mut self) -> Result<usize, CodecError> {
        let count = self.array()?;
        if count > self.remaining() / 2 {
            return Err(CodecError::Truncated);
        }
        self.nodes_left = self
            .nodes_left
            .checked_sub(count)
            .ok_or(CodecError::TooLarge)?;
        Ok(count)
    }

    fn head(&mut self) -> Result<NodeHead, CodecError> {
        let fields = self.array()?;
        if !(1..=3).contains(&fields) {
            return Err(CodecError::Shape("CBOR node arity"));
        }
        let kind = self.decoder.u64().map_err(cbor_error)?;
        let kind = u8::try_from(kind).map_err(|_| CodecError::Overflow)?;
        let expected = match kind {
            NODE_ABSENT | NODE_FALSE | NODE_TRUE => 1,
            NODE_UINT | NODE_INT | NODE_TEXT | NODE_BYTES | NODE_LIST => 2,
            NODE_RECORD => 3,
            other => return Err(CodecError::NodeTag(other)),
        };
        if fields != expected {
            return Err(CodecError::Shape("CBOR node arity"));
        }
        let value = match kind {
            NODE_ABSENT => Node::Absent,
            NODE_FALSE => Node::Bool(false),
            NODE_TRUE => Node::Bool(true),
            NODE_UINT => Node::Uint(self.decoder.u64().map_err(cbor_error)?),
            NODE_INT => {
                let bytes = self.decoder.bytes().map_err(cbor_error)?;
                let bytes = bytes
                    .try_into()
                    .map_err(|_| CodecError::Shape("256-bit integer"))?;
                Node::Int(I256::from_be_bytes(bytes))
            }
            NODE_TEXT => Node::Text(self.decoder.str().map_err(cbor_error)?.to_owned()),
            NODE_BYTES => Node::Bytes(self.decoder.bytes().map_err(cbor_error)?.to_vec()),
            NODE_LIST | NODE_RECORD => {
                let tag = if kind == NODE_RECORD {
                    Some(self.decoder.u64().map_err(cbor_error)?)
                } else {
                    None
                };
                return Ok(NodeHead::Children {
                    tag,
                    count: self.child_count()?,
                });
            }
            _ => unreachable!("node kind was validated"),
        };
        Ok(NodeHead::Value(value))
    }

    fn node(&mut self) -> Result<Node, CodecError> {
        self.nodes_left = NODES_MAX - 1;
        let mut pending: Vec<NodeFrame> = Vec::new();
        for _ in 0..NODES_MAX {
            let mut value = match self.head()? {
                NodeHead::Value(value) => value,
                NodeHead::Children { tag, count } => {
                    if count > 0 && pending.len() >= DEPTH_MAX as usize {
                        return Err(CodecError::TooDeep);
                    }
                    let frame = NodeFrame {
                        tag,
                        remaining: count,
                        children: Vec::with_capacity(count),
                    };
                    if count == 0 {
                        frame.finish()
                    } else {
                        pending.push(frame);
                        continue;
                    }
                }
            };
            for _ in 0..=DEPTH_MAX {
                let Some(frame) = pending.last_mut() else {
                    return Ok(value);
                };
                assert!(frame.remaining > 0);
                frame.children.push(value);
                frame.remaining -= 1;
                if frame.remaining != 0 {
                    break;
                }
                value = pending.pop().expect("a completed frame exists").finish();
            }
        }
        Err(CodecError::TooLarge)
    }
}

/// Decode one node that must fill `bytes` exactly and be in canonical form.
pub fn decode_node(bytes: &[u8]) -> Result<Node, CodecError> {
    if bytes.len() > BYTES_MAX {
        return Err(CodecError::TooLarge);
    }
    let mut reader = Reader::new(bytes);
    let node = reader.node()?;
    if reader.remaining() != 0 {
        return Err(CodecError::Trailing);
    }
    let mut again = Vec::new();
    encode_node(&mut again, &node);
    if again != bytes {
        return Err(CodecError::NonCanonical);
    }
    Ok(node)
}

fn decode_layer(reader: &mut Reader<'_>, layer: Layer) -> Result<LayerData, CodecError> {
    reader.require_array(2)?;
    let bytes = reader.decoder.bytes().map_err(cbor_error)?;
    let hash: [u8; 32] = reader
        .decoder
        .bytes()
        .map_err(cbor_error)?
        .try_into()
        .map_err(|_| CodecError::Shape("layer hash"))?;
    // The hash is checked first: corrupt bytes are reported as corrupt, not as malformed.
    if layer_hash(bytes) != hash {
        return Err(CodecError::Hash(layer.name()));
    }
    let mut inner = Reader::new(bytes);
    inner.require_array(3)?;
    let kind = inner.decoder.u64().map_err(cbor_error)?;
    if kind != layer.tag() {
        return Err(CodecError::LayerKind(kind));
    }
    let header = Header::from_node(&inner.node()?)?;
    let body = inner.node()?;
    if inner.remaining() != 0 {
        return Err(CodecError::Trailing);
    }
    if layer_bytes(layer, &header, &body) != bytes {
        return Err(CodecError::NonCanonical);
    }
    Ok(LayerData { header, body, hash })
}

/// Decode and verify an artifact: magic, version, both hashes, canonical form, equal headers.
/// This says the bytes are intact, not that they may be trusted: see [`Artifact::verify`].
pub fn decode_artifact(bytes: &[u8]) -> Result<Artifact, CodecError> {
    if bytes.len() > BYTES_MAX {
        return Err(CodecError::TooLarge);
    }
    let magic = bytes.get(..MAGIC.len()).ok_or(CodecError::Magic)?;
    if magic != MAGIC {
        return Err(CodecError::Magic);
    }
    let version = bytes
        .get(MAGIC.len()..MAGIC.len() + 4)
        .ok_or(CodecError::Truncated)?;
    let version = u32::from_le_bytes(version.try_into().expect("version has four bytes"));
    if version != SCHEMA_VERSION {
        return Err(CodecError::Version(version));
    }
    let mut reader = Reader::new(&bytes[MAGIC.len() + 4..]);
    reader.require_array(2)?;
    let signature = decode_layer(&mut reader, Layer::Signature)?;
    let instantiation = decode_layer(&mut reader, Layer::Instantiation)?;
    if reader.remaining() != 0 {
        return Err(CodecError::Trailing);
    }
    if signature.header != instantiation.header {
        return Err(CodecError::HeaderMismatch);
    }
    if encode_artifact(&signature.header, &signature.body, &instantiation.body) != bytes {
        return Err(CodecError::NonCanonical);
    }
    Ok(Artifact {
        signature,
        instantiation,
    })
}

impl Artifact {
    pub fn header(&self) -> &Header {
        &self.signature.header
    }

    /// Refuse an artifact that is not a cache of this build: another package, version, source
    /// digest, module, target or compiler. This matches self-declared identity, not producer
    /// authentication; callers must establish trust before relying on semantic claims.
    pub fn verify(&self, expected: &Expectation) -> Result<(), CodecError> {
        let header = self.header();
        let checks = [
            (header.module.package == expected.module.package, "package"),
            (header.module.version == expected.module.version, "version"),
            (
                header.module.source == expected.module.source,
                "source digest",
            ),
            (
                header.module.module == expected.module.module,
                "module path",
            ),
            (header.target == expected.target, "target"),
            (header.compiler == expected.compiler, "compiler"),
        ];
        for (same, what) in checks {
            if !same {
                return Err(CodecError::Provenance(what));
            }
        }
        Ok(())
    }
}

/// The fields of `node` when it is a record with this tag and arity.
pub fn record(node: &Node, tag: u64, arity: usize) -> Result<&[Node], CodecError> {
    match node {
        Node::Record(found, fields) if *found == tag && fields.len() == arity => Ok(fields),
        _ => Err(CodecError::Shape("record")),
    }
}

pub fn text<'a>(node: &'a Node, what: &'static str) -> Result<&'a str, CodecError> {
    match node {
        Node::Text(text) => Ok(text),
        _ => Err(CodecError::Shape(what)),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A deterministic, line-per-node text form of a decoded artifact, for review and diffs.
/// `names` gives a record tag a readable name (in the context of its layer); unnamed tags print
/// as numbers. The text is derived from the decoded value, so equal bytes give equal text.
pub fn dump(artifact: &Artifact, names: &dyn Fn(Layer, u64) -> Option<&'static str>) -> String {
    let mut out = String::new();
    let header = artifact.header();
    let _ = writeln!(out, "gin-artifact schema {SCHEMA_VERSION}");
    let _ = writeln!(
        out,
        "module {} {} {} source {}",
        quote(&header.module.package),
        quote(&header.module.version),
        quote(&header.module.module),
        hex(&header.module.source)
    );
    let _ = writeln!(out, "target {}", quote(&header.target));
    let _ = writeln!(out, "compiler {}", quote(&header.compiler));
    for (layer, data) in [
        (Layer::Signature, &artifact.signature),
        (Layer::Instantiation, &artifact.instantiation),
    ] {
        let _ = writeln!(out, "layer {} sha256 {}", layer.name(), hex(&data.hash));
        dump_node(&mut out, &data.body, 1, &|tag| names(layer, tag));
    }
    out
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn dump_node(
    out: &mut String,
    node: &Node,
    depth: usize,
    names: &dyn Fn(u64) -> Option<&'static str>,
) {
    let pad = "  ".repeat(depth);
    match node {
        Node::Absent => {
            let _ = writeln!(out, "{pad}-");
        }
        Node::Bool(value) => {
            let _ = writeln!(out, "{pad}{value}");
        }
        Node::Uint(value) => {
            let _ = writeln!(out, "{pad}{value}");
        }
        Node::Int(value) => {
            let _ = writeln!(out, "{pad}i{value}");
        }
        Node::Text(text) => {
            let _ = writeln!(out, "{pad}{}", quote(text));
        }
        Node::Bytes(bytes) => {
            let _ = writeln!(out, "{pad}0x{}", hex(bytes));
        }
        Node::List(items) if items.is_empty() => {
            let _ = writeln!(out, "{pad}[]");
        }
        Node::List(items) => {
            let _ = writeln!(out, "{pad}[{}]", items.len());
            for item in items {
                dump_node(out, item, depth + 1, names);
            }
        }
        Node::Record(tag, fields) => {
            match names(*tag) {
                Some(name) => {
                    let _ = writeln!(out, "{pad}{name}");
                }
                None => {
                    let _ = writeln!(out, "{pad}#{tag}");
                }
            }
            for field in fields {
                dump_node(out, field, depth + 1, names);
            }
        }
    }
}
