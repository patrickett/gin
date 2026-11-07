//! Direct, fail-closed source checker. No legacy AST or declaration interpretation is used.
use i256::I256;
use ast::*;
use std::rc::Rc;

mod aggregates;
mod artifact;
pub mod regions;
mod aliases;
mod arrays;
mod array_repeat;
mod assign;
mod body;
mod claim_arithmetic;
mod calls;
mod constants;
mod contracts;
mod diagnostics;
mod drain_loops;
mod element_writes;
mod eval;
mod borrows;
mod borrowing_types;
mod borrowing_results;
mod extents;
mod homes;
mod inference;
mod imported_traits;
mod variant_conditionals;
mod case_guards;
mod chains;
mod guards;
mod joins;
mod labeled_arrays;
mod loans;
mod integer_ops;
mod integers;
pub mod layout_queries;
mod linear;
mod loops;
mod modular;
mod open_numbers;
mod modules;
mod numbers;
mod postconditions;
mod range_bounds;
mod trait_results;
mod raw_pointers;
mod reconstruction;
mod returns;
mod schema_annotations;
mod schema_syntax;
mod schema_validation;
mod sequences;
mod state;
mod summaries;
mod sums;
#[cfg(test)]
mod tests;

use raw_pointers::{is_unchecked, unchecked_operand};
use state::{ModuleState, ProofState, ReplayState};

pub use body::{
    CheckedArithmetic, CheckedSourceBody, CheckedSourceBodyExpr, CheckedSourceBodyKind,
    CheckedSourceCallee, CheckedSourceLoopCarry, CheckedSourceMatchArm, InfixOperator, MatchRegion,
    SelectRegion,
};
pub use diagnostics::{
    SourceCheckFailure, SourceCheckNote, SourceCheckNoteKind, SourceFact, check_file_diagnostic,
};
pub use eval::{
    BodyResolver, Evaluated, Exact, FOLD_DEPTH, apply, compare, evaluate_body, evaluate_value, evaluate_value_with, exact_binary, interval, wrap_into,
};
pub use artifact::{
    ArtifactError, ArtifactIdentity, ArtifactReader, Classification, Compatibility, ItemChange, artifact_tag_name, classify_artifacts,
    classify_targets, dump_artifact, write_artifact,
};
pub use modules::{
    CheckedSourceModule, check_module, check_module_diagnostic, check_module_files,
    check_module_files_diagnostic,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCheckError {
    Budget,
    Malformed,
    Unsupported,
    Duplicate,
    UnknownName,
    MissingBody,
    Type,
    Unproved,
    Overflow,
    UseAfterMove,
    Unconsumed,
    Privacy,
    /// `#wrap` marks something that is not a named integer domain.
    /// `#instruction` or `#register` marks the wrong thing, or a text it uses is not a
    /// plain name.
    InstructionTarget,
    WrapTarget,
    /// `#unrestricted` marks something that is not a non-generic domain or record.
    UnrestrictedTarget,
    /// An `#unrestricted` record has a field that is not itself unrestricted.
    UnrestrictedField,
    /// A parameterised `has` member must be a method with `self` as its first parameter.
    MethodReceiver,
    /// A linear domain has no method that takes `self` and ends it.
    MissingTerminal,
    /// A linear type was passed to an `#unrestricted` generic type, which may only hold
    /// unrestricted things.
    UnrestrictedArgument,
    /// `#infix` names an unsupported operator or sits on a method that is not
    /// `name(self, rhs ...)`.
    InfixOperator,
    /// A `ref` or `mut` borrow was moved, ended, or returned, which only its owner may do.
    MoveOutOfBorrow,
    /// An `end` input was returned or otherwise escaped its ending boundary.
    EndEscape,
    /// Borrows or moves of one value conflict within a call, or shared access was
    /// lent exclusively.
    BorrowConflict,
    /// A `::` assignment names a place that is not a mutable binding: a `ref` borrow, an
    /// owned parameter, or a `:=` local.
    Immutable,
    /// A `::` assignment would replace a linear value, silently dropping it.
    OverwriteLinear,
    /// A borrow outlives its source: a view of a local is returned, or stored in a place that
    /// outlives it.
    BorrowEscape,
    /// An unchecked operation (a number cast to a pointer, a pointer cast to a number or
    /// another pointer, `place as @T`, `@place`) is not inside `#unchecked(...)`.
    MissingAcceptance,
    /// `#unchecked(...)` surrounds no operation that skips a proof, so accepts nothing.
    UnneededAcceptance,
    /// A function's callee outcomes multiply past `sums::WORLDS_MAX` combinations.
    WorldLimit,
}

impl Capability {
    pub(super) fn from_marker(unrestricted: bool) -> Self {
        if unrestricted {
            Self::Unrestricted
        } else {
            Self::Linear
        }
    }
}

impl AggregateKind {
    pub(super) fn of(declaration: &TypeDeclaration) -> Self {
        match declaration.body.value {
            TypeBody::Domain(_) => Self::Domain,
            TypeBody::Is(_) => Self::Sum,
            TypeBody::Has(_) => Self::Record,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSourceFile {
    /// Index into `functions` of the `#entry` function, when one is declared.
    pub entry: Option<u32>,
    pub functions: Vec<CheckedSourceFunction>,
    /// Executable bodies parallel to `functions`; `None` when a function uses a construct
    /// that is not recorded yet, so consumers fail closed instead of guessing.
    pub bodies: Vec<Option<CheckedSourceBody>>,
    pub aggregates: Vec<CheckedSourceAggregate>,
    pub instances: Vec<CheckedSourceInstance>,
    /// Non-fatal findings, in source order of discovery.
    pub notes: Vec<SourceCheckNote>,
    /// For each function (by index) whose body is `None`, why the recorder declined it. A
    /// function with no entry has none because it has no body to record (an instruction, or a
    /// bodiless trait method).
    pub declines: Vec<(u32, BodyDecline)>,
    /// Calls the recorder replaced by their value (the compile-time evaluation rule), as
    /// (calling function, callee): the dependency a recorded body no longer shows.
    pub folds: Vec<(u32, CheckedSourceCallee)>,
}

/// Why the recorder declined a body: the first construct it could not place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyDecline {
    pub kind: DeclineKind,
    /// The construct in words: `division `/``, `a loop`, `an assignment`.
    pub construct: String,
    pub span: Option<Span>,
    /// Index into the checked file list; present exactly when `span` is.
    pub file: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclineKind {
    /// An operator the recorder does not record (division, bit operations).
    Operator,
    /// An expression form it does not place.
    Expression,
    /// A statement form it does not place.
    Statement,
    /// A domain conversion at a site it skips.
    Conversion,
    /// Declined, but at no construct it could name.
    Unplaced,
}

/// Nominal declarations and applications are local to this checked source snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSourceAggregate {
    pub name: String,
    pub parameters: Vec<String>,
    pub fields: Vec<CheckedSourceField>,
    /// Empty for records; otherwise the distinct nominal alternatives of a sum alias.
    pub variants: Vec<SourceType>,
    /// Whether values may be copied and discarded. Only a written `#unrestricted`, whose
    /// fields the checker has verified, grants it; size and representation never do.
    pub capability: Capability,
    pub kind: AggregateKind,
    /// `#wrap` on a domain: a value outside its range converts to the congruent one
    /// inside it instead of being rejected.
    pub wrapping: bool,
    /// Beside each parameter, the type used when an application omits it (`alloc: Global`).
    /// Empty when no parameter has one.
    pub defaults: Vec<Option<SourceType>>,
    /// Beside each parameter, the trait an argument must provide (`element has Release`).
    /// Empty when no parameter has a bound.
    pub bounds: Vec<Option<String>>,
    /// The traits this type lists (`Type has Trait`), by name.
    pub provides: Vec<String>,
    /// Which borrows its own fields hold (`data ref T`, `data mut T`). A record with any is a
    /// *borrowing type*, a view: it owns nothing it borrows and needs no `release`.
    pub borrowing: Borrowing,
    /// Beside each parameter, the range of numbers it accepts when it is a compile-time
    /// number parameter (`capacity Length`), else `None`. Empty when no parameter is a number.
    pub numbers: Vec<Option<(I256, I256)>>,
    /// What a domain's declaration adds to its range (`and self % 4096 = 0`): every value
    /// converted into the domain must be shown to satisfy each.
    pub domain_claims: Vec<DomainClaim>,
    /// For a variant of a union that gives its variants values (`Debug = 'debug'`), this
    /// variant's value. `None` otherwise.
    pub discriminant: Option<Discriminant>,
}

/// The literal a variant is encoded as (`Success = 0`, `Debug = 'debug'`). An integer is the
/// variant's tag in memory; a text is data read from a table indexed by the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discriminant {
    Integer(I256),
    Text(String),
}

/// One refinement of a domain declaration, with its constants worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainClaim {
    /// `self <operator> number`.
    Compare(BinaryOperator, I256),
    /// `self % modulus = 0`.
    Multiple(I256),
}

/// The access a `ref` or `mut` field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    /// `ref T`: shared, read-only.
    Shared,
    /// `mut T`: exclusive, may write.
    Exclusive,
}

/// What a record's own fields borrow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Borrowing {
    /// No `ref`/`mut` field: an ordinary value.
    #[default]
    None,
    Shared,
    Exclusive,
    /// Both `ref` and `mut` fields.
    Mixed,
}

/// What a value of a type may do. A type is linear unless it opts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Must be consumed exactly once: no copying, no silent discarding.
    Linear,
    /// May be copied and discarded freely.
    Unrestricted,
}

/// How a nominal type is built, independently of what it may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateKind {
    Record,
    /// A nominal alternation of other types.
    Sum,
    /// A named integer range: one hidden integer field and no written fields.
    Domain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSourceField {
    pub name: String,
    pub ty: SourceType,
    pub private: bool,
    /// `#register('x0')`: the machine register this field occupies at an instruction.
    pub register: Option<String>,
    /// `data ref T` / `data mut T`: the field holds a borrow of a `T` place, not a `T` value.
    /// `ty` stays `T`.
    pub borrow: Option<BorrowKind>,
    /// `data [element; len]`: the index of the sibling field `len` that is the array's length.
    /// The type is a `Slice`; each value proves its own extent equals `len`.
    pub length: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSourceInstance {
    pub head: CheckedSourceInstanceHead,
    pub arguments: Vec<SourceType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckedSourceInstanceHead {
    Nominal { schema: u32 },
    Array { extent: u32 },
    /// An array whose extent is a property of each value, not of its type: `[element; len]`
    /// with `len` a sibling field, or a `.[0; n]` slice. Its length is tracked and proved per
    /// value (`Owner::extent`).
    Slice,
    /// `[element; capacity]` with `capacity` a number parameter not yet applied: the second
    /// argument is that parameter (`Generic`). Applying it to a number turns this into
    /// `Array { extent }`; see `numbers.rs`.
    Sized,
    Sequence,
    Pointer,
}

/// What a `#load` or `#store` function is: a typed read or write of memory at a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOp {
    /// `load(element Type, pointer @element, index N) element`.
    Load,
    /// `store(element Type, pointer @element, index N, value element)`.
    Store,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSourceFunction {
    /// `#load` or `#store`: this function has no body and is that memory operation.
    pub memory: Option<MemoryOp>,
    /// The trait this method came from, when a provided trait added it to its type rather than
    /// the type writing it.
    pub provided_by: Option<String>,
    /// `#instruction('bl $target')`: this function has no body and is that machine
    /// instruction. `$name` places the compile-time text a call gives for `name`.
    pub instruction: Option<String>,
    /// The unannotated parameters of an instruction function, in order: each is a symbol
    /// supplied as compile-time text at every call.
    pub literals: Vec<String>,
    /// The operator a `#infix` method implements; `a + b` dispatches here when the left
    /// operand has the type of the first parameter.
    pub infix: Option<InfixOperator>,
    /// A `#private` method: callable only inside the module that declares its type, so it is
    /// not exported with the type.
    pub private: bool,
    pub name: String,
    pub span: SpanId,
    pub parameters: Vec<SourceType>,
    /// The `mut` parameters, by index into `parameters`. A body hands back the value each
    /// holds when the function returns, after its result, in this order; a call gives
    /// each back to the place it was lent from.
    pub mutated: Vec<u32>,
    pub type_parameters: Vec<String>,
    /// Beside each type parameter, the trait an argument must provide; empty when none has one.
    pub type_bounds: Vec<Option<String>>,
    pub result: SourceType,
}

/// The contents of a single-quoted literal as written. A quote or a backslash inside is not
/// supported, so what is kept is exactly what is between the quotes.
/// A name safe to hand to the machine as an operand: nothing in it can add a constraint, a
/// second operand or a directive.
pub(crate) fn plain_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A link symbol named by compile-time text: letters, digits, `_`, and `.` so that a Gin
/// function's qualified name is one.
pub(crate) fn symbol_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
}

/// The names an `#instruction` text places with `$name`, in order of first use. `None` when a
/// `$` is anything else: it is how the machine counts operands, so no other use is allowed.
pub(crate) fn placeholders(text: &str) -> Option<Vec<&str>> {
    let mut names: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        rest = &rest[at + 1..];
        let end = rest
            .bytes()
            .position(|byte| !(byte.is_ascii_alphanumeric() || byte == b'_'))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if name.is_empty() || name.as_bytes()[0].is_ascii_digit() {
            return None;
        }
        if !names.contains(&name) {
            names.push(name);
        }
        rest = &rest[end..];
    }
    Some(names)
}

pub(crate) fn quoted(written: &str) -> Option<&str> {
    // Single quotes, or double quotes (a format string with nothing to interpolate). Either
    // way the text holds no quote of its own kind and no escape.
    if let Some(text) = written.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')) {
        return (!text.contains(['\'', '\\'])).then_some(text);
    }
    let text = written.strip_prefix('"')?.strip_suffix('"')?;
    (!text.contains(['"', '\\'])).then_some(text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceType {
    Unit,
    /// Text known at compile time. It has no name in source, so it cannot be annotated,
    /// passed or stored: it is only ever the value of a string literal, or of a function
    /// whose result is inferred to be one. It owns nothing and has no machine form.
    Text,
    Integer {
        lower: I256,
        upper: I256,
    },
    /// Index into the enclosing function or aggregate's type parameter list.
    Generic {
        index: u32,
    },
    /// Index into this checked file's instance arena, interpreted in that binder scope.
    Aggregate {
        id: u32,
    },
    /// A finite nominal sum application in the same instance arena as aggregates.
    Sum {
        id: u32,
    },
    /// Structural fixed-extent array application in the shared instance arena.
    Array {
        id: u32,
    },
    /// Structural heterogeneous positions in the shared instance arena.
    Sequence {
        id: u32,
    },
    /// Non-owning typed address. The single arena argument is not a layout child.
    Pointer {
        id: u32,
    },
}

#[derive(Clone, Copy)]
struct Value {
    lower: I256,
    upper: I256,
    identity: Option<u32>,
    ty: SourceType,
    owner: Option<usize>,
    available: bool,
    /// Static coverage view, independent of an owner's lazily selected active variant.
    sum: Option<u32>,
    /// Declared recursive positional view, independent of the original actual shape.
    structural: Option<SourceType>,
    /// Coverage uses the declared domain, independently of current scalar facts.
    integer_domain: Option<(I256, I256)>,
    /// `(base, offset)`: this number is the value `base` (an input's identity) held, plus
    /// `offset`. It is what lets a contract say `x :: x + 1` and have it proved exactly.
    affine: Option<(u32, I256)>,
    /// When this value is a handle (`ref place` / `mut place` in value position), the loan it
    /// holds on the borrowed place. See `loans.rs`.
    borrow: Option<loans::LoanRef>,
}

impl Value {
    fn unit() -> Self {
        Self {
            lower: I256::ZERO,
            upper: I256::ZERO,
            identity: None,
            ty: SourceType::Unit,
            owner: None,
            available: true,
            sum: None,
            structural: None,
            integer_domain: None,
            affine: None,
            borrow: None,
        }
    }

    fn integer(lower: I256, upper: I256) -> Self {
        assert!(lower <= upper);
        Self {
            lower,
            upper,
            identity: None,
            ty: SourceType::Integer { lower, upper },
            owner: None,
            available: true,
            sum: None,
            structural: None,
            integer_domain: Some((lower, upper)),
            affine: None,
            borrow: None,
        }
    }
}

type SymbolCache = Vec<((usize, usize, bool), Option<usize>)>;

/// A byte range of one file of the module being checked.
#[derive(Clone, Copy)]
struct FileRange {
    file: u32,
    start: usize,
    end: usize,
}

struct Checker<'a> {
    files: &'a [SourceFile],
    /// The body recorded for the function being checked; `None` until recorded.
    body: Option<CheckedSourceBody>,
    /// A domain conversion happened in the body being checked; its recording would not
    /// show it, so no body is recorded.
    conversions_checked: u32,
    /// Domain conversions the recorder reproduced; a body is recorded only when it matches
    /// what the checker did, so a conversion at a site the recorder skips fails closed.
    conversions_recorded: u32,
    /// Conversions the checker counted in every world of the function so far, and the recording
    /// a world made that no single world's count matched: the worlds together may.
    conversions_all_worlds: u32,
    /// Each conversion a world made, keyed by the branch choices it had made by then (not which
    /// outcome a callee gave) and its place among the world's conversions: worlds that share a
    /// stretch of path make the same conversions there, and the recording holds each once.
    conversion_events: Vec<(Option<SpanId>, u32)>,
    /// The conversions the current world has made at each site so far.
    world_sites: Vec<(Option<SpanId>, u32)>,
    unmatched_recording: Option<(CheckedSourceBody, u32)>,
    /// The result type of the function being recorded, which types its returned values.
    recording_result: Option<SourceType>,
    /// Whether the body being recorded reaches an instruction.
    recording_effects: bool,
    /// The `mut` parameters of the function being recorded, in order, and the node each
    /// holds where the body first returns.
    recording_mutated: Vec<&'a str>,
    /// The place a method call's receiver was written as, while that call is recorded.
    recording_receiver: Option<&'a Expression>,
    recording_outputs: Option<Vec<u32>>,
    /// Every exit of the function's block returns a `Bundle` of its result and its `mut`
    /// outputs; set when a first recording found exits that
    /// hand back different outputs (`recording_outputs_differ`).
    recording_bundle: bool,
    /// Recording a statement followed only by a bare `return` of a unit function: a nested
    /// block ending in a bare `return` there finishes as the function would, with nothing.
    recording_unit_tail: bool,
    /// The arm blocks of a `when` that is a function's returned value: a `return` ending one is
    /// that arm's value, not an early exit.
    recording_tail_blocks: Vec<SpanId>,
    recording_outputs_differ: bool,
    /// The declared type of each `mut` parameter, in `recording_mutated` order.
    recording_output_types: Vec<SourceType>,
    /// A flaw found while recording that the checker could not see: an `#instruction`
    /// target that is not compile-time text. Recording otherwise just declines.
    recording_failure: Option<SourceCheckError>,
    /// What `#target` reports.
    target: crate::Target,
    remaining: u32,
    flow_remaining: u32,
    /// The culprit of the most recent explained error, kept with the error it describes.
    detail: Option<(SourceCheckError, String)>,
    /// The function whose worlds are being enumerated, for naming it in `WorldLimit`.
    world_function: Option<&'a str>,
    /// Worlds started for that function, and the index of the choice the latest one advanced.
    world_count: usize,
    world_advanced: usize,
    /// The expression most recently visited, for locating a failure exactly (`failure.culprit`).
    culprit: Option<SpanId>,
    /// The first construct the recorder could not place in the body being recorded.
    recording_decline: Option<BodyDecline>,
    /// The statement being recorded, for a decline no expression explains.
    recording_at: Option<SpanId>,
    /// The decline and the folded calls of the function being checked, kept until it is published.
    recorded_decline: Option<BodyDecline>,
    recorded_folds: Vec<CheckedSourceCallee>,
    recording_folds: Vec<CheckedSourceCallee>,
    /// Compile-time queries (`#size(T)`, `#alignment(T)`, `#lower(r)`, `#upper(r)`) the checker
    /// settled to one number, by the expression's span, for the recorder to emit as constants.
    query_values: Vec<(SpanId, I256)>,
    /// `#unchecked(...)` markers in scope: the span of the operand of each, and whether
    /// an unchecked operation used it.
    acceptances: Vec<(SpanId, bool)>,
    /// Inline nullary accessors a claim may name: (type, method, the field it returns).
    accessors: Vec<(String, String, String)>,
    /// The fields of the type whose claims are being compiled, and its schema index.
    claim_fields: Vec<&'a Binding>,
    /// Accessors of this module's types that are exactly `return self.field`.
    plain_accessors: Vec<(String, String)>,
    claim_schema: Option<usize>,
    parameters_max: u32,
    definitions_max: u32,
    module: ModuleState<'a>,
    proof: ProofState<'a>,
    replay: ReplayState,
}

type Result<T> = std::result::Result<T, SourceCheckError>;

/// The literal's value as a non-positive accumulator, so the most negative integer is
/// representable. The caller negates it for a non-negative literal.
fn decode_integer_accumulator(text: &str) -> Result<I256> {
    let (digits, radix) = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => (digits, 16),
        None => (text, 10),
    };
    let mut value = I256::ZERO;
    let mut previous_digit = false;
    for byte in digits.bytes() {
        if byte == b'_' {
            if !previous_digit {
                return Err(SourceCheckError::Malformed);
            }
            previous_digit = false;
            continue;
        }
        let digit = (byte as char)
            .to_digit(radix)
            .ok_or(SourceCheckError::Malformed)?;
        value = value
            .checked_mul(I256::from(radix))
            .and_then(|v| v.checked_sub(I256::from(digit)))
            .ok_or(SourceCheckError::Overflow)?;
        previous_digit = true;
    }
    if !previous_digit {
        return Err(SourceCheckError::Malformed);
    }
    Ok(value)
}

/// Check one standalone snapshot using the same engine as [`check_module`].
/// The returned inspection data is not a trusted dependency interface.
pub fn check_file(source: &SourceFile, limits: crate::Limits) -> Result<CheckedSourceFile> {
    check_file_diagnostic(source, limits).map_err(|failure| failure.error)
}

impl<'a> Checker<'a> {
    #[cfg(test)]
    fn new(source: &'a SourceFile, limits: crate::Limits) -> Self {
        Self::new_module(std::slice::from_ref(source), limits)
    }

    fn new_module(files: &'a [SourceFile], limits: crate::Limits) -> Self {
        Self {
            files,
            body: None,
            conversions_checked: 0,
            conversions_recorded: 0,
            conversions_all_worlds: 0,
            conversion_events: Vec::new(),
            world_sites: Vec::new(),
            unmatched_recording: None,
            recording_result: None,
            recording_effects: false,
            recording_mutated: Vec::new(),
            recording_receiver: None,
            recording_outputs: None,
            recording_bundle: false,
            recording_unit_tail: false,
            recording_tail_blocks: Vec::new(),
            recording_outputs_differ: false,
            recording_output_types: Vec::new(),
            recording_failure: None,
            detail: None,
            world_function: None,
            world_count: 0,
            world_advanced: usize::MAX,
            culprit: None,
            acceptances: Vec::new(),
            accessors: Vec::new(),
            claim_fields: Vec::new(),
            plain_accessors: Vec::new(),
            claim_schema: None,
            recording_decline: None,
            recording_at: None,
            recorded_decline: None,
            recorded_folds: Vec::new(),
            recording_folds: Vec::new(),
            query_values: Vec::new(),
            target: limits.target,
            remaining: limits.work_max,
            flow_remaining: limits.flow_work_max,
            parameters_max: limits.parameters_max,
            definitions_max: limits.definitions_max,
            module: ModuleState::default(),
            proof: ProofState::default(),
            replay: ReplayState::default(),
        }
    }
}

impl<'a> Checker<'a> {
    fn binding_spans(&mut self, binding: &'a Binding, parent: SpanId) -> Result<()> {
        self.name(&binding.name)?;
        self.child(parent, binding.name.span_id)?;
        if let Some(parameters) = &binding.parameters {
            self.child(parent, parameters.span_id)?;
        }
        if let Some(annotation) = &binding.annotation {
            self.child(parent, annotation.span_id)?;
        }
        if let Some(body) = &binding.value {
            self.child(parent, body.expression.span_id)?;
            self.child(parent, body.operator.span_id)?;
        }
        if let Some(contract) = &binding.contract {
            self.child(parent, contract.span_id)?;
        }
        for alternative in &binding.alternatives {
            self.child(parent, alternative.span_id)?;
            self.span(alternative.span_id)?;
            self.keyword(alternative.value.or_keyword, "or")?;
            self.child(alternative.span_id, alternative.value.or_keyword)?;
            self.child(alternative.span_id, alternative.value.annotation.span_id)?;
        }
        Ok(())
    }

    fn charge(&mut self, amount: usize) -> Result<()> {
        prepay(&mut self.remaining, amount)
    }

    fn charge_product(&mut self, count: usize, width: usize) -> Result<()> {
        self.charge(count.checked_mul(width).ok_or(SourceCheckError::Budget)?)
    }

    fn charge_flow(&mut self) -> Result<()> {
        self.flow_remaining = self
            .flow_remaining
            .checked_sub(1)
            .ok_or(SourceCheckError::Budget)?;
        Ok(())
    }

    fn span(&mut self, id: SpanId) -> Result<&'a str> {
        self.charge(1)?;
        self.text_of(id).ok_or(SourceCheckError::Malformed)
    }

    /// The span of `id` in the table of the file that issued it. Empty when unknown.
    fn span_of(&self, id: SpanId) -> Span {
        self.files
            .get(id.file() as usize)
            .map_or(Span::new(0, 0), |file| file.span_table.get(id))
    }

    /// The written text of `id`, or `None` when its span is empty or outside its file.
    fn text_of(&self, id: SpanId) -> Option<&'a str> {
        let files = self.files;
        let file = files.get(id.file() as usize)?;
        let span = file.span_table.get(id);
        if span.is_empty() {
            return None;
        }
        file.text.get(span.to_range())
    }

    fn range_of(&self, id: SpanId) -> FileRange {
        let span = self.span_of(id);
        FileRange {
            file: id.file(),
            start: span.start(),
            end: span.end(),
        }
    }

    fn name(&mut self, name: &'a Name) -> Result<&'a str> {
        let text = self.span(name.span_id)?;
        self.charge(text.len())?;
        if text != name.value
            || text.is_empty()
            || (!text.as_bytes()[0].is_ascii_alphabetic() && text.as_bytes()[0] != b'_')
            || !text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(SourceCheckError::Malformed);
        }
        Ok(&name.value)
    }

    fn child(&mut self, parent: SpanId, child: SpanId) -> Result<()> {
        self.span(parent)?;
        self.span(child)?;
        if parent.file() != child.file() {
            return Err(SourceCheckError::Malformed);
        }
        let parent = self.span_of(parent);
        let child = self.span_of(child);
        if child.start() < parent.start() || child.end() > parent.end() {
            return Err(SourceCheckError::Malformed);
        }
        Ok(())
    }

    fn token(&mut self, id: SpanId, expected: &str) -> Result<()> {
        let actual = self.span(id)?;
        self.charge(expected.len())?;
        if actual != expected {
            return Err(SourceCheckError::Malformed);
        }
        Ok(())
    }

    fn keyword(&mut self, id: SpanId, expected: &str) -> Result<()> {
        self.token(id, expected)?;
        let range = self.range_of(id);
        self.keyword_boundaries(range)
    }

    fn keyword_boundaries(&mut self, range: FileRange) -> Result<()> {
        self.charge(2)?;
        let text = self
            .files
            .get(range.file as usize)
            .ok_or(SourceCheckError::Malformed)?
            .text
            .as_bytes();
        for neighbor in [range.start.checked_sub(1), Some(range.end)]
            .into_iter()
            .flatten()
        {
            if text
                .get(neighbor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                return Err(SourceCheckError::Malformed);
            }
        }
        Ok(())
    }

    fn comparison(&mut self, operator: &Spanned<BinaryOperator>) -> Result<()> {
        let expected = match operator.value {
            BinaryOperator::Equal => "=",
            BinaryOperator::Less => "<",
            BinaryOperator::LessOrEqual => "<=",
            BinaryOperator::Greater => ">",
            BinaryOperator::GreaterOrEqual => ">=",
            BinaryOperator::And => "and",
            _ => return Err(SourceCheckError::Unsupported),
        };
        self.token(operator.span_id, expected)
    }

    fn plain(&mut self, binding: &Binding) -> Result<()> {
        self.plain_with(binding, false)
    }

    /// A function header may name its result, for the claims of its `where` clause; a local may not.
    fn plain_with(&mut self, binding: &Binding, result_binder: bool) -> Result<()> {
        // Only a method may carry attributes, and only `#infix` and `#private`, validated at
        // declaration.
        let attributes_ok = binding.attributes.is_empty()
            || (self.proof.receiver.is_some()
                && binding.attributes.iter().all(|attribute| {
                    matches!(
                        attribute.value,
                        FieldAttribute::Infix { .. }
                            | FieldAttribute::Inline { .. }
                            | FieldAttribute::Private { .. }
                    )
                }));
        if !attributes_ok
            || binding.convention.is_some()
            || (binding.result_binder.is_some() && !result_binder)
        {
            return Err(SourceCheckError::Unsupported);
        }
        Ok(())
    }

    fn function(
        &mut self,
        binding: &'a Binding,
        parameters_max: u32,
    ) -> Result<(Vec<SourceType>, SourceType)> {
        assert!(self.proof.summary.is_none());
        self.plain_with(binding, true)?;
        self.proof.begin_body();
        self.proof.in_copy = self.proof.receiver.is_some() && binding.name.value == "copy";
        self.conversions_checked = 0;
        self.world_sites.clear();
        self.implicit_type_parameters(binding)?;
        let checked_parameters = self.function_parameters(binding, parameters_max)?;
        // A result written as variants declares the variants as its own (`sums.rs`).
        let own = if binding.alternatives.is_empty() { None } else { self.hidden_result(binding)? };
        let mut result = if let Some((sum, _)) = &own {
            Some(*sum)
        } else {
            binding
                .annotation
                .as_ref()
                .map(|a| self.result_annotation(a))
                .transpose()?
        };
        result = self.trait_supplied_result(result)?;
        if let Some((sum, members)) = own {
            result = Some(sum);
            self.proof.variants = members;
        } else if !binding.alternatives.is_empty() {
            // A result written as variants is the declared sum of exactly those members.
            let mut members = vec![result.ok_or(SourceCheckError::Unsupported)?];
            for alternative in &binding.alternatives {
                members.push(self.annotation_type(&alternative.value.annotation)?);
            }
            result = Some(self.generic_sum_of_members(&members)?);
            self.proof.variants = members;
        }
        if !binding.extra_results.is_empty() {
            // Several named results are the fields of one record.
            let first = binding
                .result_binder
                .as_ref()
                .ok_or(SourceCheckError::Unsupported)?;
            let mut fields = vec![(
                self.name(first)?.to_owned(),
                result.ok_or(SourceCheckError::Unsupported)?,
            )];
            for extra in &binding.extra_results {
                self.child(binding.name.span_id, extra.span_id).ok();
                let name = self.name(&extra.value.name)?.to_owned();
                if fields.iter().any(|(other, _)| *other == name) {
                    return Err(SourceCheckError::Duplicate);
                }
                fields.push((name, self.annotation_type(&extra.value.annotation)?));
            }
            result = Some(self.anonymous_record(binding, fields)?);
        }
        let infer = self.proof.infer_result && result.is_none();
        if self.memory_of(binding).is_some() {
            return self.instruction_interface(checked_parameters, Some(result.unwrap_or(SourceType::Unit)));
        }
        if let Some(text) = self.instruction_of(binding) {
            // The text and the parameters must name the same symbols: none dropped, none
            // invented.
            let placed = placeholders(&text).ok_or(SourceCheckError::InstructionTarget)?;
            self.charge_product(placed.len(), self.proof.literals.len() + 1)?;
            let agree = placed.iter().all(|name| self.proof.literals.contains(name))
                && self.proof.literals.iter().all(|name| placed.contains(name));
            if !agree {
                return Err(SourceCheckError::InstructionTarget);
            }
            return self.instruction_interface(checked_parameters, result);
        }
        let parameter_count = self.proof.names.len();
        if self.proof.abstract_trait.is_some() && binding.value.is_none() {
            // Only the signature is known; a call consumes its owners and yields the result.
            // A trait's claims on its variants are what every implementation provides.
            self.compile_variant_claims(binding)?;
            return self.instruction_interface(
                checked_parameters,
                Some(result.unwrap_or(SourceType::Unit)),
            );
        }
        let body = binding
            .value
            .as_ref()
            .ok_or(SourceCheckError::MissingBody)?;
        self.token(
            body.operator.span_id,
            match body.operator.value {
                BindingOperator::Colon => ":",
                BindingOperator::ColonEquals => ":=",
            },
        )?;
        self.contract_entry(binding, parameter_count)?;
        self.proof.result_expected = result;
        let returned = self.body(&body.expression, result)?;
        self.proof.returned = false;
        if let Some(value) = returned {
            self.return_guard(value)?;
        }
        // A claim speaks of the parameters and the result, not of what the body made.
        let locals = self.proof.names.split_off(parameter_count.min(self.proof.names.len()));
        self.claims_exit(binding, returned)?;
        self.proof.names.extend(locals);
        if let Some(value) = returned {
            self.release(value)?;
        }
        self.contract_exit()?;
        self.proof.effects = self.function_effects();
        // Text has no spelling, so a function that returns it has no result to write: the
        // result is the text itself.
        let infer = infer
            || (result.is_none() && returned.is_some_and(|actual| actual.ty == SourceType::Text));
        // A linear domain has no children to unpack, so its own methods are the only
        // place a value can end: dropping `self` there is the terminal exit.
        if self.receiver_is_domain()
            && let Some(&(name, receiver)) = self.proof.names.first()
            && name == "self"
        {
            self.release(receiver)?;
        }
        self.charge(self.proof.owners.len())?;
        if self.proof.owners.iter().any(|owner| owner.live) {
            return Err(SourceCheckError::Unconsumed);
        }
        self.proof.names.truncate(parameter_count);
        self.conversions_recorded = 0;
        self.recording_result = if infer {
            returned.map(|actual| actual.ty).or(result)
        } else {
            result
        };
        let recorded = self.record_body(binding, parameter_count);
        if let Some(error) = self.recording_failure.take() {
            return Err(error);
        }
        let converted = self.conversions_recorded == self.conversions_checked;
        // A recorder that declined names its own construct; a count mismatch is only the
        // cause when a body was recorded.
        let declined = recorded.is_none();
        if std::env::var_os("DBGCONV").is_some() { eprintln!("DBGCONV {} recorded={} checked={} body={} decline={:?} at={:?}", binding.name.value, self.conversions_recorded, self.conversions_checked, recorded.is_some(), self.recording_decline, self.recording_at); }
        // A world explores one path: a conversion in a branch it does not take is not counted
        // there, though the recording holds every branch. A world whose count matches is the
        // recording; one that does not leaves what an earlier world recorded.
        self.conversions_all_worlds = {
            let mut distinct = self.conversion_events.clone();
            distinct.sort();
            distinct.dedup();
            distinct.len() as u32
        };
        if converted {
            self.body = recorded;
        } else if let Some(body) = recorded {
            self.unmatched_recording = Some((body, self.conversions_recorded));
        }
        // Provenance for whoever consumes the file: why there is no body, and which calls the
        // recorder folded into values. Kept for the last world, as the body is.
        self.recorded_folds = std::mem::take(&mut self.recording_folds);
        self.recorded_decline = if self.body.is_some() {
            None
        } else if !converted && !(declined && self.recording_decline.is_some()) {
            Some(self.decline_of(DeclineKind::Conversion, "a domain conversion the recorder does not reproduce", binding.name.span_id))
        } else {
            self.recording_decline.take().or_else(|| {
                Some(match self.recording_at {
                    Some(at) => self.decline_of(DeclineKind::Statement, "a statement the recorder does not place", at),
                    None => self.decline_of(DeclineKind::Unplaced, "a construct the recorder could not name", binding.name.span_id),
                })
            })
        };
        let mut result = result;
        if infer && let Some(actual) = returned {
            // The returned value's own type, for example the interval `a + b` ranges over.
            result = Some(actual.ty);
        }
        match (result, returned) {
            (Some(expected), Some(actual)) if infer => {
                self.compatible(expected, actual.ty)?;
            }
            (Some(expected), Some(actual)) => {
                self.compatible(expected, actual.ty)?;
                // A result the trait supplied has no written annotation to refine.
                if let Some(annotation) = binding.annotation.as_ref() {
                    self.refinements(annotation, actual)?;
                }
            }
            (None, None) => {}
            (Some(SourceType::Unit), None) => {}
            _ => return Err(SourceCheckError::Type),
        }
        // Original parameter owners retain their child origins after consumption.
        // Capture now so the interface contains only paths actually inspected by
        // the body, not the Cartesian product of every declared sum field.
        let inputs = self.summary_inputs()?;
        self.proof.summary = Some(self.summarize(&inputs, returned)?);
        self.check_end_inputs()?;
        let result = match result {
            Some(value) => value,
            None => SourceType::Unit,
        };
        Ok((checked_parameters, result))
    }

    /// A `#instruction` function has no body to verify: it consumes its inputs and yields
    /// a fresh value of its declared result, which is everything a caller may rely on.
    fn instruction_interface(
        &mut self,
        parameters: Vec<SourceType>,
        result: Option<SourceType>,
    ) -> Result<(Vec<SourceType>, SourceType)> {
        let result = result.ok_or(SourceCheckError::InstructionTarget)?;
        let inputs: Vec<Value> = self.proof.names.iter().map(|(_, value)| *value).collect();
        self.charge(inputs.len())?;
        for value in inputs {
            self.release(value)?;
            if let Some(owner) = value.owner {
                self.proof.owners[owner].drained = true;
            }
        }
        let inputs = self.summary_inputs()?;
        // A result written as variants is one of them: one world each.
        let members = if self.proof.variants.is_empty() {
            vec![result]
        } else {
            self.proof.variants.clone()
        };
        for (index, member) in members.into_iter().enumerate() {
            let returned = self.input_value(member)?;
            self.release(returned)?;
            if let Some(owner) = returned.owner {
                self.proof.owners[owner].drained = true;
            }
            let summary = self.summarize(&inputs, Some(returned))?;
            if index == 0 {
                self.proof.summary = Some(summary);
            } else {
                self.proof.member_summaries.push(summary);
            }
        }
        Ok((parameters, result))
    }

    /// An `end` input must be ended here: it may not come back out through the result,
    /// whether directly or through an alias or a child. Dropping it is already refused.
    fn check_end_inputs(&mut self) -> Result<()> {
        if !self.proof.parameter_passing.contains(&calls::Passing::End) {
            return Ok(());
        }
        let dispositions = self
            .proof
            .summary
            .as_ref()
            .ok_or(SourceCheckError::Malformed)?
            .dispositions
            .clone();
        self.charge(dispositions.len())?;
        let values = self
            .proof
            .parameter_roles
            .iter()
            .zip(&self.proof.parameter_passing)
            .filter(|(role, _)| matches!(role, calls::Parameter::Value(_)))
            .map(|(_, passing)| *passing);
        let escaped = values.zip(&dispositions).any(|(passing, disposition)| {
            passing == calls::Passing::End && *disposition == summaries::Disposition::Returned
        });
        if escaped {
            return Err(SourceCheckError::EndEscape);
        }
        Ok(())
    }

    fn function_parameters(
        &mut self,
        binding: &'a Binding,
        parameters_max: u32,
    ) -> Result<Vec<SourceType>> {
        let Some(parameters) = binding.parameters.as_ref() else {
            return Ok(Vec::new());
        };
        self.span(parameters.span_id)?;
        self.charge(parameters.value.len())?;
        if parameters.value.len() > parameters_max as usize {
            return Err(SourceCheckError::Budget);
        }
        let mut checked_parameters = Vec::new();
        for parameter in &parameters.value {
            let parameter_span = parameter.span_id;
            self.child(parameters.span_id, parameter.span_id)?;
            self.child(parameter.span_id, parameter.name.span_id)?;
            if let Some(annotation) = &parameter.annotation {
                self.child(parameter.span_id, annotation.span_id)?;
            }
            let written = &parameter.convention;
            let parameter = &parameter.value;
            if parameter.default.is_some() {
                return Err(SourceCheckError::Unsupported);
            }
            let passing = self.parameter_passing(written, parameter_span)?;
            let name = self.name(&parameter.name)?;
            if modules::receiver_annotation(&parameter.annotation)
                && name == "self"
                && checked_parameters.is_empty()
                && let Some(receiver) = self.proof.receiver
            {
                // A method's bare `self` is the receiver: owned unless it says `ref`/`mut`.
                let mut value = self.input_value(receiver)?;
                if let Some(annotation) = parameter.annotation.as_ref() {
                    value = self.requirements(annotation, name, value)?;
                }
                self.set_passing(value, passing);
                self.insert(name, value)?;
                if passing == calls::Passing::Mut {
                    self.mark_assignable(name);
                }
                self.charge(2)?;
                checked_parameters.push(value.ty);
                self.proof
                    .parameter_roles
                    .push(calls::Parameter::Value(value.ty));
                self.proof.parameter_passing.push(passing);
                continue;
            }
            if parameter.annotation.is_none() && self.instruction_of(binding).is_some() {
                self.charge_product(
                    self.proof.literals.len() + self.proof.names.len(),
                    name.len() + 1,
                )?;
                if self.proof.literals.contains(&name)
                    || self.proof.generics.contains(&name)
                    || self.proof.names.iter().any(|(other, _)| *other == name)
                {
                    return Err(SourceCheckError::Duplicate);
                }
                self.proof.literals.push(name);
                self.proof.parameter_roles.push(calls::Parameter::Literal);
                self.proof.parameter_passing.push(passing);
                continue;
            }
            // `capacity Length` used as `[element; capacity]` is a compile-time number.
            if let Some(written) = parameter.annotation.as_ref()
                && passing == calls::Passing::Owner
                && parameter.convention.is_none()
                && Self::number_shaped(&written.value)
                && !matches!(&written.value, Annotation::Expression(e) if matches!(&e.value, Expr::Name(n) if n == "Type"))
                && numbers::names_array_length(binding, name)
            {
                if self.proof.generics.contains(&name)
                    || self.proof.names.iter().any(|(n, _)| *n == name)
                {
                    return Err(SourceCheckError::Duplicate);
                }
                let (lower, upper) = self
                    .annotation_range(written)?
                    .ok_or(SourceCheckError::Unsupported)?;
                let index = self.proof.generics.len() as u32;
                self.proof.generics.push(name);
                let value = self.fresh_value(SourceType::Integer { lower, upper })?;
                self.proof.numbers.push(numbers::NumberParam {
                    index,
                    lower,
                    upper,
                    value: Some(value),
                });
                self.charge(1)?;
                self.proof.parameter_roles.push(calls::Parameter::Type);
                self.proof.parameter_passing.push(passing);
                continue;
            }
            // A bare name is a type parameter, as `element Type` is.
            let bare = parameter.annotation.is_none();
            let annotation = parameter.annotation.as_ref();
            // `element has Release`: a type parameter that provides the trait.
            let bound = match annotation.map(|a| &a.value) {
                Some(Annotation::Bound { bound, .. }) if parameter.convention.is_none() => {
                    Some(bound)
                }
                _ => None,
            };
            if bare
                || bound.is_some()
                || self.metatype(annotation.ok_or(SourceCheckError::Type)?)?
            {
                // A method may add a bound to a parameter of its type: `copy(self ref, element has
                // Copy)`. It is no new parameter, and a call checks it against the receiver's type.
                if let Some(bound) = bound
                    && self.proof.receiver.is_some()
                    && let Some(existing) = self.proof.generics.iter().position(|other| *other == name)
                {
                    self.proof.bounds.push((existing as u32, bound));
                    self.proof.added_bounds.push(name);
                    self.charge(1)?;
                    continue;
                }
                if let Some(bound) = bound {
                    self.proof.bounds.push((self.proof.generics.len() as u32, bound));
                }
                if self.proof.generics.contains(&name)
                    || self.proof.names.iter().any(|(n, _)| *n == name)
                {
                    return Err(SourceCheckError::Duplicate);
                }
                if passing != calls::Passing::Owner {
                    return Err(SourceCheckError::Unsupported);
                }
                self.proof.generics.push(name);
                self.charge(1)?;
                self.proof.parameter_roles.push(calls::Parameter::Type);
                self.proof.parameter_passing.push(passing);
                continue;
            }
            let annotation = annotation.ok_or(SourceCheckError::Type)?;            let ty = self.annotation_type(annotation)?;
            let mut value = self.input_value(ty)?;
            value = self.requirements(annotation, name, value)?;
            if matches!(value.ty, SourceType::Integer { .. }) {
                value.integer_domain = Some((value.lower, value.upper));
            }
            self.set_passing(value, passing);
            self.insert(name, value)?;
            if passing == calls::Passing::Mut {
                self.mark_assignable(name);
            }
            self.charge(2)?;
            let parameter_type = if matches!(ty, SourceType::Sum { .. }) {
                ty
            } else {
                value.ty
            };
            checked_parameters.push(parameter_type);
            self.proof
                .parameter_roles
                .push(calls::Parameter::Value(parameter_type));
            self.proof.parameter_passing.push(passing);
        }
        Ok(checked_parameters)
    }

    /// How a parameter takes its argument: an owner unless it writes `ref`, `mut`, or
    /// `end`. The legacy `own` transfer marker and group paths have no meaning here.
    fn parameter_passing(
        &mut self,
        written: &Option<Spanned<ParameterConvention>>,
        parent: SpanId,
    ) -> Result<calls::Passing> {
        let Some(convention) = written else {
            return Ok(calls::Passing::Owner);
        };
        self.child(parent, convention.span_id)?;
        let keyword = &convention.value.keyword;
        self.child(convention.span_id, keyword.span_id)?;
        if convention.value.group.is_some() {
            return Err(SourceCheckError::Unsupported);
        }
        let (passing, text) = match keyword.value {
            Convention::Ref => (calls::Passing::Ref, "ref"),
            Convention::Mut => (calls::Passing::Mut, "mut"),
            Convention::End => (calls::Passing::End, "end"),
            Convention::Own => return Err(SourceCheckError::Unsupported),
        };
        self.keyword(keyword.span_id, text)?;
        Ok(passing)
    }

    /// Mark the owner of a freshly bound parameter with how it was passed. Aliases share
    /// the owner, so they inherit the restriction.
    fn set_passing(&mut self, value: Value, passing: calls::Passing) {
        if let Some(owner) = value.owner {
            self.proof.owners[owner].passing = passing;
        }
    }

    /// Allow `::` to reassign `name`, which was just bound.
    pub(super) fn mark_assignable(&mut self, name: &'a str) {
        self.proof.assignable.push(name);
    }

    fn insert(&mut self, name: &'a str, mut value: Value) -> Result<()> {
        self.proof.assignable.retain(|other| *other != name);
        self.charge(
            self.proof
                .names
                .len()
                .checked_mul(name.len() + 1)
                .ok_or(SourceCheckError::Budget)?
                + 1,
        )?;
        if self.proof.names.iter().any(|(other, _)| *other == name) {
            return Err(SourceCheckError::Duplicate);
        }
        if self.proof.generics.contains(&name) {
            return Err(SourceCheckError::Duplicate);
        }
        // A computed interval needs one identity at binding time, shared by all
        // subsequent aliases. Literal singletons keep their existing identity policy.
        // This does not infer any relation back to an expression's operands.
        if matches!(value.ty, SourceType::Integer { .. })
            && value.identity.is_none()
            && value.lower != value.upper
        {
            value.identity = self.fresh_value(value.ty)?.identity;
        }
        if let Some(owner) = value.owner
            && value.borrow.is_none()
        {
            self.charge_flow()?;
            self.obligate(owner);
        }
        self.proof.names.push((name, value));
        Ok(())
    }

    fn requirements(
        &mut self,
        annotation: &'a Spanned<Annotation>,
        subject: &'a str,
        value: Value,
    ) -> Result<Value> {
        if !matches!(
            annotation.value,
            Annotation::Domain { .. } | Annotation::Refined { .. }
        ) {
            return Ok(value);
        }
        self.charge_product(self.proof.names.len() + 1, 2)?;
        let mut names: Vec<_> = self.proof.names.iter().map(|(name, _)| *name).collect();
        // The parameter being declared is the subject of its own claims.
        names.push(subject);
        let mut values: Vec<_> = self.proof.names.iter().map(|(_, value)| *value).collect();
        let slot = values.len();
        values.push(value);
        let mut constraints = Vec::new();
        // A claim about a field of the subject (`buffer.length < cap`) reads the subject's type.
        self.proof.names.push((subject, value));
        let compiled = self.compile_constraints(annotation, slot, &names, &mut constraints);
        self.proof.names.pop();
        compiled?;
        self.constraints(&constraints, &mut values, true)?;
        Ok(values[slot])
    }

    fn bounds(&mut self, annotation: &'a Spanned<Annotation>) -> Result<Value> {
        self.span(annotation.span_id)?;
        let Annotation::Domain {
            is_keyword: None,
            domain,
        } = &annotation.value
        else {
            return Err(SourceCheckError::Unsupported);
        };
        self.child(annotation.span_id, domain.span_id)?;
        let (lower, upper) = self.domain_bounds(domain)?;
        Ok(Value {
            lower,
            upper,
            identity: None,
            ty: SourceType::Integer { lower, upper },
            owner: None,
            available: true,
            sum: None,
            structural: None,
            integer_domain: Some((lower, upper)),
            affine: None,
            borrow: None,
        })
    }

    /// The bounds of a written `in L to U` with constant ends.
    pub(super) fn domain_bounds(
        &mut self,
        domain: &'a Spanned<TypeDomain>,
    ) -> Result<(I256, I256)> {
        if let Some(keyword) = domain.in_keyword {
            self.token(keyword, "in")?;
        }
        self.child(domain.span_id, domain.base.span_id)?;
        // `in Self`: inside a domain's methods, the integer range the domain holds.
        if let DomainBase::Expression(expression) = &domain.base.value
            && matches!(&expression.value, Expr::Name(name) if name == "Self")
            && let Some(bounds) = self.proof.receiver.and_then(|r| self.domain_bounds_of(r))
        {
            self.token(expression.span_id, "Self")?;
            self.child(domain.base.span_id, expression.span_id)?;
            return Ok(bounds);
        }
        // `in Byte`, `in domain of Byte`: the range a declared domain holds.
        if let DomainBase::Expression(expression) = &domain.base.value {
            self.child(domain.base.span_id, expression.span_id)?;
            let scope = self.generic_scope();
            let ty = self.resolve_type(expression, &scope)?;
            return match self.domain_type(ty)? {
                SourceType::Integer { lower, upper } => Ok((lower, upper)),
                _ => Err(SourceCheckError::Type),
            };
        }
        let DomainBase::Range {
            lower,
            upper,
            to_keyword,
        } = &domain.base.value
        else {
            return Err(SourceCheckError::Unsupported);
        };
        self.token(*to_keyword, "to")?;
        self.child(domain.base.span_id, lower.span_id)?;
        self.child(domain.base.span_id, upper.span_id)?;
        let lower = self.expression(lower)?;
        let upper = self.expression(upper)?;
        self.scalar(lower)?;
        self.scalar(upper)?;
        if lower.identity.is_some()
            || upper.identity.is_some()
            || lower.lower != lower.upper
            || upper.lower != upper.upper
        {
            return Err(SourceCheckError::Unsupported);
        }
        if lower.lower > upper.upper {
            return Err(SourceCheckError::Type);
        }
        Ok((lower.lower, upper.upper))
    }

    fn refinements(&mut self, annotation: &'a Spanned<Annotation>, value: Value) -> Result<()> {
        let Annotation::Domain { domain, .. } = &annotation.value else {
            return Ok(());
        };
        self.charge(domain.refinements.len())?;
        for refinement in &domain.refinements {
            self.child(domain.span_id, refinement.span_id)?;
            self.child(refinement.span_id, refinement.and_keyword)?;
            self.child(refinement.span_id, refinement.predicate.span_id)?;
            self.token(refinement.and_keyword, "and")?;
            self.span(refinement.predicate.span_id)?;
            match &refinement.predicate.value {
                RefinementPredicate::ImplicitComparison { operator, right } => {
                    self.comparison(operator)?;
                    let right = self.expression(right)?;
                    self.compare(operator.value, value, right)?;
                }
                RefinementPredicate::Expression(expression) => self.prove(expression)?,
                RefinementPredicate::ImplicitDomain(domain) => {
                    // `and in L to U` is `>= L` and `<= U` on the annotated value.
                    let DomainBase::Range {
                        lower,
                        upper,
                        to_keyword,
                    } = &domain.base.value
                    else {
                        return Err(SourceCheckError::Unsupported);
                    };
                    if let Some(keyword) = domain.in_keyword {
                        self.keyword(keyword, "in")?;
                    }
                    self.child(refinement.predicate.span_id, domain.span_id)?;
                    self.child(domain.span_id, domain.base.span_id)?;
                    self.child(domain.base.span_id, lower.span_id)?;
                    self.child(domain.base.span_id, upper.span_id)?;
                    self.token(*to_keyword, "to")?;
                    let lower = self.expression(lower)?;
                    let upper = self.expression(upper)?;
                    self.compare(BinaryOperator::GreaterOrEqual, value, lower)?;
                    self.compare(BinaryOperator::LessOrEqual, value, upper)?;
                }
            }
        }
        Ok(())
    }

    fn body(
        &mut self,
        expression: &'a Expression,
        expected: Option<SourceType>,
    ) -> Result<Option<Value>> {
        let value = self.evaluate(expression, expected)?;
        if value.ty == SourceType::Unit {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn local_type(&mut self, local: &'a Binding, parent: SpanId) -> Result<Option<SourceType>> {
        self.plain(local)?;
        if local.parameters.is_some()
            || local.contract.is_some()
        {
            return Err(SourceCheckError::Unsupported);
        }
        self.name(&local.name)?;
        self.child(parent, local.name.span_id)?;
        let body = local.value.as_ref().ok_or(SourceCheckError::MissingBody)?;
        self.child(parent, body.expression.span_id)?;
        self.child(parent, body.operator.span_id)?;
        if let Some(annotation) = &local.annotation {
            self.child(parent, annotation.span_id)?;
        }
        self.token(
            body.operator.span_id,
            match body.operator.value {
                BindingOperator::Colon => ":",
                BindingOperator::ColonEquals => ":=",
            },
        )?;
        local
            .annotation
            .as_ref()
            .map(|annotation| self.annotation_type(annotation))
            .transpose()
    }

    fn expression(&mut self, root: &'a Expression) -> Result<Value> {
        self.charge(1)?;
        let mut tasks = vec![(root, false)];
        let mut values: Vec<Value> = Vec::new();
        while let Some((expression, visited)) = tasks.pop() {
            self.charge(1)?;
            let text = self.span(expression.span_id)?;
            match &expression.value {
                Expr::Name(name) => {
                    values.push(self.local(name, text)?);
                }
                Expr::Literal(LiteralKind::Integer) => {
                    let number = self.integer(text, false)?;
                    values.push(Value::integer(number, number));
                }
                // Text exists only at compile time, and owns nothing.
                Expr::Literal(LiteralKind::String) => {
                    let contents = quoted(text).ok_or(SourceCheckError::Unsupported)?;
                    self.charge(contents.len())?;
                    // The byte length of the literal travels with it: it is the exact length of
                    // the bytes the literal becomes when it is used at run time (`text_bytes`).
                    let mut text = self.fresh_value(SourceType::Text)?;
                    let length = I256::from(contents.len() as u64);
                    (text.lower, text.upper, text.integer_domain) = (length, length, Some((length, length)));
                    values.push(text);
                }
                Expr::Group(child) => {
                    self.group_syntax(expression, child)?;
                    self.charge(1)?;
                    tasks.push((child, false));
                }
                Expr::Unary { operator, operand } => {
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, operand.span_id)?;
                    self.token(operator.span_id, "-")?;
                    if let Expr::Literal(LiteralKind::Integer) = operand.value {
                        let text = self.span(operand.span_id)?;
                        let number = self.integer(text, true)?;
                        values.push(Value::integer(number, number));
                    } else if !visited {
                        self.charge(2)?;
                        tasks.push((expression, true));
                        tasks.push((operand, false));
                    } else {
                        let value = values.pop().ok_or(SourceCheckError::Malformed)?;
                        self.scalar(value)?;
                        let lower = value
                            .upper
                            .checked_neg()
                            .ok_or(SourceCheckError::Overflow)?;
                        let upper = value
                            .lower
                            .checked_neg()
                            .ok_or(SourceCheckError::Overflow)?;
                        values.push(Value::integer(lower, upper));
                    }
                }
                Expr::Member { base, name } => {
                    self.child(expression.span_id, base.span_id)?;
                    self.child(expression.span_id, name.span_id)?;
                    values.push(self.projection(base, name)?);
                }
                // `piece.start + 1` in a claim: `+ - *` over numbers, related to their operands.
                Expr::Binary {
                    left,
                    operator,
                    right,
                } if CheckedArithmetic::from_operator(operator.value).is_some() => {
                    if visited {
                        let right = values.pop().ok_or(SourceCheckError::Malformed)?;
                        let left = values.pop().ok_or(SourceCheckError::Malformed)?;
                        let left = self.open_number(left)?;
                        let right = self.open_number(right)?;
                        let arithmetic = CheckedArithmetic::from_operator(operator.value)
                            .ok_or(SourceCheckError::Malformed)?;
                        values.push(self.arithmetic(arithmetic, left, right)?);
                    } else {
                        self.child(expression.span_id, left.span_id)?;
                        self.child(expression.span_id, operator.span_id)?;
                        self.child(expression.span_id, right.span_id)?;
                        self.charge(3)?;
                        tasks.push((expression, true));
                        tasks.push((right, false));
                        tasks.push((left, false));
                    }
                }
                Expr::Select(select) => {
                    if visited {
                        let index = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.array_select(expression, select, index)?);
                    } else {
                        self.child(expression.span_id, select.index.span_id)?;
                        self.charge(2)?;
                        tasks.push((expression, true));
                        tasks.push((&select.index, false));
                    }
                }
                Expr::NumericSelect(select) => {
                    let index = self.numeric_select_index(expression, select)?;
                    values.push(self.position_select(&select.base, index)?);
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        values.pop().ok_or(SourceCheckError::Malformed)
    }

    fn local(&mut self, name: &str, text: &str) -> Result<Value> {
        self.charge(name.len())?;
        if text != name {
            return Err(SourceCheckError::Malformed);
        }
        if let Some(number) = self.number_value(name) {
            return self.scalar_facts(number);
        }
        let Some(index) = self.local_position(name)? else {
            // Not a local: a module constant is a number known now.
            let value = self
                .constant_value(name)?
                .ok_or(SourceCheckError::UnknownName)?;
            return self.scalar_facts(Value::integer(value, value));
        };
        let value = self.proof.names[index].1;
        self.array_complete(value)?;
        if value.borrow.is_some() {
            // A handle is a borrow: reading it copies what it may copy and moves nothing.
            let copy = self.handle_read(value)?;
            return self.scalar_facts(copy);
        }
        if let Some(owner) = value.owner {
            self.charge_flow()?;
            // Moving or ending the local while a loan on it is live is refused.
            if self.proof.owners[owner].unrestricted {
                self.read_guard(value)?;
            } else {
                self.move_guard(value, name)?;
            }
            // A borrow may be read only by copying: moving, ending or returning it is the
            // owner's right, never the borrower's.
            if self.proof.owners[owner].passing.borrows() && !self.proof.owners[owner].unrestricted
            {
                return Err(SourceCheckError::MoveOutOfBorrow);
            }
            // Reading an unrestricted value copies it; the binding stays available.
            if !self.proof.owners[owner].unrestricted {
                if !value.available {
                    return Err(SourceCheckError::UseAfterMove);
                }
                self.proof.names[index].1.available = false;
                self.proof.owners[owner].live = false;
            }
        }
        self.scalar_facts(value)
    }

    fn local_position(&mut self, name: &str) -> Result<Option<usize>> {
        for index in 0..self.proof.names.len() {
            self.charge(1)?;
            let other = self.proof.names[index].0;
            if other.len() == name.len() {
                self.charge(name.len())?;
                if other == name {
                    return Ok(Some(index));
                }
            }
        }
        Ok(None)
    }

    fn integer(&mut self, text: &str, negative: bool) -> Result<I256> {
        self.charge(text.len())?;
        let value = decode_integer_accumulator(text)?;
        if negative {
            Ok(value)
        } else {
            value.checked_neg().ok_or(SourceCheckError::Overflow)
        }
    }

    fn prove(&mut self, root: &'a Expression) -> Result<()> {
        self.charge(1)?;
        let mut pending = vec![root];
        while let Some(expression) = pending.pop() {
            self.span(expression.span_id)?;
            match &expression.value {
                Expr::Group(child) => {
                    self.group_syntax(expression, child)?;
                    self.charge(1)?;
                    pending.push(child);
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } => {
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    self.comparison(operator)?;
                    if operator.value == BinaryOperator::And {
                        self.charge(2)?;
                        pending.push(right);
                        pending.push(left);
                    } else {
                        let left = self.expression(left)?;
                        let right = self.expression(right)?;
                        self.compare(operator.value, left, right)?;
                    }
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        Ok(())
    }

    fn compare(&mut self, operator: BinaryOperator, left: Value, right: Value) -> Result<()> {
        // A claim over a field of a domain type speaks of the number the domain holds.
        let left = self.domain_number(left)?;
        let right = self.domain_number(right)?;
        let left = self.scalar_facts(left)?;
        let right = self.scalar_facts(right)?;
        self.scalar(left)?;
        self.scalar(right)?;
        if let (Some(left), Some(right)) = (left.identity, right.identity)
            && (self.entails(operator, left, right)?
                || self.linear_entails(operator, left, right)?)
        {
            return Ok(());
        }
        // Against a constant, the interval the relations imply is the one to compare.
        let tighten = |checker: &mut Self, mut value: Value| -> Result<Value> {
            if let Some(identity) = value.identity
                && let Some((lower, upper)) = checker.linear_bounds(identity)?
            {
                value.lower = value.lower.max(lower);
                value.upper = value.upper.min(upper);
            }
            Ok(value)
        };
        let (left, right) = if left.identity.is_some() != right.identity.is_some() {
            (tighten(self, left)?, tighten(self, right)?)
        } else {
            (left, right)
        };
        let identical = left.identity.is_some() && left.identity == right.identity;
        let valid = match operator {
            BinaryOperator::Equal => {
                identical
                    || (left.lower == left.upper
                        && left.lower == right.lower
                        && right.lower == right.upper)
            }
            BinaryOperator::Less => left.upper < right.lower,
            BinaryOperator::LessOrEqual => identical || left.upper <= right.lower,
            BinaryOperator::Greater => left.lower > right.upper,
            BinaryOperator::GreaterOrEqual => identical || left.lower >= right.upper,
            _ => return Err(SourceCheckError::Unsupported),
        };
        if valid {
            Ok(())
        } else {
            Err(SourceCheckError::Unproved)
        }
    }
}

impl Checker<'_> {
    /// Count one conversion of the world being checked, at the expression being evaluated: the
    /// recording holds each conversion site once, and worlds that run the same site make the same
    /// conversion there, whatever outcome a callee gave to get them there.
    pub(super) fn count_conversion(&mut self) {
        self.conversions_checked += 1;
        let site = self.culprit;
        let nth = match self.world_sites.iter_mut().find(|(at, _)| *at == site) {
            Some((_, count)) => {
                *count += 1;
                *count - 1
            }
            None => {
                self.world_sites.push((site, 1));
                0
            }
        };
        self.conversion_events.push((site, nth));
    }
}

fn prepay(remaining: &mut u32, amount: usize) -> Result<()> {
    let amount = u32::try_from(amount).map_err(|_| SourceCheckError::Budget)?;
    *remaining = remaining
        .checked_sub(amount)
        .ok_or(SourceCheckError::Budget)?;
    Ok(())
}
