//! Executable checked bodies derived from the direct source checker.
//!
//! Interface summaries carry ownership and coverage facts; bodies carry the
//! resolved typed expression tree a consumer lowers to code. Recording is a
//! fail-closed slice: a body is present only when every construct in it is
//! representable here, so consumers treat `None` as "not lowerable yet" rather
//! than guessing at semantics.

use super::*;
use i256::I256;
use num_traits::ToPrimitive;
use ast::SpanId;

mod chain_records;
mod templates;

/// An executable checked body for one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSourceBody {
    /// Running this body touches the machine, directly or through a call. Such a body is
    /// never evaluated at compile time, even when every input is known.
    pub effects: bool,
    /// Postorder arena: every child precedes its parent.
    pub expressions: Vec<CheckedSourceBodyExpr>,
    /// Root node id.
    pub root: u32,
    /// The node holding each `mut` parameter's value when the body returns, in the order
    /// of the function's `mutated`.
    pub outputs: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSourceBodyExpr {
    pub span: SpanId,
    pub ty: SourceType,
    pub kind: CheckedSourceBodyKind,
}

/// Which function a call reaches, without naming it by string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedSourceCallee {
    /// Index into this module's `functions`.
    Local { function: u32 },
    /// `function` indexes `functions` of the module at `dependency` in the list
    /// that was passed to the check.
    Dependency { dependency: u32, function: u32 },
}

/// Exact integer arithmetic. The checked domains already prove that the result fits the
/// node's type, so a consumer needs no overflow handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedArithmetic {
    Add,
    Subtract,
    Multiply,
    /// Division truncates toward zero. A recorded division has a divisor the checker proved is
    /// not zero where it runs, but a consumer that runs every arm (a branch-free lowering, the
    /// checker's eager evaluator) must not assume it runs only there: see `speculation_safe`.
    Divide,
    /// A remainder follows its dividend.
    Remainder,
    BitAnd,
    BitOr,
    BitXor,
    /// A shift by 0 to 254 bits, proved by the checker.
    ShiftLeft,
    /// Keeps the sign.
    ShiftRight,
}

impl CheckedArithmetic {
    pub(super) fn from_operator(operator: BinaryOperator) -> Option<Self> {
        match operator {
            BinaryOperator::Add => Some(Self::Add),
            BinaryOperator::Subtract => Some(Self::Subtract),
            BinaryOperator::Multiply => Some(Self::Multiply),
            _ => None,
        }
    }

    /// The source spelling of the operator.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Remainder => "%",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
        }
    }

    /// The source operator, which [`eval::exact_binary`] defines.
    pub fn binary(self) -> BinaryOperator {
        match self {
            Self::Add => BinaryOperator::Add,
            Self::Subtract => BinaryOperator::Subtract,
            Self::Multiply => BinaryOperator::Multiply,
            Self::Divide => BinaryOperator::Divide,
            Self::Remainder => BinaryOperator::Remainder,
            Self::BitAnd => BinaryOperator::BitAnd,
            Self::BitOr => BinaryOperator::BitOr,
            Self::BitXor => BinaryOperator::BitXor,
            Self::ShiftLeft => BinaryOperator::ShiftLeft,
            Self::ShiftRight => BinaryOperator::ShiftRight,
        }
    }

    /// Whether running this operation where its operands are not the ones it was checked for is
    /// harmless. A division or remainder by zero is not (it is undefined in machine code), and
    /// a shift by too many bits is not: those are only safe on the path the checker proved.
    pub fn speculation_safe(self) -> bool {
        !matches!(self, Self::Divide | Self::Remainder | Self::ShiftLeft | Self::ShiftRight)
    }
}

/// A binary operator a nominal type can implement with `#infix('op')`. The checker computes
/// only `+ - *` on integers itself; for a nominal left operand every one of these calls the
/// method the type declares, whatever it returns, so libraries define their own meanings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfixOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

impl InfixOperator {
    pub(super) fn from_binary(operator: BinaryOperator) -> Option<Self> {
        Some(match operator {
            BinaryOperator::Add => Self::Add,
            BinaryOperator::Subtract => Self::Subtract,
            BinaryOperator::Multiply => Self::Multiply,
            BinaryOperator::Divide => Self::Divide,
            BinaryOperator::Remainder => Self::Remainder,
            BinaryOperator::Equal => Self::Equal,
            BinaryOperator::Less => Self::Less,
            BinaryOperator::LessOrEqual => Self::LessOrEqual,
            BinaryOperator::Greater => Self::Greater,
            BinaryOperator::GreaterOrEqual => Self::GreaterOrEqual,
            BinaryOperator::BitAnd => Self::BitAnd,
            BinaryOperator::BitOr => Self::BitOr,
            BinaryOperator::BitXor => Self::BitXor,
            BinaryOperator::ShiftLeft => Self::ShiftLeft,
            BinaryOperator::ShiftRight => Self::ShiftRight,
            BinaryOperator::And | BinaryOperator::Or => return None,
        })
    }

    /// The source spelling, as written inside `#infix('...')`.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Remainder => "%",
            Self::Equal => "=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
        }
    }

    /// Every operator a type may define.
    pub const ALL: [Self; 15] = [
        Self::Add,
        Self::Subtract,
        Self::Multiply,
        Self::Divide,
        Self::Remainder,
        Self::Equal,
        Self::Less,
        Self::LessOrEqual,
        Self::Greater,
        Self::GreaterOrEqual,
        Self::BitAnd,
        Self::BitOr,
        Self::BitXor,
        Self::ShiftLeft,
        Self::ShiftRight,
    ];

    /// `/ % & | ^ << >>`: computed by the checker as a result domain only (`integer_ops.rs`).
    pub(super) fn is_integer_operation(self) -> bool {
        matches!(
            self,
            Self::Divide
                | Self::Remainder
                | Self::BitAnd
                | Self::BitOr
                | Self::BitXor
                | Self::ShiftLeft
                | Self::ShiftRight
        )
    }

    /// Every operation the recorder records on integers: `arithmetic` and the integer
    /// operations. (`arithmetic` alone selects the checker's own interval-and-offset rules.)
    pub(super) fn recordable(self) -> Option<CheckedArithmetic> {
        Some(match self {
            Self::Divide => CheckedArithmetic::Divide,
            Self::Remainder => CheckedArithmetic::Remainder,
            Self::BitAnd => CheckedArithmetic::BitAnd,
            Self::BitOr => CheckedArithmetic::BitOr,
            Self::BitXor => CheckedArithmetic::BitXor,
            Self::ShiftLeft => CheckedArithmetic::ShiftLeft,
            Self::ShiftRight => CheckedArithmetic::ShiftRight,
            other => return other.arithmetic(),
        })
    }

    pub(super) fn arithmetic(self) -> Option<CheckedArithmetic> {
        match self {
            Self::Add => Some(CheckedArithmetic::Add),
            Self::Subtract => Some(CheckedArithmetic::Subtract),
            Self::Multiply => Some(CheckedArithmetic::Multiply),
            _ => None,
        }
    }
}

/// One arm of a recorded `Match`: the member it takes and the node holding its result.
/// Where the two arms of a `Select` begin in the arena: the nodes from `then_start` up to
/// `else_start` compute the `then` arm and those from `else_start` up to the `Select` the `else`
/// arm. A lowering that must not run an arm unless it is taken puts those nodes in a branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectRegion {
    pub then_start: u32,
    pub else_start: u32,
    /// Each arm's nodes belong to it alone. A chain of `and`/`or` shares its arms between
    /// several selects, and those nodes cannot be moved into any one branch.
    pub exclusive: bool,
}

/// The same for a `Match` that is a test of one member: the arm whose body is `member_body`
/// begins at `member_start` and every other arm, which share one body, at `other_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchRegion {
    pub member_body: u32,
    pub member_start: u32,
    pub other_start: u32,
}

/// Binders of the arm read the subject through `Field` nodes the body refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckedSourceMatchArm {
    pub schema: u32,
    pub body: u32,
}

/// A value a loop carries from one pass to the next: `placeholder` stands for it inside
/// the body, `initial` is what it is before the first pass, and `next` is what the pass
/// leaves for the one after (and, last, for the code after the loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckedSourceLoopCarry {
    pub placeholder: u32,
    pub initial: u32,
    pub next: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedSourceBodyKind {
    /// Exit the enclosing function with this value.
    Return { value: u32 },
    /// A checked reference to a value parameter by its zero-based index.
    Parameter { index: u32 },
    /// A checked non-negative integer literal.
    Integer(I256),
    /// A string literal, or the text a fold produced. Compile-time only.
    Text(String),
    /// The unit value.
    Unit,
    /// An explicit `#todo` hole standing for a value of this node's type. Running it is an
    /// error, so only a consumer that can report it (the interpreter) accepts a body holding
    /// one; lowering refuses.
    Hole,
    /// `left <operator> right` over integers. Operands whose values are all known at
    /// compile time never appear here: they are folded into an `Integer` when recorded.
    Binary {
        operator: CheckedArithmetic,
        left: u32,
        right: u32,
    },
    /// `operand` reduced into this node's integer domain, a `#wrap` domain it is stored
    /// in. Known operands are reduced when recorded, so no `Wrap` remains for them.
    Wrap { operand: u32 },
    /// A record of `schema` built from its fields, in schema field order. The node's type
    /// is the record's own instance, also when a sum later holds it.
    Construct { schema: u32, fields: Vec<u32> },
    /// `when subject is ...` over a sum whose member is not known here. A known member
    /// selects its arm when recorded, so no `Match` remains.
    Match {
        subject: u32,
        arms: Vec<CheckedSourceMatchArm>,
        /// Where the arms' nodes are, for a test of one member.
        region: Option<MatchRegion>,
    },
    /// Field `index` (schema order) of a record that is not itself a known construction;
    /// a destructured known construction binds its field nodes directly.
    Field { record: u32, index: u32 },
    /// Field `index` of the payload of member `schema` of the sum `subject` is, read in the
    /// arm that member selects. Its value means something only when the member is the subject's.
    VariantField {
        subject: u32,
        schema: u32,
        index: u32,
    },
    /// A call. `arguments` are body expression ids in parameter order. A call of a generic
    /// function (or a method of a generic type) also gives the `type_arguments` it applies the
    /// callee to, in the callee's `type_parameters` order, as types of the body's own function:
    /// in a template body they may be `Generic`. Empty for a function that is not generic.
    Call {
        callee: CheckedSourceCallee,
        arguments: Vec<u32>,
        type_arguments: Vec<SourceType>,
    },
    /// A call through a bound, `element.release(value)` or `value.release`, where `element` is a
    /// type parameter that must provide a trait. Which function it reaches is known only for a
    /// concrete type: `generic` indexes the body's type parameter that stands for the type, and
    /// `method` is the plain name of the trait's method, found among the methods of whichever type
    /// is given for it. `trait_arguments` are the types the bound applies the trait to. `arguments`
    /// are the values passed, the receiver first.
    BoundCall {
        generic: u32,
        method: String,
        trait_arguments: Vec<SourceType>,
        arguments: Vec<u32>,
    },
    /// `when left <comparison> right then A else B` over integers: `then_branch` when the
    /// comparison holds, else `else_branch`. Both branches are recorded, and a consumer
    /// takes the one the comparison selects.
    Select {
        comparison: InfixOperator,
        left: u32,
        right: u32,
        then_branch: u32,
        else_branch: u32,
        /// Where the arms' nodes are, when the construct that recorded this knows.
        region: Option<SelectRegion>,
    },
    /// The machine instruction a `#instruction` function is, applied to `input`, a record
    /// whose fields carry `#register`. The node's type names the registers it yields.
    Instruction {
        text: String,
        /// The compile-time text given for each `$name` of `text`, as (name, symbol).
        symbols: Vec<(String, String)>,
        input: u32,
    },
    /// A `#load` call: the element at `index` from `pointer`, typed as this node.
    Load { pointer: u32, index: u32 },
    /// A `#store` call: write `value` as the element at `index` from `pointer`.
    Store { pointer: u32, index: u32, value: u32 },
    /// `operand` is unchanged; this node's type is the wider range the value is declared to
    /// hold, which is what a loop must assume of a local it changes.
    Widen { operand: u32 },
    /// `for counter in start to end`, inclusive, run pass by pass. The nodes from
    /// `body_start` up to this one are the body: they run once per pass, with `counter`
    /// (a `LoopCounter` node) the pass's number and each carry's `placeholder` (a
    /// `LoopCarried` node) the value carried in. Nothing else of the body is visible
    /// outside it; the carried values after the last pass are `LoopResult` nodes.
    Loop {
        start: u32,
        end: u32,
        counter: u32,
        body_start: u32,
        carried: Vec<CheckedSourceLoopCarry>,
    },
    /// `while left <comparison> right do body`, run pass by pass. The nodes from `body_start` up
    /// to `split` compute `left` and `right` from the carried values (each carry's `placeholder`,
    /// a `LoopCarried` node, is the value held on entering the pass); those from `split` up to
    /// this node are the body, which runs only while the comparison holds. Nothing else of
    /// either is visible outside; the carried values when the comparison first fails are
    /// `LoopResult` nodes.
    While {
        comparison: InfixOperator,
        /// The loop goes on while the comparison does *not* hold: the recorded comparisons
        /// have no not-equal, so `a /= b` is a negated `a = b`.
        negated: bool,
        left: u32,
        right: u32,
        body_start: u32,
        split: u32,
        carried: Vec<CheckedSourceLoopCarry>,
        /// The carried value that is set once a `return` inside the body has run: the loop
        /// continues only while it is still zero. Its companion in `carried` holds the value
        /// returned. `None` for a loop with no `return`.
        exit: Option<u32>,
    },
    /// The value `mut` parameter `index` (into the callee's `mutated`) holds after the
    /// call `call` returns.
    CallOut { call: u32, index: u32 },
    /// `ref base.[start; n]` (or `mut`): the part of the borrowed array `base` that begins `start`
    /// elements in. The array is its address, so this is that address moved on `start` elements;
    /// the length `n` is the view's sibling field and the bound is the checker's claim.
    SubArray { base: u32, start: u32 },
    /// `[element; count]`: an owned array of `count` copies of `element`, held at an address (a
    /// slot of the function that makes it).
    RepeatArray { element: u32, count: u32 },
    /// A text literal used at run time as a read-only view of its bytes (`len`, `data`): its
    /// registers are the literal's UTF-8 byte length and the address of those bytes, which
    /// lowering places in a read-only global of the program.
    TextBytes { text: String },
    /// `#size(T)` (or `#alignment(T)` when `alignment`) of a type that depends on a type
    /// parameter: a number only once the instance is made, when lowering replaces it by one.
    Layout { queried: SourceType, alignment: bool },
    /// The number of the current pass of the enclosing loop.
    LoopCounter,
    /// A value carried into the current pass of the enclosing loop.
    LoopCarried,
    /// The carried value `index` of `looped` (a `Loop` node) after its last pass.
    LoopResult { looped: u32, index: u32 },
    /// A source block. `statements` are in source order; `result` is the final
    /// value or return.
    Block { statements: Vec<u32>, result: u32 },
    /// The `parts` side by side, typed as a `Sequence` of their types: no tag, so its
    /// registers are the parts' registers in order. An exit's result and `mut` outputs, or the
    /// locals an arm of a conditional changed.
    Bundle { parts: Vec<u32> },
    /// Part `index` of the bundle `bundle`.
    Part { bundle: u32, index: u32 },
    /// A value of this node's type that is never read: what a loop that can `return` carries
    /// as its returned value before any pass has returned.
    Undefined,
}

fn push(
    arena: &mut Vec<CheckedSourceBodyExpr>,
    span: SpanId,
    ty: SourceType,
    kind: CheckedSourceBodyKind,
) -> u32 {
    let id = u32::try_from(arena.len()).expect("body size is bounded by the checked source");
    arena.push(CheckedSourceBodyExpr { span, ty, kind });
    id
}

/// Names bound by locals and destructuring in the body being recorded, innermost last,
/// each to the node holding its value.
type Scope<'a> = Vec<(&'a str, u32)>;

/// The value a node already holds at compile time: an integer, or a record of such.
fn constant(arena: &[CheckedSourceBodyExpr], id: u32) -> Option<eval::Evaluated> {
    match &arena[id as usize].kind {
        CheckedSourceBodyKind::Integer(value) => Some(eval::Evaluated::Integer(*value)),
        CheckedSourceBodyKind::Widen { operand } => constant(arena, *operand),
        CheckedSourceBodyKind::Text(text) => Some(eval::Evaluated::Text(text.clone())),
        CheckedSourceBodyKind::Construct { schema, fields } => fields
            .iter()
            .map(|field| constant(arena, *field))
            .collect::<Option<Vec<_>>>()
            .map(|fields| eval::Evaluated::Record {
                schema: *schema,
                fields,
            }),
        _ => None,
    }
}

/// The integer a node holds at compile time, whatever range it is declared to hold.
fn known_integer(arena: &[CheckedSourceBodyExpr], id: u32) -> Option<I256> {
    match &arena[id as usize].kind {
        CheckedSourceBodyKind::Integer(value) => Some(*value),
        CheckedSourceBodyKind::Widen { operand } => known_integer(arena, *operand),
        _ => None,
    }
}

/// What a guard inside a loop tests, as the syntax gives it.
enum Test<'a> {
    Condition(&'a Expression),
    Pattern(&'a WhenIsStatement),
}

/// What a guard inside a loop tests, recorded: a comparison of two numbers, or which member a
/// value is.
#[derive(Clone)]
enum ExitTest {
    Compare {
        relation: super::guards::Relation,
        left: u32,
        right: u32,
        /// For a membership, the upper bound; `right` is then the lower.
        upper: Option<u32>,
    },
    Tag {
        subject: u32,
        members: Vec<usize>,
        schema: usize,
    },
}

/// The arms of a case that nest patterns, one level: the payload they match and, per variant
/// of it, the arm's body or the level below.
struct NestedGroup {
    payload: u32,
    arms: Vec<(u32, NestedArm)>,
}

enum NestedArm {
    Body(u32),
    Group(NestedGroup),
}

impl NestedGroup {
    /// Place `body` at the path `levels` names (each level's payload and variant).
    fn insert(&mut self, levels: &[(u32, u32)], body: u32) -> Option<()> {
        let (&(_, schema), rest) = levels.split_first()?;
        let Some(&(next, _)) = rest.first() else {
            self.arms.push((schema, NestedArm::Body(body)));
            return Some(());
        };
        match self.arms.iter_mut().find(|(other, _)| *other == schema) {
            Some((_, NestedArm::Group(inner))) => inner.insert(rest, body),
            Some(_) => None,
            None => {
                let mut inner = NestedGroup { payload: next, arms: Vec::new() };
                inner.insert(rest, body)?;
                self.arms.push((schema, NestedArm::Group(inner)));
                Some(())
            }
        }
    }
}

/// The type of the node that is one of `nodes`: their common type, or for integers the
/// smallest domain holding them all.
fn join(arena: &[CheckedSourceBodyExpr], nodes: impl Iterator<Item = u32>) -> Option<SourceType> {
    let mut joined: Option<SourceType> = None;
    for node in nodes {
        let ty = arena[node as usize].ty;
        joined = Some(match (joined, ty) {
            (None, ty) => ty,
            (
                Some(SourceType::Integer { lower, upper }),
                SourceType::Integer {
                    lower: next_lower,
                    upper: next_upper,
                },
            ) => SourceType::Integer {
                lower: lower.min(next_lower),
                upper: upper.max(next_upper),
            },
            (Some(first), ty) if first == ty => ty,
            _ => return None,
        });
    }
    joined
}

/// Nodes a single fold may evaluate before the call is left as a call.
const FOLD_FUEL: u64 = 10_000;

/// Recorded bodies reachable from one function: `Local` callees are looked up from
/// `base` in the function table. A `Dependency` callee is not resolved, so evaluation
/// stops (the call stays a call) at a dependency of a dependency.
struct TableBodies<'m> {
    functions: &'m [Option<Rc<calls::Function>>],
    base: usize,
}

impl eval::BodyResolver for TableBodies<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        let CheckedSourceCallee::Local { function } = callee else {
            return None;
        };
        let entry = self.functions.get(self.base + function as usize)?;
        entry.as_ref()?.body.as_deref()
    }
}

fn push_integer(arena: &mut Vec<CheckedSourceBodyExpr>, span: SpanId, value: I256) -> u32 {
    let ty = SourceType::Integer {
        lower: value,
        upper: value,
    };
    push(arena, span, ty, CheckedSourceBodyKind::Integer(value))
}

/// The operator the checker's integer rules are keyed by.
fn infix_of(operator: CheckedArithmetic) -> InfixOperator {
    match operator {
        CheckedArithmetic::Add => InfixOperator::Add,
        CheckedArithmetic::Subtract => InfixOperator::Subtract,
        CheckedArithmetic::Multiply => InfixOperator::Multiply,
        CheckedArithmetic::Divide => InfixOperator::Divide,
        CheckedArithmetic::Remainder => InfixOperator::Remainder,
        CheckedArithmetic::BitAnd => InfixOperator::BitAnd,
        CheckedArithmetic::BitOr => InfixOperator::BitOr,
        CheckedArithmetic::BitXor => InfixOperator::BitXor,
        CheckedArithmetic::ShiftLeft => InfixOperator::ShiftLeft,
        CheckedArithmetic::ShiftRight => InfixOperator::ShiftRight,
    }
}

/// Record `left <operator> right`, evaluating it now when both operands are constants.
/// `start` is the arena length before the operands, so folded operands leave no nodes.
fn record_arithmetic(
    operator: CheckedArithmetic,
    span: SpanId,
    start: usize,
    (left, right): (u32, u32),
    arena: &mut Vec<CheckedSourceBodyExpr>,
) -> Option<u32> {
    let (
        SourceType::Integer {
            lower: left_lower,
            upper: left_upper,
        },
        SourceType::Integer {
            lower: right_lower,
            upper: right_upper,
        },
    ) = (arena[left as usize].ty, arena[right as usize].ty)
    else {
        return None;
    };
    if let (Some(a), Some(b)) = (known_integer(arena, left), known_integer(arena, right)) {
        let value = eval::apply(operator, a, b)?;
        arena.truncate(start);
        return Some(push_integer(arena, span, value));
    }
    let (lower, upper) = match operator {
        CheckedArithmetic::Add | CheckedArithmetic::Subtract | CheckedArithmetic::Multiply => eval::interval(
            operator,
            (left_lower, left_upper),
            (right_lower, right_upper),
        )?,
        other => integer_ops::recorded_domain(
            infix_of(other),
            (left_lower, left_upper),
            (right_lower, right_upper),
        )?,
    };
    let ty = SourceType::Integer { lower, upper };
    let kind = CheckedSourceBodyKind::Binary {
        operator,
        left,
        right,
    };
    Some(push(arena, span, ty, kind))
}

/// `in lower to upper`: an annotation naming an integer range.
fn is_integer_range(annotation: &Spanned<Annotation>) -> bool {
    matches!(
        &annotation.value,
        Annotation::Domain {
            is_keyword: None,
            domain,
        } if domain.refinements.is_empty() && matches!(domain.base.value, DomainBase::Range { .. })
    )
}

/// Most passes of a loop a recording unrolls, and most nodes a body may grow to.
const RECORD_UNROLL_MAX: u32 = 64;
const RECORD_NODES_MAX: usize = 1 << 16;

/// Recursion bound for the recording walk. The source was already accepted, but a
/// deeply nested group must not turn a fail-closed recorder into a stack overflow.
const RECORD_DEPTH_MAX: usize = 256;

impl<'a> Checker<'a> {
    /// Record the executable body for the real world. Returns `None` when any
    /// construct is not yet representable, so lowering fails closed.
    pub(super) fn record_body(
        &mut self,
        binding: &'a Binding,
        parameter_count: usize,
    ) -> Option<CheckedSourceBody> {
        self.recording_bundle = false;
        self.recording_outputs_differ = false;
        let conversions = self.conversions_recorded;
        let first = self.record_body_once(binding, parameter_count);
        if first.is_some() || !self.recording_outputs_differ || self.recording_failure.is_some() {
            return first;
        }
        // Exits that hand back different `mut` values: each exit returns its own bundle.
        let decline = self.recording_decline.take();
        self.conversions_recorded = conversions;
        self.recording_bundle = true;
        let second = self.record_body_once(binding, parameter_count);
        self.recording_bundle = false;
        if second.is_none() && self.recording_decline.is_none() {
            self.recording_decline = decline;
        }
        second
    }

    fn record_body_once(
        &mut self,
        binding: &'a Binding,
        parameter_count: usize,
    ) -> Option<CheckedSourceBody> {
        let body = binding.value.as_ref()?;
        let mut expressions = Vec::new();
        let mut scope = Vec::new();
        self.recording_effects = false;
        self.recording_decline = None;
        self.recording_at = None;
        self.recording_folds.clear();
        self.recording_failure = None;
        self.recording_mutated.clear();
        self.recording_output_types.clear();
        self.recording_outputs = None;
        // A `mut` integer parameter is a local of the body that starts as the argument: the
        // caller gets back the value it holds where the body returns. The leading roles of a
        // method of a generic type are its type parameters: they are no values and have no name.
        let mut value_index = 0usize;
        for (index, passing) in self.proof.parameter_passing.iter().enumerate() {
            if !matches!(self.proof.parameter_roles.get(index), Some(calls::Parameter::Value(_))) {
                continue;
            }
            let at = value_index;
            value_index += 1;
            // An owned record parameter is a local of the body too: a `mut` call on it
            // (`writer.discard`) rebinds it there, and nothing is handed back.
            if *passing == calls::Passing::Owner
                && at < parameter_count
                && let Some(&(name, value)) = self.proof.names.get(at)
                && matches!(value.ty, SourceType::Aggregate { .. })
            {
                let node = push(
                    &mut expressions,
                    binding.name.span_id,
                    value.ty,
                    CheckedSourceBodyKind::Parameter { index: at as u32 },
                );
                scope.push((name, node));
                continue;
            }
            if *passing != calls::Passing::Mut {
                continue;
            }
            let (name, value) = *self.proof.names.get(at)?;
            if at >= parameter_count {
                return None;
            }
            let ty = match (value.ty, value.integer_domain) {
                (SourceType::Integer { .. }, Some((lower, upper))) => {
                    SourceType::Integer { lower, upper }
                }
                (ty @ SourceType::Integer { .. }, None) => ty,
                // A record is handed back as its fields, like a number is as itself.
                (ty @ SourceType::Aggregate { .. }, _) => ty,
                _ => return None,
            };
            let node = push(
                &mut expressions,
                binding.name.span_id,
                ty,
                CheckedSourceBodyKind::Parameter {
                    index: at as u32,
                },
            );
            scope.push((name, node));
            self.recording_mutated.push(name);
            self.recording_output_types.push(ty);
        }
        let root = self.record_expr(
            &body.expression,
            parameter_count,
            0,
            &mut expressions,
            &mut scope,
        )?;
        // A value written straight as the body is returned, so it converts like a return.
        let root = match (&body.expression.value, self.recording_result) {
            (Expr::Block(_), _) | (_, None) => root,
            (_, Some(result)) => {
                self.coerce(root, result, body.expression.span_id, &mut expressions)?
            }
        };
        let outputs = match self.recording_outputs.take() {
            Some(outputs) => outputs,
            // A body that never returns has nothing to hand back.
            None if self.recording_mutated.is_empty() => Vec::new(),
            None => return None,
        };
        let (root, outputs) = if self.recording_bundle {
            self.unbundle_root(root, &mut expressions)?
        } else {
            (root, outputs)
        };
        Some(CheckedSourceBody {
            effects: self.recording_effects,
            expressions,
            root,
            outputs,
        })
    }

    /// Record an expression; when it cannot be placed, the first such construct (the innermost,
    /// reached first) is kept as the reason the body is declined.
    fn record_expr(
        &mut self,
        expression: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let recorded = self.record_expr_inner(expression, parameter_count, depth, arena, scope);
        if recorded.is_none() && self.recording_decline.is_none() {
            let (kind, construct) = describe_construct(expression);
            self.recording_decline = Some(self.decline_of(kind, construct, expression.span_id));
        }
        recorded
    }

    pub(super) fn decline_of(&self, kind: DeclineKind, construct: impl Into<String>, at: SpanId) -> BodyDecline {
        let span = self.span_of(at);
        let located = !span.is_empty() && self.text_of(at).is_some();
        BodyDecline {
            kind,
            construct: construct.into(),
            span: located.then_some(span),
            file: located.then(|| at.file() as usize),
        }
    }

    fn record_expr_inner(
        &mut self,
        expression: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        if depth > RECORD_DEPTH_MAX {
            return None;
        }
        let span = expression.span_id;
        match &expression.value {
            Expr::Block(block) => {
                self.record_block(expression, block, parameter_count, depth, arena, scope)
            }
            Expr::Group(child) => self.record_expr(child, parameter_count, depth + 1, arena, scope),
            // `()`: the unit value, which has no registers.
            Expr::Sequence(sequence) if sequence.elements.is_empty() => {
                Some(push(arena, span, SourceType::Unit, CheckedSourceBodyKind::Unit))
            }
            // `#unchecked(x)`: the acceptance is the checker's; the value is `x`.
            Expr::Call { .. } if is_unchecked(expression) => {
                let value = unchecked_operand(expression)?;
                self.record_expr(value, parameter_count, depth + 1, arena, scope)
            }
            // `value as Target`. An address is one register and so is a number or a domain: a
            // cast to or from a pointer keeps the register and changes only the node's type. Any
            // other cast converts as the checker did (a number built into a domain, a domain read
            // as its number).
            Expr::Cast { value, target, .. } => {
                // `address as ref [item; n]`: the array starts at the address, which is one
                // register; the length is the sibling field, so the node only changes its type.
                if let Expr::PrefixedType(prefixed) = &target.value
                    && let Expr::Array(array) = &prefixed.target.value
                    && array.length.is_some()
                    && array.elements.len() == 1
                {
                    let node = self.record_expr(value, parameter_count, depth + 1, arena, scope)?;
                    let element = self.cast_target(&array.elements[0]).ok()?;
                    let ty = self.slice_type(element).ok()?;
                    return Some(push(arena, expression.span_id, ty, CheckedSourceBodyKind::Widen { operand: node }));
                }
                let node = self.record_expr(value, parameter_count, depth + 1, arena, scope)?;
                let to = self.cast_target(target).ok()?;
                let from = arena[node as usize].ty;
                let address = |ty: SourceType| matches!(ty, SourceType::Pointer { .. });
                if address(to) || address(from) {
                    let register = |ty: SourceType| {
                        matches!(ty, SourceType::Integer { .. } | SourceType::Pointer { .. })
                    };
                    // A domain's value is its number: read it first, so the register is a plain one.
                    let node = if address(to) && !register(from) {
                        let before = self.conversions_recorded;
                        let read = self.coerce(node, SourceType::Integer { lower: I256::ZERO, upper: I256::from(u64::MAX) }, expression.span_id, arena)?;
                        // Reading a domain's number for an address is not a conversion the
                        // checker counted (`reconstruct` reads it without one).
                        self.conversions_recorded = before;
                        read
                    } else {
                        node
                    };
                    if !register(to) && !address(to) {
                        // A pointer read as a domain: not reproduced here.
                        return None;
                    }
                    return Some(push(
                        arena,
                        expression.span_id,
                        to,
                        CheckedSourceBodyKind::Widen { operand: node },
                    ));
                }
                // A domain written as another domain: its number first, then that number built
                // into the target. The checker counts the one conversion, so the first step is
                // not counted.
                let from = arena[node as usize].ty;
                if matches!(from, SourceType::Aggregate { .. })
                    && matches!(to, SourceType::Aggregate { .. })
                    && let Some((from_lower, from_upper)) = self.domain_bounds_of(from)
                    && let Some((to_lower, to_upper)) = self.domain_bounds_of(to)
                {
                    // The checker accepted the cast, so the number lies in the target's range
                    // wherever this runs (a guard `x is in D` is how it was shown).
                    let (lower, upper) = (from_lower.max(to_lower), from_upper.min(to_upper));
                    if lower > upper {
                        return None;
                    }
                    let before = self.conversions_recorded;
                    let number = self.coerce(node, SourceType::Integer { lower: from_lower, upper: from_upper }, expression.span_id, arena)?;
                    self.conversions_recorded = before;
                    let number = push(
                        arena,
                        expression.span_id,
                        SourceType::Integer { lower, upper },
                        CheckedSourceBodyKind::Widen { operand: number },
                    );
                    return self.coerce(number, to, expression.span_id, arena);
                }
                self.coerce(node, to, expression.span_id, arena)
            }
            // `deref x`: the value a borrow refers to, copied out. A borrowed parameter is passed
            // as the value itself (only `mut` is handed back), so the copy is a read of it. The
            // checker already refused what would move a linear value out of a borrow
            // (`copy_out`), so what reaches here is a copy.
            Expr::Unary { operator, operand } if operator.value == UnaryOperator::Deref => {
                self.record_expr(operand, parameter_count, depth + 1, arena, scope)
            }
            // `a.[i]` on a borrowed array (`data ref [element; len]`): the array is the address
            // of its first element, so the read is a typed load at `i`. The bound is the
            // checker's claim, as for `load`. An array held inline is not an address: declined.
            Expr::Select(select) if select.length.is_none() => {
                let base = self.record_expr(&select.base, parameter_count, depth + 1, arena, scope)?;
                let SourceType::Array { id } = arena[base as usize].ty else {
                    return None;
                };
                let instance = self.module.instances.get(id as usize)?;
                // A borrowed array, or an owned one of a known length (held at an address too).
                if !matches!(instance.head, CheckedSourceInstanceHead::Slice | CheckedSourceInstanceHead::Array { .. }) {
                    return None;
                }
                let element = *instance.arguments.first()?;
                let index = self.record_expr(&select.index, parameter_count, depth + 1, arena, scope)?;
                Some(push(arena, span, element, CheckedSourceBodyKind::Load { pointer: base, index }))
            }
            // `ref a.[start; n]` / `mut a.[start; n]`: a part of a borrowed array.
            Expr::PrefixedType(prefixed)
                if matches!(prefixed.keyword.value, TypePrefixKind::Ref | TypePrefixKind::Mut)
                    && prefixed.group.is_none()
                    && matches!(&prefixed.target.value, Expr::Select(select) if select.length.is_some()) =>
            {
                let Expr::Select(select) = &prefixed.target.value else {
                    return None;
                };
                let base = self.record_expr(&select.base, parameter_count, depth + 1, arena, scope)?;
                let ty = arena[base as usize].ty;
                let SourceType::Array { id } = ty else {
                    return None;
                };
                if self.module.instances.get(id as usize)?.head != CheckedSourceInstanceHead::Slice {
                    return None;
                }
                let start = self.record_expr(&select.index, parameter_count, depth + 1, arena, scope)?;
                let start = self.read_number(start, select.index.span_id, arena)?;
                Some(push(arena, span, ty, CheckedSourceBodyKind::SubArray { base, start }))
            }
            // `ref a.[i]` of a scalar element is a read of it: only `mut` is handed back as an
            // address, and that is not built.
            Expr::PrefixedType(prefixed)
                if prefixed.keyword.value == TypePrefixKind::Ref
                    && prefixed.group.is_none()
                    && matches!(prefixed.target.value, Expr::Select(_)) =>
            {
                self.record_expr(&prefixed.target, parameter_count, depth + 1, arena, scope)
            }
            Expr::Bind(binding) => {
                let value = binding.value.as_ref()?;
                let mut node =
                    self.record_expr(&value.expression, parameter_count, depth + 1, arena, scope)?;
                let annotation = binding.annotation.as_ref()?;
                if is_integer_range(annotation) {
                    if !matches!(arena[node as usize].ty, SourceType::Integer { .. }) {
                        return None;
                    }
                } else {
                    let declared = self.recorded_annotation(annotation)?;
                    node = self.coerce(node, declared, value.expression.span_id, arena)?;
                }
                Some(node)
            }
            Expr::Name(name) => {
                // A function with no parameters, named alone, is called (`print_newline`).
                if let Some(target) = self.bare_callee(name, parameter_count, scope) {
                    return self.record_invocation(
                        expression,
                        target,
                        None,
                        &[],
                        parameter_count,
                        depth,
                        arena,
                        scope,
                    );
                }
                // A variant with no payload that the checker built over type arguments: its type
                // says which of the several variants of that name it is.
                if !scope.iter().any(|(local, _)| local == name)
                    && !self.proof.names.iter().any(|(other, _)| other == name)
                    && let Some(schema) = self.variant_built_here(expression, name)
                    && self.module.layouts[schema].schema.fields.is_empty()
                {
                    let ty = self.info_of(span)?.result;
                    return Some(push(
                        arena,
                        span,
                        ty,
                        CheckedSourceBodyKind::Construct { schema: schema as u32, fields: Vec::new() },
                    ));
                }
                self.record_name(name, span, parameter_count, arena, scope)
            }
            Expr::Match(case) => {
                self.record_match(expression, case, parameter_count, depth, arena, scope)
            }
            // A compile-time query the checker settled to one number (`#size(T)`, `#alignment(T)`,
            // `#lower(r)`, `#upper(r)` with known arguments) is that number. One that depends on
            // a type parameter is an interval, not a number, and is not recorded.
            Expr::Call { callee, .. }
                if matches!(&callee.value, Expr::Query { .. })
                    && self.query_values.iter().any(|(at, _)| *at == expression.span_id) =>
            {
                let value = self.query_values.iter().rev().find(|(at, _)| *at == expression.span_id)?.1;
                Some(push_integer(arena, span, value))
            }
            // `#size(T)` / `#alignment(T)` of a type that is not known until an instance is made.
            Expr::Call { callee, arguments }
                if matches!(&callee.value, Expr::Query { name, .. } if name.value == "size" || name.value == "alignment")
                    && arguments.value.len() == 1 =>
            {
                let Expr::Query { name, .. } = &callee.value else {
                    return None;
                };
                let Argument::Positional(written) = &arguments.value[0].value else {
                    return None;
                };
                let queried = self.cast_target(written).ok()?;
                let ty = SourceType::Integer { lower: I256::ZERO, upper: I256::from(1i64 << 62) };
                Some(push(arena, span, ty, CheckedSourceBodyKind::Layout { queried, alignment: name.value == "alignment" }))
            }
            // `level.value` on a union whose variants have values (see `sums.rs`).
            Expr::Member { base, name } if name.value == "value" && self.discriminant_union(base).is_some() => {
                self.record_discriminant(span, base, parameter_count, depth, arena, scope)
            }
            // `#target.register_bits` and `#target.triple` are facts of the target being checked.
            Expr::Member { base, name }
                if matches!(&base.value, Expr::Query { name: query, .. } if query.value == "target") =>
            {
                match name.value.as_str() {
                    "register_bits" => Some(push_integer(arena, span, I256::from(self.target.register_bits))),
                    "triple" => Some(push(
                        arena,
                        span,
                        SourceType::Text,
                        CheckedSourceBodyKind::Text(self.target.triple.to_owned()),
                    )),
                    _ => None,
                }
            }
            // `#lower(T)` / `#upper(T)` of a named domain are the numbers it declares.
            Expr::Call { callee, arguments }
                if matches!(&callee.value, Expr::Query { name, .. } if matches!(name.value.as_str(), "lower" | "upper")) =>
            {
                let Expr::Query { name: query, .. } = &callee.value else {
                    return None;
                };
                let [argument] = arguments.value.as_slice() else {
                    return None;
                };
                let Argument::Positional(Spanned {
                    value: Expr::Name(type_name),
                    ..
                }) = &argument.value
                else {
                    return None;
                };
                let schema = &self.module.layouts[self.recorded_record(type_name)?].schema;
                let Some(SourceType::Integer { lower, upper }) =
                    schema.fields.first().map(|field| field.ty)
                else {
                    return None;
                };
                if schema.kind != AggregateKind::Domain {
                    return None;
                }
                let bound = if query.value == "lower" { lower } else { upper };
                Some(push_integer(arena, span, bound))
            }
            Expr::Call { callee, arguments } => self.record_call(
                expression,
                callee,
                arguments,
                parameter_count,
                depth,
                arena,
                scope,
            ),
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                let infix = InfixOperator::from_binary(operator.value)?;
                let start = arena.len();
                let left = self.record_expr(left, parameter_count, depth + 1, arena, scope)?;
                let right = self.record_expr(right, parameter_count, depth + 1, arena, scope)?;
                let (mut left, mut right) = (left, right);
                if let SourceType::Aggregate { .. } = arena[left as usize].ty {
                    // An open domain with no operator of its own is the number it holds.
                    let ty = arena[left as usize].ty;
                    let open = (infix.arithmetic().is_some() || infix.is_integer_operation())
                        && self.is_open_domain(ty)
                        && self.find_infix(ty, infix).ok()?.is_none();
                    if !open {
                        return self.record_infix(span, infix, (left, right), arena);
                    }
                    left = self.read_number(left, span, arena)?;
                    right = self.read_number(right, span, arena)?;
                }
                let node =
                    record_arithmetic(infix.recordable()?, span, start, (left, right), arena)?;
                Some(node)
            }
            Expr::Chain(_) => {
                self.record_chain(expression, 0, parameter_count, depth, arena, scope)
            }
            Expr::Conditional(case) => {
                if let Some((negated, test)) = super::guards::variant_test(&case.condition) {
                    return self.record_variant_conditional(
                        case,
                        negated,
                        test,
                        span,
                        parameter_count,
                        depth,
                        arena,
                        scope,
                    );
                }
                let (_, leaves) = super::guards::conditions(&case.condition)?;
                let [leaf] = leaves[..] else {
                    return None;
                };
                let test = self.record_leaf(&leaf, parameter_count, depth, arena, scope)?;
                // An arm that changes a local of the enclosing body gives it beside its value: the
                // arms are recorded once to learn which, then again ending in bundles.
                let kept = scope.len();
                let start = arena.len();
                let saved = (self.conversions_recorded, self.recording_folds.len());
                let mut changed: Vec<usize> = Vec::new();
                let mut lead: Option<SourceType> = None;
                for pass in 0..2 {
                    // Each value is typed by what the test tells of the numbers it compares.
                    let then_start = arena.len() as u32;
                    let mut inside = scope.clone();
                    self.narrow_by_test(&leaf, &test, true, parameter_count, arena, &mut inside);
                    let narrowed = inside.clone();
                    let mut then_branch =
                        self.record_expr(&case.then_branch, parameter_count, depth + 1, arena, &mut inside)?;
                    let mut found: Vec<usize> = (0..kept).filter(|&i| inside[i] != narrowed[i]).collect();
                    if let Some(ty) = lead {
                        then_branch = self.lead_to(then_branch, ty, span, arena)?;
                        then_branch = self.arm_bundle(Some(then_branch), &changed, &inside, &narrowed, scope, span, arena)?;
                        self.bundle_lead_as(then_branch, ty, arena)?;
                    }
                    let else_start = arena.len() as u32;
                    let mut inside = scope.clone();
                    self.narrow_by_test(&leaf, &test, false, parameter_count, arena, &mut inside);
                    let narrowed = inside.clone();
                    let mut else_branch =
                        self.record_expr(&case.else_branch, parameter_count, depth + 1, arena, &mut inside)?;
                    found.extend((0..kept).filter(|&i| inside[i] != narrowed[i]));
                    found.sort_unstable();
                    found.dedup();
                    if let Some(ty) = lead {
                        else_branch = self.lead_to(else_branch, ty, span, arena)?;
                        else_branch = self.arm_bundle(Some(else_branch), &changed, &inside, &narrowed, scope, span, arena)?;
                        self.bundle_lead_as(else_branch, ty, arena)?;
                    }
                    if found != changed {
                        if pass == 1 {
                            return None;
                        }
                        // The value's type both arms are widened to: one type, or a joined range.
                        let arms = || [then_branch, else_branch].into_iter();
                        lead = Some(
                            join(arena, arms())
                                .or_else(|| self.sum_of(arena, arms()))
                                .or_else(|| self.sum_of_generic(arena, arms()))?,
                        );
                        arena.truncate(start);
                        self.conversions_recorded = saved.0;
                        self.recording_folds.truncate(saved.1);
                        changed = found;
                        continue;
                    }
                    let region = SelectRegion {
                        then_start,
                        else_start,
                        exclusive: true,
                    };
                    let chosen = self.select_node(&test, then_branch, else_branch, span, Some(region), arena)?;
                    if changed.is_empty() {
                        return Some(chosen);
                    }
                    self.unpack_changed(&changed, 1, chosen, scope, span, arena)?;
                    return Some(push(arena, span, lead?, CheckedSourceBodyKind::Part { bundle: chosen, index: 0 }));
                }
                None
            }
            // `#target.triple` is text and `#target.register_bits` a number, both known now.
            Expr::Member { base, name }
                if matches!(&base.value, Expr::Query { name: query, .. } if query.value == "target") =>
            {
                match name.value.as_str() {
                    "triple" => Some(push(
                        arena,
                        span,
                        SourceType::Text,
                        CheckedSourceBodyKind::Text(self.target.triple.to_owned()),
                    )),
                    "register_bits" => {
                        Some(push_integer(arena, span, I256::from(self.target.register_bits)))
                    }
                    _ => None,
                }
            }
            Expr::Member { base, name }
                if matches!(&base.value, Expr::Name(union)
                    if !scope.iter().any(|(local, _)| *local == union)
                        && !self.proof.names.iter().any(|(other, _)| other == union)
                        && self.recorded_qualified_variant(union, &name.value).is_some()) =>
            {
                let Expr::Name(union) = &base.value else {
                    return None;
                };
                let schema = self.recorded_qualified_variant(union, &name.value)?;
                let ty = self.recorded_instance(schema)?;
                Some(push(
                    arena,
                    span,
                    ty,
                    CheckedSourceBodyKind::Construct {
                        schema: schema as u32,
                        fields: Vec::new(),
                    },
                ))
            }
            // `Union.Member`: the member written out, as a value (it has no fields to give).
            Expr::Member { base, name }
                if matches!(&base.value, Expr::Name(union) if self.recorded_qualified_variant(union, &name.value).is_some()) =>
            {
                let Expr::Name(union) = &base.value else {
                    return None;
                };
                let schema = self.recorded_qualified_variant(union, &name.value)?;
                if !self.module.layouts[schema].schema.fields.is_empty() {
                    return None;
                }
                let ty = self.recorded_instance(schema)?;
                Some(push(
                    arena,
                    span,
                    ty,
                    CheckedSourceBodyKind::Construct { schema: schema as u32, fields: Vec::new() },
                ))
            }
            Expr::Member { .. } => {
                // A method called without parentheses, else a field of a record.
                match self.record_bare_method(expression, parameter_count, depth, arena, scope) {
                    Some(node) => Some(node),
                    None => self.record_field(expression, parameter_count, depth, arena, scope),
                }
            }
            Expr::Loop(loop_expression) => self.record_loop(
                expression,
                loop_expression,
                parameter_count,
                depth,
                arena,
                scope,
            ),
            // `[value; n]` with a known `n`: an owned array of copies of an unrestricted element.
            Expr::Array(array) if array.elements.len() == 1 && array.length.is_some() => {
                let length = array.length.as_ref()?;
                let element = self.record_expr(&array.elements[0], parameter_count, depth + 1, arena, scope)?;
                let length = self.record_expr(&length.value, parameter_count, depth + 1, arena, scope)?;
                let count = known_integer(arena, length)?;
                let count = count.to_u32()?;
                let element_ty = arena[element as usize].ty;
                let ty = self.array_type(element_ty, count).ok()?;
                Some(push(arena, span, ty, CheckedSourceBodyKind::RepeatArray { element, count }))
            }
            Expr::Literal(LiteralKind::String) => {
                let text = quoted(self.literal_text(span)?)?;
                Some(push(
                    arena,
                    span,
                    SourceType::Text,
                    CheckedSourceBodyKind::Text(text.to_owned()),
                ))
            }
            Expr::Literal(LiteralKind::Integer) => {
                let text = self.literal_text(span)?;
                let value = decode_integer_accumulator(text).ok()?.checked_neg()?;
                let ty = SourceType::Integer {
                    lower: value,
                    upper: value,
                };
                Some(push(arena, span, ty, CheckedSourceBodyKind::Integer(value)))
            }
            // `-5`: a negative literal is the number itself (the literal accumulates negatively).
            Expr::Unary { operator, operand }
                if operator.value == UnaryOperator::Negate
                    && matches!(operand.value, Expr::Literal(LiteralKind::Integer)) =>
            {
                let value = decode_integer_accumulator(self.literal_text(operand.span_id)?).ok()?;
                Some(push_integer(arena, span, value))
            }
            _ => None,
        }
    }

    /// Convert `node` to the type the context expects, as the checker did: a domain value
    /// read as its integer, and an integer built into a domain (reduced first when the
    /// domain wraps and the value is outside it). Anything else is left as it is.
    fn coerce(
        &mut self,
        node: u32,
        expected: SourceType,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        let mut node = node;
        let ty = arena[node as usize].ty;
        if ty == expected {
            return Some(node);
        }
        // A text discriminant read (`level.value`) where a view of bytes is wanted: each arm is
        // its literal's bytes, from the read-only table the literals' globals make.
        if ty == SourceType::Text
            && self.is_byte_view(expected)
            && let CheckedSourceBodyKind::Match { arms, .. } = &arena[node as usize].kind
            && arms.iter().all(|arm| matches!(arena[arm.body as usize].kind, CheckedSourceBodyKind::Text(_)))
        {
            let bodies: Vec<u32> = arms.iter().map(|arm| arm.body).collect();
            for body in bodies {
                let CheckedSourceBodyKind::Text(text) = &arena[body as usize].kind else {
                    return None;
                };
                let text = text.clone();
                arena[body as usize].kind = CheckedSourceBodyKind::TextBytes { text };
                arena[body as usize].ty = expected;
            }
            arena[node as usize].ty = expected;
            self.conversions_recorded += 1;
            return Some(node);
        }
        // A literal where a read-only view of bytes is wanted.
        if ty == SourceType::Text
            && let CheckedSourceBodyKind::Text(contents) = &arena[node as usize].kind
            && self.is_byte_view(expected)
        {
            let text = contents.clone();
            // The checker built the view's length into its domain once.
            self.conversions_recorded += 1;
            return Some(push(arena, span, expected, CheckedSourceBodyKind::TextBytes { text }));
        }
        if let Some((lower, upper)) = self.domain_bounds_of(ty)
            && matches!(
                expected,
                SourceType::Integer { .. } | SourceType::Aggregate { .. }
            )
        {
            // A known construction yields its field; otherwise read the hidden integer.
            node = match &arena[node as usize].kind {
                CheckedSourceBodyKind::Construct { fields, .. } => *fields.first()?,
                _ => push(
                    arena,
                    span,
                    SourceType::Integer { lower, upper },
                    CheckedSourceBodyKind::Field {
                        record: node,
                        index: 0,
                    },
                ),
            };
            // An `#unrestricted` domain read as its number outside its own methods is no
            // conversion to the checker (`open_number`), so it is not counted here either.
            let open_read = self.is_open_domain(ty)
                && !(self.receiver_is_domain() && self.proof.receiver == Some(ty));
            if !open_read {
                self.conversions_recorded += 1;
            }
        }
        let SourceType::Integer {
            lower: low,
            upper: high,
        } = arena[node as usize].ty
        else {
            return Some(node);
        };
        let (Some((lower, upper)), SourceType::Aggregate { id }) =
            (self.domain_bounds_of(expected), expected)
        else {
            return Some(node);
        };
        let mut inner = node;
        if low < lower || high > upper {
            // Outside the domain: the checker only accepted it because the domain wraps.
            inner = match arena[node as usize].kind {
                CheckedSourceBodyKind::Integer(value) => {
                    push_integer(arena, span, eval::wrap_into(value, lower, upper)?)
                }
                _ => push(
                    arena,
                    span,
                    SourceType::Integer { lower, upper },
                    CheckedSourceBodyKind::Wrap { operand: node },
                ),
            };
        }
        self.conversions_recorded += 1;
        let schema = self.module.instances[id as usize].schema() as u32;
        Some(push(
            arena,
            span,
            expected,
            CheckedSourceBodyKind::Construct {
                schema,
                fields: vec![inner],
            },
        ))
    }

    /// The type a local's written annotation names, for the forms a recording can resolve
    /// without the checker's metering: a record or domain name, `Self`, and `in Self`.
    fn recorded_annotation(&mut self, annotation: &'a Spanned<Annotation>) -> Option<SourceType> {
        match &annotation.value {
            Annotation::Expression(expression) => {
                let Expr::Name(name) = &expression.value else {
                    // A type applied to arguments (`Live(item)`): the checker's own resolution.
                    return self.resolve_type(expression, &self.generic_scope()).ok();
                };
                if name == "Self" {
                    return self.proof.receiver;
                }
                match self.recorded_record(name) {
                    Some(schema) => self.recorded_instance(schema),
                    // A union or another type the checker resolves.
                    None => self.resolve_type(expression, &self.generic_scope()).ok(),
                }
            }
            Annotation::Domain {
                is_keyword: None,
                domain,
            } if domain.refinements.is_empty() => match &domain.base.value {
                DomainBase::Expression(expression) if matches!(&expression.value, Expr::Name(name) if name == "Self") =>
                {
                    let (lower, upper) = self.domain_bounds_of(self.proof.receiver?)?;
                    Some(SourceType::Integer { lower, upper })
                }
                // `in SignedBigInt`: the number a domain holds, in the domain's range.
                DomainBase::Expression(expression) => {
                    let ty = self.resolve_type(expression, &self.generic_scope()).ok()?;
                    let (lower, upper) = self.domain_bounds_of(ty)?;
                    Some(SourceType::Integer { lower, upper })
                }
                // Integer ranges stay unrecorded: only nominal locals are in this slice.
                DomainBase::Range { .. } => None,
            },
            _ => None,
        }
    }

    /// `left <operator> right` on a nominal left operand: a call of its `#infix` method.
    fn record_infix(
        &mut self,
        span: SpanId,
        operator: InfixOperator,
        (left, right): (u32, u32),
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        let ty = arena[left as usize].ty;
        let function = self.find_infix(ty, operator).ok()??;
        let interface = self.module.functions.get(function)?.clone()?;
        if interface.passing.contains(&calls::Passing::Mut) {
            return None;
        }
        let callee = self.callee_ref(function)?;
        let mut arguments = vec![left, right];
        for (argument, parameter) in arguments.iter_mut().zip(&interface.parameters) {
            let calls::Parameter::Value(expected) = parameter else {
                return None;
            };
            *argument = self.coerce(*argument, *expected, span, arena)?;
        }
        let kind = CheckedSourceBodyKind::Call {
            callee,
            arguments,
            type_arguments: Vec::new(),
        };
        Some(push(arena, span, interface.signature.result, kind))
    }

    #[allow(clippy::too_many_arguments)] // the recording walk threads its state explicitly
    fn record_call(
        &mut self,
        expression: &'a Expression,
        callee: &'a Expression,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        // `Union.Variant(value)`: the variant built with its payload.
        if let Expr::Member { base, name } = &callee.value
            && let Expr::Name(union) = &base.value
            && !scope.iter().any(|(local, _)| *local == union)
            && !self.proof.names.iter().any(|(other, _)| other == union)
            && let Some(schema) = self.recorded_qualified_variant(union, &name.value)
            && !self.module.layouts[schema].schema.fields.is_empty()
        {
            return self.record_construct_schema(expression, schema, arguments, parameter_count, depth, arena, scope);
        }
        if let Expr::Member { base, name } = &callee.value {
            let shape = (&**base, name.value.as_str());
            if let Some(bound) = self.record_bound_call(
                expression,
                shape,
                &arguments.value,
                parameter_count,
                depth,
                arena,
                scope,
            ) {
                return bound;
            }
            let (function, interface, receiver) =
                self.recorded_method(base, &name.value, parameter_count, depth, arena, scope)?;
            self.recording_receiver = receiver.map(|_| &**base);
            let recorded = self.record_invocation(
                expression,
                (function, interface),
                receiver,
                &arguments.value,
                parameter_count,
                depth,
                arena,
                scope,
            );
            self.recording_receiver = None;
            return recorded;
        }
        let Expr::Name(name) = &callee.value else {
            return None;
        };
        let Some(target) = self.recorded_callee(name) else {
            return self.record_construct(
                expression,
                name,
                arguments,
                parameter_count,
                depth,
                arena,
                scope,
            );
        };
        self.record_invocation(
            expression,
            target,
            None,
            &arguments.value,
            parameter_count,
            depth,
            arena,
            scope,
        )
    }

    /// `record.field`: the field's node when the record is a known construction, otherwise
    /// a `Field` read of the record's node.
    fn record_field(
        &mut self,
        expression: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let Expr::Member { base, name } = &expression.value else {
            return None;
        };
        let record = self.record_expr(base, parameter_count, depth + 1, arena, scope)?;
        let SourceType::Aggregate { id } = arena[record as usize].ty else {
            return None;
        };
        let schema = self.module.instances[id as usize].schema();
        let layout = &self.module.layouts[schema].schema;
        if !layout.parameters.is_empty() && layout.kind != AggregateKind::Domain {
            return self.record_generic_field(expression, record, &name.value, arena);
        }
        if !layout.parameters.is_empty() || layout.kind == AggregateKind::Domain {
            return None;
        }
        let index = layout
            .fields
            .iter()
            .position(|field| field.name == name.value)?;
        let ty = layout.fields[index].ty;
        if let CheckedSourceBodyKind::Construct { fields, .. } = &arena[record as usize].kind {
            return fields.get(index).copied();
        }
        Some(push(
            arena,
            expression.span_id,
            ty,
            CheckedSourceBodyKind::Field {
                record,
                index: u32::try_from(index).ok()?,
            },
        ))
    }

    /// `p.sum` written without parentheses: a call that supplies only the receiver.
    fn record_bare_method(
        &mut self,
        expression: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let Expr::Member { base, name } = &expression.value else {
            return None;
        };
        let shape = (&**base, name.value.as_str());
        if let Some(bound) =
            self.record_bound_call(expression, shape, &[], parameter_count, depth, arena, scope)
        {
            return bound;
        }
        let mark = arena.len();
        let (function, interface, receiver) =
            self.recorded_method(base, &name.value, parameter_count, depth, arena, scope)?;
        // A plain accessor (`origin(self ref) Address: return self.base`) read from a place is
        // the field it returns: the checker reads it that way and keeps no call, so neither does
        // the recording.
        if let Some(node) = receiver
            && let Some(path) = self.plain_accessor_field(arena[node as usize].ty, &name.value)
        {
            return self.record_accessor_path(expression.span_id, node, &path, arena, 0);
        }
        let _ = mark;
        self.recording_receiver = receiver.map(|_| &**base);
        let recorded = self.record_invocation(
            expression,
            (function, interface),
            receiver,
            &[],
            parameter_count,
            depth,
            arena,
            scope,
        );
        self.recording_receiver = None;
        recorded
    }

    /// Read `path` (`bytes.length`) from the record `node`: each hop is a field, or a plain
    /// accessor of the type reached, which reads on in the same way.
    fn record_accessor_path(
        &mut self,
        span: SpanId,
        node: u32,
        path: &str,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        depth: usize,
    ) -> Option<u32> {
        if depth > 8 {
            return None;
        }
        let mut current = node;
        for segment in path.split('.') {
            let ty = arena[current as usize].ty;
            let (SourceType::Aggregate { id } | SourceType::Sum { id }) = ty else {
                return None;
            };
            let schema = self.module.instances[id as usize].schema();
            let has_field = self.module.layouts[schema].schema.fields.iter().any(|f| f.name == segment);
            if has_field {
                if self.module.layouts[schema].schema.parameters.is_empty() {
                    let index = self.module.layouts[schema].schema.fields.iter().position(|f| f.name == segment)?;
                    let field_ty = self.module.layouts[schema].schema.fields[index].ty;
                    current = push(
                        arena,
                        span,
                        field_ty,
                        CheckedSourceBodyKind::Field { record: current, index: u32::try_from(index).ok()? },
                    );
                } else {
                    current = self.record_generic_field_named(span, current, segment, arena)?;
                }
            } else {
                let inner = self.plain_accessor_field(ty, segment)?;
                current = self.record_accessor_path(span, current, &inner, arena, depth + 1)?;
            }
        }
        Some(current)
    }

    /// The field a plain accessor method of `ty` returns, when `method` is one.
    fn plain_accessor_field(&self, ty: SourceType, method: &str) -> Option<String> {
        let (SourceType::Aggregate { id } | SourceType::Sum { id }) = ty else {
            return None;
        };
        let schema = self.module.instances.get(id as usize)?.schema();
        let owner = &self.module.layouts.get(schema)?.schema.name;
        if !self.plain_accessors.iter().any(|(type_name, name)| type_name == owner && name == method) {
            return None;
        }
        self.accessors
            .iter()
            .find(|(type_name, name, _)| type_name == owner && name == method)
            .map(|(_, _, field)| field.clone())
    }

    /// Resolve `base.method` to `Type.method`. A local receiver is recorded and becomes the
    /// first argument; a type name is a path and supplies none.
    fn recorded_method(
        &mut self,
        base: &'a Expression,
        method: &str,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<(usize, Rc<calls::Function>, Option<u32>)> {
        // `Union.Variant.method`: the receiver is a variant written out, and the method is
        // the union's.
        if let Expr::Member {
            base: union,
            name: variant,
        } = &base.value
        {
            if let Expr::Name(union) = &union.value
                && let Some(schema) = self.recorded_qualified_variant(union, &variant.value)
            {
                let owner = self.module.layouts[schema].schema.name.clone();
                let node = self.record_expr(base, parameter_count, depth + 1, arena, scope)?;
                let (function, interface) = self.recorded_method_of(&owner, method)?;
                return Some((function, interface, Some(node)));
            }
            // A field path of a local (`self.inner.size`): the field is the receiver.
            let mut root = base;
            while let Expr::Member { base: next, .. } = &root.value {
                root = next;
            }
            let Expr::Name(head) = &root.value else {
                return None;
            };
            let local = scope.iter().any(|(local, _)| *local == head.as_str())
                || self
                    .proof
                    .names
                    .iter()
                    .position(|(other, _)| *other == head.as_str())
                    .is_some_and(|position| position < parameter_count);
            if !local {
                return None;
            }
            let node = self.record_expr(base, parameter_count, depth + 1, arena, scope)?;
            let (SourceType::Aggregate { id } | SourceType::Sum { id }) = arena[node as usize].ty
            else {
                return None;
            };
            let schema = self.module.instances[id as usize].schema();
            let owner = self.module.layouts[schema].schema.name.clone();
            let (function, interface) = self.recorded_method_of(&owner, method)?;
            return Some((function, interface, Some(node)));
        }
        // `Type(arguments).function`: the type is written out; the checker noted its arguments.
        if let Expr::Call { callee: written, .. } = &base.value {
            let Expr::Name(written) = &written.value else {
                return None;
            };
            let schema = self.recorded_generic_schema(written)?;
            let owner = self.module.layouts[schema].schema.name.clone();
            let (function, interface) = self.recorded_method_of(&owner, method)?;
            return Some((function, interface, None));
        }
        let Expr::Name(head) = &base.value else {
            return None;
        };
        let local = scope.iter().any(|(local, _)| *local == head.as_str())
            || self
                .proof
                .names
                .iter()
                .position(|(other, _)| *other == head.as_str())
                .is_some_and(|position| position < parameter_count);
        let (owner, receiver) = if local {
            let node = self.record_expr(base, parameter_count, depth + 1, arena, scope)?;
            let (SourceType::Aggregate { id } | SourceType::Sum { id }) = arena[node as usize].ty
            else {
                return None;
            };
            let schema = self.module.instances[id as usize].schema();
            (self.module.layouts[schema].schema.name.clone(), Some(node))
        } else {
            // A generic type's function, called through the type: its arguments are inferred.
            let schema = self
                .recorded_record(head)
                .or_else(|| self.recorded_generic_schema(head))?;
            (self.module.layouts[schema].schema.name.clone(), None)
        };
        let (function, interface) = self.recorded_method_of(&owner, method)?;
        Some((function, interface, receiver))
    }

    /// Uncharged: `owner.method`, or for a variant a union declared (`Union.Variant`), the
    /// method of its union.
    fn recorded_method_of(
        &self,
        owner: &str,
        method: &str,
    ) -> Option<(usize, Rc<calls::Function>)> {
        self.recorded_callee(&format!("{owner}.{method}"))
            .or_else(|| {
                let (union, _) = owner.rsplit_once('.')?;
                self.recorded_callee(&format!("{union}.{method}"))
            })
    }

    #[allow(clippy::too_many_arguments)] // the recording walk threads its state explicitly
    fn record_invocation(
        &mut self,
        expression: &'a Expression,
        (function, interface): (usize, Rc<calls::Function>),
        receiver: Option<u32>,
        arguments: &'a [Spanned<Argument>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let receiver_place = self.recording_receiver.take();
        // `Range(domain of Self).min()`: a function that is one bound of its type argument is the
        // number the checker knew at this call, whatever its instance would compute.
        if interface.bound_query.is_some()
            && let Some(info) = self.info_of(expression.span_id)
            && let Some(known) = self.known_bound(interface.bound_query, &info.types.clone())
        {
            return Some(push_integer(arena, expression.span_id, known.lower));
        }
        let effects = interface.body.as_ref().is_some_and(|body| body.effects);
        self.recording_effects |= effects || interface.signature.instruction.is_some();
        if let Some(op) = interface.signature.memory {
            self.recording_effects = true;
            return self.record_memory(expression.span_id, op, &interface, (receiver, arguments, parameter_count, depth), arena, scope);
        }
        if interface.signature.instruction.is_some() {
            let span = expression.span_id;
            let shape = (receiver, arguments, parameter_count, depth);
            return self.record_instruction(span, &interface, shape, arena, scope);
        }
        let values_only = interface
            .parameters
            .iter()
            .all(|parameter| matches!(parameter, calls::Parameter::Value(_)));
        // A `mut` argument is a number lent from a local of this body; the call hands back
        // what it holds afterwards. Nothing else a call may change is recorded.
        let mutated = &interface.signature.mutated;
        // The receiver of a call is lent `mut` from a local, like an argument.
        if !mutated.is_empty()
            && ((receiver.is_some() && receiver_place.is_none())
                || self.recording_mutation_unsupported(&interface))
        {
            return None;
        }
        if !values_only || !interface.signature.type_parameters.is_empty() {
            let shape = (receiver, arguments, parameter_count, depth);
            self.recording_receiver = receiver_place;
            let recorded = self.record_generic_invocation(expression, (function, &interface), shape, arena, scope);
            self.recording_receiver = None;
            return recorded;
        }
        if arguments.len() + usize::from(receiver.is_some()) != interface.parameters.len() {
            return None;
        }
        let start = arena.len();
        let mut ids = Vec::with_capacity(arguments.len() + 1);
        ids.extend(receiver);
        for argument in arguments {
            let Argument::Positional(value) = &argument.value else {
                return None;
            };
            ids.push(self.record_expr(value, parameter_count, depth + 1, arena, scope)?);
        }
        for (id, parameter) in ids.iter_mut().zip(&interface.parameters) {
            let calls::Parameter::Value(expected) = parameter else {
                return None;
            };
            // A domain value given where its number is taken (`self in Self`) is read, not
            // converted: the checker counts no conversion for it, as for a binding `in D`.
            let before = self.conversions_recorded;
            let reads_number = matches!(expected, SourceType::Integer { .. })
                && self.domain_bounds_of(arena[*id as usize].ty).is_some();
            *id = self.coerce(*id, *expected, expression.span_id, arena)?;
            if reads_number {
                self.conversions_recorded = before;
            }
        }
        if mutated.is_empty()
            && let Some(value) = self.fold_call(function, &interface, &ids, arena)
        {
            // Every input is known: the call is its value. The callee is remembered, since the
            // body no longer shows the dependency.
            if let Some(callee) = self.callee_ref(function) {
                self.recording_folds.push(callee);
            }
            arena.truncate(start);
            return self.push_value(
                &value,
                interface.signature.result,
                expression.span_id,
                arena,
            );
        }
        let callee = self.callee_ref(function)?;
        let kind = CheckedSourceBodyKind::Call {
            callee,
            arguments: ids,
            type_arguments: Vec::new(),
        };
        let call = push(
            arena,
            expression.span_id,
            interface.signature.result,
            kind,
        );
        // Each lent local now names what the call handed back.
        let tys: Vec<SourceType> = mutated
            .iter()
            .map(|&parameter| interface.signature.parameters[parameter as usize])
            .collect();
        let mut places: Vec<Option<&'a Expression>> = Vec::new();
        places.extend(receiver.map(|_| receiver_place));
        for argument in arguments {
            places.push(match &argument.value {
                Argument::Positional(value) => Some(value),
                _ => None,
            });
        }
        self.record_handed_back(expression.span_id, call, mutated, &tys, &places, arena, scope)?;
        Some(call)
    }

    /// After a call that lent places `mut`: each now names the value the call handed back
    /// (`CallOut`), the record rebuilt around it when the place was a field path. `receiver` is
    /// the place the receiver was written as, when the call had one (it is parameter 0).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_handed_back(
        &mut self,
        span: SpanId,
        call: u32,
        mutated: &[u32],
        types: &[SourceType],
        places: &[Option<&'a Expression>],
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        for (index, &parameter) in mutated.iter().enumerate() {
            // `places` is by value parameter: the receiver first, then the written values.
            let value = (*places.get(parameter as usize)?)?;
            // A local, or a field path of one: the record is rebuilt around what came back.
            let (root, path) = assign::split_place(value)?;
            let position = scope.iter().rposition(|(local, _)| *local == root)?;
            let declared = arena[scope[position].1 as usize].ty;
            let ty = types[index];
            // A sum narrowed to a member is not a plain record: its registers differ.
            if matches!(declared, SourceType::Sum { .. }) && !matches!(ty, SourceType::Sum { .. }) {
                return None;
            }
            let mut out = push(
                arena,
                span,
                ty,
                CheckedSourceBodyKind::CallOut {
                    call,
                    index: u32::try_from(index).ok()?,
                },
            );
            if path.is_empty() {
                if declared != ty {
                    out = push(arena, span, declared, CheckedSourceBodyKind::Widen { operand: out });
                }
                scope[position].1 = out;
            } else {
                let record = scope[position].1;
                scope[position].1 = self.replace_field(record, &path, out, span, arena)?;
            }
        }
        Some(())
    }

    /// Whether lending to this call is beyond what recording can follow: every `mut`
    /// parameter must take a number or a record.
    fn recording_mutation_unsupported(&self, interface: &calls::Function) -> bool {
        interface.signature.mutated.iter().any(|&index| {
            !matches!(
                interface.signature.parameters.get(index as usize),
                Some(SourceType::Integer { .. } | SourceType::Aggregate { .. })
            )
        })
    }

    /// The text `expression` folds to. It is recorded in the body, where its locals are, and
    /// then taken back out so that nothing of it remains: it is read at compile time and
    /// never run.
    fn constant_text(
        &mut self,
        expression: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<String> {
        let start = arena.len();
        let node = self.record_expr(expression, parameter_count, depth + 1, arena, scope);
        let text = match &arena[node? as usize].kind {
            CheckedSourceBodyKind::Text(text) => Some(text.clone()),
            _ => None,
        };
        arena.truncate(start);
        text
    }

    /// A call of a `#load` or `#store` function: the element type is the type argument, and
    /// the values are the pointer, the index and, for a store, the element written.
    fn record_memory(
        &mut self,
        span: SpanId,
        op: MemoryOp,
        interface: &calls::Function,
        (receiver, arguments, parameter_count, depth): (
            Option<u32>,
            &'a [Spanned<Argument>],
            usize,
            usize,
        ),
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        if receiver.is_some() || arguments.len() != interface.parameters.len() {
            return None;
        }
        let mut types = Vec::new();
        let mut values = Vec::new();
        for (argument, parameter) in arguments.iter().zip(&interface.parameters) {
            let Argument::Positional(value) = &argument.value else {
                return None;
            };
            match parameter {
                calls::Parameter::Type => types.push(self.resolve_type(value, &self.generic_scope()).ok()?),
                calls::Parameter::Value(_) => {
                    values.push(self.record_expr(value, parameter_count, depth + 1, arena, scope)?);
                }
                calls::Parameter::Literal => return None,
            }
        }
        let result = self.substitute(interface.signature.result, &types).ok()?;
        let kind = match (op, values.as_slice()) {
            (MemoryOp::Load, &[pointer, index]) => CheckedSourceBodyKind::Load { pointer, index },
            (MemoryOp::Store, &[pointer, index, value]) => {
                CheckedSourceBodyKind::Store { pointer, index, value }
            }
            _ => return None,
        };
        Some(push(arena, span, result, kind))
    }

    /// A call of a `#instruction` function: a literal names the symbol, type arguments the
    /// result, and the one value argument is the record of registers going in.
    fn record_instruction(
        &mut self,
        span: SpanId,
        interface: &calls::Function,
        (receiver, arguments, parameter_count, depth): (
            Option<u32>,
            &'a [Spanned<Argument>],
            usize,
            usize,
        ),
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        if receiver.is_some()
            || !self.proof.generics.is_empty()
            || arguments.len() != interface.parameters.len()
        {
            return None;
        }
        let mut types = Vec::new();
        let mut symbols: Vec<(String, String)> = Vec::new();
        let mut input = None;
        for (argument, parameter) in arguments.iter().zip(&interface.parameters) {
            let Argument::Positional(value) = &argument.value else {
                return None;
            };
            match parameter {
                calls::Parameter::Type => types.push(self.resolve_type(value, &[]).ok()?),
                calls::Parameter::Value(_) if input.is_none() => {
                    let node = self.record_expr(value, parameter_count, depth + 1, arena, scope)?;
                    input = Some(node);
                }
                calls::Parameter::Value(_) => return None,
                calls::Parameter::Literal => {
                    // Compile-time text, whatever it was written as: a literal, or a method
                    // such as `Symbol.Putchar.to_string` that folds to one.
                    let text = self
                        .constant_text(value, parameter_count, depth, arena, scope)
                        .filter(|text| symbol_name(text));
                    let name = interface.signature.literals.get(symbols.len());
                    let (Some(text), Some(name)) = (text, name) else {
                        self.recording_failure = Some(SourceCheckError::InstructionTarget);
                        return None;
                    };
                    symbols.push((name.clone(), text));
                }
            }
        }
        let result = self.substitute(interface.signature.result, &types).ok()?;
        let text = interface.signature.instruction.clone()?;
        let input = input?;
        let kind = CheckedSourceBodyKind::Instruction {
            text,
            symbols,
            input,
        };
        Some(push(arena, span, result, kind))
    }

    /// The value of a call whose arguments are all constants and whose callee has a
    /// recorded body: its own, or the body its defining dependency carries in its capsule.
    /// `function` is the callee's index in this module's function table.
    fn fold_call(
        &self,
        function: usize,
        interface: &calls::Function,
        arguments: &[u32],
        arena: &[CheckedSourceBodyExpr],
    ) -> Option<eval::Evaluated> {
        let known: Vec<eval::Evaluated> = arguments
            .iter()
            .map(|id| constant(arena, *id))
            .collect::<Option<_>>()?;
        let target = interface.body.as_ref()?;
        // Calls inside an imported body name functions of its defining module, whose own
        // functions sit contiguously from `base` in this table.
        let own = self.module.functions.len() - self.module.imported.len();
        let base = match function.checked_sub(own) {
            None => 0,
            Some(index) => {
                let &(_, position) = self.module.imported.get(index)?;
                function.checked_sub(position as usize)?
            }
        };
        let resolver = TableBodies {
            functions: &self.module.functions,
            base,
        };
        let mut fuel = FOLD_FUEL;
        eval::evaluate_value(target, &known, &resolver, &mut fuel)
    }

    /// Nodes holding `value`, typed `ty`: an integer is an `Integer`, a record is a
    /// `Construct` of its fields.
    fn push_value(
        &self,
        value: &eval::Evaluated,
        ty: SourceType,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        match (value, ty) {
            (eval::Evaluated::Integer(value), SourceType::Integer { .. }) => {
                Some(push_integer(arena, span, *value))
            }
            (eval::Evaluated::Text(text), SourceType::Text) => Some(push(
                arena,
                span,
                SourceType::Text,
                CheckedSourceBodyKind::Text(text.clone()),
            )),
            (
                eval::Evaluated::Record {
                    schema,
                    fields: values,
                },
                SourceType::Aggregate { .. } | SourceType::Sum { .. },
            ) => {
                // A sum holds one of its members: the node is typed as that member.
                let ty = self.recorded_instance(*schema as usize)?;
                let fields = &self.module.layouts.get(*schema as usize)?.schema.fields;
                if fields.len() != values.len() {
                    return None;
                }
                let ids = values
                    .iter()
                    .zip(fields)
                    .map(|(value, field)| self.push_value(value, field.ty, span, arena))
                    .collect::<Option<Vec<_>>>()?;
                Some(push(
                    arena,
                    span,
                    ty,
                    CheckedSourceBodyKind::Construct {
                        schema: *schema,
                        fields: ids,
                    },
                ))
            }
            _ => None,
        }
    }

    /// Uncharged: the non-generic sum whose members include the type of every node.
    fn sum_of(
        &self,
        arena: &[CheckedSourceBodyExpr],
        nodes: impl Iterator<Item = u32> + Clone,
    ) -> Option<SourceType> {
        self.module
            .instances
            .iter()
            .enumerate()
            .find_map(|(id, instance)| {
                let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
                    return None;
                };
                let variants = &self.module.layouts.get(schema as usize)?.schema.variants;
                let joined = self.head_type(instance.head, id as u32);
                // Each value is one of the members, or already the union itself (a value an
                // earlier choice between members produced).
                let sum = instance.arguments.is_empty()
                    && nodes.clone().all(|node| {
                        let ty = arena[node as usize].ty;
                        variants.contains(&ty) || ty == joined
                    });
                (sum && !variants.is_empty()).then_some(joined)
            })
    }

    /// Uncharged: the schema a non-generic record is declared by.
    fn recorded_record(&self, name: &str) -> Option<usize> {
        if name == "Self" {
            // Inside a type's methods, `Self` is the receiver's own type.
            let SourceType::Aggregate { id } = self.proof.receiver? else {
                return None;
            };
            let schema = self.module.instances.get(id as usize)?.schema();
            let layout = self.module.layouts.get(schema)?;
            return (layout.schema.parameters.is_empty() && layout.schema.variants.is_empty())
                .then_some(schema);
        }
        let symbols = &self.module.type_names;
        let position = symbols
            .binary_search_by(|(other, _)| other.as_str().cmp(name))
            .ok()?;
        let schema = symbols[position].1;
        let layout = self.module.layouts.get(schema)?;
        (layout.schema.parameters.is_empty() && layout.schema.variants.is_empty()).then_some(schema)
    }

    /// Uncharged: the variants a `when` over a value of `ty` can name. A sum lists them; a
    /// single variant is looked up in any sum that holds it.
    fn recorded_variants(&self, ty: SourceType) -> Vec<SourceType> {
        match ty {
            SourceType::Sum { id } => {
                let schema = self.module.instances[id as usize].schema();
                self.module.layouts[schema].schema.variants.clone()
            }
            SourceType::Aggregate { .. } => self
                .module
                .layouts
                .iter()
                .find(|layout| layout.schema.variants.contains(&ty))
                .map(|layout| layout.schema.variants.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The member `variant` of the sum `sum` as a type of that sum's instance: a generic sum's
    /// variants are written with its parameters, and this instance gives them their arguments.
    fn member_of_instance(&self, sum: SourceType, variant: SourceType) -> Option<SourceType> {
        let SourceType::Sum { id } = sum else {
            return Some(variant);
        };
        let arguments = &self.module.instances.get(id as usize)?.arguments;
        if arguments.is_empty() {
            return Some(variant);
        }
        self.peek_substitute(variant, arguments)
    }

    /// Uncharged: the schema a pattern's constructor names, among `ty`'s variants first.
    fn recorded_pattern(&self, ty: SourceType, name: &str) -> Option<usize> {
        let variants = self.recorded_variants(ty);
        self.variant_schema(&variants, name)
            .or_else(|| self.recorded_record(name))
    }

    /// Uncharged: `Union.Variant` as a schema, for a union by name.
    fn recorded_qualified_variant(&self, union: &str, variant: &str) -> Option<usize> {
        let symbols = &self.module.type_names;
        let position = symbols
            .binary_search_by(|(other, _)| other.as_str().cmp(union))
            .ok()?;
        let schema = symbols[position].1;
        let variants = &self.module.layouts.get(schema)?.schema.variants;
        self.variant_schema(variants, variant)
    }

    /// Uncharged: the one variant a union declares inline, with a payload and no parameters,
    /// named `name` (`MemorySizeResult is MemorySize or Flaw(MemorySizeError)`).
    fn recorded_payload_variant(&self, name: &str) -> Option<usize> {
        let mut found = None;
        for (index, layout) in self.module.layouts.iter().enumerate() {
            let schema = &layout.schema;
            let declared = schema.variants.is_empty()
                && schema.parameters.is_empty()
                && !schema.fields.is_empty()
                && schema.name.rsplit_once('.').is_some_and(|(_, last)| last == name);
            if declared && found.replace(index).is_some() {
                return None;
            }
        }
        found
    }

    /// Uncharged: the one qualified variant a bare `name` can mean when no type is
    /// expected here. More than one is ambiguous, so it is left unrecorded.
    fn recorded_unique_variant(&self, name: &str) -> Option<usize> {
        let mut found = None;
        for (index, layout) in self.module.layouts.iter().enumerate() {
            let schema = &layout.schema;
            let qualified = schema.name.rsplit_once('.');
            let bare = schema.fields.is_empty()
                && schema.variants.is_empty()
                && schema.parameters.is_empty()
                && qualified.is_some_and(|(_, last)| last == name);
            if bare && found.replace(index).is_some() {
                return None;
            }
        }
        found
    }

    /// Uncharged: the checker interned this record's instance when it accepted the body.
    fn recorded_instance(&self, schema: usize) -> Option<SourceType> {
        let head = CheckedSourceInstanceHead::Nominal {
            schema: schema as u32,
        };
        let index = self
            .module
            .instance_index
            .binary_search_by(|id| {
                let instance = &self.module.instances[*id];
                instance
                    .head
                    .cmp(&head)
                    .then_with(|| instance.arguments.is_empty().cmp(&true))
            })
            .ok()?;
        Some(self.head_type(head, self.module.instance_index[index] as u32))
    }

    #[allow(clippy::too_many_arguments)]
    fn record_construct(
        &mut self,
        expression: &'a Expression,
        name: &str,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        // The variant the checker built, found by the type it gave the construction: a bare
        // `Failed(...)` is one of several functions' variants, told apart by the type.
        let built = self.variant_built_here(expression, name);
        if let Some(schema) = built
            && self.module.layouts[schema].schema.parameters.is_empty()
        {
            return self.record_construct_schema(expression, schema, arguments, parameter_count, depth, arena, scope);
        }
        let Some(schema) = built
            .is_none()
            .then(|| self.recorded_record(name).or_else(|| self.recorded_payload_variant(name)))
            .flatten()
        else {
            return self.record_generic_construct(
                expression,
                name,
                arguments,
                parameter_count,
                depth,
                arena,
                scope,
            );
        };
        self.record_construct_schema(expression, schema, arguments, parameter_count, depth, arena, scope)
    }

    /// Uncharged: when the checker typed `expression` as a variant called `name` (the last segment of
    /// a `Union.name` or `function.name` layout), that variant's schema.
    pub(super) fn variant_built_here(&self, expression: &Expression, name: &str) -> Option<usize> {
        let result = self.info_of(expression.span_id)?.result;
        // Built where a sum was expected: the variant of that name among the sum's.
        if matches!(result, SourceType::Sum { .. }) {
            let variants = self.recorded_variants(result);
            let schema = self.variant_schema(&variants, name)?;
            return self.module.layouts.get(schema)?.schema.name.contains('.').then_some(schema);
        }
        let SourceType::Aggregate { id } = result else {
            return None;
        };
        let schema = self.module.instances.get(id as usize)?.schema();
        let written = self.module.layouts.get(schema)?.schema.name.as_str();
        (written.contains('.') && written.rsplit('.').next() == Some(name)).then_some(schema)
    }

    /// `Name(a, b)` of the record or variant `schema`, the fields recorded in written order.
    fn record_construct_schema(
        &mut self,
        expression: &'a Expression,
        schema: usize,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let ty = self.recorded_instance(schema)?;
        let names: Vec<String> = self.module.layouts[schema]
            .schema
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();
        if names.len() != arguments.value.len() {
            return None;
        }
        // Fields are named, so values are recorded in written order and placed by name.
        let mut fields: Vec<Option<u32>> = vec![None; names.len()];
        for (position, argument) in arguments.value.iter().enumerate() {
            let (field, value) = match &argument.value {
                Argument::Named { name, value } => (name.value.as_str(), value),
                Argument::Typed { .. } => return None,
                // A variant's first payload is positional even when it has a name.
                Argument::Positional(value) if position == 0 && self.is_declared_variant(schema) => {
                    (names.first()?.as_str(), value)
                }
                Argument::Positional(value) => match &value.value {
                    Expr::Name(field) => (field.as_str(), value),
                    // A value that is not a field's name goes in the field at its position.
                    _ => (names.get(position)?.as_str(), value),
                },
            };
            let index = names.iter().position(|candidate| candidate == field)?;
            let id = self.record_expr(value, parameter_count, depth + 1, arena, scope)?;
            let declared = self.module.layouts[schema].schema.fields[index].ty;
            let id = self.coerce(id, declared, value.span_id, arena)?;
            if fields[index].replace(id).is_some() {
                return None;
            }
        }
        let fields = fields.into_iter().collect::<Option<Vec<_>>>()?;
        Some(push(
            arena,
            expression.span_id,
            ty,
            CheckedSourceBodyKind::Construct {
                schema: schema as u32,
                fields,
            },
        ))
    }

    /// `Name(a, b) := value`: bind each binder to the matching field of the record.
    fn record_destructure(
        &mut self,
        statement: &'a DestructureStatement,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let pattern = &statement.pattern.value;
        let record =
            self.record_expr(&statement.value, parameter_count, depth + 1, arena, scope)?;
        // A variant of the value's sum (`Value(n) := got`) first, else a record of that name.
        let held = match arena[record as usize].ty {
            SourceType::Sum { .. } => self.recorded_pattern(arena[record as usize].ty, &pattern.constructor.value),
            // A sum narrowed to its member by a guard is that member's value.
            SourceType::Aggregate { id } => {
                let schema = self.module.instances[id as usize].schema();
                let written = self.module.layouts[schema].schema.name.as_str();
                (written.rsplit('.').next() == Some(pattern.constructor.value.as_str())
                    && written.contains('.'))
                .then_some(schema)
            }
            _ => None,
        };
        let schema = held
        .or_else(|| self.recorded_record(&pattern.constructor.value))
        .or_else(|| self.recorded_generic_schema(&pattern.constructor.value))?;
        self.bind_fields(record, schema, pattern.fields.as_ref(), arena, scope)
    }

    /// Bind each binder of a pattern to the matching field of `record`: the field node
    /// itself when the record is a known construction, otherwise a `Field` read.
    /// Uncharged form of `pattern_order`: the field each binder of a pattern takes.
    fn recorded_pattern_order(
        &self,
        schema: usize,
        fields: &PatternFields,
    ) -> Option<Vec<usize>> {
        let count = fields.binders.value.len();
        if fields.selectors.iter().all(Option::is_none) {
            return Some((0..count).collect());
        }
        let declared = &self.module.layouts[schema].schema.fields;
        if count != declared.len() {
            return None;
        }
        let mut order = Vec::with_capacity(count);
        for (binder, selector) in fields.binders.value.iter().zip(&fields.selectors) {
            let name = selector.as_ref().unwrap_or(binder).value.as_str();
            let index = declared.iter().position(|field| field.name == name)?;
            if order.contains(&index) {
                return None;
            }
            order.push(index);
        }
        Some(order)
    }

    fn bind_fields(
        &self,
        record: u32,
        schema: usize,
        fields: Option<&'a PatternFields>,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let types = self.field_types_of(arena[record as usize].ty, schema)?;
        // A member named without parentheses binds nothing (it only narrows the subject).
        let Some(fields) = fields else {
            return Some(());
        };
        if fields.binders.value.len() != types.len() {
            return None;
        }
        let order = self.recorded_pattern_order(schema, fields)?;
        for (position, binder) in fields.binders.value.iter().enumerate() {
            let index = order[position];
            let node = match &arena[record as usize].kind {
                CheckedSourceBodyKind::Construct { fields, .. } => fields[index],
                _ => {
                    let kind = if matches!(arena[record as usize].ty, SourceType::Sum { .. }) {
                        CheckedSourceBodyKind::VariantField {
                            subject: record,
                            schema: schema as u32,
                            index: index as u32,
                        }
                    } else {
                        CheckedSourceBodyKind::Field {
                            record,
                            index: index as u32,
                        }
                    };
                    push(arena, binder.span_id, types[index], kind)
                }
            };
            scope.push((binder.value.as_str(), node));
        }
        Some(())
    }

    /// `when subject is Member(binders) then body ...` over a sum. A subject whose member
    /// is known is its arm and nothing else (the evaluation rule); otherwise every arm is
    /// recorded and the evaluator picks one.
    fn record_match(
        &mut self,
        expression: &'a Expression,
        case: &'a MatchExpression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let subject = self.record_expr(&case.subject, parameter_count, depth + 1, arena, scope)?;
        // A case over text is decided while compiling: the arm whose text the subject is, or
        // `else`. Text has no run-time form, so a subject that is not known text is not recorded.
        if arena[subject as usize].ty == SourceType::Text {
            let CheckedSourceBodyKind::Text(text) = &arena[subject as usize].kind else {
                return None;
            };
            let text = text.clone();
            for arm in &case.arms.value {
                let chosen = match &arm.value.pattern.value {
                    MatchPattern::Text(literal) => {
                        quoted(self.literal_text(literal.span_id)?)? == text
                    }
                    MatchPattern::Else { .. } => true,
                    _ => return None,
                };
                if chosen {
                    return self.record_expr(
                        &arm.value.body,
                        parameter_count,
                        depth + 1,
                        arena,
                        scope,
                    );
                }
            }
            return None;
        }
        let known = match &arena[subject as usize].kind {
            CheckedSourceBodyKind::Construct { schema, .. } => Some(*schema),
            _ => None,
        };
        let mut arms = Vec::new();
        // The arms that name a variant of the payload (`Flaw(OutOfMemory)`), per outer variant:
        // the payload node they match, and their inner arms. Each becomes one outer arm whose
        // body is a match over that payload, at the place of the first.
        let mut groups: Vec<(u32, NestedGroup)> = Vec::new();
        let mut group_places: Vec<(usize, usize)> = Vec::new();
        // An arm runs only when its member is the subject's, so what it assigns to an outer
        // local would be recorded as if every path did. Such an arm is left unrecorded.
        let outer = scope.clone();
        // Guarded arms waiting for the unguarded arm of the same member (and inner members) that
        // ends them: each becomes a choice, its guard's test picking its body over the rest.
        // They must be adjacent to it, so the nodes of each choice's branches are its own.
        let mut pending: Vec<((u32, Vec<(usize, &'a str)>), ExitTest, u32, SelectRegion)> = Vec::new();
        // Where each recorded arm's nodes begin: with two arms, the region a branch needs when
        // an arm runs an instruction.
        let mut starts: Vec<u32> = Vec::new();
        let mut arm_start = arena.len() as u32;
        for arm in &case.arms.value {
            if pending.is_empty() {
                arm_start = arena.len() as u32;
            }
            let MatchPattern::Variant(pattern) = &arm.value.pattern.value else {
                return None;
            };
            let subject_ty = arena[subject as usize].ty;
            let schema = self.recorded_pattern(subject_ty, &pattern.constructor.value)? as u32;
            // The patterns written inside it, level by level: where each stands and its name.
            let mut inner_key: Vec<(usize, &'a str)> = Vec::new();
            let mut walked: &'a VariantPattern = pattern;
            while let Some(fields) = &walked.fields
                && let Some(position) = fields.nested.iter().position(Option::is_some)
            {
                let inner = fields.nested[position].as_deref()?;
                inner_key.push((position, inner.value.constructor.value.as_str()));
                walked = &inner.value;
            }
            let nested = !inner_key.is_empty();
            if known.is_some() && nested {
                return None;
            }
            if known.is_some_and(|member| member != schema) {
                continue;
            }
            let key = (schema, inner_key.clone());
            if pending.last().is_some_and(|(other, ..)| *other != key) {
                return None;
            }
            let locals = scope.len();
            self.bind_fields(
                subject,
                schema as usize,
                pattern.fields.as_ref(),
                arena,
                scope,
            )?;
            if nested {
                // Each level's payload is bound by the level around it; its variant's binders
                // follow in scope.
                let mut levels: Vec<(u32, u32)> = Vec::new();
                let mut current: &'a VariantPattern = pattern;
                let mut level_locals = locals;
                for &(position, _) in &inner_key {
                    let inner = current.fields.as_ref()?.nested[position].as_deref()?;
                    let payload = scope.get(level_locals + position)?.1;
                    let payload_ty = arena[payload as usize].ty;
                    let inner_schema = self.recorded_pattern(payload_ty, &inner.value.constructor.value)? as u32;
                    level_locals = scope.len();
                    self.bind_fields(payload, inner_schema as usize, inner.value.fields.as_ref(), arena, scope)?;
                    levels.push((payload, inner_schema));
                    current = &inner.value;
                }
                if let Some(guard) = &arm.value.guard {
                    let entry = self.record_arm_guard(guard, &arm.value.body, key, parameter_count, depth, arena, scope, &outer)?;
                    pending.push(entry);
                    scope.truncate(locals);
                    continue;
                }
                let mut body =
                    self.record_expr(&arm.value.body, parameter_count, depth + 1, arena, scope)?;
                scope.truncate(locals);
                if *scope != outer {
                    return None;
                }
                body = self.close_guards(&mut pending, body, expression.span_id, arena)?;
                let at = match groups.iter().position(|(outer_schema, _)| *outer_schema == schema) {
                    Some(at) => at,
                    None => {
                        groups.push((schema, NestedGroup { payload: levels[0].0, arms: Vec::new() }));
                        group_places.push((arms.len(), groups.len() - 1));
                        arms.push(CheckedSourceMatchArm { schema, body });
                        groups.len() - 1
                    }
                };
                groups[at].1.insert(&levels, body)?;
                continue;
            }
            if let Some(guard) = &arm.value.guard {
                let entry = self.record_arm_guard(guard, &arm.value.body, key, parameter_count, depth, arena, scope, &outer)?;
                pending.push(entry);
                scope.truncate(locals);
                continue;
            }
            let mut body =
                self.record_expr(&arm.value.body, parameter_count, depth + 1, arena, scope)?;
            scope.truncate(locals);
            if *scope != outer {
                return None;
            }
            body = self.close_guards(&mut pending, body, expression.span_id, arena)?;
            starts.push(arm_start);
            arms.push(CheckedSourceMatchArm { schema, body });
        }
        if !pending.is_empty() {
            return None;
        }
        if known.is_some() {
            return arms.first().map(|arm| arm.body);
        }
        for (place, group) in group_places {
            let group = std::mem::replace(&mut groups[group].1, NestedGroup { payload: 0, arms: Vec::new() });
            arms[place].body = self.nested_match(group, expression.span_id, arena)?;
        }
        // Arms that give a number and a domain value (`Pending then 0`, `Done(c) then c`) give a
        // number: each domain value is read as its number, as the checker read it.
        let read_numbers = self.number_arms(&mut arms, expression.span_id, arena)?;
        let bodies = || arms.iter().map(|arm| arm.body);
        let ty = join(arena, bodies())
            .or_else(|| self.sum_of(arena, bodies()))
            .or_else(|| self.sum_of_generic(arena, bodies()))?;
        // Two plain arms, each its own run of nodes: a branch per arm when one runs an
        // instruction (`Done(copied) then copied.release`, `Failed(_) then return`).
        let region = match (&arms[..], &starts[..]) {
            ([first, _], [member_start, other_start])
                if groups.is_empty() && !read_numbers && *other_start > first.body =>
            {
                Some(MatchRegion { member_body: first.body, member_start: *member_start, other_start: *other_start })
            }
            // More arms, one of which calls or runs an instruction: arm `k` against a case of
            // the arms after it, so each choice is between two runs of nodes (one branch each).
            (_, _)
                if arms.len() > 2
                    && starts.len() == arms.len()
                    && groups.is_empty()
                    && !read_numbers
                    && arena[starts[0] as usize..].iter().any(|node| {
                        matches!(
                            node.kind,
                            CheckedSourceBodyKind::Call { .. }
                                | CheckedSourceBodyKind::Instruction { .. }
                                | CheckedSourceBodyKind::Load { .. }
                                | CheckedSourceBodyKind::Store { .. }
                        )
                    }) =>
            {
                let count = arms.len();
                let mut rest = arms[count - 1].body;
                let mut rest_arms = vec![arms[count - 1]];
                for k in (0..count - 1).rev() {
                    let member = arms[k];
                    let mut nested = vec![member];
                    nested.extend(rest_arms.iter().map(|arm| CheckedSourceMatchArm { schema: arm.schema, body: rest }));
                    let parts = [member.body, rest];
                    let part_ty = join(arena, parts.into_iter())
                        .or_else(|| self.sum_of(arena, parts.into_iter()))
                        .or_else(|| self.sum_of_generic(arena, parts.into_iter()))?;
                    let region = MatchRegion { member_body: member.body, member_start: starts[k], other_start: starts[k + 1] };
                    rest = push(
                        arena,
                        expression.span_id,
                        if k == 0 { ty } else { part_ty },
                        CheckedSourceBodyKind::Match { subject, arms: nested.clone(), region: Some(region) },
                    );
                    rest_arms = nested;
                }
                return Some(rest);
            }
            _ => None,
        };
        Some(push(
            arena,
            expression.span_id,
            ty,
            CheckedSourceBodyKind::Match {
                subject,
                arms,
                region,
            },
        ))
    }

    /// Arms that give a number and a domain value (`Pending then 0`, `Done(c) then c`) give a
    /// number: each domain value is read as its number, as the checker read it. The checker
    /// counts the one conversion where the case's value is used, not one per arm, so these reads
    /// are not counted. Whether any arm was read.
    fn number_arms(&mut self, arms: &mut [CheckedSourceMatchArm], span: SpanId, arena: &mut Vec<CheckedSourceBodyExpr>) -> Option<bool> {
        let mut read = false;
        if join(arena, arms.iter().map(|arm| arm.body)).is_none()
            && arms.iter().any(|arm| matches!(arena[arm.body as usize].ty, SourceType::Integer { .. }))
        {
            let before = self.conversions_recorded;
            for arm in arms.iter_mut() {
                let ty = arena[arm.body as usize].ty;
                if let Some((lower, upper)) = self.domain_bounds_of(ty) {
                    arm.body = self.coerce(arm.body, SourceType::Integer { lower, upper }, span, arena)?;
                    read = true;
                }
            }
            self.conversions_recorded = before;
        }
        Some(read)
    }

    /// One level of nested arms as a `Match` over its payload, the levels below first.
    fn nested_match(&mut self, group: NestedGroup, span: SpanId, arena: &mut Vec<CheckedSourceBodyExpr>) -> Option<u32> {
        let mut arms = Vec::with_capacity(group.arms.len());
        for (schema, arm) in group.arms {
            let body = match arm {
                NestedArm::Body(body) => body,
                NestedArm::Group(inner) => self.nested_match(inner, span, arena)?,
            };
            arms.push(CheckedSourceMatchArm { schema, body });
        }
        self.number_arms(&mut arms, span, arena)?;
        let bodies = || arms.iter().map(|arm| arm.body);
        let ty = join(arena, bodies())
            .or_else(|| self.sum_of(arena, bodies()))
            .or_else(|| self.sum_of_generic(arena, bodies()))?;
        Some(push(arena, span, ty, CheckedSourceBodyKind::Match { subject: group.payload, arms, region: None }))
    }

    /// A guarded arm of a case over a sum, its binders in scope: its guard's test (one
    /// comparison) and its body, recorded where the test holds, with the region of nodes that is
    /// the body's. The rest of the case, up to the arm that ends the guard, is the other branch.
    #[allow(clippy::too_many_arguments)] // one arm, threaded explicitly
    fn record_arm_guard(
        &mut self,
        guard: &'a Guard,
        body: &'a Expression,
        key: (u32, Vec<(usize, &'a str)>),
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
        outer: &Scope<'a>,
    ) -> Option<((u32, Vec<(usize, &'a str)>), ExitTest, u32, SelectRegion)> {
        let (_, leaves) = super::guards::conditions(&guard.condition)?;
        let [leaf] = leaves[..] else {
            return None;
        };
        let test = self.record_leaf(&leaf, parameter_count, depth, arena, scope)?;
        let then_start = arena.len() as u32;
        let mut inside = scope.clone();
        self.narrow_by_test(&leaf, &test, true, parameter_count, arena, &mut inside);
        let value = self.record_expr(body, parameter_count, depth + 1, arena, &mut inside)?;
        if inside[..outer.len()] != outer[..] {
            return None;
        }
        let else_start = arena.len() as u32;
        Some((key, test, value, SelectRegion { then_start, else_start, exclusive: true }))
    }

    /// End the guarded arms waiting for `fallback`: the last guard chooses between its body and
    /// `fallback`, each earlier one between its body and that choice.
    fn close_guards(
        &mut self,
        pending: &mut Vec<((u32, Vec<(usize, &'a str)>), ExitTest, u32, SelectRegion)>,
        fallback: u32,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        let mut chosen = fallback;
        while let Some((_, test, mut value, region)) = pending.pop() {
            // A number beside a domain value: the domain value is read as its number, uncounted,
            // as in the arms of a case (`record_match`).
            let (value_ty, chosen_ty) = (arena[value as usize].ty, arena[chosen as usize].ty);
            let before = self.conversions_recorded;
            if matches!(value_ty, SourceType::Integer { .. })
                && let Some((lower, upper)) = self.domain_bounds_of(chosen_ty)
            {
                chosen = self.coerce(chosen, SourceType::Integer { lower, upper }, span, arena)?;
            } else if matches!(chosen_ty, SourceType::Integer { .. })
                && let Some((lower, upper)) = self.domain_bounds_of(value_ty)
            {
                value = self.coerce(value, SourceType::Integer { lower, upper }, span, arena)?;
            }
            self.conversions_recorded = before;
            chosen = self.select_node_with(&test, value, chosen, span, Some(region), arena, false)?;
        }
        Some(chosen)
    }

    /// Uncharged lookup: recording must not change what the checker accepts.
    /// The function a name alone calls: no local or parameter of that name, and a function of
    /// that name that takes nothing.
    fn bare_callee(
        &self,
        name: &str,
        parameter_count: usize,
        scope: &Scope<'a>,
    ) -> Option<(usize, Rc<calls::Function>)> {
        if scope.iter().any(|(local, _)| *local == name)
            || self.proof.names.iter().take(parameter_count).any(|(other, _)| *other == name)
        {
            return None;
        }
        let target = self.recorded_callee(name)?;
        target.1.parameters.is_empty().then_some(target)
    }

    fn recorded_callee(&self, name: &str) -> Option<(usize, Rc<calls::Function>)> {
        let symbols = &self.module.function_names;
        let position = symbols
            .binary_search_by(|(other, _)| other.as_str().cmp(name))
            .ok()?;
        let function = symbols[position].1;
        let interface = self.module.functions.get(function)?.clone()?;
        Some((function, interface))
    }

    fn callee_ref(&self, function: usize) -> Option<CheckedSourceCallee> {
        let own = self.module.functions.len() - self.module.imported.len();
        if function < own {
            return Some(CheckedSourceCallee::Local {
                function: function as u32,
            });
        }
        let &(dependency, function) = self.module.imported.get(function - own)?;
        Some(CheckedSourceCallee::Dependency {
            dependency,
            function,
        })
    }

    /// Whether an arm has changed a `mut` parameter of the body (a call that lends it `mut`, or an
    /// assignment): the value graph selects only the arms' values, so the change would be lost.
    fn arm_changed_mut(&self, before: &Scope<'a>, after: &Scope<'a>) -> bool {
        self.recording_mutated.iter().any(|name| {
            let find = |scope: &Scope<'a>| scope.iter().rev().find(|(local, _)| local == name).map(|(_, node)| *node);
            find(before) != find(after)
        })
    }

    fn record_block(
        &mut self,
        expression: &'a Expression,
        block: &'a Block,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        // A `return` in a block inside another expression leaves the function; a value graph has
        // no way to say that, so such a block is not recorded. The block of an arm of the `when`
        // a function returns ends in its value: `return x` there is the arm's value.
        if depth > 0
            && self.recording_mutated.is_empty()
            && let Some((last, rest)) = block.statements.split_last()
            && let Some(value) = (match &last.value {
                Statement::Return(ret) if self.recording_tail_blocks.contains(&expression.span_id) => ret.value.as_ref(),
                // Written without `return`: the last line is the arm's value (decision 1).
                Statement::Expression(value) => Some(value),
                _ => None,
            })
            && !rest.iter().any(|statement| matches!(statement.value, Statement::Return(_)))
        {
            let locals = scope.len();
            let mut statements = Vec::new();
            let first = arena.len();
            self.record_statements(rest, parameter_count, depth, arena, scope, &mut statements)?;
            // The effects written before the value stay reachable from the block, so a lowering
            // that cannot run one only where its arm is taken can see it and decline.
            statements.extend((first..arena.len()).filter(|&at| {
                matches!(
                    arena[at].kind,
                    CheckedSourceBodyKind::Call { .. }
                        | CheckedSourceBodyKind::Instruction { .. }
                        | CheckedSourceBodyKind::Store { .. }
                )
            }).map(|at| at as u32));
            let result = self.record_expr(value, parameter_count, depth + 1, arena, scope)?;
            scope.truncate(locals);
            // Destructures only bind: the arm is then just its value.
            if statements.is_empty() {
                return Some(result);
            }
            statements.push(result);
            let ty = arena[result as usize].ty;
            return Some(push(
                arena,
                expression.span_id,
                ty,
                CheckedSourceBodyKind::Block { statements, result },
            ));
        }
        if depth > 0
            && block
                .statements
                .iter()
                .any(|statement| matches!(statement.value, Statement::Return(_)))
        {
            // Unless all the function does after it is a bare `return`: then a block that ends in
            // one ends with nothing, as the function would (`Failed(_) then return`).
            let (last, rest) = block.statements.split_last()?;
            let bare = matches!(&last.value, Statement::Return(ret) if ret.value.is_none());
            if !self.recording_unit_tail
                || !bare
                || rest.iter().any(|statement| matches!(statement.value, Statement::Return(_)))
            {
                return None;
            }
            let locals = scope.len();
            let mut statements = Vec::new();
            self.record_statements(rest, parameter_count, depth, arena, scope, &mut statements)?;
            scope.truncate(locals);
            let result = push(arena, last.span_id, SourceType::Unit, CheckedSourceBodyKind::Unit);
            statements.push(result);
            return Some(push(
                arena,
                expression.span_id,
                SourceType::Unit,
                CheckedSourceBodyKind::Block { statements, result },
            ));
        }
        let locals = scope.len();
        let mut statements = Vec::new();
        self.record_statements(
            &block.statements,
            parameter_count,
            depth,
            arena,
            scope,
            &mut statements,
        )?;
        scope.truncate(locals);
        let result = *statements.last()?;
        let ty = arena[result as usize].ty;
        Some(push(
            arena,
            expression.span_id,
            ty,
            CheckedSourceBodyKind::Block { statements, result },
        ))
    }

    /// The statements of a block in order, leaving its locals in `scope` for the caller to
    /// drop. `statements` collects the returns.
    fn record_statements(
        &mut self,
        block: &'a [Spanned<Statement>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
        statements: &mut Vec<u32>,
    ) -> Option<()> {
        for (index, statement) in block.iter().enumerate() {
            self.recording_at = Some(statement.span_id);
            match &statement.value {
                Statement::Return(_) | Statement::Expression(_)
                    if matches!(statement.value, Statement::Return(_))
                        || (depth == 0 && index + 1 == block.len()) =>
                {
                    // The last expression of a function's body is its result, as a `return`.
                    let written = match &statement.value {
                        Statement::Return(ret) => ret.value.as_ref(),
                        Statement::Expression(value) => Some(value),
                        _ => return None,
                    };
                    let value = match written {
                        // `return #todo`: the hole stands for the function's result.
                        Some(value) if matches!(value.value, Expr::Hole) => push(
                            arena,
                            value.span_id,
                            self.recording_result?,
                            CheckedSourceBodyKind::Hole,
                        ),
                        Some(value) => {
                            if depth == 0
                                && let Expr::Match(case) = &value.value
                            {
                                for arm in &case.arms.value {
                                    if matches!(arm.value.body.value, Expr::Block(_)) {
                                        self.recording_tail_blocks.push(arm.value.body.span_id);
                                    }
                                }
                            }
                            let node =
                                self.record_expr(value, parameter_count, depth + 1, arena, scope)?;
                            match self.recording_result {
                                Some(result) => {
                                    let node = self.coerce(node, result, value.span_id, arena)?;
                                    // A number the checker proved to fit the result, computed in a
                                    // wider range (a chain whose arms reach past 64 bits): it takes
                                    // the result's range, which lowering cuts back to a register.
                                    match (arena[node as usize].ty, result) {
                                        (SourceType::Integer { lower, upper }, SourceType::Integer { lower: low, upper: high })
                                            if lower < low || upper > high =>
                                        {
                                            push(arena, value.span_id, result, CheckedSourceBodyKind::Widen { operand: node })
                                        }
                                        _ => node,
                                    }
                                }
                                None => node,
                            }
                        }
                        None => push(
                            arena,
                            statement.span_id,
                            SourceType::Unit,
                            CheckedSourceBodyKind::Unit,
                        ),
                    };
                    let mut bundled = None;
                    if depth == 0 {
                        let outputs = self
                            .recording_mutated
                            .iter()
                            .map(|name| {
                                scope
                                    .iter()
                                    .rev()
                                    .find(|(local, _)| local == name)
                                    .map(|(_, node)| *node)
                            })
                            .collect::<Option<Vec<_>>>()?;
                        if self.recording_bundle {
                            // Each exit hands back its own outputs, beside its result.
                            bundled = Some(outputs.clone());
                            self.recording_outputs.get_or_insert_with(Vec::new);
                        } else {
                            match &self.recording_outputs {
                                None => self.recording_outputs = Some(outputs),
                                // Another path out of the function hands back other values: the
                                // body is recorded again with an output bundle per exit.
                                Some(first) if *first != outputs => {
                                    self.recording_outputs_differ = true;
                                    return None;
                                }
                                Some(_) => {}
                            }
                        }
                    }
                    // After a loop that can `return`, what the function returns is that value if
                    // it ran, and otherwise this one.
                    let value = if let Some(outputs) = bundled {
                        // After a loop that can `return`, the locals already hold the values of
                        // whichever exit ran; only the result is chosen.
                        let value = self.wrap_exit(value, statement.span_id, arena, scope)?;
                        self.exit_bundle(value, &outputs, statement.span_id, arena)?
                    } else if depth == 0 {
                        self.wrap_exit(value, statement.span_id, arena, scope)?
                    } else {
                        value
                    };
                    let ty = arena[value as usize].ty;
                    statements.push(push(
                        arena,
                        statement.span_id,
                        ty,
                        CheckedSourceBodyKind::Return { value },
                    ));
                }
                Statement::Destructure(pattern) => {
                    self.record_destructure(pattern, parameter_count, depth, arena, scope)?;
                }
                Statement::Binding(local) => {
                    // A plain `name: value`; anything written beyond that is not recorded.
                    let plain = local.parameters.is_none()
                        && local.convention.is_none()
                        && local.result_binder.is_none()
                        && local.attributes.is_empty();
                    let value = local.value.as_ref().filter(|_| plain)?;
                    let mut node = self.record_expr(
                        &value.expression,
                        parameter_count,
                        depth + 1,
                        arena,
                        scope,
                    )?;
                    if let Some(annotation) = &local.annotation {
                        if is_integer_range(annotation) {
                            // The checker proved the value fits; a plain integer needs no
                            // conversion, and a domain value cannot be read as one here.
                            if !matches!(arena[node as usize].ty, SourceType::Integer { .. }) {
                                return None;
                            }
                        } else {
                            let declared = self.recorded_annotation(annotation)?;
                            // `in D` over a domain value reads its number; the checker counts no
                            // conversion for that, so the recording counts none.
                            let is_self = matches!(&annotation.value, Annotation::Domain { domain, .. }
                                if matches!(&domain.base.value, DomainBase::Expression(e)
                                    if matches!(&e.value, Expr::Name(name) if name == "Self")));
                            let reads_number = !is_self
                                && matches!(declared, SourceType::Integer { .. })
                                && matches!(arena[node as usize].ty, SourceType::Aggregate { .. });
                            let before = self.conversions_recorded;
                            node = self.coerce(node, declared, value.expression.span_id, arena)?;
                            if reads_number {
                                self.conversions_recorded = before;
                            }
                        }
                    }
                    // A local that can change holds its declared range, not just what it was
                    // first given: a loop must assume the wider.
                    if matches!(value.operator.value, BindingOperator::Colon)
                        && let SourceType::Integer { .. } = arena[node as usize].ty
                    {
                        let declared = match &local.annotation {
                            Some(annotation) if is_integer_range(annotation) => self
                                .declared_range(annotation, parameter_count, depth, arena, scope)?,
                            // `in SignedBigInt`: the domain's range, as for one written out.
                            Some(annotation) => match self.recorded_annotation(annotation) {
                                Some(range @ SourceType::Integer { .. }) => range,
                                _ => arena[node as usize].ty,
                            },
                            None => arena[node as usize].ty,
                        };
                        if declared != arena[node as usize].ty {
                            node = push(
                                arena,
                                local.name.span_id,
                                declared,
                                CheckedSourceBodyKind::Widen { operand: node },
                            );
                        }
                    }
                    scope.push((local.name.value.as_str(), node));
                }
                // Its value is unused, but its node stays in the arena and so still runs.
                Statement::Expression(value) => {
                    // Followed only by a bare `return` of a unit function: an arm may end in one.
                    let tail = depth == 0
                        && matches!(self.recording_result, None | Some(SourceType::Unit))
                        && matches!(&block[index + 1..], [only] if matches!(&only.value, Statement::Return(ret) if ret.value.is_none()));
                    let saved = std::mem::replace(&mut self.recording_unit_tail, tail);
                    let recorded = self.record_expr(value, parameter_count, depth + 1, arena, scope);
                    self.recording_unit_tail = saved;
                    recorded?;
                }
                // Taken by the arm above.
                Statement::Return(_) => return None,
                Statement::Assign(assign) => {
                    self.record_assign(assign, parameter_count, depth, arena, scope)?;
                }
                // `when x is Member(binders) then <calls>` that falls through: a choice of nothing
                // on the member, the calls run only when it is the subject's.
                Statement::WhenIs(when)
                    if !super::guards::returns(&when.then_block.value) && Self::case_runs_calls(when) =>
                {
                    self.record_effect_case(when, parameter_count, depth, arena, scope)?;
                }
                // Branches that call something and fall through, changing no local: a choice of
                // nothing, each arm run only when taken.
                Statement::When(when)
                    if !super::guards::returns(&when.then_block.value) && Self::runs_calls(when) =>
                {
                    self.record_effect_when(when, parameter_count, depth, arena, scope)?;
                }
                // Both branches fall through: each local they assign is a `Select` of the two.
                Statement::When(when) if !super::guards::returns(&when.then_block.value) => {
                    self.record_join(when, parameter_count, depth, arena, scope)?;
                }
                // Inside a loop that can `return`, a guard that returns sets the exit values and
                // the statements after it go on.
                Statement::When(when) if depth > 0 && Self::exit_state(scope).is_some() => {
                    self.record_loop_exit(
                        Test::Condition(&when.condition),
                        &when.then_block.value.statements,
                        &block[index + 1..],
                        statement.span_id,
                        parameter_count,
                        depth,
                        arena,
                        scope,
                    )?;
                    return Some(());
                }
                Statement::WhenIs(when) if depth > 0 && Self::exit_state(scope).is_some() => {
                    self.record_loop_exit(
                        Test::Pattern(when),
                        &when.then_block.value.statements,
                        &block[index + 1..],
                        statement.span_id,
                        parameter_count,
                        depth,
                        arena,
                        scope,
                    )?;
                    return Some(());
                }
                // A guard that returns makes the rest of the body its other arm.
                Statement::When(when) => {
                    let rest = &block[index + 1..];
                    let select = self.record_guard(
                        when,
                        statement.span_id,
                        rest,
                        parameter_count,
                        depth,
                        arena,
                        scope,
                    )?;
                    let ty = arena[select as usize].ty;
                    statements.push(push(
                        arena,
                        statement.span_id,
                        ty,
                        CheckedSourceBodyKind::Return { value: select },
                    ));
                    return Some(());
                }
                // A one-armed case that returns makes the rest of the body its other arms.
                Statement::WhenIs(when) => {
                    let rest = &block[index + 1..];
                    let chosen = self.record_case_guard(
                        when,
                        statement.span_id,
                        rest,
                        parameter_count,
                        depth,
                        arena,
                        scope,
                    )?;
                    let ty = arena[chosen as usize].ty;
                    statements.push(push(
                        arena,
                        statement.span_id,
                        ty,
                        CheckedSourceBodyKind::Return { value: chosen },
                    ));
                    return Some(());
                }
                Statement::Error => return None,
            }
        }
        Some(())
    }

    /// The positions in `scope` of the two values a loop that can `return` carries: whether it
    /// has returned, and what it returned.
    fn exit_state(scope: &Scope<'a>) -> Option<(usize, usize)> {
        let done = scope.iter().rposition(|(name, _)| *name == "%done")?;
        let ret = scope.iter().rposition(|(name, _)| *name == "%ret")?;
        Some((done, ret))
    }

    /// `when subject is Member then a else b`: a `Match` whose arm for `Member` is `a` (with the
    /// pattern's binders in scope) and whose every other arm is `b`; swapped under `not`. A
    /// subject whose member is known picks its branch and nothing else is recorded.
    #[allow(clippy::too_many_arguments)]
    fn record_variant_conditional(
        &mut self,
        case: &'a ConditionalExpression,
        negated: bool,
        test: &'a IsTest,
        span: SpanId,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let MatchPattern::Variant(pattern) = &test.pattern.value else {
            return None;
        };
        let subject = self.record_expr(&test.subject, parameter_count, depth + 1, arena, scope)?;
        let subject_ty = arena[subject as usize].ty;
        let schema = self.recorded_pattern(subject_ty, &pattern.constructor.value)?;
        let mut members = Vec::new();
        for variant in self.recorded_variants(subject_ty) {
            let SourceType::Aggregate { id } = variant else {
                return None;
            };
            members.push(self.module.instances[id as usize].schema());
        }
        if members.is_empty() {
            members.push(schema);
        }
        let known = match &arena[subject as usize].kind {
            CheckedSourceBodyKind::Construct { schema, .. } => Some(*schema as usize),
            _ => None,
        };
        // The branch taken where the member is the pattern's, and where it is not.
        let (on_member, off_member) = if negated {
            (&case.else_branch, &case.then_branch)
        } else {
            (&case.then_branch, &case.else_branch)
        };
        let locals = scope.len();
        let before = scope.clone();
        let member_start = arena.len() as u32;
        if !negated {
            self.bind_fields(subject, schema, pattern.fields.as_ref(), arena, scope)?;
        }
        let member_value = self.record_expr(on_member, parameter_count, depth + 1, arena, scope)?;
        scope.truncate(locals);
        // An arm that changes a local of the enclosing body (a `mut` receiver or argument it
        // lends) would need that local selected between the arms; it is declined, not dropped.
        if known == Some(schema) {
            return Some(member_value);
        }
        if self.arm_changed_mut(&before, scope) {
            return None;
        }
        let other_start = arena.len() as u32;
        let other_value = self.record_expr(off_member, parameter_count, depth + 1, arena, scope)?;
        if known.is_some() {
            return Some(other_value);
        }
        if self.arm_changed_mut(&before, scope) {
            return None;
        }
        let arms: Vec<CheckedSourceMatchArm> = members
            .into_iter()
            .map(|member| CheckedSourceMatchArm {
                schema: member as u32,
                body: if member == schema {
                    member_value
                } else {
                    other_value
                },
            })
            .collect();
        let bodies = || [member_value, other_value].into_iter();
        let ty = join(arena, bodies()).or_else(|| self.sum_of(arena, bodies()))?;
        Some(push(
            arena,
            span,
            ty,
            CheckedSourceBodyKind::Match {
                subject,
                arms,
                region: Some(MatchRegion {
                    member_body: member_value,
                    member_start,
                    other_start,
                }),
            },
        ))
    }

    /// The recorded comparison for a relation, and whether it is the negation of it: a recorded
    /// body has `=` and the orderings, and a not-equal is the negation of an equal.
    fn infix_of(relation: super::guards::Relation) -> (InfixOperator, bool) {
        use super::guards::Relation;
        match relation {
            Relation::Equal => (InfixOperator::Equal, false),
            Relation::NotEqual => (InfixOperator::Equal, true),
            Relation::Less => (InfixOperator::Less, false),
            Relation::LessOrEqual => (InfixOperator::LessOrEqual, false),
            Relation::Greater => (InfixOperator::Greater, false),
            Relation::GreaterOrEqual => (InfixOperator::GreaterOrEqual, false),
            // A membership is two comparisons, built by `select_node`, never one.
            Relation::Within | Relation::Outside => (InfixOperator::Equal, false),
        }
    }

    /// The two bounds of the domain after `is in`: a written range with constant ends, or a type
    /// that declares one.
    fn recorded_domain_bounds(
        &mut self,
        domain: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<(I256, I256)> {
        match &domain.value {
            Expr::Range { start, end, .. } => {
                let start = self.record_expr(start, parameter_count, depth + 1, arena, scope)?;
                let end = self.record_expr(end, parameter_count, depth + 1, arena, scope)?;
                Some((known_integer(arena, start)?, known_integer(arena, end)?))
            }
            Expr::Name(name) => {
                let schema = &self.module.layouts[self.recorded_record(name)?].schema;
                let Some(SourceType::Integer { lower, upper }) =
                    schema.fields.first().map(|field| field.ty)
                else {
                    return None;
                };
                (schema.kind == AggregateKind::Domain).then_some((lower, upper))
            }
            _ => None,
        }
    }

    /// Both sides of a relation, recorded; both must be numbers.
    fn record_leaf(
        &mut self,
        leaf: &super::guards::Leaf<'a>,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<ExitTest> {
        let left = self.record_expr(leaf.left, parameter_count, depth + 1, arena, scope)?;
        let left = self.read_number(left, leaf.left.span_id, arena)?;
        if !matches!(arena[left as usize].ty, SourceType::Integer { .. }) {
            return None;
        }
        let (right, upper) = match leaf.right {
            super::guards::Side::Expr(expression) => {
                let right =
                    self.record_expr(expression, parameter_count, depth + 1, arena, scope)?;
                let right = self.read_number(right, expression.span_id, arena)?;
                if !matches!(arena[right as usize].ty, SourceType::Integer { .. }) {
                    return None;
                }
                (right, None)
            }
            super::guards::Side::Domain(domain) => {
                let (low, high) =
                    self.recorded_domain_bounds(domain, parameter_count, depth, arena, scope)?;
                (
                    push_integer(arena, domain.span_id, low),
                    Some(push_integer(arena, domain.span_id, high)),
                )
            }
        };
        Some(ExitTest::Compare {
            relation: leaf.relation,
            left,
            right,
            upper,
        })
    }

    /// A domain value read as its number for a test (the checker compares the number and counts
    /// no conversion); a number is itself.
    fn read_number(&mut self, node: u32, span: SpanId, arena: &mut Vec<CheckedSourceBodyExpr>) -> Option<u32> {
        let ty = arena[node as usize].ty;
        if !matches!(ty, SourceType::Aggregate { .. }) {
            return Some(node);
        }
        let (lower, upper) = self.domain_bounds_of(ty)?;
        let before = self.conversions_recorded;
        let number = self.coerce(node, SourceType::Integer { lower, upper }, span, arena)?;
        self.conversions_recorded = before;
        Some(number)
    }

    /// `then` if `test` holds and `other` if it does not, as a node.
    fn select_node(
        &mut self,
        test: &ExitTest,
        then: u32,
        other: u32,
        span: SpanId,
        region: Option<SelectRegion>,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        self.select_node_with(test, then, other, span, region, arena, true)
    }

    /// `select_node`, optionally without viewing a number beside a domain value as that domain's:
    /// the arms of a case are joined as the case's arms are, the checker converting none of them.
    #[allow(clippy::too_many_arguments)] // one choice, threaded explicitly
    fn select_node_with(
        &mut self,
        test: &ExitTest,
        then: u32,
        other: u32,
        span: SpanId,
        region: Option<SelectRegion>,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        coerce: bool,
    ) -> Option<u32> {
        // A number beside a domain value is that domain's: the checker viewed it as such.
        let (mut then, mut other) = (then, other);
        let (then_ty, other_ty) = (arena[then as usize].ty, arena[other as usize].ty);
        if coerce && then_ty != other_ty {
            if matches!(then_ty, SourceType::Aggregate { .. }) && matches!(other_ty, SourceType::Integer { .. }) && self.domain_bounds_of(then_ty).is_some() {
                other = self.coerce(other, then_ty, span, arena)?;
            } else if matches!(other_ty, SourceType::Aggregate { .. }) && matches!(then_ty, SourceType::Integer { .. }) && self.domain_bounds_of(other_ty).is_some() {
                then = self.coerce(then, other_ty, span, arena)?;
            }
        }
        let branches = || [then, other].into_iter();
        let ty = join(arena, branches())
            .or_else(|| self.sum_of(arena, branches()))
            .or_else(|| self.sum_of_generic(arena, branches()))
            .or_else(|| self.recording_result.filter(|ty| matches!(ty, SourceType::Sum { .. })))?;
        Some(match test {
            ExitTest::Compare {
                relation,
                left,
                right,
                upper: Some(upper),
            } => {
                // `x is in D` is `x >= low ? (x <= high ? then : other) : other`; outside is the
                // same with the branches swapped.
                let (on_hold, on_fail) = if *relation == super::guards::Relation::Within {
                    (then, other)
                } else {
                    (other, then)
                };
                // An upper bound the operand's own interval already meets is not compared: a domain
                // wider than the machine word (`0 to 2^64 - 1` over a signed word) has a bound
                // no register comparison can hold, and its test is decided by the type.
                let operand = match arena[*left as usize].ty {
                    SourceType::Integer { lower, upper } => Some((lower, upper)),
                    _ => None,
                };
                let bound = |node: u32| match arena[node as usize].kind {
                    CheckedSourceBodyKind::Integer(value) => Some(value),
                    _ => None,
                };
                let upper_met = matches!((operand, bound(*upper)), (Some((_, high)), Some(limit)) if high <= limit);
                let inner = if upper_met {
                    on_hold
                } else {
                    push(
                        arena,
                        span,
                        ty,
                        CheckedSourceBodyKind::Select {
                            comparison: InfixOperator::LessOrEqual,
                            left: *left,
                            right: *upper,
                            then_branch: on_hold,
                            else_branch: on_fail,
                            // The two comparisons of a membership share their arms.
                            region: region.map(|region| SelectRegion {
                                exclusive: false,
                                ..region
                            }),
                        },
                    )
                };
                push(
                    arena,
                    span,
                    ty,
                    CheckedSourceBodyKind::Select {
                        comparison: InfixOperator::GreaterOrEqual,
                        left: *left,
                        right: *right,
                        then_branch: inner,
                        else_branch: on_fail,
                        region: region.map(|region| SelectRegion {
                            exclusive: false,
                            ..region
                        }),
                    },
                )
            }
            ExitTest::Compare {
                relation,
                left,
                right,
                upper: None,
            } => {
                // A recorded comparison has no not-equal: `a /= b ? x : y` is `a = b ? y : x`.
                let (comparison, negated) = Self::infix_of(*relation);
                let (on_hold, on_fail) = if negated { (other, then) } else { (then, other) };
                push(
                    arena,
                    span,
                    ty,
                    CheckedSourceBodyKind::Select {
                        comparison,
                        left: *left,
                        right: *right,
                        then_branch: on_hold,
                        else_branch: on_fail,
                        region,
                    },
                )
            }
            ExitTest::Tag {
                subject,
                members,
                schema,
            } => {
                let arms = members
                    .iter()
                    .map(|member| CheckedSourceMatchArm {
                        schema: *member as u32,
                        body: if member == schema { then } else { other },
                    })
                    .collect();
                push(
                    arena,
                    span,
                    ty,
                    CheckedSourceBodyKind::Match {
                        subject: *subject,
                        arms,
                        region: region.map(|region| MatchRegion {
                            member_body: then,
                            member_start: region.then_start,
                            other_start: region.else_start,
                        }),
                    },
                )
            }
        })
    }

    /// `parts` side by side, typed as the sequence of their types.
    fn push_bundle(&mut self, parts: Vec<u32>, span: SpanId, arena: &mut Vec<CheckedSourceBodyExpr>) -> Option<u32> {
        let types = parts.iter().map(|part| arena[*part as usize].ty).collect();
        let ty = self.sequence_type(types).ok()?;
        Some(push(arena, span, ty, CheckedSourceBodyKind::Bundle { parts }))
    }

    /// An arm's value as the lead of a bundle typed `declared`: a member of a sum stays
    /// itself (the bundle is typed by the sum, and lowering puts the member in the sum's
    /// shape, as for an exit's bundle); anything else is widened.
    fn lead_to(&mut self, node: u32, declared: SourceType, span: SpanId, arena: &mut Vec<CheckedSourceBodyExpr>) -> Option<u32> {
        let ty = arena[node as usize].ty;
        if let SourceType::Sum { id } = declared
            && ty != declared
            && self.sum_variants(id).ok()?.contains(&ty)
        {
            return Some(node);
        }
        self.widened_to(node, declared, span, arena)
    }

    /// Type a bundle's first part as `lead`, a sum one of whose members the part is.
    fn bundle_lead_as(&mut self, bundle: u32, lead: SourceType, arena: &mut [CheckedSourceBodyExpr]) -> Option<()> {
        let CheckedSourceBodyKind::Bundle { parts } = &arena[bundle as usize].kind else {
            return Some(());
        };
        let mut types: Vec<SourceType> = parts.iter().map(|part| arena[*part as usize].ty).collect();
        if types.first() == Some(&lead) {
            return Some(());
        }
        *types.first_mut()? = lead;
        arena[bundle as usize].ty = self.sequence_type(types).ok()?;
        Some(())
    }

    /// `node` as a value of `declared`: itself, or a number widened to the declared range.
    /// Anything else would be a conversion, which a bundle does not make.
    fn widened_to(&mut self, node: u32, declared: SourceType, span: SpanId, arena: &mut Vec<CheckedSourceBodyExpr>) -> Option<u32> {
        let ty = arena[node as usize].ty;
        match (ty, declared) {
            _ if ty == declared => Some(node),
            (SourceType::Integer { lower, upper }, SourceType::Integer { lower: low, upper: high })
                if low <= lower && upper <= high =>
            {
                Some(push(arena, span, declared, CheckedSourceBodyKind::Widen { operand: node }))
            }
            _ => None,
        }
    }

    /// An exit in bundle mode: its result and the value each `mut` parameter holds there.
    fn exit_bundle(
        &mut self,
        value: u32,
        outputs: &[u32],
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        let mut parts = Vec::with_capacity(outputs.len() + 1);
        parts.push(value);
        for (index, output) in outputs.iter().enumerate() {
            let declared = *self.recording_output_types.get(index)?;
            parts.push(self.widened_to(*output, declared, span, arena)?);
        }
        // An exit that returns one variant of a sum result holds the sum, so every exit's
        // bundle has one type (lowering puts the member in the sum's shape).
        let mut types: Vec<SourceType> = parts.iter().map(|part| arena[*part as usize].ty).collect();
        if let Some(result @ SourceType::Sum { id }) = self.recording_result
            && types[0] != result
            && self.sum_variants(id).ok()?.contains(&types[0])
        {
            types[0] = result;
        }
        let ty = self.sequence_type(types).ok()?;
        Some(push(arena, span, ty, CheckedSourceBodyKind::Bundle { parts }))
    }

    /// The root of a body recorded in bundle mode, `return b`, as `return b.0` with outputs
    /// `b.1..`: the shape every consumer reads.
    fn unbundle_root(&mut self, root: u32, arena: &mut Vec<CheckedSourceBodyExpr>) -> Option<(u32, Vec<u32>)> {
        let CheckedSourceBodyKind::Block { statements, result } = &arena[root as usize].kind else {
            return None;
        };
        if statements.len() != 1 || *result != statements[0] {
            return None;
        }
        let returned = statements[0];
        let CheckedSourceBodyKind::Return { value: bundle } = arena[returned as usize].kind else {
            return None;
        };
        let SourceType::Sequence { id } = arena[bundle as usize].ty else {
            return None;
        };
        let types = self.module.instances.get(id as usize)?.arguments.clone();
        if types.len() != self.recording_mutated.len() + 1 {
            return None;
        }
        let span = arena[root as usize].span;
        let parts: Vec<u32> = types
            .iter()
            .enumerate()
            .map(|(index, ty)| push(arena, span, *ty, CheckedSourceBodyKind::Part { bundle, index: index as u32 }))
            .collect();
        let value = parts[0];
        let ret = push(arena, span, types[0], CheckedSourceBodyKind::Return { value });
        let block = push(arena, span, types[0], CheckedSourceBodyKind::Block { statements: vec![ret], result: ret });
        Some((block, parts[1..].to_vec()))
    }

    /// The two locals a loop that can `return` carries: `%done` (0 until a pass returns) and
    /// `%ret` (the value returned, never read before). A loop inside another that can return
    /// is refused: the rest of the outer pass would need the same guard.
    fn begin_exit_state(&mut self, span: SpanId, arena: &mut Vec<CheckedSourceBodyExpr>, scope: &mut Scope<'a>) -> Option<()> {
        if Self::exit_state(scope).is_some() {
            return None;
        }
        let result = self.recording_result?;
        let flag = SourceType::Integer { lower: I256::ZERO, upper: I256::ONE };
        let done = push(arena, span, flag, CheckedSourceBodyKind::Integer(I256::ZERO));
        let ret = match result {
            SourceType::Integer { lower, .. } => push(arena, span, result, CheckedSourceBodyKind::Integer(lower)),
            SourceType::Unit => push(arena, span, result, CheckedSourceBodyKind::Unit),
            SourceType::Aggregate { .. } | SourceType::Sum { .. } => push(arena, span, result, CheckedSourceBodyKind::Undefined),
            _ => return None,
        };
        scope.push(("%done", done));
        scope.push(("%ret", ret));
        Some(())
    }

    /// After a loop that can `return`: `done == 1 ? ret : value`.
    fn wrap_exit(
        &mut self,
        value: u32,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &Scope<'a>,
    ) -> Option<u32> {
        let Some((done, ret)) = Self::exit_state(scope) else {
            return Some(value);
        };
        let (done, ret) = (scope[done].1, scope[ret].1);
        let one = push_integer(arena, span, I256::ONE);
        let test = ExitTest::Compare {
            relation: super::guards::Relation::Equal,
            left: done,
            right: one,
            upper: None,
        };
        // Both values are already computed, each where it ran: the choice has empty arms, so
        // an effect that led to either is not run again or refused.
        let at = arena.len() as u32;
        let region = SelectRegion { then_start: at, else_start: at, exclusive: true };
        self.select_node(&test, ret, value, span, Some(region), arena)
    }

    /// A guard that returns, inside a loop body, followed by `rest` of the pass: a choice whose
    /// `then` arm runs the block and sets `%done` and `%ret`, and whose other arm runs `rest`.
    /// Each local either changes is its part of the chosen bundle afterwards, so what `rest`
    /// does runs only on a pass that did not return.
    #[allow(clippy::too_many_arguments)]
    fn record_loop_exit(
        &mut self,
        test: Test<'a>,
        block: &'a [Spanned<Statement>],
        rest: &'a [Spanned<Statement>],
        span: SpanId,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let exit = match test {
            Test::Condition(condition) => {
                let (_, leaves) = super::guards::conditions(condition)?;
                let [leaf] = leaves[..] else {
                    return None;
                };
                (
                    self.record_leaf(&leaf, parameter_count, depth, arena, scope)?,
                    None,
                )
            }
            Test::Pattern(when) => {
                if when.else_branch.is_some() {
                    return None;
                }
                let MatchPattern::Variant(pattern) = &when.pattern.value else {
                    return None;
                };
                let subject =
                    self.record_expr(&when.subject, parameter_count, depth + 1, arena, scope)?;
                let subject_ty = arena[subject as usize].ty;
                let schema = self.recorded_pattern(subject_ty, &pattern.constructor.value)?;
                let mut members = Vec::new();
                for variant in self.recorded_variants(subject_ty) {
                    let SourceType::Aggregate { id } = variant else {
                        return None;
                    };
                    members.push(self.module.instances[id as usize].schema());
                }
                if members.is_empty() {
                    members.push(schema);
                }
                (
                    ExitTest::Tag {
                        subject,
                        members,
                        schema,
                    },
                    Some((subject, schema, pattern.fields.as_ref())),
                )
            }
        };
        let (exit, bind) = exit;
        let (done_at, ret_at) = Self::exit_state(scope)?;
        let kept = scope.len();
        let start = arena.len();
        let saved = (self.conversions_recorded, self.recording_folds.len());
        // Which locals the arms change is known once they are recorded; the `then` bundle must
        // come before the `else` arm, so with any changed they are recorded again knowing it.
        let mut changed: Vec<usize> = Vec::new();
        for pass in 0..2 {
            let then_start = arena.len() as u32;
            let mut inside = scope.clone();
            if let Some((subject, schema, fields)) = bind {
                self.bind_fields(subject, schema, fields, arena, &mut inside)?;
            }
            let value = self.record_branch(block, parameter_count, depth, arena, &mut inside)?;
            inside.truncate(kept);
            inside[done_at].1 = push_integer(arena, span, I256::ONE);
            if arena[value as usize].ty != SourceType::Unit {
                inside[ret_at].1 = value;
            }
            let mut found: Vec<usize> = (0..kept).filter(|&i| inside[i] != scope[i]).collect();
            let then_value = self.arm_bundle(None, &changed, &inside, scope, scope, span, arena)?;
            let else_start = arena.len() as u32;
            let mut inside = scope.clone();
            let mut returns = Vec::new();
            self.record_statements(rest, parameter_count, depth, arena, &mut inside, &mut returns)?;
            if !returns.is_empty() {
                return None;
            }
            inside.truncate(kept);
            found.extend((0..kept).filter(|&i| inside[i] != scope[i]));
            found.sort_unstable();
            found.dedup();
            let else_value = self.arm_bundle(None, &changed, &inside, scope, scope, span, arena)?;
            if found != changed {
                if pass == 1 {
                    return None;
                }
                arena.truncate(start);
                self.conversions_recorded = saved.0;
                self.recording_folds.truncate(saved.1);
                changed = found;
                continue;
            }
            let region = SelectRegion { then_start, else_start, exclusive: true };
            let chosen = self.select_node(&exit, then_value, else_value, span, Some(region), arena)?;
            self.unpack_changed(&changed, 0, chosen, scope, span, arena)?;
            return Some(());
        }
        None
    }

    /// Whether any statement of this loop body can `return`.
    fn body_returns(body: &Expression) -> bool {
        let mut pending = vec![body];
        let mut blocks: Vec<&Block> = Vec::new();
        while let Some(expression) = pending.pop() {
            match &expression.value {
                Expr::Block(block) => blocks.push(block),
                Expr::Loop(looped) => match looped.as_ref() {
                    LoopExpression::For { body, .. } | LoopExpression::While { body, .. } => {
                        pending.push(body)
                    }
                },
                _ => {}
            }
        }
        while let Some(block) = blocks.pop() {
            for statement in &block.statements {
                match &statement.value {
                    Statement::Return(_) => return true,
                    Statement::When(when) => {
                        blocks.push(&when.then_block.value);
                        blocks.extend(when.else_branch.iter().map(|branch| &branch.block.value));
                    }
                    Statement::WhenIs(when) => {
                        blocks.push(&when.then_block.value);
                        blocks.extend(when.else_branch.iter().map(|branch| &branch.block.value));
                    }
                    Statement::Expression(expression) => pending.push(expression),
                    _ => {}
                }
            }
            while let Some(expression) = pending.pop() {
                match &expression.value {
                    Expr::Block(block) => blocks.push(block),
                    Expr::Loop(looped) => match looped.as_ref() {
                        LoopExpression::For { body, .. } | LoopExpression::While { body, .. } => {
                            pending.push(body)
                        }
                    },
                    _ => {}
                }
            }
        }
        false
    }

    /// `when c then ... return a` followed by `rest`, which ends in a return of its own: the
    /// body is `a` when the comparison holds and what `rest` returns when it does not, so the
    /// recorded value is a `Select`, folded when the comparison is decided. Both arms run (the
    /// recorder has no lazy arms yet), which is only sound because the arms are pure; a lowering
    /// that cannot prove that refuses the body.
    #[allow(clippy::too_many_arguments)]
    /// The tests of a guard as one comparison of a count: each test is 1 where it holds and 0 where
    /// it does not (a choice between two numbers), and the guard holds when every one is 1 (`and`)
    /// or one is (`or`). A guard of one plain comparison is left as it is. With a single test an
    /// arm can be put in a branch of its own, which a chain of tests sharing arms cannot be.
    fn combine_tests(
        &mut self,
        tests: Vec<ExitTest>,
        connective: super::guards::Connective,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<Vec<ExitTest>> {
        use super::guards::Relation;
        let plain = |test: &ExitTest| {
            matches!(test, ExitTest::Compare { relation, upper: None, .. }
                if !matches!(relation, Relation::Within | Relation::Outside))
        };
        if tests.len() == 1 && plain(&tests[0]) {
            return Some(tests);
        }
        let one = push_integer(arena, span, I256::ONE);
        let zero = push_integer(arena, span, I256::ZERO);
        let mut flags = Vec::new();
        for test in &tests {
            let ExitTest::Compare { relation, left, right, upper } = test else {
                return Some(tests);
            };
            let flag = match (relation, upper) {
                (Relation::Within | Relation::Outside, Some(upper)) => {
                    let below = ExitTest::Compare { relation: Relation::GreaterOrEqual, left: *left, right: *right, upper: None };
                    let above = ExitTest::Compare { relation: Relation::LessOrEqual, left: *left, right: *upper, upper: None };
                    let a = self.select_node(&below, one, zero, span, None, arena)?;
                    let b = self.select_node(&above, one, zero, span, None, arena)?;
                    let start = arena.len();
                    let sum = record_arithmetic(CheckedArithmetic::Add, span, start, (a, b), arena)?;
                    let two = push_integer(arena, span, I256::from(2));
                    let both = ExitTest::Compare {
                        relation: if *relation == Relation::Within { Relation::Equal } else { Relation::Less },
                        left: sum,
                        right: two,
                        upper: None,
                    };
                    self.select_node(&both, one, zero, span, None, arena)?
                }
                (_, None) => self.select_node(test, one, zero, span, None, arena)?,
                _ => return None,
            };
            flags.push(flag);
        }
        let mut sum = flags[0];
        for &flag in &flags[1..] {
            let start = arena.len();
            sum = record_arithmetic(CheckedArithmetic::Add, span, start, (sum, flag), arena)?;
        }
        let (relation, bound) = match connective {
            super::guards::Connective::And => (Relation::Equal, flags.len() as i64),
            super::guards::Connective::Or => (Relation::GreaterOrEqual, 1),
        };
        let right = push_integer(arena, span, I256::from(bound));
        Some(vec![ExitTest::Compare { relation, left: sum, right, upper: None }])
    }

    /// Inside the arm a test guards, the number it compares is known to satisfy it: the name is
    /// rebound to the same value typed by what the test says (`x > 9` makes `x` at least 10), so
    /// arithmetic in the arm is typed by the values it can see, as the checker proved it. The
    /// narrowing is a claim of the test just taken, never of anything assumed.
    fn narrow_by_test(
        &mut self,
        leaf: &super::guards::Leaf<'a>,
        test: &ExitTest,
        holds: bool,
        parameter_count: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) {
        use super::guards::Relation;
        let ExitTest::Compare { relation, right, upper: member_upper, .. } = test else {
            return;
        };
        let Expr::Name(name) = &leaf.left.value else {
            return;
        };
        let relation = if holds {
            *relation
        } else {
            match relation {
                Relation::Equal => Relation::NotEqual,
                Relation::NotEqual => Relation::Equal,
                Relation::Less => Relation::GreaterOrEqual,
                Relation::LessOrEqual => Relation::Greater,
                Relation::Greater => Relation::LessOrEqual,
                Relation::GreaterOrEqual => Relation::Less,
                Relation::Within | Relation::Outside => return,
            }
        };
        let SourceType::Integer { lower: right_low, upper: right_high } = arena[*right as usize].ty else {
            return;
        };
        let Some(node) = self.record_name(name, leaf.left.span_id, parameter_count, arena, scope) else {
            return;
        };
        let SourceType::Integer { lower, upper } = arena[node as usize].ty else {
            return;
        };
        let (mut low, mut high) = (lower, upper);
        match relation {
            Relation::Less => {
                let Some(bound) = right_high.checked_sub(I256::ONE) else { return };
                high = high.min(bound);
            }
            Relation::LessOrEqual => high = high.min(right_high),
            Relation::Greater => {
                let Some(bound) = right_low.checked_add(I256::ONE) else { return };
                low = low.max(bound);
            }
            Relation::GreaterOrEqual => low = low.max(right_low),
            Relation::Equal => {
                low = low.max(right_low);
                high = high.min(right_high);
            }
            // `x is in D` holds: between the domain's two bounds.
            Relation::Within => {
                let Some(top) = member_upper else { return };
                let SourceType::Integer { upper: top_high, .. } = arena[*top as usize].ty else { return };
                low = low.max(right_low);
                high = high.min(top_high);
            }
            _ => return,
        }
        if low > high || (low == lower && high == upper) {
            return;
        }
        let widened = push(
            arena,
            leaf.left.span_id,
            SourceType::Integer { lower: low, upper: high },
            CheckedSourceBodyKind::Widen { operand: node },
        );
        // A local is narrowed where it stands, so an assignment in the arm reaches it; a parameter
        // (which is not in the scope) gets an entry of its own. The caller owns a scope it throws
        // away after the arm.
        match scope.iter().rposition(|(local, _)| *local == name.as_str()) {
            Some(position) => scope[position].1 = widened,
            None => scope.push((name.as_str(), widened)),
        }
    }

    fn record_guard(
        &mut self,
        when: &'a WhenStatement,
        span: SpanId,
        rest: &'a [Spanned<Statement>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        // Only the function's own block, with no `mut` parameter outputs to pick per path.
        if depth != 0 || when.else_branch.is_some() {
            return None;
        }
        let (connective, leaves) = super::guards::conditions(&when.condition)?;
        let mut tests = Vec::new();
        for leaf in &leaves {
            tests.push(self.record_leaf(leaf, parameter_count, depth, arena, scope)?);
        }
        let narrowing: Vec<ExitTest> = tests.clone();
        let tests = self.combine_tests(tests, connective, span, arena)?;
        use super::guards::Connective;
        // What the test tells of a number on each path: the arm taken when it holds knows the
        // relation, the rest of the block knows its negation (a chain tells one side only).
        let then_truth = connective == Connective::And || leaves.len() == 1;
        let rest_truth = connective == Connective::Or || leaves.len() == 1;
        let mut inside = scope.clone();
        if then_truth {
            for (leaf, test) in leaves.iter().zip(&narrowing) {
                self.narrow_by_test(leaf, test, true, parameter_count, arena, &mut inside);
            }
        }
        let then_start = arena.len() as u32;
        let then_branch = self.record_branch(
            &when.then_block.value.statements,
            parameter_count,
            depth,
            arena,
            &mut inside,
        )?;
        let then_scope = inside;
        // What the rest knows is computed in the rest's own region: a narrowed value placed
        // before it would be in the `then` arm's, which the rest never runs.
        let else_start = arena.len() as u32;
        let mut inside = scope.clone();
        if rest_truth {
            for (leaf, test) in leaves.iter().zip(&narrowing) {
                self.narrow_by_test(leaf, test, false, parameter_count, arena, &mut inside);
            }
        }
        let else_branch = self.record_branch(rest, parameter_count, depth, arena, &mut inside)?;
        // The two exits must hand back the same `mut` values: one value graph has one set of
        // outputs, so a body that mutates on one path only is declined, never lowered wrongly.
        if !self.recording_bundle && self.arm_changed_mut(&then_scope, &inside) {
            self.recording_outputs_differ = true;
            return None;
        }
        // A single comparison owns its arms; a chain shares them between its selects.
        let region = SelectRegion {
            then_start,
            else_start,
            exclusive: tests.len() == 1,
        };
        // `a and b` is `a ? (b ? then : rest) : rest` and `a or b` is `a ? then : (b ? then :
        // rest)`: the same two nodes are shared, never copied.
        let mut chosen = None;
        for test in tests.into_iter().rev() {
            let (on_hold, on_fail) = match (connective, chosen) {
                (_, None) => (then_branch, else_branch),
                (super::guards::Connective::And, Some(inner)) => (inner, else_branch),
                (super::guards::Connective::Or, Some(inner)) => (then_branch, inner),
            };
            chosen = Some(self.select_node(&test, on_hold, on_fail, span, Some(region), arena)?);
        }
        chosen
    }

    /// `when c then ... else ...` with branches that fall through. The branches are recorded
    /// against copies of the scope; each local they leave different becomes a `Select` of its two
    /// values, so the recorded body is a value graph with no statement left in it. Both branches
    /// are computed, so they may hold only pure numbers: a call, an instruction or a loop in
    /// one is refused rather than run when its branch is not taken.
    fn record_join(
        &mut self,
        when: &'a WhenStatement,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let (_, leaves) = super::guards::conditions(&when.condition)?;
        let [leaf] = leaves[..] else {
            return None;
        };
        let test = self.record_leaf(&leaf, parameter_count, depth, arena, scope)?;
        let start = arena.len();
        let kept = scope.len();
        let mut then_scope = scope.clone();
        self.narrow_by_test(&leaf, &test, true, parameter_count, arena, &mut then_scope);
        let then_narrowed = then_scope.clone();
        self.record_fall_through(
            &when.then_block.value.statements,
            parameter_count,
            depth,
            arena,
            &mut then_scope,
        )?;
        let mut else_scope = scope.clone();
        self.narrow_by_test(&leaf, &test, false, parameter_count, arena, &mut else_scope);
        let else_narrowed = else_scope.clone();
        if let Some(branch) = &when.else_branch {
            self.record_fall_through(
                &branch.block.value.statements,
                parameter_count,
                depth,
                arena,
                &mut else_scope,
            )?;
        }
        let pure = arena[start..].iter().all(|node| {
            matches!(
                node.kind,
                CheckedSourceBodyKind::Parameter { .. }
                    | CheckedSourceBodyKind::Integer(_)
                    | CheckedSourceBodyKind::Binary { .. }
                    | CheckedSourceBodyKind::Wrap { .. }
                    | CheckedSourceBodyKind::Select { .. }
                    | CheckedSourceBodyKind::Widen { .. }
            )
        });
        if !pure {
            return None;
        }
        // The names the test narrowed were only shadows for the arms.
        then_scope.truncate(kept);
        else_scope.truncate(kept);
        for index in 0..kept {
            // A local the arm did not assign is as it was before the arm narrowed it.
            if then_scope[index].1 == then_narrowed[index].1 {
                then_scope[index].1 = scope[index].1;
            }
            if else_scope[index].1 == else_narrowed[index].1 {
                else_scope[index].1 = scope[index].1;
            }
        }
        for index in 0..scope.len() {
            let (then_node, else_node) = (then_scope[index].1, else_scope[index].1);
            if then_node == else_node {
                scope[index].1 = then_node;
                continue;
            }
            if !matches!(arena[then_node as usize].ty, SourceType::Integer { .. })
                || !matches!(arena[else_node as usize].ty, SourceType::Integer { .. })
            {
                return None;
            }
            scope[index].1 =
                self.select_node(&test, then_node, else_node, when.when_keyword, None, arena)?;
        }
        Some(())
    }

    /// Whether a branch of `when` holds a statement that is only an expression (a call run for
    /// what it does).
    fn runs_calls(when: &WhenStatement) -> bool {
        fn calls(statements: &[Spanned<Statement>]) -> bool {
            statements.iter().any(|statement| match &statement.value {
                Statement::Expression(_) => true,
                // `a.[i] :: v`: a store is an effect of the arm too.
                Statement::Assign(assign) => matches!(assign.place.value, Expr::Select(_)),
                Statement::When(inner) => {
                    calls(&inner.then_block.value.statements)
                        || inner.else_branch.as_ref().is_some_and(|branch| calls(&branch.block.value.statements))
                }
                _ => false,
            })
        }
        calls(&when.then_block.value.statements)
            || when.else_branch.as_ref().is_some_and(|branch| calls(&branch.block.value.statements))
    }

    fn case_runs_calls(when: &WhenIsStatement) -> bool {
        let calls = |statements: &[Spanned<Statement>]| {
            statements.iter().any(|statement| matches!(statement.value, Statement::Expression(_)))
        };
        calls(&when.then_block.value.statements)
            || when.else_branch.as_ref().is_some_and(|branch| calls(&branch.block.value.statements))
    }

    /// The member test of [`Self::record_effect_when`]: a `Match` whose arms hold the calls and
    /// give nothing; the member's arm binds its fields, and no local may change.
    fn record_effect_case(
        &mut self,
        when: &'a WhenIsStatement,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let MatchPattern::Variant(pattern) = &when.pattern.value else {
            return None;
        };
        let subject = self.record_expr(&when.subject, parameter_count, depth + 1, arena, scope)?;
        let subject_ty = arena[subject as usize].ty;
        let schema = self.recorded_pattern(subject_ty, &pattern.constructor.value)?;
        let mut members = Vec::new();
        for variant in self.recorded_variants(subject_ty) {
            let SourceType::Aggregate { id } = variant else {
                return None;
            };
            members.push(self.module.instances[id as usize].schema());
        }
        if members.is_empty() {
            members.push(schema);
        }
        let kept = scope.len();
        let member_start = arena.len() as u32;
        let mut inside = scope.clone();
        self.bind_fields(subject, schema, pattern.fields.as_ref(), arena, &mut inside)?;
        let bound = inside.clone();
        self.record_fall_through_calls(&when.then_block.value.statements, parameter_count, depth, arena, &mut inside)?;
        if inside[..bound.len()] != bound[..] {
            return None;
        }
        let then_unit = push(arena, when.when_keyword, SourceType::Unit, CheckedSourceBodyKind::Unit);
        let other_start = arena.len() as u32;
        if let Some(branch) = &when.else_branch {
            let mut inside = scope.clone();
            let before = inside.clone();
            self.record_fall_through_calls(&branch.block.value.statements, parameter_count, depth, arena, &mut inside)?;
            if inside[..kept] != before[..kept] {
                return None;
            }
        }
        let else_unit = push(arena, when.when_keyword, SourceType::Unit, CheckedSourceBodyKind::Unit);
        let arms = members
            .into_iter()
            .map(|member| CheckedSourceMatchArm {
                schema: member as u32,
                body: if member == schema { then_unit } else { else_unit },
            })
            .collect();
        push(
            arena,
            when.when_keyword,
            SourceType::Unit,
            CheckedSourceBodyKind::Match {
                subject,
                arms,
                region: Some(MatchRegion { member_body: then_unit, member_start, other_start }),
            },
        );
        Some(())
    }

    /// `when test then <calls> [else <calls>]` that falls through: a choice whose arms hold the
    /// calls, so each runs only when its arm is taken. With no local changed it chooses between
    /// two empty values; otherwise each arm ends in a bundle of the changed locals' values on
    /// that arm, and after the choice each changed local is its part of the chosen bundle.
    fn record_effect_when(
        &mut self,
        when: &'a WhenStatement,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let (connective, leaves) = super::guards::conditions(&when.condition)?;
        let mut tests = Vec::new();
        for leaf in &leaves {
            tests.push(self.record_leaf(leaf, parameter_count, depth, arena, scope)?);
        }
        let narrowing = tests.clone();
        let tests = self.combine_tests(tests, connective, when.when_keyword, arena)?;
        let [test] = &tests[..] else {
            return None;
        };
        use super::guards::Connective;
        let then_truth = connective == Connective::And || leaves.len() == 1;
        let else_truth = connective == Connective::Or || leaves.len() == 1;
        let kept = scope.len();
        let start = arena.len();
        let saved = (self.conversions_recorded, self.recording_folds.len());
        // Which locals the arms change is known once they are recorded; the `then` bundle must
        // come before the `else` arm, so with any changed they are recorded again knowing it.
        let mut changed: Vec<usize> = Vec::new();
        for pass in 0..2 {
            let then_start = arena.len() as u32;
            let mut inside = scope.clone();
            if then_truth {
                for (leaf, test) in leaves.iter().zip(&narrowing) {
                    self.narrow_by_test(leaf, test, true, parameter_count, arena, &mut inside);
                }
            }
            let narrowed = inside.clone();
            self.record_fall_through_calls(&when.then_block.value.statements, parameter_count, depth, arena, &mut inside)?;
            let mut found: Vec<usize> = (0..kept).filter(|&i| inside[i] != narrowed[i]).collect();
            let then_value = self.arm_bundle(None, &changed, &inside, &narrowed, scope, when.when_keyword, arena)?;
            let else_start = arena.len() as u32;
            let mut inside = scope.clone();
            if else_truth {
                for (leaf, test) in leaves.iter().zip(&narrowing) {
                    self.narrow_by_test(leaf, test, false, parameter_count, arena, &mut inside);
                }
            }
            let narrowed = inside.clone();
            if let Some(branch) = &when.else_branch {
                self.record_fall_through_calls(&branch.block.value.statements, parameter_count, depth, arena, &mut inside)?;
            }
            found.extend((0..kept).filter(|&i| inside[i] != narrowed[i]));
            found.sort_unstable();
            found.dedup();
            let else_value = self.arm_bundle(None, &changed, &inside, &narrowed, scope, when.when_keyword, arena)?;
            if found != changed {
                // A second pass must find what the first did.
                if pass == 1 {
                    return None;
                }
                arena.truncate(start);
                self.conversions_recorded = saved.0;
                self.recording_folds.truncate(saved.1);
                changed = found;
                continue;
            }
            let region = SelectRegion { then_start, else_start, exclusive: true };
            let chosen = self.select_node(test, then_value, else_value, when.when_keyword, Some(region), arena)?;
            self.unpack_changed(&changed, 0, chosen, scope, when.when_keyword, arena)?;
            return Some(());
        }
        None
    }

    /// What an arm of a conditional gives: its value (`lead`, or nothing) when no local changes,
    /// else the bundle of that value and each changed local's value on the arm (the value from
    /// before the conditional where the arm left it), typed as the local was.
    #[allow(clippy::too_many_arguments)]
    fn arm_bundle(
        &mut self,
        lead: Option<u32>,
        changed: &[usize],
        inside: &Scope<'a>,
        narrowed: &Scope<'a>,
        before: &Scope<'a>,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        if changed.is_empty() {
            return Some(match lead {
                Some(value) => value,
                None => push(arena, span, SourceType::Unit, CheckedSourceBodyKind::Unit),
            });
        }
        let mut parts = Vec::with_capacity(changed.len() + 1);
        let mut types = Vec::with_capacity(changed.len() + 1);
        if let Some(value) = lead {
            parts.push(value);
            types.push(arena[value as usize].ty);
        }
        for &index in changed {
            let node = if inside[index] != narrowed[index] { inside[index].1 } else { before[index].1 };
            let declared = arena[before[index].1 as usize].ty;
            // A member where the local holds a sum (`%ret` given `Gone` of a `Pick`) stays itself:
            // the bundle is typed by the sum, and lowering puts the member in the sum's shape.
            parts.push(self.lead_to(node, declared, span, arena)?);
            types.push(declared);
        }
        let ty = self.sequence_type(types).ok()?;
        Some(push(arena, span, ty, CheckedSourceBodyKind::Bundle { parts }))
    }

    /// After a conditional whose arms changed `changed`: each names its part of `chosen`.
    fn unpack_changed(
        &mut self,
        changed: &[usize],
        first: usize,
        chosen: u32,
        scope: &mut Scope<'a>,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<()> {
        for (part, &index) in changed.iter().enumerate() {
            let ty = arena[scope[index].1 as usize].ty;
            let index_of = u32::try_from(first + part).ok()?;
            scope[index].1 = push(arena, span, ty, CheckedSourceBodyKind::Part { bundle: chosen, index: index_of });
        }
        Some(())
    }

    /// The statements of a branch of an effectful `when`: bindings, assignments, calls and
    /// conditionals, never a return.
    fn record_fall_through_calls(
        &mut self,
        statements: &'a [Spanned<Statement>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let mut returns = Vec::new();
        self.record_statements(statements, parameter_count, depth + 1, arena, scope, &mut returns)?;
        returns.is_empty().then_some(())
    }

    /// A branch that falls through: bindings, assignments and conditionals, never a return or
    /// a bare expression (which would run whether or not the branch is taken).
    fn record_fall_through(
        &mut self,
        statements: &'a [Spanned<Statement>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        if statements
            .iter()
            .any(|statement| matches!(statement.value, Statement::Expression(_)))
        {
            return None;
        }
        let mut returns = Vec::new();
        self.record_statements(
            statements,
            parameter_count,
            depth + 1,
            arena,
            scope,
            &mut returns,
        )?;
        returns.is_empty().then_some(())
    }

    /// `when subject is Member(binders) then ... return` followed by `rest`: a `Match` whose
    /// arm for `Member` is the block's value and whose every other arm is what `rest` returns.
    /// The subject's member, when known, picks its arm and nothing else is recorded. Both
    /// arms are computed (see `record_guard`), so the lowering refuses a `rest` that would run
    /// an instruction.
    #[allow(clippy::too_many_arguments)]
    fn record_case_guard(
        &mut self,
        when: &'a WhenIsStatement,
        _span: SpanId,
        rest: &'a [Spanned<Statement>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        if depth != 0 || when.else_branch.is_some() {
            return None;
        }
        let MatchPattern::Variant(pattern) = &when.pattern.value else {
            return None;
        };
        let subject = self.record_expr(&when.subject, parameter_count, depth + 1, arena, scope)?;
        let subject_ty = arena[subject as usize].ty;
        let schema = self.recorded_pattern(subject_ty, &pattern.constructor.value)?;
        let mut members = Vec::new();
        for variant in self.recorded_variants(subject_ty) {
            let SourceType::Aggregate { id } = variant else {
                return None;
            };
            members.push(self.module.instances[id as usize].schema());
        }
        if members.is_empty() {
            members.push(schema);
        }
        let known = match &arena[subject as usize].kind {
            CheckedSourceBodyKind::Construct { schema, .. } => Some(*schema as usize),
            _ => None,
        };
        // The rest of the body is the other path: it sees the locals as they were before the
        // member's arm, never what the arm changed.
        let before = scope.clone();
        let member_start = arena.len() as u32;
        self.bind_fields(subject, schema, pattern.fields.as_ref(), arena, scope)?;
        // A member named without parentheses narrows the subject: inside the branch the local
        // is that member, rebuilt from the sum's fields, so its own methods can be called.
        let mut narrowed = None;
        if pattern.fields.is_none()
            && let Expr::Name(local) = &when.subject.value
            && matches!(subject_ty, SourceType::Sum { .. })
            && let Some(position) = scope.iter().rposition(|(name, _)| *name == local.as_str())
            && let Some(member) = self.recorded_variants(subject_ty).into_iter().find(|variant| {
                matches!(variant, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == schema)
            })
        {
            let member = self.member_of_instance(subject_ty, member)?;
            let types = self.field_types_of(member, schema)?;
            let mut fields = Vec::with_capacity(types.len());
            for (index, ty) in types.iter().enumerate() {
                fields.push(push(
                    arena,
                    when.when_keyword,
                    *ty,
                    CheckedSourceBodyKind::VariantField {
                        subject,
                        schema: schema as u32,
                        index: index as u32,
                    },
                ));
            }
            let node = push(
                arena,
                when.when_keyword,
                member,
                CheckedSourceBodyKind::Construct { schema: schema as u32, fields },
            );
            narrowed = Some((position, scope[position].1));
            scope[position].1 = node;
        }
        let then_branch = self.record_branch(
            &when.then_block.value.statements,
            parameter_count,
            depth,
            arena,
            scope,
        );
        if let Some((position, old)) = narrowed {
            scope[position].1 = old;
        }
        let then_branch = then_branch?;
        scope.clone_from(&before);
        if known == Some(schema) {
            return Some(then_branch);
        }
        let other_start = arena.len() as u32;
        // With two members, where this one is not the subject's the other is: a local subject is
        // that member in the rest, rebuilt from the sum's fields.
        let mut narrowed_rest = None;
        if members.len() == 2
            && let Expr::Name(local) = &when.subject.value
            && matches!(subject_ty, SourceType::Sum { .. })
            && let Some(position) = scope.iter().rposition(|(name, _)| *name == local.as_str())
            && let Some(other) = members.iter().copied().find(|member| *member != schema)
            && let Some(member) = self.recorded_variants(subject_ty).into_iter().find(|variant| {
                matches!(variant, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == other)
            })
        {
            let member = self.member_of_instance(subject_ty, member)?;
            let types = self.field_types_of(member, other)?;
            let mut fields = Vec::with_capacity(types.len());
            for (index, ty) in types.iter().enumerate() {
                fields.push(push(
                    arena,
                    when.when_keyword,
                    *ty,
                    CheckedSourceBodyKind::VariantField { subject, schema: other as u32, index: index as u32 },
                ));
            }
            let node = push(
                arena,
                when.when_keyword,
                member,
                CheckedSourceBodyKind::Construct { schema: other as u32, fields },
            );
            narrowed_rest = Some((position, scope[position].1));
            scope[position].1 = node;
        }
        let else_branch = self.record_branch(rest, parameter_count, depth, arena, scope);
        if let Some((position, old)) = narrowed_rest {
            scope[position].1 = old;
        }
        let else_branch = else_branch?;
        if known.is_some() {
            return Some(else_branch);
        }
        let arms: Vec<CheckedSourceMatchArm> = members
            .into_iter()
            .map(|member| CheckedSourceMatchArm {
                schema: member as u32,
                body: if member == schema {
                    then_branch
                } else {
                    else_branch
                },
            })
            .collect();
        let bodies = || [then_branch, else_branch].into_iter();
        let ty = join(arena, bodies())
            .or_else(|| self.sum_of(arena, bodies()))
            .or_else(|| self.sum_of_generic(arena, bodies()))
            // The function's own result sum (`text String(alloc) .. or ListFailure(alloc)`).
            .or_else(|| self.recording_result.filter(|ty| matches!(ty, SourceType::Sum { .. })))?;
        Some(push(
            arena,
            when.when_keyword,
            ty,
            CheckedSourceBodyKind::Match {
                subject,
                arms,
                region: Some(MatchRegion {
                    member_body: then_branch,
                    member_start,
                    other_start,
                }),
            },
        ))
    }

    /// The value a run of statements returns, with its locals dropped afterwards.
    fn record_branch(
        &mut self,
        statements: &'a [Spanned<Statement>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let locals = scope.len();
        let mut returns = Vec::new();
        self.record_statements(statements, parameter_count, depth, arena, scope, &mut returns)?;
        scope.truncate(locals);
        match arena[*returns.last()? as usize].kind {
            CheckedSourceBodyKind::Return { value } => Some(value),
            _ => None
        }
    }

    /// `place :: value` and `place op: value` on a local: the local now names the new value's
    /// node, and a field assignment rebuilds the record around the replaced field. Names
    /// are resolved by scope, so the recording stays single-assignment: nothing mutates.
    fn record_assign(
        &mut self,
        assign: &'a AssignStatement,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        // `a.[i] :: v` on a borrowed array: a typed store at the element's address. The bound and
        // the write permission are the checker's (a `ref` array refuses the write).
        if let Expr::Select(select) = &assign.place.value
            && select.length.is_none()
            && assign.compound.is_none()
        {
            let base = self.record_expr(&select.base, parameter_count, depth + 1, arena, scope)?;
            let SourceType::Array { id } = arena[base as usize].ty else {
                return None;
            };
            let instance = self.module.instances.get(id as usize)?;
            if !matches!(instance.head, CheckedSourceInstanceHead::Slice | CheckedSourceInstanceHead::Array { .. }) {
                return None;
            }
            let element = *instance.arguments.first()?;
            let index = self.record_expr(&select.index, parameter_count, depth + 1, arena, scope)?;
            let value = self.record_expr(&assign.value, parameter_count, depth + 1, arena, scope)?;
            let value = self.coerce(value, element, assign.value.span_id, arena)?;
            self.recording_effects = true;
            push(arena, assign.place.span_id, SourceType::Unit, CheckedSourceBodyKind::Store { pointer: base, index, value });
            return Some(());
        }
        let (root, path) = assign::split_place(&assign.place)?;
        // A `mut` parameter is a borrow of the caller's value: no recorded form yet.
        let position = scope.iter().rposition(|(local, _)| *local == root)?;
        let node = match &assign.compound {
            None => self.record_expr(&assign.value, parameter_count, depth + 1, arena, scope)?,
            Some(compound) => {
                let infix = InfixOperator::from_binary(compound.value)?;
                let left =
                    self.record_expr(&assign.place, parameter_count, depth + 1, arena, scope)?;
                let start = arena.len();
                let right =
                    self.record_expr(&assign.value, parameter_count, depth + 1, arena, scope)?;
                if matches!(arena[left as usize].ty, SourceType::Aggregate { .. }) {
                    self.record_infix(assign.place.span_id, infix, (left, right), arena)?
                } else {
                    record_arithmetic(
                        infix.recordable()?,
                        assign.place.span_id,
                        start,
                        (left, right),
                        arena,
                    )?
                }
            }
        };
        let old = scope[position].1;
        let new = if path.is_empty() {
            let expected = arena[old as usize].ty;
            let new = self.coerce(node, expected, assign.value.span_id, arena)?;
            // An integer local keeps the range it is declared to hold.
            if matches!(expected, SourceType::Integer { .. }) && arena[new as usize].ty != expected
            {
                push(
                    arena,
                    assign.value.span_id,
                    expected,
                    CheckedSourceBodyKind::Widen { operand: new },
                )
            } else {
                new
            }
        } else {
            self.replace_field(old, &path, node, assign.value.span_id, arena)?
        };
        scope[position].1 = new;
        Some(())
    }

    /// The range an `in lower to upper` annotation declares, when both bounds are known.
    fn declared_range(
        &mut self,
        annotation: &'a Spanned<Annotation>,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<SourceType> {
        let Annotation::Domain { domain, .. } = &annotation.value else {
            return None;
        };
        let DomainBase::Range { lower, upper, .. } = &domain.base.value else {
            return None;
        };
        let mark = arena.len();
        let low = self.record_expr(lower, parameter_count, depth + 1, arena, scope)?;
        let high = self.record_expr(upper, parameter_count, depth + 1, arena, scope)?;
        let bounds = (known_integer(arena, low)?, known_integer(arena, high)?);
        arena.truncate(mark);
        Some(SourceType::Integer {
            lower: bounds.0,
            upper: bounds.1,
        })
    }

    /// The record `record` with the field at `path` replaced by `value`.
    fn replace_field(
        &mut self,
        record: u32,
        path: &[&Name],
        value: u32,
        span: SpanId,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        let (first, rest) = path.split_first()?;
        let ty = arena[record as usize].ty;
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let schema = self.module.instances[id as usize].schema();
        let layout = &self.module.layouts[schema].schema;
        if layout.kind == AggregateKind::Domain {
            return None;
        }
        let index = layout
            .fields
            .iter()
            .position(|field| field.name == first.value)?;
        // A generic record's fields are read at the arguments of this instance.
        let field_types: Vec<SourceType> = self.field_types_of(ty, schema)?;
        let mut fields = Vec::with_capacity(field_types.len());
        for (position, field_ty) in field_types.iter().enumerate() {
            let current = match &arena[record as usize].kind {
                CheckedSourceBodyKind::Construct { fields, .. } => fields.get(position).copied(),
                _ => None,
            };
            let current = match current {
                Some(node) => node,
                None => push(
                    arena,
                    span,
                    *field_ty,
                    CheckedSourceBodyKind::Field {
                        record,
                        index: u32::try_from(position).ok()?,
                    },
                ),
            };
            fields.push(current);
        }
        fields[index] = if rest.is_empty() {
            self.coerce(value, field_types[index], span, arena)?
        } else {
            self.replace_field(fields[index], rest, value, span, arena)?
        };
        Some(push(
            arena,
            span,
            ty,
            CheckedSourceBodyKind::Construct {
                schema: u32::try_from(schema).ok()?,
                fields,
            },
        ))
    }

    /// `for i in lo to hi do body`. With both bounds known and a small count, the passes are
    /// recorded one after another, each with its counter a constant. Otherwise the body is
    /// recorded once as a `Loop`, carrying the integer locals it assigns from pass to pass.
    fn record_loop(
        &mut self,
        expression: &'a Expression,
        loop_expression: &'a LoopExpression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        if let LoopExpression::While {
            condition, body, ..
        } = loop_expression
        {
            return self.record_while(
                expression,
                condition,
                body,
                parameter_count,
                depth,
                arena,
                scope,
            );
        }
        let LoopExpression::For {
            binder,
            iterable,
            body,
            invariant,
            ..
        } = loop_expression
        else {
            return None;
        };
        let Expr::Range { start, end, .. } = &iterable.value else {
            // `for item in array do ()` that only ends an owned array of numbers: nothing runs.
            let array = self.record_expr(iterable, parameter_count, depth + 1, arena, scope)?;
            let SourceType::Array { id } = arena[array as usize].ty else {
                return None;
            };
            let instance = self.module.instances.get(id as usize)?;
            let numbers = matches!(instance.head, CheckedSourceInstanceHead::Array { .. })
                && instance.arguments.first().is_some_and(|element| {
                    matches!(element, SourceType::Integer { .. }) || self.domain_bounds_of(*element).is_some()
                });
            let empty = self.text_of(body.span_id).is_some_and(|text| text.trim() == "()");
            if !numbers || !empty {
                return None;
            }
            return Some(push(arena, expression.span_id, SourceType::Unit, CheckedSourceBodyKind::Unit));
        };
        let first = self.record_expr(start, parameter_count, depth + 1, arena, scope)?;
        let last = self.record_expr(end, parameter_count, depth + 1, arena, scope)?;
        let unit = |arena: &mut Vec<CheckedSourceBodyExpr>| {
            push(
                arena,
                expression.span_id,
                SourceType::Unit,
                CheckedSourceBodyKind::Unit,
            )
        };
        // A loop that can `return` carries whether it has, and what; each pass, unrolled or not,
        // runs only while it has not.
        let exits = Self::body_returns(body);
        if exits {
            self.begin_exit_state(expression.span_id, arena, scope)?;
        }
        // A loop with an invariant is checked once whatever its count, so it is recorded once too:
        // unrolled, its conversions would be counted once per pass.
        if invariant.is_none()
            && let (Some(low), Some(high)) = (known_integer(arena, first), known_integer(arena, last))
        {
            if low > high {
                return Some(unit(arena));
            }
            if high - low < I256::from(RECORD_UNROLL_MAX) {
                let mut number = low;
                loop {
                    if arena.len() > RECORD_NODES_MAX {
                        return None;
                    }
                    let locals = scope.len();
                    let counter = push_integer(arena, binder.span_id, number);
                    scope.push((binder.value.as_str(), counter));
                    self.record_for_pass(body, exits, locals, expression.span_id, parameter_count, depth, arena, scope)?;
                    scope.truncate(locals);
                    if number == high {
                        break;
                    }
                    number = number.checked_add(I256::ONE)?;
                }
                return Some(unit(arena));
            }
        }
        let (SourceType::Integer { lower, .. }, SourceType::Integer { upper, .. }) =
            (arena[first as usize].ty, arena[last as usize].ty)
        else {
            return None;
        };
        let counter_ty = SourceType::Integer { lower, upper };

        // First find which outer locals a pass assigns, then record the body for real with
        // those standing for the value carried in.
        let outer = scope.clone();
        let mark = arena.len();
        let saved = (self.conversions_recorded, self.recording_effects);
        let counter = push(
            arena,
            binder.span_id,
            counter_ty,
            CheckedSourceBodyKind::LoopCounter,
        );
        scope.push((binder.value.as_str(), counter));
        let probed = self.record_for_pass(body, exits, outer.len(), expression.span_id, parameter_count, depth, arena, scope);
        let changed: Vec<usize> = (0..outer.len())
            .filter(|index| scope.get(*index) != outer.get(*index))
            .collect();
        arena.truncate(mark);
        *scope = outer.clone();
        (self.conversions_recorded, self.recording_effects) = saved;
        probed?;

        let body_start = arena.len();
        let counter = push(
            arena,
            binder.span_id,
            counter_ty,
            CheckedSourceBodyKind::LoopCounter,
        );
        let mut carried = Vec::new();
        for &index in &changed {
            let initial = outer[index].1;
            let ty = arena[initial as usize].ty;
            // A number, or a record or sum (carried as its registers).
            let (SourceType::Integer { .. } | SourceType::Aggregate { .. } | SourceType::Sum { .. }) = ty else {
                return None;
            };
            let placeholder = push(
                arena,
                binder.span_id,
                ty,
                CheckedSourceBodyKind::LoopCarried,
            );
            scope[index].1 = placeholder;
            carried.push(CheckedSourceLoopCarry {
                placeholder,
                initial,
                next: placeholder,
            });
        }
        scope.push((binder.value.as_str(), counter));
        self.record_for_pass(body, exits, outer.len(), expression.span_id, parameter_count, depth, arena, scope)?;
        for (carry, &index) in carried.iter_mut().zip(&changed) {
            carry.next = scope[index].1;
            // What a pass leaves must fit what the next one assumes. A record is carried whole:
            // what the pass leaves is the same record type.
            let (placeholder_ty, next_ty) = (arena[carry.placeholder as usize].ty, arena[carry.next as usize].ty);
            if matches!(placeholder_ty, SourceType::Aggregate { .. } | SourceType::Sum { .. }) {
                if placeholder_ty != next_ty {
                    return None;
                }
                continue;
            }
            let (
                SourceType::Integer { lower, upper },
                SourceType::Integer {
                    lower: low,
                    upper: high,
                },
            ) = (placeholder_ty, next_ty)
            else {
                return None;
            };
            if low < lower || high > upper {
                return None;
            }
        }
        *scope = outer;
        let looped = push(
            arena,
            expression.span_id,
            SourceType::Unit,
            CheckedSourceBodyKind::Loop {
                start: first,
                end: last,
                counter,
                body_start: u32::try_from(body_start).ok()?,
                carried: carried.clone(),
            },
        );
        for (index, &position) in changed.iter().enumerate() {
            let ty = arena[carried[index].placeholder as usize].ty;
            scope[position].1 = push(
                arena,
                expression.span_id,
                ty,
                CheckedSourceBodyKind::LoopResult {
                    looped,
                    index: u32::try_from(index).ok()?,
                },
            );
        }
        Some(looped)
    }

    /// `while`: the locals a pass assigns are carried, each standing for the value it entered
    /// the pass with; the condition and body are recorded in terms of them. A loop that changes
    /// nothing is refused (it could only be an effect or a hang, neither of which a value graph
    /// can say).
    #[allow(clippy::too_many_arguments)]
    fn record_while(
        &mut self,
        expression: &'a Expression,
        condition: &'a Expression,
        body: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let (_, leaves) = super::guards::conditions(condition)?;
        let [leaf] = leaves[..] else {
            return None;
        };
        // A loop that goes on while a number is in a domain is not recorded.
        let super::guards::Side::Expr(right) = leaf.right else {
            return None;
        };
        let left = leaf.left;
        let (comparison, negated) = Self::infix_of(leaf.relation);
        // A body that can `return` carries two more values: whether it has, and what it
        // returned. They start as "not yet" and the result's lowest value.
        let exits = Self::body_returns(body);
        if exits {
            self.begin_exit_state(expression.span_id, arena, scope)?;
        }
        // First find which outer locals a pass assigns, then record for real with those
        // standing for the value carried in.
        let outer = scope.clone();
        let mark = arena.len();
        let saved = (self.conversions_recorded, self.recording_effects);
        let probed = self.record_loop_body(body, parameter_count, depth, arena, scope);
        let changed: Vec<usize> = (0..outer.len())
            .filter(|index| scope.get(*index) != outer.get(*index))
            .collect();
        arena.truncate(mark);
        *scope = outer.clone();
        (self.conversions_recorded, self.recording_effects) = saved;
        probed?;

        // A condition that is false for the values the loop is entered with never runs it.
        let entry = arena.len();
        let first_left = self.record_expr(left, parameter_count, depth + 1, arena, scope)?;
        let first_right = self.record_expr(right, parameter_count, depth + 1, arena, scope)?;
        let never = match (known_integer(arena, first_left), known_integer(arena, first_right)) {
            (Some(left), Some(right)) => {
                let fails = match comparison {
                    InfixOperator::Equal => left != right,
                    InfixOperator::Less => left >= right,
                    InfixOperator::LessOrEqual => left > right,
                    InfixOperator::Greater => left <= right,
                    InfixOperator::GreaterOrEqual => left < right,
                    _ => return None,
                };
                // A negated comparison runs the loop where the plain one fails.
                Some(fails != negated)
            }
            _ => None,
        };
        arena.truncate(entry);
        if never == Some(true) {
            return Some(push(
                arena,
                expression.span_id,
                SourceType::Unit,
                CheckedSourceBodyKind::Unit,
            ));
        }

        let body_start = arena.len();
        let mut carried = Vec::new();
        for &index in &changed {
            let initial = outer[index].1;
            let ty = arena[initial as usize].ty;
            // A number, or a record or sum (carried as its registers).
            let (SourceType::Integer { .. } | SourceType::Aggregate { .. } | SourceType::Sum { .. }) = ty else {
                return None;
            };
            let placeholder = push(arena, expression.span_id, ty, CheckedSourceBodyKind::LoopCarried);
            scope[index].1 = placeholder;
            carried.push(CheckedSourceLoopCarry {
                placeholder,
                initial,
                next: placeholder,
            });
        }
        let left = self.record_expr(left, parameter_count, depth + 1, arena, scope)?;
        let right = self.record_expr(right, parameter_count, depth + 1, arena, scope)?;
        if !matches!(arena[left as usize].ty, SourceType::Integer { .. })
            || !matches!(arena[right as usize].ty, SourceType::Integer { .. })
        {
            return None;
        }
        let split = arena.len();
        // The rest of a pass after a `return` is the other arm of its guard (`record_loop_exit`),
        // so a body that returns may run effects: they run only on a pass that did not return.
        self.record_loop_body(body, parameter_count, depth, arena, scope)?;
        for (carry, &index) in carried.iter_mut().zip(&changed) {
            carry.next = scope[index].1;
            // What a pass leaves must fit what the next one assumes. A record is carried whole:
            // what the pass leaves is the same record type.
            let (placeholder_ty, next_ty) = (arena[carry.placeholder as usize].ty, arena[carry.next as usize].ty);
            if matches!(placeholder_ty, SourceType::Aggregate { .. } | SourceType::Sum { .. }) {
                if placeholder_ty != next_ty {
                    return None;
                }
                continue;
            }
            let (
                SourceType::Integer { lower, upper },
                SourceType::Integer {
                    lower: low,
                    upper: high,
                },
            ) = (
                arena[carry.placeholder as usize].ty,
                arena[carry.next as usize].ty,
            )
            else {
                return None;
            };
            if low < lower || high > upper {
                return None;
            }
        }
        let exit_position = changed
            .iter()
            .position(|&index| outer[index].0 == "%done")
            .map(|position| position as u32);
        *scope = outer;
        let looped = push(
            arena,
            expression.span_id,
            SourceType::Unit,
            CheckedSourceBodyKind::While {
                comparison,
                negated,
                left,
                right,
                body_start: u32::try_from(body_start).ok()?,
                split: u32::try_from(split).ok()?,
                carried: carried.clone(),
                exit: exit_position,
            },
        );
        for (index, &position) in changed.iter().enumerate() {
            let ty = arena[carried[index].placeholder as usize].ty;
            scope[position].1 = push(
                arena,
                expression.span_id,
                ty,
                CheckedSourceBodyKind::LoopResult {
                    looped,
                    index: u32::try_from(index).ok()?,
                },
            );
        }
        Some(looped)
    }

    /// A pass of a `for` loop. One that can `return` runs only while `%done` is 0: `scf.for`
    /// cannot stop, so a pass after the return is a choice that hands every local through.
    #[allow(clippy::too_many_arguments)]
    fn record_for_pass(
        &mut self,
        body: &'a Expression,
        exits: bool,
        kept: usize,
        span: SpanId,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        if !exits {
            return self.record_loop_body(body, parameter_count, depth, arena, scope);
        }
        let (done_at, _) = Self::exit_state(scope)?;
        let zero = push_integer(arena, span, I256::ZERO);
        let test = ExitTest::Compare {
            relation: super::guards::Relation::Equal,
            left: scope[done_at].1,
            right: zero,
            upper: None,
        };
        let start = arena.len();
        let saved = (self.conversions_recorded, self.recording_folds.len());
        let mut changed: Vec<usize> = Vec::new();
        for pass in 0..2 {
            let then_start = arena.len() as u32;
            let mut inside = scope.clone();
            self.record_loop_body(body, parameter_count, depth, arena, &mut inside)?;
            let found: Vec<usize> = (0..kept).filter(|&i| inside[i] != scope[i]).collect();
            let then_value = self.arm_bundle(None, &changed, &inside, scope, scope, span, arena)?;
            let else_start = arena.len() as u32;
            let else_value = self.arm_bundle(None, &changed, scope, scope, scope, span, arena)?;
            if found != changed {
                if pass == 1 {
                    return None;
                }
                arena.truncate(start);
                self.conversions_recorded = saved.0;
                self.recording_folds.truncate(saved.1);
                changed = found;
                continue;
            }
            let region = SelectRegion { then_start, else_start, exclusive: true };
            let chosen = self.select_node(&test, then_value, else_value, span, Some(region), arena)?;
            return self.unpack_changed(&changed, 0, chosen, scope, span, arena);
        }
        None
    }

    /// One pass of a loop body; the caller binds the counter and drops the body's locals.
    fn record_loop_body(
        &mut self,
        body: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<()> {
        let locals = scope.len();
        match &body.value {
            Expr::Block(block) => {
                let mut returns = Vec::new();
                self.record_statements(
                    &block.statements,
                    parameter_count,
                    depth + 1,
                    arena,
                    scope,
                    &mut returns,
                )?;
                if !returns.is_empty() {
                    return None;
                }
            }
            _ => {
                self.record_expr(body, parameter_count, depth + 1, arena, scope)?;
            }
        }
        scope.truncate(locals);
        Some(())
    }

    /// `base.value` for a union whose variants have values: the literal of the variant `base`
    /// holds, as a `Match` over the union's variants, or the one literal when the variant is known.
    fn record_discriminant(
        &mut self,
        span: SpanId,
        base: &'a Expression,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let union = self.discriminant_union(base)?;
        let subject = self.record_expr(base, parameter_count, depth, arena, scope)?;
        let literal = |checker: &Self, arena: &mut Vec<CheckedSourceBodyExpr>, schema: usize| -> Option<u32> {
            Some(match checker.module.layouts.get(schema)?.schema.discriminant.as_ref()? {
                crate::Discriminant::Integer(value) => push_integer(arena, span, *value),
                crate::Discriminant::Text(text) => {
                    push(arena, span, SourceType::Text, CheckedSourceBodyKind::Text(text.clone()))
                }
            })
        };
        let known = match (&arena[subject as usize].kind, arena[subject as usize].ty) {
            (CheckedSourceBodyKind::Construct { schema, .. }, _) => Some(*schema as usize),
            (_, SourceType::Aggregate { id }) => Some(self.module.instances[id as usize].schema()),
            _ => None,
        };
        if let Some(schema) = known {
            return literal(self, arena, schema);
        }
        let variants: Vec<usize> = self.module.layouts[union]
            .schema
            .variants
            .iter()
            .filter_map(|variant| match variant {
                SourceType::Aggregate { id } => Some(self.module.instances[*id as usize].schema()),
                _ => None,
            })
            .collect();
        let mut arms = Vec::with_capacity(variants.len());
        for schema in variants {
            let body = literal(self, arena, schema)?;
            arms.push(CheckedSourceMatchArm { schema: schema as u32, body });
        }
        let ty = join(arena, arms.iter().map(|arm| arm.body))?;
        Some(push(arena, span, ty, CheckedSourceBodyKind::Match { subject, arms, region: None }))
    }

    fn record_name(
        &mut self,
        name: &str,
        span: SpanId,
        parameter_count: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &Scope<'a>,
    ) -> Option<u32> {
        // A local shadows a parameter of the same name.
        if let Some((_, node)) = scope.iter().rev().find(|(local, _)| *local == name) {
            return Some(*node);
        }
        let Some(position) = self
            .proof
            .names
            .iter()
            .position(|(other, _)| *other == name)
            .filter(|position| *position < parameter_count)
        else {
            if let Some(value) = self.known_constant(name) {
                return Some(push_integer(arena, span, value));
            }
            // Not a value in scope: a member written without fields, like `Empty`, or the
            // variant of a union that this name alone identifies.
            let schema = self
                .recorded_record(name)
                .or_else(|| self.recorded_unique_variant(name))?;
            let ty = self.recorded_instance(schema)?;
            let fields = Vec::new();
            return Some(push(
                arena,
                span,
                ty,
                CheckedSourceBodyKind::Construct {
                    schema: schema as u32,
                    fields,
                },
            ));
        };
        let ty = self.proof.names[position].1.ty;
        Some(push(
            arena,
            span,
            ty,
            CheckedSourceBodyKind::Parameter {
                index: position as u32,
            },
        ))
    }

    pub(super) fn literal_text(&self, span: SpanId) -> Option<&'a str> {
        self.text_of(span)
    }
}

/// An expression form in words, for the reason a body is declined.
fn describe_construct(expression: &Expression) -> (DeclineKind, String) {
    use DeclineKind::{Expression as Form, Operator};
    match &expression.value {
        Expr::Binary { operator, .. } => {
            let (kind, what) = match operator.value {
                BinaryOperator::Divide => (Operator, "division `/`"),
                BinaryOperator::Remainder => (Operator, "remainder `%`"),
                BinaryOperator::BitAnd => (Operator, "bit and `&`"),
                BinaryOperator::BitOr => (Operator, "bit or `|`"),
                BinaryOperator::BitXor => (Operator, "bit xor `^`"),
                BinaryOperator::ShiftLeft => (Operator, "shift `<<`"),
                BinaryOperator::ShiftRight => (Operator, "shift `>>`"),
                _ => (Form, "an arithmetic or logical expression"),
            };
            (kind, what.to_owned())
        }
        Expr::Loop(_) => (Form, "a loop form".to_owned()),
        Expr::Array(_) | Expr::Repeat(_) => (Form, "an array".to_owned()),
        Expr::Sequence(_) => (Form, "a sequence".to_owned()),
        Expr::Select(_) | Expr::NumericSelect(_) => (Form, "an index or position select".to_owned()),
        Expr::Range { .. } => (Form, "a range".to_owned()),
        Expr::Cast { .. } => (Form, "a cast".to_owned()),
        Expr::Unary { .. } => (Form, "a unary operator".to_owned()),
        Expr::PrefixedType(prefixed)
            if prefixed.keyword.value == TypePrefixKind::Mut && matches!(prefixed.target.value, Expr::Select(_)) =>
        {
            (Form, "a `mut` borrow of an element (an element address handed out is not built; write with `a.[i] :: v`)".to_owned())
        }
        Expr::Call { .. } => (Form, "a call".to_owned()),
        Expr::Member { .. } => (Form, "a member access".to_owned()),
        Expr::Match(_) | Expr::Conditional(_) | Expr::Chain(_) | Expr::IsTest(_) => (Form, "a conditional form".to_owned()),
        Expr::Query { .. } => (Form, "a compile-time query".to_owned()),
        Expr::Hole => (Form, "a `#todo` hole outside the returned value".to_owned()),
        Expr::Literal(_) => (Form, "a literal".to_owned()),
        Expr::Name(name) => (Form, format!("the name `{name}`")),
        _ => (Form, "an expression form".to_owned()),
    }
}
