//! The interface artifact of a checked module: a writer
//! from a [`CheckedSourceModule`] and a reader that turns the bytes back into a capsule a
//! dependent can be checked against.
//!
//! Two layers, each with its own hash. The signature layer (`sig`) holds what a dependent's
//! checker reads about the items reachable from the exports: types (their public fields,
//! variants, capability, domain and claims), exported functions (signature, conventions,
//! claims, transitions, ownership summaries), constants, aliases, traits and accessors. The
//! instantiation layer (`inst`) holds the private fields of those types and the layouts only
//! they reach, plus recorded and template bodies for evaluation and lowering.
//!
//! Hashes detect corruption, not provenance or semantic correctness. The reader validates
//! structural invariants; callers must obtain artifacts from a trusted compiler/producer.
//!
//! No table index of the checked module survives: a layout is named by its home module's
//! [`ModuleId`] and its name, a function by its name, and type applications by their position
//! in a per-layer table built in a fixed traversal from the sorted exports. Items are sorted by
//! those names, so the order of declarations in the source does not change the bytes.
use super::*;
use calls::{Function, Parameter, Passing};
use contracts::{Constraint, Layout, Origin, Term};
use interface_codec::{CodecError, Expectation, Header, ModuleId, Node};
use modules::Export;
use postconditions::{ClauseExpr, Effect, PlacePath};
use std::collections::{BTreeMap, HashMap};
use std::result::Result;
use std::sync::Arc;
use summaries::{Disposition, Input, Output, Summary};

/// Who a module is and what it was built for: the header of both layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactIdentity {
    /// This module. `module.module` must be the capsule's name.
    pub module: ModuleId,
    /// Every module whose types this capsule carries (its dependencies and theirs), matched to
    /// the capsule's home names by `module`.
    pub dependencies: Vec<ModuleId>,
    pub target: String,
    pub compiler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactError {
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("no module identity for home `{0}`")]
    UnknownHome(String),
    #[error("two module identities for home `{0}`")]
    AmbiguousHome(String),
    #[error("the identity names module `{0}`, not this capsule")]
    WrongModule(String),
    #[error("two layouts of one module are both named `{0}`")]
    AmbiguousName(String),
    #[error("malformed artifact: {0}")]
    Shape(&'static str),
    #[error("artifact names an item it does not define")]
    UnknownItem,
}

fn shape(what: &'static str) -> ArtifactError {
    ArtifactError::Shape(what)
}

// Stable record tags. Never reuse or renumber one; add new tags at the end.
const SIG: u64 = 10;
const INST: u64 = 11;
const ITEM: u64 = 12;
const LAYOUT: u64 = 13;
const FIELD: u64 = 14;
const PRIVATE_FIELDS: u64 = 15;
const INSTANCE: u64 = 16;
const FUNCTION: u64 = 17;
const SIGNATURE: u64 = 18;
const EXPORT: u64 = 19;
const CONSTANT: u64 = 20;
const ALIAS: u64 = 21;
const TRAIT: u64 = 22;
const TRAIT_NAME: u64 = 23;
const ACCESSOR: u64 = 24;
const PLAIN_ACCESSOR: u64 = 25;
const CONSTRAINT: u64 = 26;
const SUMMARY: u64 = 27;
const INPUT: u64 = 28;
const OUTPUT: u64 = 29;
const PLACE: u64 = 30;
const VARIANT_CLAIMS: u64 = 31;
const VARIANT_EFFECTS: u64 = 32;
const TRANSITION: u64 = 33;
const PAIR: u64 = 34;
const BOUND_QUERY: u64 = 35;
// Type heads.
const HEAD_NOMINAL: u64 = 40;
const HEAD_ARRAY: u64 = 41;
const HEAD_SLICE: u64 = 42;
const HEAD_SIZED: u64 = 43;
const HEAD_SEQUENCE: u64 = 44;
const HEAD_POINTER: u64 = 45;
// Types.
const TY_UNIT: u64 = 50;
const TY_TEXT: u64 = 51;
const TY_INTEGER: u64 = 52;
const TY_GENERIC: u64 = 53;
const TY_AGGREGATE: u64 = 54;
const TY_SUM: u64 = 55;
const TY_ARRAY: u64 = 56;
const TY_SEQUENCE: u64 = 57;
const TY_POINTER: u64 = 58;
// Claim terms.
const TERM_SLOT: u64 = 60;
const TERM_CONSTANT: u64 = 61;
const TERM_NUMBER: u64 = 62;
const TERM_IDENTITY: u64 = 63;
const TERM_PATH: u64 = 64;
const TERM_ARITHMETIC: u64 = 65;
const TERM_LAYOUT: u64 = 66;
// Transition expressions and effects.
const CLAUSE_CONSTANT: u64 = 70;
const CLAUSE_PLACE: u64 = 71;
const CLAUSE_BINARY: u64 = 72;
const CLAUSE_SELECT: u64 = 73;
const EFFECT_SET: u64 = 74;
const EFFECT_FORGOTTEN: u64 = 75;
// Parameters, discriminants, domain claims.
const PARAMETER_TYPE: u64 = 80;
const PARAMETER_VALUE: u64 = 81;
const PARAMETER_LITERAL: u64 = 82;
const DISCRIMINANT_INTEGER: u64 = 83;
const DISCRIMINANT_TEXT: u64 = 84;
const DOMAIN_COMPARE: u64 = 85;
const DOMAIN_MULTIPLE: u64 = 86;
// Body expression kinds.
const K_RETURN: u64 = 100;
const K_PARAMETER: u64 = 101;
const K_INTEGER: u64 = 102;
const K_TEXT: u64 = 103;
const K_UNIT: u64 = 104;
const K_HOLE: u64 = 105;
const K_BINARY: u64 = 106;
const K_WRAP: u64 = 107;
const K_CONSTRUCT: u64 = 108;
const K_MATCH: u64 = 109;
const K_FIELD: u64 = 110;
const K_VARIANT_FIELD: u64 = 111;
const K_CALL: u64 = 112;
const K_BOUND_CALL: u64 = 113;
const K_SELECT: u64 = 114;
const K_INSTRUCTION: u64 = 115;
const K_LOAD: u64 = 116;
const K_STORE: u64 = 117;
const K_WIDEN: u64 = 118;
const K_LOOP: u64 = 119;
const K_WHILE: u64 = 120;
const K_CALL_OUT: u64 = 121;
const K_SUB_ARRAY: u64 = 122;
const K_REPEAT_ARRAY: u64 = 123;
const K_TEXT_BYTES: u64 = 124;
const K_LAYOUT: u64 = 125;
const K_LOOP_COUNTER: u64 = 126;
const K_LOOP_CARRIED: u64 = 127;
const K_LOOP_RESULT: u64 = 128;
const K_BLOCK: u64 = 129;
const K_BUNDLE: u64 = 130;
const K_PART: u64 = 131;
const K_UNDEFINED: u64 = 132;
// Bodies and the payload around them.
const BODY: u64 = 140;
const EXPR: u64 = 141;
const ARM: u64 = 142;
const MATCH_REGION: u64 = 143;
const SELECT_REGION: u64 = 144;
const CARRY: u64 = 145;
const CALLEE_LOCAL: u64 = 146;
const CALLEE_DEPENDENCY: u64 = 147;
const PAYLOAD: u64 = 148;
const DECLINE: u64 = 149;
const NOTE: u64 = 150;
const FACT_INTERVAL: u64 = 151;
const FACT_KNOWN: u64 = 152;
const FACT_MULTIPLE: u64 = 153;
const FACT_OFFSET: u64 = 154;
const FACT_RELATION: u64 = 155;

/// The readable name of a record tag, for [`dump_artifact`].
pub fn artifact_tag_name(tag: u64) -> Option<&'static str> {
    Some(match tag {
        1 => "header",
        2 => "module",
        SIG => "sig",
        INST => "inst",
        ITEM => "item",
        LAYOUT => "layout",
        FIELD => "field",
        PRIVATE_FIELDS => "private-fields",
        INSTANCE => "instance",
        FUNCTION => "function",
        SIGNATURE => "signature",
        EXPORT => "export",
        CONSTANT => "constant",
        ALIAS => "alias",
        TRAIT => "trait",
        TRAIT_NAME => "trait-name",
        ACCESSOR => "accessor",
        PLAIN_ACCESSOR => "plain-accessor",
        CONSTRAINT => "constraint",
        SUMMARY => "summary",
        INPUT => "input",
        OUTPUT => "output",
        PLACE => "place",
        VARIANT_CLAIMS => "variant-claims",
        VARIANT_EFFECTS => "variant-effects",
        TRANSITION => "transition",
        PAIR => "pair",
        BOUND_QUERY => "bound-query",
        HEAD_NOMINAL => "nominal",
        HEAD_ARRAY => "array",
        HEAD_SLICE => "slice",
        HEAD_SIZED => "sized",
        HEAD_SEQUENCE => "sequence",
        HEAD_POINTER => "pointer",
        TY_UNIT => "unit",
        TY_TEXT => "text",
        TY_INTEGER => "integer",
        TY_GENERIC => "generic",
        TY_AGGREGATE => "aggregate",
        TY_SUM => "sum",
        TY_ARRAY => "array-type",
        TY_SEQUENCE => "sequence-type",
        TY_POINTER => "pointer-type",
        TERM_SLOT => "slot",
        TERM_CONSTANT => "constant-term",
        TERM_NUMBER => "number",
        TERM_IDENTITY => "identity",
        TERM_PATH => "path",
        TERM_ARITHMETIC => "arithmetic",
        TERM_LAYOUT => "layout-term",
        CLAUSE_CONSTANT => "clause-constant",
        CLAUSE_PLACE => "clause-place",
        CLAUSE_BINARY => "clause-binary",
        CLAUSE_SELECT => "clause-select",
        EFFECT_SET => "set",
        EFFECT_FORGOTTEN => "forgotten",
        PARAMETER_TYPE => "type-parameter",
        PARAMETER_VALUE => "value-parameter",
        PARAMETER_LITERAL => "literal-parameter",
        DISCRIMINANT_INTEGER => "discriminant-integer",
        DISCRIMINANT_TEXT => "discriminant-text",
        DOMAIN_COMPARE => "domain-compare",
        DOMAIN_MULTIPLE => "domain-multiple",
        K_RETURN => "return",
        K_PARAMETER => "parameter",
        K_INTEGER => "integer-node",
        K_TEXT => "text-node",
        K_UNIT => "unit-node",
        K_HOLE => "hole",
        K_BINARY => "binary",
        K_WRAP => "wrap",
        K_CONSTRUCT => "construct",
        K_MATCH => "match",
        K_FIELD => "field-node",
        K_VARIANT_FIELD => "variant-field",
        K_CALL => "call",
        K_BOUND_CALL => "bound-call",
        K_SELECT => "select",
        K_INSTRUCTION => "instruction",
        K_LOAD => "load",
        K_STORE => "store",
        K_WIDEN => "widen",
        K_LOOP => "loop",
        K_WHILE => "while",
        K_CALL_OUT => "call-out",
        K_SUB_ARRAY => "sub-array",
        K_REPEAT_ARRAY => "repeat-array",
        K_TEXT_BYTES => "text-bytes",
        K_LAYOUT => "layout-node",
        K_LOOP_COUNTER => "loop-counter",
        K_LOOP_CARRIED => "loop-carried",
        K_LOOP_RESULT => "loop-result",
        K_BLOCK => "block",
        K_BUNDLE => "bundle",
        K_PART => "part",
        K_UNDEFINED => "undefined",
        BODY => "body",
        EXPR => "expr",
        ARM => "arm",
        MATCH_REGION => "match-region",
        SELECT_REGION => "select-region",
        CARRY => "carry",
        CALLEE_LOCAL => "local",
        CALLEE_DEPENDENCY => "dependency",
        PAYLOAD => "payload",
        DECLINE => "decline",
        NOTE => "note",
        FACT_INTERVAL => "fact-interval",
        FACT_KNOWN => "fact-known",
        FACT_MULTIPLE => "fact-multiple",
        FACT_OFFSET => "fact-offset",
        FACT_RELATION => "fact-relation",
        _ => return None,
    })
}

/// Stable numbers of the small enums. Each pair of functions must stay inverse.
macro_rules! stable_enum {
    ($encode:ident, $decode:ident, $ty:ty { $($variant:path = $tag:expr),+ $(,)? }) => {
        fn $encode(value: $ty) -> Node {
            Node::Uint(match value { $($variant => $tag),+ })
        }

        fn $decode(node: &Node) -> Result<$ty, ArtifactError> {
            match uint(node)? {
                $($tag => Ok($variant),)+
                _ => Err(shape(stringify!($ty))),
            }
        }
    };
}

stable_enum!(enc_binary, dec_binary, BinaryOperator {
    BinaryOperator::Add = 1, BinaryOperator::Subtract = 2, BinaryOperator::Multiply = 3,
    BinaryOperator::Divide = 4, BinaryOperator::Remainder = 5, BinaryOperator::Equal = 6,
    BinaryOperator::Less = 7, BinaryOperator::LessOrEqual = 8, BinaryOperator::Greater = 9,
    BinaryOperator::GreaterOrEqual = 10, BinaryOperator::BitAnd = 11, BinaryOperator::BitOr = 12,
    BinaryOperator::BitXor = 13, BinaryOperator::ShiftLeft = 14, BinaryOperator::ShiftRight = 15,
    BinaryOperator::And = 16, BinaryOperator::Or = 17,
});
stable_enum!(enc_arithmetic, dec_arithmetic, CheckedArithmetic {
    CheckedArithmetic::Add = 1, CheckedArithmetic::Subtract = 2, CheckedArithmetic::Multiply = 3,
    CheckedArithmetic::Divide = 4, CheckedArithmetic::Remainder = 5, CheckedArithmetic::BitAnd = 6,
    CheckedArithmetic::BitOr = 7, CheckedArithmetic::BitXor = 8, CheckedArithmetic::ShiftLeft = 9,
    CheckedArithmetic::ShiftRight = 10,
});
stable_enum!(enc_infix, dec_infix, InfixOperator {
    InfixOperator::Add = 1, InfixOperator::Subtract = 2, InfixOperator::Multiply = 3,
    InfixOperator::Divide = 4, InfixOperator::Remainder = 5, InfixOperator::Equal = 6,
    InfixOperator::Less = 7, InfixOperator::LessOrEqual = 8, InfixOperator::Greater = 9,
    InfixOperator::GreaterOrEqual = 10, InfixOperator::BitAnd = 11, InfixOperator::BitOr = 12,
    InfixOperator::BitXor = 13, InfixOperator::ShiftLeft = 14, InfixOperator::ShiftRight = 15,
});
stable_enum!(enc_memory, dec_memory, MemoryOp { MemoryOp::Load = 1, MemoryOp::Store = 2 });
stable_enum!(enc_capability, dec_capability, Capability { Capability::Linear = 1, Capability::Unrestricted = 2 });
stable_enum!(enc_kind, dec_kind, AggregateKind { AggregateKind::Record = 1, AggregateKind::Sum = 2, AggregateKind::Domain = 3 });
stable_enum!(enc_borrowing, dec_borrowing, Borrowing {
    Borrowing::None = 1, Borrowing::Shared = 2, Borrowing::Exclusive = 3, Borrowing::Mixed = 4,
});
stable_enum!(enc_borrow, dec_borrow, BorrowKind { BorrowKind::Shared = 1, BorrowKind::Exclusive = 2 });
stable_enum!(enc_disposition, dec_disposition, Disposition {
    Disposition::Scalar = 1, Disposition::Returned = 2, Disposition::Unpacked = 3, Disposition::Ended = 4,
});
stable_enum!(enc_passing, dec_passing, Passing { Passing::Owner = 1, Passing::End = 2, Passing::Ref = 3, Passing::Mut = 4 });
stable_enum!(enc_decline, dec_decline, DeclineKind {
    DeclineKind::Operator = 1, DeclineKind::Expression = 2, DeclineKind::Statement = 3,
    DeclineKind::Conversion = 4, DeclineKind::Unplaced = 5,
});
stable_enum!(enc_note, dec_note, SourceCheckNoteKind {
    SourceCheckNoteKind::RedundantTerminal = 1, SourceCheckNoteKind::UnreachableCode = 2, SourceCheckNoteKind::Hole = 3,
});

/// The relations a [`SourceFact::Relation`] names, as the static text the checker uses.
fn relation(text: &str) -> Result<&'static str, ArtifactError> {
    ["<", "<=", ">", ">=", "="]
        .into_iter()
        .find(|known| *known == text)
        .ok_or(shape("relation"))
}

fn uint(node: &Node) -> Result<u64, ArtifactError> {
    match node {
        Node::Uint(value) => Ok(*value),
        _ => Err(shape("unsigned")),
    }
}

fn index(node: &Node) -> Result<usize, ArtifactError> {
    usize::try_from(uint(node)?).map_err(|_| shape("index"))
}

fn small(node: &Node) -> Result<u32, ArtifactError> {
    u32::try_from(uint(node)?).map_err(|_| shape("index"))
}

fn int(node: &Node) -> Result<I256, ArtifactError> {
    match node {
        Node::Int(value) => Ok(*value),
        _ => Err(shape("integer")),
    }
}

fn boolean(node: &Node) -> Result<bool, ArtifactError> {
    match node {
        Node::Bool(value) => Ok(*value),
        _ => Err(shape("bool")),
    }
}

fn string(node: &Node) -> Result<String, ArtifactError> {
    match node {
        Node::Text(text) => Ok(text.clone()),
        _ => Err(shape("text")),
    }
}

fn list(node: &Node) -> Result<&[Node], ArtifactError> {
    match node {
        Node::List(items) => Ok(items),
        _ => Err(shape("list")),
    }
}

fn record(node: &Node, tag: u64, arity: usize) -> Result<&[Node], ArtifactError> {
    Ok(interface_codec::record(node, tag, arity)?)
}

fn optional<T>(
    node: &Node,
    f: impl FnOnce(&Node) -> Result<T, ArtifactError>,
) -> Result<Option<T>, ArtifactError> {
    match node {
        Node::Absent => Ok(None),
        other => f(other).map(Some),
    }
}

fn each<T>(
    node: &Node,
    mut f: impl FnMut(&Node) -> Result<T, ArtifactError>,
) -> Result<Vec<T>, ArtifactError> {
    list(node)?.iter().map(&mut f).collect()
}

fn texts(items: &[String]) -> Node {
    Node::List(items.iter().map(|item| Node::text(item)).collect())
}

fn uints<T: Copy + Into<u64>>(items: &[T]) -> Node {
    Node::List(
        items
            .iter()
            .map(|item| Node::Uint((*item).into()))
            .collect(),
    )
}

fn usizes(items: &[usize]) -> Node {
    Node::List(items.iter().map(|item| Node::Uint(*item as u64)).collect())
}

fn pair(left: Node, right: Node) -> Node {
    Node::record(PAIR, vec![left, right])
}

fn unpair(node: &Node) -> Result<(&Node, &Node), ArtifactError> {
    let fields = record(node, PAIR, 2)?;
    Ok((&fields[0], &fields[1]))
}

/// A layout's stable identity: its home module and its name there.
type ItemKey = (ModuleId, String);

fn item_node(key: &ItemKey) -> Node {
    Node::record(ITEM, vec![key.0.to_node(), Node::text(&key.1)])
}

fn item_key(node: &Node) -> Result<ItemKey, ArtifactError> {
    let fields = record(node, ITEM, 2)?;
    Ok((ModuleId::from_node(&fields[0])?, string(&fields[1])?))
}

// ---------------------------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------------------------

/// The types of one layer: type applications are numbered in the order first met.
struct Table<'m> {
    module: &'m CheckedSourceModule,
    keys: &'m [ItemKey],
    instances: Vec<Node>,
    numbers: HashMap<u32, u64>,
    /// Layouts this layer reached, in the order it reached them.
    reached: Vec<usize>,
    seen: Vec<bool>,
}

impl<'m> Table<'m> {
    fn new(module: &'m CheckedSourceModule, keys: &'m [ItemKey]) -> Self {
        Self {
            module,
            keys,
            instances: Vec::new(),
            numbers: HashMap::new(),
            reached: Vec::new(),
            seen: vec![false; module.layouts.len()],
        }
    }

    fn reach(&mut self, layout: usize) {
        if !self.seen[layout] {
            self.seen[layout] = true;
            self.reached.push(layout);
        }
    }

    fn instance(&mut self, id: u32) -> Node {
        if let Some(number) = self.numbers.get(&id) {
            return Node::Uint(*number);
        }
        let instance = &self.module.checked.instances[id as usize];
        // Arguments come first, so a reader can build the table front to back.
        let arguments: Vec<Node> = instance
            .arguments
            .clone()
            .iter()
            .map(|ty| self.ty(*ty))
            .collect();
        let head = match instance.head {
            CheckedSourceInstanceHead::Nominal { schema } => {
                self.reach(schema as usize);
                Node::record(HEAD_NOMINAL, vec![item_node(&self.keys[schema as usize])])
            }
            CheckedSourceInstanceHead::Array { extent } => {
                Node::record(HEAD_ARRAY, vec![Node::Uint(extent.into())])
            }
            CheckedSourceInstanceHead::Slice => Node::record(HEAD_SLICE, Vec::new()),
            CheckedSourceInstanceHead::Sized => Node::record(HEAD_SIZED, Vec::new()),
            CheckedSourceInstanceHead::Sequence => Node::record(HEAD_SEQUENCE, Vec::new()),
            CheckedSourceInstanceHead::Pointer => Node::record(HEAD_POINTER, Vec::new()),
        };
        let number = self.instances.len() as u64;
        self.instances
            .push(Node::record(INSTANCE, vec![head, Node::List(arguments)]));
        self.numbers.insert(id, number);
        Node::Uint(number)
    }

    fn ty(&mut self, ty: SourceType) -> Node {
        match ty {
            SourceType::Unit => Node::record(TY_UNIT, Vec::new()),
            SourceType::Text => Node::record(TY_TEXT, Vec::new()),
            SourceType::Integer { lower, upper } => {
                Node::record(TY_INTEGER, vec![Node::Int(lower), Node::Int(upper)])
            }
            SourceType::Generic { index } => {
                Node::record(TY_GENERIC, vec![Node::Uint(index.into())])
            }
            SourceType::Aggregate { id } => Node::record(TY_AGGREGATE, vec![self.instance(id)]),
            SourceType::Sum { id } => Node::record(TY_SUM, vec![self.instance(id)]),
            SourceType::Array { id } => Node::record(TY_ARRAY, vec![self.instance(id)]),
            SourceType::Sequence { id } => Node::record(TY_SEQUENCE, vec![self.instance(id)]),
            SourceType::Pointer { id } => Node::record(TY_POINTER, vec![self.instance(id)]),
        }
    }

    fn term(&mut self, term: &Term) -> Node {
        match term {
            Term::Slot(slot) => Node::record(TERM_SLOT, vec![Node::Uint(*slot as u64)]),
            Term::Constant(value) => Node::record(TERM_CONSTANT, vec![Node::Int(*value)]),
            Term::Number(number) => Node::record(TERM_NUMBER, vec![Node::Uint((*number).into())]),
            Term::Identity(identity) => {
                Node::record(TERM_IDENTITY, vec![Node::Uint((*identity).into())])
            }
            Term::Path {
                slot,
                field,
                deeper,
            } => Node::record(
                TERM_PATH,
                vec![
                    Node::Uint(*slot as u64),
                    Node::Uint((*field).into()),
                    uints(deeper),
                ],
            ),
            Term::Arithmetic(operator, left, right) => Node::record(
                TERM_ARITHMETIC,
                vec![enc_arithmetic(*operator), self.term(left), self.term(right)],
            ),
            Term::Layout { alignment, ty } => {
                Node::record(TERM_LAYOUT, vec![Node::Bool(*alignment), self.ty(*ty)])
            }
        }
    }

    /// Claims keep their written order: the doc leaves open whether the prover treats them as
    /// a set, and keeping the order can only make two artifacts differ, never wrongly equal.
    fn constraints(&mut self, constraints: &[Constraint]) -> Node {
        Node::List(
            constraints
                .iter()
                .map(|c| {
                    Node::record(
                        CONSTRAINT,
                        vec![
                            Node::Uint(c.left as u64),
                            Node::option(c.left_field, |field| Node::Uint(field.into())),
                            match &c.left_term {
                                Some(term) => self.term(term),
                                None => Node::Absent,
                            },
                            Node::option(c.modulus, Node::Int),
                            enc_binary(c.operator),
                            self.term(&c.right),
                        ],
                    )
                })
                .collect(),
        )
    }

    fn field(&mut self, position: usize, field: &CheckedSourceField) -> Node {
        Node::record(
            FIELD,
            vec![
                Node::Uint(position as u64),
                Node::text(&field.name),
                self.ty(field.ty),
                Node::option(field.register.as_deref(), Node::text),
                Node::option(field.borrow, enc_borrow),
                Node::option(field.length, |length| Node::Uint(length.into())),
                Node::Bool(field.private),
            ],
        )
    }

    /// A layout. `public_only`: its private fields go to the other layer, and only their
    /// number is said here (it decides whether a dependent may build or take apart a value).
    fn layout(&mut self, index: usize, public_only: bool) -> Node {
        let layout: &Layout = &self.module.layouts[index];
        let schema = &layout.schema;
        let mut fields = Vec::new();
        let mut hidden = 0u64;
        for (position, field) in schema.fields.iter().enumerate() {
            if public_only && hidden_field(layout, field) {
                hidden += 1;
            } else {
                fields.push(self.field(position, field));
            }
        }
        let variants = schema.variants.iter().map(|ty| self.ty(*ty)).collect();
        let defaults = schema
            .defaults
            .iter()
            .map(|ty| match ty {
                Some(ty) => self.ty(*ty),
                None => Node::Absent,
            })
            .collect();
        let constraints = self.constraints(&layout.constraints);
        Node::record(
            LAYOUT,
            vec![
                item_node(&self.keys[index]),
                Node::text(&schema.name),
                texts(&schema.parameters),
                Node::List(fields),
                Node::Uint(hidden),
                Node::Uint(schema.fields.len() as u64),
                Node::List(variants),
                enc_capability(schema.capability),
                enc_kind(schema.kind),
                Node::Bool(schema.wrapping),
                Node::List(defaults),
                Node::List(
                    schema
                        .bounds
                        .iter()
                        .map(|b| Node::option(b.as_deref(), Node::text))
                        .collect(),
                ),
                texts(&schema.provides),
                enc_borrowing(schema.borrowing),
                Node::List(
                    schema
                        .numbers
                        .iter()
                        .map(|range| {
                            Node::option(*range, |(low, high)| {
                                pair(Node::Int(low), Node::Int(high))
                            })
                        })
                        .collect(),
                ),
                Node::List(
                    schema
                        .domain_claims
                        .iter()
                        .map(|claim| match claim {
                            DomainClaim::Compare(operator, value) => Node::record(
                                DOMAIN_COMPARE,
                                vec![enc_binary(*operator), Node::Int(*value)],
                            ),
                            DomainClaim::Multiple(value) => {
                                Node::record(DOMAIN_MULTIPLE, vec![Node::Int(*value)])
                            }
                        })
                        .collect(),
                ),
                Node::option(schema.discriminant.as_ref(), |d| match d {
                    Discriminant::Integer(value) => {
                        Node::record(DISCRIMINANT_INTEGER, vec![Node::Int(*value)])
                    }
                    Discriminant::Text(text) => {
                        Node::record(DISCRIMINANT_TEXT, vec![Node::text(text)])
                    }
                }),
                constraints,
            ],
        )
    }

    fn place(&mut self, place: &PlacePath) -> Node {
        Node::record(
            PLACE,
            vec![Node::Uint(place.parameter.into()), texts(&place.fields)],
        )
    }

    fn clause(&mut self, expr: &ClauseExpr) -> Node {
        match expr {
            ClauseExpr::Constant(value) => Node::record(CLAUSE_CONSTANT, vec![Node::Int(*value)]),
            ClauseExpr::Place(place) => Node::record(CLAUSE_PLACE, vec![self.place(place)]),
            ClauseExpr::Binary(operator, left, right) => Node::record(
                CLAUSE_BINARY,
                vec![
                    enc_arithmetic(*operator),
                    self.clause(left),
                    self.clause(right),
                ],
            ),
            ClauseExpr::Select {
                comparison,
                left,
                right,
                then_branch,
                else_branch,
            } => Node::record(
                CLAUSE_SELECT,
                vec![
                    enc_binary(*comparison),
                    self.clause(left),
                    self.clause(right),
                    self.clause(then_branch),
                    self.clause(else_branch),
                ],
            ),
        }
    }

    fn effect(&mut self, effect: &Effect) -> Node {
        match effect {
            Effect::Set(place, expr) => {
                Node::record(EFFECT_SET, vec![self.place(place), self.clause(expr)])
            }
            Effect::Forgotten(place) => Node::record(EFFECT_FORGOTTEN, vec![self.place(place)]),
        }
    }

    fn summary(&mut self, summary: &Summary) -> Node {
        let inputs = summary
            .inputs
            .iter()
            .map(|input| {
                Node::record(
                    INPUT,
                    vec![
                        self.ty(input.ty),
                        usizes(&input.children),
                        uints(&input.slots),
                    ],
                )
            })
            .collect();
        let outputs = summary
            .outputs
            .iter()
            .map(|output| {
                Node::record(
                    OUTPUT,
                    vec![
                        self.ty(output.ty),
                        Node::Int(output.lower),
                        Node::Int(output.upper),
                        Node::option(output.input, |input| Node::Uint(input as u64)),
                        usizes(&output.children),
                        uints(&output.slots),
                        match output.sum {
                            Some(id) => self.instance(id),
                            None => Node::Absent,
                        },
                        Node::Bool(output.materialized),
                        match output.structural {
                            Some(ty) => self.ty(ty),
                            None => Node::Absent,
                        },
                        Node::option(output.integer_domain, |(low, high)| {
                            pair(Node::Int(low), Node::Int(high))
                        }),
                        Node::Bool(output.produced),
                    ],
                )
            })
            .collect();
        Node::record(
            SUMMARY,
            vec![
                Node::List(inputs),
                usizes(&summary.roots),
                Node::List(summary.borrowed.iter().map(|b| Node::Bool(*b)).collect()),
                Node::List(
                    summary
                        .dispositions
                        .iter()
                        .map(|d| enc_disposition(*d))
                        .collect(),
                ),
                Node::List(outputs),
                Node::List(
                    summary
                        .integer_guards
                        .iter()
                        .map(|(input, value)| pair(Node::Uint(*input as u64), Node::Int(*value)))
                        .collect(),
                ),
                Node::List(
                    summary
                        .input_ranges
                        .iter()
                        .map(|(input, low, high)| {
                            pair(
                                Node::Uint(*input as u64),
                                pair(Node::Int(*low), Node::Int(*high)),
                            )
                        })
                        .collect(),
                ),
            ],
        )
    }

    fn function(&mut self, name: &str, function: &Function) -> Node {
        let s = &function.signature;
        let signature = Node::record(
            SIGNATURE,
            vec![
                Node::option(s.memory, enc_memory),
                Node::option(s.provided_by.as_deref(), Node::text),
                Node::option(s.instruction.as_deref(), Node::text),
                texts(&s.literals),
                Node::option(s.infix, enc_infix),
                Node::Bool(s.private),
                Node::text(&s.name),
                Node::List(s.parameters.iter().map(|ty| self.ty(*ty)).collect()),
                uints(&s.mutated),
                texts(&s.type_parameters),
                Node::List(
                    s.type_bounds
                        .iter()
                        .map(|b| Node::option(b.as_deref(), Node::text))
                        .collect(),
                ),
                self.ty(s.result),
            ],
        );
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| match parameter {
                Parameter::Type => Node::record(PARAMETER_TYPE, Vec::new()),
                Parameter::Value(ty) => Node::record(PARAMETER_VALUE, vec![self.ty(*ty)]),
                Parameter::Literal => Node::record(PARAMETER_LITERAL, Vec::new()),
            })
            .collect();
        let constraints = self.constraints(&function.constraints);
        let postconditions = self.constraints(&function.postconditions);
        let variant_claims = function
            .variant_postconditions
            .iter()
            .map(|(member, claims)| {
                Node::record(
                    VARIANT_CLAIMS,
                    vec![self.ty(*member), self.constraints(claims)],
                )
            })
            .collect();
        let variant_effects = function
            .variant_effects
            .iter()
            .map(|(member, transitions)| {
                let member = self.ty(*member);
                let transitions = transitions
                    .iter()
                    .map(|(place, expr)| {
                        Node::record(TRANSITION, vec![self.place(place), self.clause(expr)])
                    })
                    .collect();
                Node::record(VARIANT_EFFECTS, vec![member, Node::List(transitions)])
            })
            .collect();
        let summaries = function
            .summaries
            .iter()
            .map(|summary| self.summary(summary))
            .collect();
        let effects = match &function.effects {
            Some(effects) => Node::List(effects.iter().map(|effect| self.effect(effect)).collect()),
            None => Node::Absent,
        };
        Node::record(
            FUNCTION,
            vec![
                Node::text(name),
                signature,
                Node::List(parameters),
                Node::List(function.passing.iter().map(|p| enc_passing(*p)).collect()),
                constraints,
                postconditions,
                Node::List(variant_claims),
                Node::List(variant_effects),
                Node::Uint(function.implicit_types as u64),
                Node::List(summaries),
                effects,
                Node::option(function.result_borrow, enc_borrow),
                Node::List(
                    function
                        .result_sources
                        .iter()
                        .map(|source| {
                            pair(
                                Node::Uint(source.argument as u64),
                                Node::Bool(source.exclusive),
                            )
                        })
                        .collect(),
                ),
                // A call that knows `r` answers `#lower(r)`/`#upper(r)` exactly: the importer's
                // checker reads it, so it is signature.
                Node::option(function.bound_query.as_ref(), |query| {
                    Node::record(
                        BOUND_QUERY,
                        vec![Node::Bool(query.upper), Node::Uint(query.generic as u64)],
                    )
                }),
            ],
        )
    }

    fn callee(
        &self,
        callees: &Callees,
        callee: CheckedSourceCallee,
    ) -> Result<Node, ArtifactError> {
        Ok(match callee {
            CheckedSourceCallee::Local { function } => {
                let name = callees
                    .own
                    .get(function as usize)
                    .ok_or(ArtifactError::UnknownItem)?;
                Node::record(CALLEE_LOCAL, vec![Node::text(name)])
            }
            CheckedSourceCallee::Dependency {
                dependency,
                function,
            } => {
                let (home, name) = callees
                    .imported
                    .get(&(dependency, function))
                    .ok_or(ArtifactError::UnknownItem)?;
                Node::record(CALLEE_DEPENDENCY, vec![home.to_node(), Node::text(name)])
            }
        })
    }

    fn layout_ref(&mut self, schema: u32) -> Result<Node, ArtifactError> {
        let key = self
            .keys
            .get(schema as usize)
            .ok_or(ArtifactError::UnknownItem)?;
        let node = item_node(key);
        self.reach(schema as usize);
        Ok(node)
    }

    fn types(&mut self, types: &[SourceType]) -> Node {
        Node::List(types.iter().map(|ty| self.ty(*ty)).collect())
    }

    fn carried(carried: &[CheckedSourceLoopCarry]) -> Node {
        Node::List(
            carried
                .iter()
                .map(|c| {
                    Node::record(
                        CARRY,
                        vec![
                            Node::Uint(c.placeholder.into()),
                            Node::Uint(c.initial.into()),
                            Node::Uint(c.next.into()),
                        ],
                    )
                })
                .collect(),
        )
    }

    /// A recorded or template body. Node ids are local to the body and kept; layouts and
    /// callees become stable names; spans are dropped (a moved line must not change the bytes).
    fn body(&mut self, callees: &Callees, body: &CheckedSourceBody) -> Result<Node, ArtifactError> {
        use CheckedSourceBodyKind as K;
        let n = |id: u32| Node::Uint(id.into());
        let ns = |ids: &[u32]| uints(ids);
        let mut expressions = Vec::with_capacity(body.expressions.len());
        for expr in &body.expressions {
            let kind = match &expr.kind {
                K::Return { value } => Node::record(K_RETURN, vec![n(*value)]),
                K::Parameter { index } => Node::record(K_PARAMETER, vec![n(*index)]),
                K::Integer(value) => Node::record(K_INTEGER, vec![Node::Int(*value)]),
                K::Text(text) => Node::record(K_TEXT, vec![Node::text(text)]),
                K::Unit => Node::record(K_UNIT, Vec::new()),
                K::Hole => Node::record(K_HOLE, Vec::new()),
                K::Binary {
                    operator,
                    left,
                    right,
                } => Node::record(
                    K_BINARY,
                    vec![enc_arithmetic(*operator), n(*left), n(*right)],
                ),
                K::Wrap { operand } => Node::record(K_WRAP, vec![n(*operand)]),
                K::Construct { schema, fields } => {
                    Node::record(K_CONSTRUCT, vec![self.layout_ref(*schema)?, ns(fields)])
                }
                K::Match {
                    subject,
                    arms,
                    region,
                } => {
                    let mut arm_nodes = Vec::with_capacity(arms.len());
                    for arm in arms {
                        arm_nodes.push(Node::record(
                            ARM,
                            vec![self.layout_ref(arm.schema)?, n(arm.body)],
                        ));
                    }
                    let region = Node::option(region.as_ref(), |r| {
                        Node::record(
                            MATCH_REGION,
                            vec![n(r.member_body), n(r.member_start), n(r.other_start)],
                        )
                    });
                    Node::record(K_MATCH, vec![n(*subject), Node::List(arm_nodes), region])
                }
                K::Field { record, index } => Node::record(K_FIELD, vec![n(*record), n(*index)]),
                K::VariantField {
                    subject,
                    schema,
                    index,
                } => Node::record(
                    K_VARIANT_FIELD,
                    vec![n(*subject), self.layout_ref(*schema)?, n(*index)],
                ),
                K::Call {
                    callee,
                    arguments,
                    type_arguments,
                } => Node::record(
                    K_CALL,
                    vec![
                        self.callee(callees, *callee)?,
                        ns(arguments),
                        self.types(type_arguments),
                    ],
                ),
                K::BoundCall {
                    generic,
                    method,
                    trait_arguments,
                    arguments,
                } => Node::record(
                    K_BOUND_CALL,
                    vec![
                        n(*generic),
                        Node::text(method),
                        self.types(trait_arguments),
                        ns(arguments),
                    ],
                ),
                K::Select {
                    comparison,
                    left,
                    right,
                    then_branch,
                    else_branch,
                    region,
                } => Node::record(
                    K_SELECT,
                    vec![
                        enc_infix(*comparison),
                        n(*left),
                        n(*right),
                        n(*then_branch),
                        n(*else_branch),
                        Node::option(region.as_ref(), |r| {
                            Node::record(
                                SELECT_REGION,
                                vec![n(r.then_start), n(r.else_start), Node::Bool(r.exclusive)],
                            )
                        }),
                    ],
                ),
                K::Instruction {
                    text,
                    symbols,
                    input,
                } => Node::record(
                    K_INSTRUCTION,
                    vec![
                        Node::text(text),
                        Node::List(
                            symbols
                                .iter()
                                .map(|(a, b)| pair(Node::text(a), Node::text(b)))
                                .collect(),
                        ),
                        n(*input),
                    ],
                ),
                K::Load { pointer, index } => Node::record(K_LOAD, vec![n(*pointer), n(*index)]),
                K::Store {
                    pointer,
                    index,
                    value,
                } => Node::record(K_STORE, vec![n(*pointer), n(*index), n(*value)]),
                K::Widen { operand } => Node::record(K_WIDEN, vec![n(*operand)]),
                K::Loop {
                    start,
                    end,
                    counter,
                    body_start,
                    carried,
                } => Node::record(
                    K_LOOP,
                    vec![
                        n(*start),
                        n(*end),
                        n(*counter),
                        n(*body_start),
                        Self::carried(carried),
                    ],
                ),
                K::While {
                    comparison,
                    negated,
                    left,
                    right,
                    body_start,
                    split,
                    carried,
                    exit,
                } => Node::record(
                    K_WHILE,
                    vec![
                        enc_infix(*comparison),
                        Node::Bool(*negated),
                        n(*left),
                        n(*right),
                        n(*body_start),
                        n(*split),
                        Self::carried(carried),
                        Node::option(*exit, n),
                    ],
                ),
                K::CallOut { call, index } => Node::record(K_CALL_OUT, vec![n(*call), n(*index)]),
                K::SubArray { base, start } => Node::record(K_SUB_ARRAY, vec![n(*base), n(*start)]),
                K::RepeatArray { element, count } => {
                    Node::record(K_REPEAT_ARRAY, vec![n(*element), n(*count)])
                }
                K::TextBytes { text } => Node::record(K_TEXT_BYTES, vec![Node::text(text)]),
                K::Layout { queried, alignment } => {
                    Node::record(K_LAYOUT, vec![self.ty(*queried), Node::Bool(*alignment)])
                }
                K::LoopCounter => Node::record(K_LOOP_COUNTER, Vec::new()),
                K::LoopCarried => Node::record(K_LOOP_CARRIED, Vec::new()),
                K::LoopResult { looped, index } => {
                    Node::record(K_LOOP_RESULT, vec![n(*looped), n(*index)])
                }
                K::Block { statements, result } => {
                    Node::record(K_BLOCK, vec![ns(statements), n(*result)])
                }
                K::Bundle { parts } => Node::record(K_BUNDLE, vec![ns(parts)]),
                K::Part { bundle, index } => Node::record(K_PART, vec![n(*bundle), n(*index)]),
                K::Undefined => Node::record(K_UNDEFINED, Vec::new()),
            };
            expressions.push(Node::record(EXPR, vec![self.ty(expr.ty), kind]));
        }
        Ok(Node::record(
            BODY,
            vec![
                Node::Bool(body.effects),
                Node::Uint(body.root.into()),
                ns(&body.outputs),
                Node::List(expressions),
            ],
        ))
    }

    /// What the instantiation layer says of own function `index` beyond its signature.
    fn payload(
        &mut self,
        callees: &Callees,
        index: usize,
        private: Option<Node>,
    ) -> Result<Node, ArtifactError> {
        let function = &self.module.functions[index];
        let body = match &function.body {
            Some(body) => self.body(callees, body)?,
            None => Node::Absent,
        };
        let decline = Node::option(function.decline.as_ref(), |d| {
            Node::record(DECLINE, vec![enc_decline(d.kind), Node::text(&d.construct)])
        });
        let mut folds = Vec::with_capacity(function.folds.len());
        for callee in &function.folds {
            folds.push(self.callee(callees, *callee)?);
        }
        Ok(Node::record(
            PAYLOAD,
            vec![
                Node::text(&callees.own[index]),
                Node::Uint(index as u64),
                private.unwrap_or(Node::Absent),
                body,
                decline,
                Node::List(folds),
            ],
        ))
    }
}

/// Stable names of every function a body of the module may call.
struct Callees {
    /// Own functions, by own index.
    own: Vec<String>,
    /// `Dependency { dependency, function }` callees: the defining module and the stable name
    /// there (the name its own artifact gives it).
    imported: HashMap<(u32, u32), (ModuleId, String)>,
}

impl Callees {
    fn of(
        module: &CheckedSourceModule,
        ids: &[&ModuleId],
        own: usize,
    ) -> Result<Self, ArtifactError> {
        let own_names = disambiguate(
            module.functions[..own]
                .iter()
                .map(|f| (0, f.signature.name.clone())),
        );
        // Each home's own functions, by own index: an importer carries the whole table of each
        // dependency, so every function of a home is here and the names agree with the home's.
        let mut by_home: BTreeMap<&str, BTreeMap<u32, &str>> = BTreeMap::new();
        for full in own..module.functions.len() {
            let Some((home, at)) = module.function_home(full) else {
                continue;
            };
            by_home
                .entry(home)
                .or_default()
                .insert(at, module.functions[full].signature.name.as_str());
        }
        let mut names: HashMap<(&str, u32), String> = HashMap::new();
        for (home, table) in &by_home {
            let stable = disambiguate(table.values().map(|name| (0, (*name).to_owned())));
            for ((at, _), name) in table.iter().zip(stable) {
                names.insert((home, *at), name);
            }
        }
        let mut imported = HashMap::new();
        for (position, (dependency, function)) in module.homes.imported.iter().enumerate() {
            let Some((home, at)) = module.function_home(own + position) else {
                continue;
            };
            let Some(id) = ids.iter().find(|id| id.module == home) else {
                continue;
            };
            if let Some(name) = names.get(&(home, at)) {
                imported.insert((*dependency, *function), ((*id).clone(), name.clone()));
            }
        }
        for (callee, (home, name)) in &module.homes.artifact_callees {
            if !ids.contains(&home) {
                return Err(ArtifactError::UnknownHome(home.module.clone()));
            }
            imported.insert(*callee, (home.clone(), name.clone()));
        }
        Ok(Self {
            own: own_names,
            imported,
        })
    }
}

/// Whether a field goes to the instantiation layer: a private field of a record or a variant.
/// A domain's one hidden field is its range, which is signature.
fn hidden_field(layout: &Layout, field: &CheckedSourceField) -> bool {
    field.private && layout.schema.kind != AggregateKind::Domain
}

/// Names that are unique within their home module: a second layout or function of the same
/// name gets `#2`, `#3`, ... in table order.
fn disambiguate(names: impl Iterator<Item = (usize, String)>) -> Vec<String> {
    let mut counts: HashMap<(usize, String), usize> = HashMap::new();
    names
        .map(|(home, name)| {
            let count = counts.entry((home, name.clone())).or_insert(0);
            *count += 1;
            if *count == 1 {
                name
            } else {
                format!("{name}#{count}")
            }
        })
        .collect()
}

/// Write both layers of `module`'s artifact. The bytes are canonical: the same module with the
/// same identity always gives the same bytes.
pub fn write_artifact(
    module: &CheckedSourceModule,
    identity: &ArtifactIdentity,
) -> Result<Vec<u8>, ArtifactError> {
    let (header, signature, instantiation) = artifact_nodes(module, identity)?;
    Ok(interface_codec::encode_artifact(
        &header,
        &signature,
        &instantiation,
    ))
}

fn artifact_nodes(
    module: &CheckedSourceModule,
    identity: &ArtifactIdentity,
) -> Result<(Header, Node, Node), ArtifactError> {
    if identity.module.module != module.name {
        return Err(ArtifactError::WrongModule(identity.module.module.clone()));
    }
    let mut ids: Vec<&ModuleId> = vec![&identity.module];
    for dependency in &identity.dependencies {
        if ids
            .iter()
            .any(|id| id.module == dependency.module && *id != dependency)
        {
            return Err(ArtifactError::AmbiguousHome(dependency.module.clone()));
        }
        if !ids.contains(&dependency) {
            ids.push(dependency);
        }
    }
    let home_of = |name: &str| -> Result<usize, ArtifactError> {
        ids.iter()
            .position(|id| id.module == name)
            .ok_or_else(|| ArtifactError::UnknownHome(name.to_owned()))
    };
    let mut homes = Vec::with_capacity(module.layouts.len());
    for index in 0..module.layouts.len() {
        let home = module.layout_home(index).unwrap_or(&module.name);
        homes.push(home_of(home)?);
    }
    // A layout's stable name: the key the checker recorded when it made the layout, else its
    // declared name. Two layouts of one home with one name cannot be told apart: refused.
    let names: Vec<String> = module
        .layouts
        .iter()
        .map(|layout| {
            layout
                .key
                .clone()
                .unwrap_or_else(|| layout.schema.name.clone())
        })
        .collect();
    let mut seen: HashMap<(usize, &str), ()> = HashMap::new();
    for (home, name) in homes.iter().zip(&names) {
        if seen.insert((*home, name.as_str()), ()).is_some() {
            return Err(ArtifactError::AmbiguousName(name.clone()));
        }
    }
    let keys: Vec<ItemKey> = homes
        .iter()
        .zip(names)
        .map(|(home, name)| (ids[*home].clone(), name))
        .collect();
    let own = module.homes.own.min(module.functions.len());
    let callees = Callees::of(module, &ids, own)?;
    let function_names = &callees.own;

    // The signature layer, walked from the sorted exports.
    let mut sig = Table::new(module, &keys);
    let mut exports: Vec<(&String, &Export)> = module
        .exports
        .iter()
        .map(|(name, export)| (name, export))
        .collect();
    exports.sort_by(|a, b| a.0.cmp(b.0));
    let mut export_nodes = Vec::new();
    let mut functions: BTreeMap<&str, Node> = BTreeMap::new();
    for (name, export) in &exports {
        match export {
            Export::Type(layout) => {
                sig.reach(*layout);
                export_nodes.push(Node::record(
                    EXPORT,
                    vec![Node::text(name), Node::Uint(1), item_node(&keys[*layout])],
                ));
            }
            Export::Function(function) => {
                let stable = function_names[*function].as_str();
                export_nodes.push(Node::record(
                    EXPORT,
                    vec![Node::text(name), Node::Uint(2), Node::text(stable)],
                ));
                if !functions.contains_key(stable) {
                    let node = sig.function(stable, &module.functions[*function]);
                    functions.insert(stable, node);
                }
            }
        }
    }
    let mut aliases: Vec<&(String, SourceType)> = module.aliases.iter().collect();
    aliases.sort_by(|a, b| a.0.cmp(&b.0));
    let alias_nodes: Vec<Node> = aliases
        .iter()
        .map(|(name, ty)| Node::record(ALIAS, vec![Node::text(name), sig.ty(*ty)]))
        .collect();
    let mut traits: Vec<&imported_traits::TraitSource> = module.traits.iter().collect();
    traits.sort_by(|a, b| a.name.cmp(&b.name));
    let mut trait_nodes = Vec::new();
    for source in traits {
        let mut spellings: Vec<&(String, usize)> = source.names.iter().collect();
        spellings.sort_by(|a, b| a.0.cmp(&b.0));
        let mut names = Vec::new();
        for (spelling, layout) in spellings {
            if *layout >= module.layouts.len() {
                return Err(ArtifactError::UnknownItem);
            }
            sig.reach(*layout);
            names.push(Node::record(
                TRAIT_NAME,
                vec![Node::text(spelling), item_node(&keys[*layout])],
            ));
        }
        trait_nodes.push(Node::record(
            TRAIT,
            vec![
                Node::text(&source.name),
                Node::text(&source.text),
                Node::List(names),
            ],
        ));
    }
    // Every layout the surface reaches, and the layouts those reach.
    let mut layout_nodes: BTreeMap<Vec<u8>, Node> = BTreeMap::new();
    let mut at = 0;
    while at < sig.reached.len() {
        let layout = sig.reached[at];
        at += 1;
        let node = sig.layout(layout, true);
        layout_nodes.insert(sort_key(&keys[layout]), node);
    }
    let mut constants: Vec<&(String, I256)> = module.constants.iter().collect();
    constants.sort_by(|a, b| a.0.cmp(&b.0));
    let mut accessors: Vec<&(String, String, String)> = module.accessors.iter().collect();
    accessors.sort();
    let mut plain: Vec<&(String, String)> = module.plain_accessors.iter().collect();
    plain.sort();
    let in_sig = sig.seen.clone();
    let signature = Node::record(
        SIG,
        vec![
            Node::List(std::mem::take(&mut sig.instances)),
            Node::List(layout_nodes.into_values().collect()),
            Node::List(functions.into_values().collect()),
            Node::List(export_nodes),
            Node::List(
                constants
                    .iter()
                    .map(|(name, value)| {
                        Node::record(CONSTANT, vec![Node::text(name), Node::Int(*value)])
                    })
                    .collect(),
            ),
            Node::List(alias_nodes),
            Node::List(trait_nodes),
            Node::List(
                accessors
                    .iter()
                    .map(|(ty, method, field)| {
                        Node::record(
                            ACCESSOR,
                            vec![Node::text(ty), Node::text(method), Node::text(field)],
                        )
                    })
                    .collect(),
            ),
            Node::List(
                plain
                    .iter()
                    .map(|(ty, method)| {
                        Node::record(PLAIN_ACCESSOR, vec![Node::text(ty), Node::text(method)])
                    })
                    .collect(),
            ),
        ],
    );

    // The instantiation layer: private fields of the signature's layouts, in their order, and
    // whole layouts that only those reach.
    let mut inst = Table::new(module, &keys);
    let mut sig_layouts: Vec<usize> = (0..module.layouts.len())
        .filter(|index| in_sig[*index])
        .collect();
    sig_layouts.sort_by_key(|index| sort_key(&keys[*index]));
    let mut private = Vec::new();
    for layout in sig_layouts {
        let fields: Vec<Node> = module.layouts[layout]
            .schema
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| hidden_field(&module.layouts[layout], field))
            .map(|(position, field)| inst.field(position, field))
            .collect();
        if !fields.is_empty() {
            private.push(Node::record(
                PRIVATE_FIELDS,
                vec![item_node(&keys[layout]), Node::List(fields)],
            ));
        }
    }
    // Every own function's payload, in own-index order (the index is what `Homes` and the
    // callees of other modules' bodies resolve to). A function no export names carries its
    // whole record here: a body may call it.
    let exported: Vec<bool> = (0..own)
        .map(|index| {
            exports
                .iter()
                .any(|(_, export)| matches!(export, Export::Function(f) if *f == index))
        })
        .collect();
    let mut payloads = Vec::with_capacity(own);
    for index in 0..own {
        let private = (!exported[index])
            .then(|| inst.function(&callees.own[index], &module.functions[index]));
        payloads.push(inst.payload(&callees, index, private)?);
    }
    let entry = match module.checked.entry {
        Some(entry) => Node::text(
            callees
                .own
                .get(entry as usize)
                .ok_or(ArtifactError::UnknownItem)?,
        ),
        None => Node::Absent,
    };
    let notes = Node::List(module.checked.notes.iter().map(note_node).collect());
    let mut extra: BTreeMap<Vec<u8>, Node> = BTreeMap::new();
    let mut at = 0;
    while at < inst.reached.len() {
        let layout = inst.reached[at];
        at += 1;
        if in_sig[layout] {
            continue;
        }
        let node = inst.layout(layout, false);
        extra.insert(sort_key(&keys[layout]), node);
    }
    let instantiation = Node::record(
        INST,
        vec![
            Node::List(std::mem::take(&mut inst.instances)),
            Node::List(private),
            Node::List(extra.into_values().collect()),
            Node::List(payloads),
            entry,
            notes,
        ],
    );
    let header = Header {
        module: identity.module.clone(),
        target: identity.target.clone(),
        compiler: identity.compiler.clone(),
    };
    Ok((header, signature, instantiation))
}

/// A note without its position: positions are not in an artifact.
fn note_node(note: &SourceCheckNote) -> Node {
    let facts = note
        .facts
        .iter()
        .map(|fact| match fact {
            SourceFact::Interval { name, lower, upper } => Node::record(
                FACT_INTERVAL,
                vec![Node::text(name), Node::Int(*lower), Node::Int(*upper)],
            ),
            SourceFact::Known { name, value } => {
                Node::record(FACT_KNOWN, vec![Node::text(name), Node::Int(*value)])
            }
            SourceFact::Multiple { name, modulus } => {
                Node::record(FACT_MULTIPLE, vec![Node::text(name), Node::Int(*modulus)])
            }
            SourceFact::Offset { name, base, offset } => Node::record(
                FACT_OFFSET,
                vec![Node::text(name), Node::text(base), Node::Int(*offset)],
            ),
            SourceFact::Relation {
                left,
                operator,
                right,
            } => Node::record(
                FACT_RELATION,
                vec![Node::text(left), Node::text(operator), Node::text(right)],
            ),
        })
        .collect();
    Node::record(
        NOTE,
        vec![
            enc_note(note.kind),
            Node::text(&note.detail),
            Node::List(facts),
        ],
    )
}

fn sort_key(key: &ItemKey) -> Vec<u8> {
    let mut out = Vec::new();
    interface_codec::encode_node(&mut out, &item_node(key));
    out
}

/// The deterministic text form of an artifact's bytes, after the same checks as a read.
pub fn dump_artifact(bytes: &[u8]) -> Result<String, ArtifactError> {
    let artifact = interface_codec::decode_artifact(bytes)?;
    Ok(interface_codec::dump(&artifact, &|_, tag| {
        artifact_tag_name(tag)
    }))
}

// ---------------------------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------------------------

/// Reads artifacts into capsules. Equal module identities give equal nominal origins across
/// every capsule one reader makes, and different identities (another package, version or
/// source) different ones, so two `Buffer`s never unify and one `core 1.2.0` is one module.
#[derive(Default)]
pub struct ArtifactReader {
    modules: HashMap<ModuleId, Arc<()>>,
    declarations: HashMap<ItemKey, usize>,
    /// The own index of each function of every module read so far, by its stable name: what
    /// a later module's body callee into that module resolves to.
    functions: HashMap<(ModuleId, String), (u32, validation::FunctionShape)>,
}

/// The decoded layers of one artifact, before any capsule is made of them.
struct Layers<'a> {
    sig: &'a [Node],
    inst: &'a [Node],
}

struct Build<'r> {
    reader: &'r mut ArtifactReader,
    layouts: Vec<Layout>,
    homes: Vec<String>,
    /// The capsule's dependency names, in the slot order its bodies' callees use.
    dependencies: Vec<String>,
    index: HashMap<ItemKey, usize>,
    instances: Vec<CheckedSourceInstance>,
    /// Where each layer's type table starts in `instances`, and how long it is.
    sig_table: (usize, usize),
    inst_table: (usize, usize),
    field_slots_remaining: usize,
    artifact_callees: HashMap<(u32, u32), (ModuleId, String)>,
    dependency_shapes: HashMap<(u32, u32), validation::FunctionShape>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Sig,
    Inst,
}

impl ArtifactReader {
    pub fn new() -> Self {
        Self::default()
    }

    fn origin(&mut self, key: &ItemKey) -> Origin {
        let module = self
            .modules
            .entry(key.0.clone())
            .or_insert_with(|| Arc::new(()))
            .clone();
        let next = self.declarations.len();
        let declaration = *self.declarations.entry(key.clone()).or_insert(next);
        Origin {
            module,
            declaration,
        }
    }

    /// Check magic, version, canonical form, hashes, structural invariants, and the artifact's
    /// self-declared identity against `expected`, then construct a capsule including its bodies.
    ///
    /// The caller must trust the producer to have checked the bodies and their summaries.
    /// Neither hashes nor matching identity metadata authenticate provenance or prove arbitrary
    /// bodies semantically valid.
    pub fn read(
        &mut self,
        bytes: &[u8],
        expected: &Expectation,
    ) -> Result<CheckedSourceModule, ArtifactError> {
        let artifact = interface_codec::decode_artifact(bytes)?;
        artifact.verify(expected)?;
        let layers = Layers {
            sig: record(&artifact.signature.body, SIG, 9)?,
            inst: record(&artifact.instantiation.body, INST, 6)?,
        };
        self.capsule(&artifact.header().module, &layers)
    }

    fn capsule(
        &mut self,
        id: &ModuleId,
        layers: &Layers<'_>,
    ) -> Result<CheckedSourceModule, ArtifactError> {
        let mut build = Build {
            reader: self,
            layouts: Vec::new(),
            homes: Vec::new(),
            dependencies: Vec::new(),
            index: HashMap::new(),
            instances: Vec::new(),
            sig_table: (0, 0),
            inst_table: (0, 0),
            field_slots_remaining: validation::field_slots(layers)?,
            artifact_callees: HashMap::new(),
            dependency_shapes: HashMap::new(),
        };
        // Every layout gets its slot and origin first, so the type tables can name them.
        let sig_layouts = list(&layers.sig[1])?;
        let extra_layouts = list(&layers.inst[2])?;
        for node in sig_layouts.iter().chain(extra_layouts) {
            let fields = record(node, LAYOUT, 18)?;
            let key = item_key(&fields[0])?;
            if build.index.contains_key(&key) {
                return Err(shape("duplicate layout"));
            }
            let origin = build.reader.origin(&key);
            build.index.insert(key.clone(), build.layouts.len());
            build.homes.push(key.0.module.clone());
            build.layouts.push(Layout {
                origin,
                schema: CheckedSourceAggregate {
                    name: String::new(),
                    parameters: Vec::new(),
                    fields: Vec::new(),
                    variants: Vec::new(),
                    capability: Capability::Linear,
                    kind: dec_kind(&fields[8])?,
                    wrapping: false,
                    defaults: Vec::new(),
                    bounds: Vec::new(),
                    provides: Vec::new(),
                    borrowing: Borrowing::None,
                    numbers: Vec::new(),
                    domain_claims: Vec::new(),
                    discriminant: None,
                },
                constraints: Vec::new(),
                key: None,
            });
        }
        build.table(&layers.sig[0], Which::Sig)?;
        build.table(&layers.inst[0], Which::Inst)?;
        let mut slots: Vec<Option<Vec<Option<CheckedSourceField>>>> =
            vec![None; build.layouts.len()];
        for (position, node) in sig_layouts.iter().enumerate() {
            let fields = build.layout(position, node, Which::Sig)?;
            slots[position] = Some(fields);
        }
        for (offset, node) in extra_layouts.iter().enumerate() {
            let position = sig_layouts.len() + offset;
            let fields = build.layout(position, node, Which::Inst)?;
            slots[position] = Some(fields);
        }
        for node in list(&layers.inst[1])? {
            let fields = record(node, PRIVATE_FIELDS, 2)?;
            let layout = build.lookup(&item_key(&fields[0])?)?;
            if layout >= sig_layouts.len() {
                return Err(shape("private fields of an instantiation layout"));
            }
            for field in list(&fields[1])? {
                let (position, field) = build.field(field, Which::Inst)?;
                let slot = slots[layout]
                    .as_mut()
                    .unwrap()
                    .get_mut(position)
                    .ok_or(shape("field position"))?;
                if slot.is_some() {
                    return Err(shape("field given twice"));
                }
                if !field.private {
                    return Err(shape("public field in the instantiation layer"));
                }
                *slot = Some(field);
            }
        }
        for (layout, slots) in slots.into_iter().enumerate() {
            let fields = slots
                .unwrap()
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or(shape("missing field"))?;
            build.layouts[layout].schema.fields = fields;
        }
        validation::instances(&build.instances, &build.layouts)?;
        // Layout homes first, so the slots bodies add for callees come after them.
        let mut dependencies: Vec<String> = build
            .homes
            .iter()
            .filter(|home| **home != id.module)
            .cloned()
            .collect();
        dependencies.sort();
        dependencies.dedup();
        build.dependencies = dependencies;
        let mut exported: HashMap<String, Function> = HashMap::new();
        for node in list(&layers.sig[2])? {
            let (name, function) = build.function(node, Which::Sig)?;
            if exported.insert(name, function).is_some() {
                return Err(shape("duplicate function"));
            }
        }
        // The own table, at the indexes the module had: payloads give each name its index.
        let payloads = list(&layers.inst[3])?;
        let count = payloads.len();
        let mut function_index: HashMap<String, usize> = HashMap::new();
        let mut slots: Vec<Option<Function>> = (0..count).map(|_| None).collect();
        for node in payloads {
            let fields = record(node, PAYLOAD, 6)?;
            let name = string(&fields[0])?;
            let at = index(&fields[1])?;
            if at >= count
                || slots[at].is_some()
                || function_index.insert(name.clone(), at).is_some()
            {
                return Err(shape("function index"));
            }
            let function = match &fields[2] {
                Node::Absent => exported.remove(&name).ok_or(ArtifactError::UnknownItem)?,
                private => {
                    let (written, function) = build.function(private, Which::Inst)?;
                    if written != name || exported.contains_key(&name) {
                        return Err(shape("function name"));
                    }
                    function
                }
            };
            slots[at] = Some(function);
        }
        if !exported.is_empty() {
            return Err(shape("signature function without a payload"));
        }
        let mut functions: Vec<Function> = slots
            .into_iter()
            .map(|slot| slot.expect("every index is filled"))
            .collect();
        for node in payloads {
            let fields = record(node, PAYLOAD, 6)?;
            let at = index(&fields[1])?;
            let body = optional(&fields[3], |node| build.body(node, &function_index))?;
            let decline = optional(&fields[4], |node| {
                let parts = record(node, DECLINE, 2)?;
                Ok(BodyDecline {
                    kind: dec_decline(&parts[0])?,
                    construct: string(&parts[1])?,
                    span: None,
                    file: None,
                })
            })?;
            let mut folds = Vec::new();
            for callee in list(&fields[5])? {
                folds.push(build.callee(callee, &function_index)?);
            }
            let function = &mut functions[at];
            if let Some(body) = &body {
                validation::body_parameters(body, &function.signature)?;
            }
            function.body = body.map(Arc::new);
            function.decline = decline;
            function.folds = folds;
        }
        let entry = optional(&layers.inst[4], |node| {
            let at = *function_index
                .get(&string(node)?)
                .ok_or(ArtifactError::UnknownItem)?;
            Ok(at as u32)
        })?;
        let notes = each(&layers.inst[5], decode_note)?;
        let mut exports = Vec::new();
        for node in list(&layers.sig[3])? {
            let fields = record(node, EXPORT, 3)?;
            let name = string(&fields[0])?;
            let export = match uint(&fields[1])? {
                1 => Export::Type(build.lookup(&item_key(&fields[2])?)?),
                2 => Export::Function(
                    *function_index
                        .get(&string(&fields[2])?)
                        .ok_or(ArtifactError::UnknownItem)?,
                ),
                _ => return Err(shape("export kind")),
            };
            exports.push((name, export));
        }
        let constants = each(&layers.sig[4], |node| {
            let fields = record(node, CONSTANT, 2)?;
            Ok((string(&fields[0])?, int(&fields[1])?))
        })?;
        let aliases = each(&layers.sig[5], |node| {
            let fields = record(node, ALIAS, 2)?;
            Ok((string(&fields[0])?, build.ty(&fields[1], Which::Sig)?))
        })?;
        let traits = each(&layers.sig[6], |node| {
            let fields = record(node, TRAIT, 3)?;
            let names = each(&fields[2], |node| {
                let fields = record(node, TRAIT_NAME, 2)?;
                Ok((string(&fields[0])?, build.lookup(&item_key(&fields[1])?)?))
            })?;
            Ok(imported_traits::TraitSource {
                name: string(&fields[0])?,
                text: Arc::from(string(&fields[1])?),
                names,
            })
        })?;
        let accessors = each(&layers.sig[7], |node| {
            let fields = record(node, ACCESSOR, 3)?;
            Ok((
                string(&fields[0])?,
                string(&fields[1])?,
                string(&fields[2])?,
            ))
        })?;
        let plain_accessors = each(&layers.sig[8], |node| {
            let fields = record(node, PLAIN_ACCESSOR, 2)?;
            Ok((string(&fields[0])?, string(&fields[1])?))
        })?;
        for function in &functions {
            if let Some(body) = &function.body {
                validation::body_calls(body, |callee| match callee {
                    CheckedSourceCallee::Local { function } => functions
                        .get(function as usize)
                        .map(validation::FunctionShape::of)
                        .ok_or(shape("local callee")),
                    CheckedSourceCallee::Dependency {
                        dependency,
                        function,
                    } => build
                        .dependency_shapes
                        .get(&(dependency, function))
                        .copied()
                        .ok_or(shape("dependency callee")),
                })?;
            }
        }
        for (name, at) in &function_index {
            let function = (*at as u32, validation::FunctionShape::of(&functions[*at]));
            build
                .reader
                .functions
                .insert((id.clone(), name.clone()), function);
        }
        let dependencies = std::mem::take(&mut build.dependencies);
        let declines = functions
            .iter()
            .enumerate()
            .filter_map(|(at, f)| f.decline.clone().map(|decline| (at as u32, decline)))
            .collect();
        let folds = functions
            .iter()
            .enumerate()
            .flat_map(|(at, f)| f.folds.iter().map(move |callee| (at as u32, *callee)))
            .collect();
        Ok(CheckedSourceModule {
            name: id.module.clone(),
            checked: CheckedSourceFile {
                entry,
                functions: functions
                    .iter()
                    .map(|f: &Function| f.signature.clone())
                    .collect(),
                bodies: functions
                    .iter()
                    .map(|f| f.body.as_deref().cloned())
                    .collect(),
                aggregates: build
                    .layouts
                    .iter()
                    .map(|layout| layout.schema.clone())
                    .collect(),
                instances: build.instances,
                notes,
                declines,
                folds,
            },
            layouts: build.layouts,
            functions,
            exports,
            constants,
            aliases,
            homes: homes::Homes {
                dependencies,
                layouts: build.homes,
                functions: (0..count)
                    .map(|index| (id.module.clone(), index as u32))
                    .collect(),
                imported: Vec::new(),
                artifact_callees: build.artifact_callees,
                own: count,
            },
            traits,
            accessors,
            plain_accessors,
        })
    }
}

impl Build<'_> {
    fn lookup(&self, key: &ItemKey) -> Result<usize, ArtifactError> {
        self.index
            .get(key)
            .copied()
            .ok_or(ArtifactError::UnknownItem)
    }

    fn table(&mut self, node: &Node, which: Which) -> Result<(), ArtifactError> {
        let start = self.instances.len();
        let entries = list(node)?;
        let range = (start, 0);
        match which {
            Which::Sig => self.sig_table = range,
            Which::Inst => self.inst_table = range,
        }
        for entry in entries {
            let fields = record(entry, INSTANCE, 2)?;
            let head = match &fields[0] {
                Node::Record(HEAD_NOMINAL, head) if head.len() == 1 => {
                    CheckedSourceInstanceHead::Nominal {
                        schema: self.lookup(&item_key(&head[0])?)? as u32,
                    }
                }
                Node::Record(HEAD_ARRAY, head) if head.len() == 1 => {
                    CheckedSourceInstanceHead::Array {
                        extent: small(&head[0])?,
                    }
                }
                Node::Record(HEAD_SLICE, head) if head.is_empty() => {
                    CheckedSourceInstanceHead::Slice
                }
                Node::Record(HEAD_SIZED, head) if head.is_empty() => {
                    CheckedSourceInstanceHead::Sized
                }
                Node::Record(HEAD_SEQUENCE, head) if head.is_empty() => {
                    CheckedSourceInstanceHead::Sequence
                }
                Node::Record(HEAD_POINTER, head) if head.is_empty() => {
                    CheckedSourceInstanceHead::Pointer
                }
                _ => return Err(shape("type head")),
            };
            // An argument names only entries before this one, so the table has no cycle.
            let arguments = each(&fields[1], |ty| self.ty(ty, which))?;
            self.instances
                .push(CheckedSourceInstance { head, arguments });
            let length = self.instances.len() - start;
            match which {
                Which::Sig => self.sig_table.1 = length,
                Which::Inst => self.inst_table.1 = length,
            }
        }
        Ok(())
    }

    fn instance(&self, node: &Node, which: Which) -> Result<u32, ArtifactError> {
        let (start, length) = match which {
            Which::Sig => self.sig_table,
            Which::Inst => self.inst_table,
        };
        let number = index(node)?;
        if number >= length {
            return Err(shape("type number"));
        }
        u32::try_from(start + number).map_err(|_| shape("type number"))
    }

    fn ty(&self, node: &Node, which: Which) -> Result<SourceType, ArtifactError> {
        let Node::Record(tag, fields) = node else {
            return Err(shape("type"));
        };
        let one = |what| -> Result<&Node, ArtifactError> {
            match fields.as_slice() {
                [only] => Ok(only),
                _ => Err(shape(what)),
            }
        };
        let id = |kind| -> Result<u32, ArtifactError> {
            let id = self.instance(one("type")?, which)?;
            let head = self.instances[id as usize].head;
            // The type's kind must agree with its head.
            let fits = match head {
                CheckedSourceInstanceHead::Nominal { schema } => {
                    let sum = self.layouts[schema as usize].schema.kind == AggregateKind::Sum;
                    if sum {
                        kind == TY_SUM
                    } else {
                        kind == TY_AGGREGATE
                    }
                }
                CheckedSourceInstanceHead::Array { .. }
                | CheckedSourceInstanceHead::Slice
                | CheckedSourceInstanceHead::Sized => kind == TY_ARRAY,
                CheckedSourceInstanceHead::Sequence => kind == TY_SEQUENCE,
                CheckedSourceInstanceHead::Pointer => kind == TY_POINTER,
            };
            if fits {
                Ok(id)
            } else {
                Err(shape("type kind"))
            }
        };
        Ok(match *tag {
            TY_UNIT if fields.is_empty() => SourceType::Unit,
            TY_TEXT if fields.is_empty() => SourceType::Text,
            TY_INTEGER if fields.len() == 2 => {
                let (lower, upper) = (int(&fields[0])?, int(&fields[1])?);
                if lower > upper {
                    return Err(shape("integer range"));
                }
                SourceType::Integer { lower, upper }
            }
            TY_GENERIC => SourceType::Generic {
                index: small(one("generic")?)?,
            },
            TY_AGGREGATE => SourceType::Aggregate {
                id: id(TY_AGGREGATE)?,
            },
            TY_SUM => SourceType::Sum { id: id(TY_SUM)? },
            TY_ARRAY => SourceType::Array { id: id(TY_ARRAY)? },
            TY_SEQUENCE => SourceType::Sequence {
                id: id(TY_SEQUENCE)?,
            },
            TY_POINTER => SourceType::Pointer {
                id: id(TY_POINTER)?,
            },
            _ => return Err(shape("type")),
        })
    }

    fn field(
        &self,
        node: &Node,
        which: Which,
    ) -> Result<(usize, CheckedSourceField), ArtifactError> {
        let fields = record(node, FIELD, 7)?;
        Ok((
            index(&fields[0])?,
            CheckedSourceField {
                name: string(&fields[1])?,
                ty: self.ty(&fields[2], which)?,
                private: boolean(&fields[6])?,
                register: optional(&fields[3], string)?,
                borrow: optional(&fields[4], dec_borrow)?,
                length: optional(&fields[5], small)?,
            },
        ))
    }

    fn term(&self, node: &Node, which: Which) -> Result<Term, ArtifactError> {
        let Node::Record(tag, fields) = node else {
            return Err(shape("term"));
        };
        Ok(match (*tag, fields.as_slice()) {
            (TERM_SLOT, [slot]) => Term::Slot(index(slot)?),
            (TERM_CONSTANT, [value]) => Term::Constant(int(value)?),
            (TERM_NUMBER, [number]) => Term::Number(small(number)?),
            (TERM_IDENTITY, [identity]) => Term::Identity(small(identity)?),
            (TERM_PATH, [slot, field, deeper]) => Term::Path {
                slot: index(slot)?,
                field: small(field)?,
                deeper: each(deeper, small)?,
            },
            (TERM_ARITHMETIC, [operator, left, right]) => Term::Arithmetic(
                dec_arithmetic(operator)?,
                Box::new(self.term(left, which)?),
                Box::new(self.term(right, which)?),
            ),
            (TERM_LAYOUT, [alignment, ty]) => Term::Layout {
                alignment: boolean(alignment)?,
                ty: self.ty(ty, which)?,
            },
            _ => return Err(shape("term")),
        })
    }

    fn constraints(&self, node: &Node, which: Which) -> Result<Vec<Constraint>, ArtifactError> {
        each(node, |node| {
            let fields = record(node, CONSTRAINT, 6)?;
            Ok(Constraint {
                left: index(&fields[0])?,
                left_field: optional(&fields[1], small)?,
                left_term: optional(&fields[2], |term| self.term(term, which))?,
                modulus: optional(&fields[3], int)?,
                operator: dec_binary(&fields[4])?,
                right: self.term(&fields[5], which)?,
            })
        })
    }

    /// Fill layout `position`'s schema from its node; returns its field slots, the private
    /// ones still empty when `which` is the signature layer.
    fn layout(
        &mut self,
        position: usize,
        node: &Node,
        which: Which,
    ) -> Result<Vec<Option<CheckedSourceField>>, ArtifactError> {
        let fields = record(node, LAYOUT, 18)?;
        let hidden = index(&fields[4])?;
        let total = index(&fields[5])?;
        let written = list(&fields[3])?;
        if written.len().checked_add(hidden) != Some(total) || (which == Which::Inst && hidden != 0)
        {
            return Err(shape("field count"));
        }
        self.field_slots_remaining = self
            .field_slots_remaining
            .checked_sub(total)
            .ok_or(shape("field budget"))?;
        let mut slots = vec![None; total];
        for field in written {
            let (at, field) = self.field(field, which)?;
            let slot = slots.get_mut(at).ok_or(shape("field position"))?;
            if slot.is_some() {
                return Err(shape("field given twice"));
            }
            *slot = Some(field);
        }
        let schema = CheckedSourceAggregate {
            name: string(&fields[1])?,
            parameters: each(&fields[2], string)?,
            fields: Vec::new(),
            variants: each(&fields[6], |ty| self.ty(ty, which))?,
            capability: dec_capability(&fields[7])?,
            kind: dec_kind(&fields[8])?,
            wrapping: boolean(&fields[9])?,
            defaults: each(&fields[10], |ty| optional(ty, |ty| self.ty(ty, which)))?,
            bounds: each(&fields[11], |bound| optional(bound, string))?,
            provides: each(&fields[12], string)?,
            borrowing: dec_borrowing(&fields[13])?,
            numbers: each(&fields[14], |range| {
                optional(range, |range| {
                    let (low, high) = unpair(range)?;
                    Ok((int(low)?, int(high)?))
                })
            })?,
            domain_claims: each(&fields[15], |claim| match claim {
                Node::Record(DOMAIN_COMPARE, parts) if parts.len() == 2 => Ok(
                    DomainClaim::Compare(dec_binary(&parts[0])?, int(&parts[1])?),
                ),
                Node::Record(DOMAIN_MULTIPLE, parts) if parts.len() == 1 => {
                    Ok(DomainClaim::Multiple(int(&parts[0])?))
                }
                _ => Err(shape("domain claim")),
            })?,
            discriminant: optional(&fields[16], |d| match d {
                Node::Record(DISCRIMINANT_INTEGER, parts) if parts.len() == 1 => {
                    Ok(Discriminant::Integer(int(&parts[0])?))
                }
                Node::Record(DISCRIMINANT_TEXT, parts) if parts.len() == 1 => {
                    Ok(Discriminant::Text(string(&parts[0])?))
                }
                _ => Err(shape("discriminant")),
            })?,
        };
        let constraints = self.constraints(&fields[17], which)?;
        let layout = &mut self.layouts[position];
        let (_, name) = item_key(&fields[0])?;
        layout.key = (name != schema.name).then_some(name);
        layout.schema = schema;
        layout.constraints = constraints;
        Ok(slots)
    }

    fn place(&self, node: &Node) -> Result<PlacePath, ArtifactError> {
        let fields = record(node, PLACE, 2)?;
        Ok(PlacePath {
            parameter: small(&fields[0])?,
            fields: each(&fields[1], string)?,
        })
    }

    fn clause(&self, node: &Node) -> Result<ClauseExpr, ArtifactError> {
        let Node::Record(tag, fields) = node else {
            return Err(shape("clause"));
        };
        Ok(match (*tag, fields.as_slice()) {
            (CLAUSE_CONSTANT, [value]) => ClauseExpr::Constant(int(value)?),
            (CLAUSE_PLACE, [place]) => ClauseExpr::Place(self.place(place)?),
            (CLAUSE_BINARY, [operator, left, right]) => ClauseExpr::Binary(
                dec_arithmetic(operator)?,
                Box::new(self.clause(left)?),
                Box::new(self.clause(right)?),
            ),
            (CLAUSE_SELECT, [comparison, left, right, then_branch, else_branch]) => {
                ClauseExpr::Select {
                    comparison: dec_binary(comparison)?,
                    left: Box::new(self.clause(left)?),
                    right: Box::new(self.clause(right)?),
                    then_branch: Box::new(self.clause(then_branch)?),
                    else_branch: Box::new(self.clause(else_branch)?),
                }
            }
            _ => return Err(shape("clause")),
        })
    }

    fn summary(&self, node: &Node, which: Which) -> Result<Summary, ArtifactError> {
        let fields = record(node, SUMMARY, 7)?;
        let inputs = each(&fields[0], |node| {
            let fields = record(node, INPUT, 3)?;
            Ok(Input {
                ty: self.ty(&fields[0], which)?,
                children: each(&fields[1], index)?,
                slots: each(&fields[2], small)?,
            })
        })?;
        let outputs = each(&fields[4], |node| {
            let fields = record(node, OUTPUT, 11)?;
            Ok(Output {
                ty: self.ty(&fields[0], which)?,
                lower: int(&fields[1])?,
                upper: int(&fields[2])?,
                input: optional(&fields[3], index)?,
                children: each(&fields[4], index)?,
                slots: each(&fields[5], small)?,
                sum: optional(&fields[6], |node| {
                    let id = self.instance(node, which)?;
                    validation::sum_instance(id, &self.instances, &self.layouts)?;
                    Ok(id)
                })?,
                materialized: boolean(&fields[7])?,
                structural: optional(&fields[8], |ty| self.ty(ty, which))?,
                integer_domain: optional(&fields[9], |range| {
                    let (low, high) = unpair(range)?;
                    Ok((int(low)?, int(high)?))
                })?,
                produced: boolean(&fields[10])?,
            })
        })?;
        let summary = Summary {
            inputs,
            roots: each(&fields[1], index)?,
            borrowed: each(&fields[2], boolean)?,
            dispositions: each(&fields[3], dec_disposition)?,
            outputs,
            integer_guards: each(&fields[5], |node| {
                let (input, value) = unpair(node)?;
                Ok((index(input)?, int(value)?))
            })?,
            input_ranges: each(&fields[6], |node| {
                let (input, range) = unpair(node)?;
                let (low, high) = unpair(range)?;
                Ok((index(input)?, int(low)?, int(high)?))
            })?,
        };
        validation::summary(&summary, &self.instances, &self.layouts)?;
        Ok(summary)
    }

    fn function(&self, node: &Node, which: Which) -> Result<(String, Function), ArtifactError> {
        let fields = record(node, FUNCTION, 14)?;
        let s = record(&fields[1], SIGNATURE, 12)?;
        let signature = CheckedSourceFunction {
            memory: optional(&s[0], dec_memory)?,
            provided_by: optional(&s[1], string)?,
            instruction: optional(&s[2], string)?,
            literals: each(&s[3], string)?,
            infix: optional(&s[4], dec_infix)?,
            private: boolean(&s[5])?,
            name: string(&s[6])?,
            span: SpanId::INVALID,
            parameters: each(&s[7], |ty| self.ty(ty, which))?,
            mutated: each(&s[8], small)?,
            type_parameters: each(&s[9], string)?,
            type_bounds: each(&s[10], |bound| optional(bound, string))?,
            result: self.ty(&s[11], which)?,
        };
        let parameters = each(&fields[2], |node| match node {
            Node::Record(PARAMETER_TYPE, parts) if parts.is_empty() => Ok(Parameter::Type),
            Node::Record(PARAMETER_VALUE, parts) if parts.len() == 1 => {
                Ok(Parameter::Value(self.ty(&parts[0], which)?))
            }
            Node::Record(PARAMETER_LITERAL, parts) if parts.is_empty() => Ok(Parameter::Literal),
            _ => Err(shape("parameter")),
        })?;
        let passing = each(&fields[3], dec_passing)?;
        if passing.len() != parameters.len() {
            return Err(shape("passing"));
        }
        let variant_postconditions = each(&fields[6], |node| {
            let parts = record(node, VARIANT_CLAIMS, 2)?;
            Ok((
                self.ty(&parts[0], which)?,
                self.constraints(&parts[1], which)?,
            ))
        })?;
        let variant_effects = each(&fields[7], |node| {
            let parts = record(node, VARIANT_EFFECTS, 2)?;
            let transitions = each(&parts[1], |node| {
                let parts = record(node, TRANSITION, 2)?;
                Ok((self.place(&parts[0])?, self.clause(&parts[1])?))
            })?;
            Ok((self.ty(&parts[0], which)?, transitions))
        })?;
        let effects = optional(&fields[10], |node| {
            each(node, |node| match node {
                Node::Record(EFFECT_SET, parts) if parts.len() == 2 => {
                    Ok(Effect::Set(self.place(&parts[0])?, self.clause(&parts[1])?))
                }
                Node::Record(EFFECT_FORGOTTEN, parts) if parts.len() == 1 => {
                    Ok(Effect::Forgotten(self.place(&parts[0])?))
                }
                _ => Err(shape("effect")),
            })
        })?;
        let function = Function {
            signature,
            parameters,
            passing,
            constraints: self.constraints(&fields[4], which)?,
            postconditions: self.constraints(&fields[5], which)?,
            variant_postconditions,
            variant_effects,
            implicit_types: index(&fields[8])?,
            summaries: each(&fields[9], |node| self.summary(node, which))?,
            body: None,
            effects,
            result_borrow: optional(&fields[11], dec_borrow)?,
            result_sources: each(&fields[12], |node| {
                let (argument, exclusive) = unpair(node)?;
                Ok(borrowing_results::ResultSource {
                    argument: index(argument)?,
                    exclusive: boolean(exclusive)?,
                })
            })?,
            bound_query: optional(&fields[13], |node| {
                let parts = record(node, BOUND_QUERY, 2)?;
                Ok(range_bounds::BoundQuery {
                    upper: boolean(&parts[0])?,
                    generic: index(&parts[1])?,
                })
            })?,
            decline: None,
            folds: Vec::new(),
        };
        validation::function(&function)?;
        Ok((string(&fields[0])?, function))
    }

    fn callee(
        &mut self,
        node: &Node,
        own: &HashMap<String, usize>,
    ) -> Result<CheckedSourceCallee, ArtifactError> {
        match node {
            Node::Record(CALLEE_LOCAL, parts) if parts.len() == 1 => {
                let function = *own
                    .get(&string(&parts[0])?)
                    .ok_or(ArtifactError::UnknownItem)?;
                Ok(CheckedSourceCallee::Local {
                    function: u32::try_from(function).map_err(|_| shape("callee"))?,
                })
            }
            Node::Record(CALLEE_DEPENDENCY, parts) if parts.len() == 2 => {
                let home = ModuleId::from_node(&parts[0])?;
                let name = string(&parts[1])?;
                // The home must have been read by this reader: its own index is what the callee
                // resolves to, and an unread module's numbering is unknown.
                let (function, function_shape) = *self
                    .reader
                    .functions
                    .get(&(home.clone(), name.clone()))
                    .ok_or(ArtifactError::UnknownItem)?;
                let dependency = match self
                    .dependencies
                    .iter()
                    .position(|name| *name == home.module)
                {
                    Some(slot) => slot,
                    None => {
                        self.dependencies.push(home.module.clone());
                        self.dependencies.len() - 1
                    }
                };
                let key = (dependency as u32, function);
                self.dependency_shapes.insert(key, function_shape);
                match self
                    .artifact_callees
                    .insert(key, (home.clone(), name.clone()))
                {
                    Some(previous) if previous != (home, name) => {
                        return Err(shape("ambiguous dependency callee"));
                    }
                    _ => {}
                }
                Ok(CheckedSourceCallee::Dependency {
                    dependency: dependency as u32,
                    function,
                })
            }
            _ => Err(shape("callee")),
        }
    }

    fn layout_ref(&self, node: &Node) -> Result<u32, ArtifactError> {
        Ok(self.lookup(&item_key(node)?)? as u32)
    }

    fn body(
        &mut self,
        node: &Node,
        own: &HashMap<String, usize>,
    ) -> Result<CheckedSourceBody, ArtifactError> {
        use CheckedSourceBodyKind as K;
        let fields = record(node, BODY, 4)?;
        let nodes = list(&fields[3])?;
        let length = nodes.len();
        // Every node a body names is one of its own: a forged id must not reach past it.
        let id = |node: &Node| -> Result<u32, ArtifactError> {
            let id = small(node)?;
            if (id as usize) < length {
                Ok(id)
            } else {
                Err(shape("body node"))
            }
        };
        let ids = |node: &Node| each(node, id);
        let carried = |node: &Node| {
            each(node, |node| {
                let parts = record(node, CARRY, 3)?;
                Ok(CheckedSourceLoopCarry {
                    placeholder: id(&parts[0])?,
                    initial: id(&parts[1])?,
                    next: id(&parts[2])?,
                })
            })
        };
        let mut expressions = Vec::with_capacity(length);
        for expr in nodes {
            let id = |node: &Node| -> Result<u32, ArtifactError> {
                let at = id(node)?;
                if (at as usize) < expressions.len() {
                    Ok(at)
                } else {
                    Err(shape("body postorder"))
                }
            };
            let ids = |node: &Node| each(node, id);
            let parts = record(expr, EXPR, 2)?;
            let ty = self.ty(&parts[0], Which::Inst)?;
            let Node::Record(tag, f) = &parts[1] else {
                return Err(shape("body kind"));
            };
            let arity = |count: usize| {
                if f.len() == count {
                    Ok(())
                } else {
                    Err(shape("body kind"))
                }
            };
            let kind = match *tag {
                K_RETURN => {
                    arity(1)?;
                    K::Return { value: id(&f[0])? }
                }
                K_PARAMETER => {
                    arity(1)?;
                    K::Parameter {
                        index: small(&f[0])?,
                    }
                }
                K_INTEGER => {
                    arity(1)?;
                    K::Integer(int(&f[0])?)
                }
                K_TEXT => {
                    arity(1)?;
                    K::Text(string(&f[0])?)
                }
                K_UNIT => {
                    arity(0)?;
                    K::Unit
                }
                K_HOLE => {
                    arity(0)?;
                    K::Hole
                }
                K_BINARY => {
                    arity(3)?;
                    K::Binary {
                        operator: dec_arithmetic(&f[0])?,
                        left: id(&f[1])?,
                        right: id(&f[2])?,
                    }
                }
                K_WRAP => {
                    arity(1)?;
                    K::Wrap {
                        operand: id(&f[0])?,
                    }
                }
                K_CONSTRUCT => {
                    arity(2)?;
                    K::Construct {
                        schema: self.layout_ref(&f[0])?,
                        fields: ids(&f[1])?,
                    }
                }
                K_MATCH => {
                    arity(3)?;
                    let arms = each(&f[1], |node| {
                        let parts = record(node, ARM, 2)?;
                        Ok(CheckedSourceMatchArm {
                            schema: self.layout_ref(&parts[0])?,
                            body: id(&parts[1])?,
                        })
                    })?;
                    let region = optional(&f[2], |node| {
                        let parts = record(node, MATCH_REGION, 3)?;
                        Ok(MatchRegion {
                            member_body: id(&parts[0])?,
                            member_start: small(&parts[1])?,
                            other_start: small(&parts[2])?,
                        })
                    })?;
                    K::Match {
                        subject: id(&f[0])?,
                        arms,
                        region,
                    }
                }
                K_FIELD => {
                    arity(2)?;
                    K::Field {
                        record: id(&f[0])?,
                        index: small(&f[1])?,
                    }
                }
                K_VARIANT_FIELD => {
                    arity(3)?;
                    K::VariantField {
                        subject: id(&f[0])?,
                        schema: self.layout_ref(&f[1])?,
                        index: small(&f[2])?,
                    }
                }
                K_CALL => {
                    arity(3)?;
                    let callee = self.callee(&f[0], own)?;
                    K::Call {
                        callee,
                        arguments: ids(&f[1])?,
                        type_arguments: each(&f[2], |ty| self.ty(ty, Which::Inst))?,
                    }
                }
                K_BOUND_CALL => {
                    arity(4)?;
                    K::BoundCall {
                        generic: small(&f[0])?,
                        method: string(&f[1])?,
                        trait_arguments: each(&f[2], |ty| self.ty(ty, Which::Inst))?,
                        arguments: ids(&f[3])?,
                    }
                }
                K_SELECT => {
                    arity(6)?;
                    K::Select {
                        comparison: dec_infix(&f[0])?,
                        left: id(&f[1])?,
                        right: id(&f[2])?,
                        then_branch: id(&f[3])?,
                        else_branch: id(&f[4])?,
                        region: optional(&f[5], |node| {
                            let parts = record(node, SELECT_REGION, 3)?;
                            Ok(SelectRegion {
                                then_start: small(&parts[0])?,
                                else_start: small(&parts[1])?,
                                exclusive: boolean(&parts[2])?,
                            })
                        })?,
                    }
                }
                K_INSTRUCTION => {
                    arity(3)?;
                    let symbols = each(&f[1], |node| {
                        let (a, b) = unpair(node)?;
                        Ok((string(a)?, string(b)?))
                    })?;
                    K::Instruction {
                        text: string(&f[0])?,
                        symbols,
                        input: id(&f[2])?,
                    }
                }
                K_LOAD => {
                    arity(2)?;
                    K::Load {
                        pointer: id(&f[0])?,
                        index: id(&f[1])?,
                    }
                }
                K_STORE => {
                    arity(3)?;
                    K::Store {
                        pointer: id(&f[0])?,
                        index: id(&f[1])?,
                        value: id(&f[2])?,
                    }
                }
                K_WIDEN => {
                    arity(1)?;
                    K::Widen {
                        operand: id(&f[0])?,
                    }
                }
                K_LOOP => {
                    arity(5)?;
                    K::Loop {
                        start: id(&f[0])?,
                        end: id(&f[1])?,
                        counter: id(&f[2])?,
                        body_start: small(&f[3])?,
                        carried: carried(&f[4])?,
                    }
                }
                K_WHILE => {
                    arity(8)?;
                    K::While {
                        comparison: dec_infix(&f[0])?,
                        negated: boolean(&f[1])?,
                        left: id(&f[2])?,
                        right: id(&f[3])?,
                        body_start: small(&f[4])?,
                        split: small(&f[5])?,
                        carried: carried(&f[6])?,
                        exit: optional(&f[7], small)?,
                    }
                }
                K_CALL_OUT => {
                    arity(2)?;
                    K::CallOut {
                        call: id(&f[0])?,
                        index: small(&f[1])?,
                    }
                }
                K_SUB_ARRAY => {
                    arity(2)?;
                    K::SubArray {
                        base: id(&f[0])?,
                        start: id(&f[1])?,
                    }
                }
                K_REPEAT_ARRAY => {
                    arity(2)?;
                    K::RepeatArray {
                        element: id(&f[0])?,
                        count: id(&f[1])?,
                    }
                }
                K_TEXT_BYTES => {
                    arity(1)?;
                    K::TextBytes {
                        text: string(&f[0])?,
                    }
                }
                K_LAYOUT => {
                    arity(2)?;
                    K::Layout {
                        queried: self.ty(&f[0], Which::Inst)?,
                        alignment: boolean(&f[1])?,
                    }
                }
                K_LOOP_COUNTER => {
                    arity(0)?;
                    K::LoopCounter
                }
                K_LOOP_CARRIED => {
                    arity(0)?;
                    K::LoopCarried
                }
                K_LOOP_RESULT => {
                    arity(2)?;
                    K::LoopResult {
                        looped: id(&f[0])?,
                        index: small(&f[1])?,
                    }
                }
                K_BLOCK => {
                    arity(2)?;
                    K::Block {
                        statements: ids(&f[0])?,
                        result: id(&f[1])?,
                    }
                }
                K_BUNDLE => {
                    arity(1)?;
                    K::Bundle { parts: ids(&f[0])? }
                }
                K_PART => {
                    arity(2)?;
                    K::Part {
                        bundle: id(&f[0])?,
                        index: small(&f[1])?,
                    }
                }
                K_UNDEFINED => {
                    arity(0)?;
                    K::Undefined
                }
                _ => return Err(shape("body kind")),
            };
            validation::body_node(&kind, &expressions, &self.layouts, &self.instances)?;
            expressions.push(CheckedSourceBodyExpr {
                span: SpanId::INVALID,
                ty,
                kind,
            });
        }
        Ok(CheckedSourceBody {
            effects: boolean(&fields[0])?,
            root: id(&fields[1])?,
            outputs: ids(&fields[2])?,
            expressions,
        })
    }
}

fn decode_note(node: &Node) -> Result<SourceCheckNote, ArtifactError> {
    let parts = record(node, NOTE, 3)?;
    let facts = each(&parts[2], |node| match node {
        Node::Record(FACT_INTERVAL, f) if f.len() == 3 => Ok(SourceFact::Interval {
            name: string(&f[0])?,
            lower: int(&f[1])?,
            upper: int(&f[2])?,
        }),
        Node::Record(FACT_KNOWN, f) if f.len() == 2 => Ok(SourceFact::Known {
            name: string(&f[0])?,
            value: int(&f[1])?,
        }),
        Node::Record(FACT_MULTIPLE, f) if f.len() == 2 => Ok(SourceFact::Multiple {
            name: string(&f[0])?,
            modulus: int(&f[1])?,
        }),
        Node::Record(FACT_OFFSET, f) if f.len() == 3 => Ok(SourceFact::Offset {
            name: string(&f[0])?,
            base: string(&f[1])?,
            offset: int(&f[2])?,
        }),
        Node::Record(FACT_RELATION, f) if f.len() == 3 => Ok(SourceFact::Relation {
            left: string(&f[0])?,
            operator: relation(&string(&f[1])?)?,
            right: string(&f[2])?,
        }),
        _ => Err(shape("fact")),
    })?;
    Ok(SourceCheckNote {
        kind: dec_note(&parts[0])?,
        span: None,
        file: None,
        detail: string(&parts[1])?,
        facts,
    })
}

mod compat;
mod validation;
pub use compat::{Classification, Compatibility, ItemChange, classify_artifacts, classify_targets};

#[cfg(test)]
mod tests;
