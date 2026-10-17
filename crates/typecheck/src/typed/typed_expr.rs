use super::*;

use soa_derive::StructOfArray;

#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct ConstFacts {
    pub const_value: Option<ast::ConstValue>,
    pub const_object: Option<crate::ConstObjectId>,
    pub sequence_literal: Option<Box<TypedSequenceLiteral>>,
    pub literal_materialization: Option<Box<LiteralMaterializationPlan>>,
}

#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct SemanticFacts {
    pub integer_knowledge: Option<ast::integer::IntegerKnowledge>,
    pub result_evidence: Option<Box<ValueEvidence>>,
    pub projected_result_evidence: ProjectedResultEvidence,
    pub availability: Availability,
    pub flaws: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct OwnershipFacts {
    pub references: Option<Box<ReferenceValue>>,
    pub reference_reads: HashSet<ReferenceLoan>,
    pub target_group: Option<ReferenceTargetSet>,
    pub result_target_groups: Vec<Option<ReferenceTargetSet>>,
    pub place: Option<PlaceId>,
    pub place_version: Option<PlaceVersionId>,
}

#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct ExprFacts {
    #[nested_soa]
    pub const_facts: ConstFacts,
    #[nested_soa]
    pub semantic_facts: SemanticFacts,
    #[nested_soa]
    pub ownership_facts: OwnershipFacts,
}

/// A single expression in the typed AST arena.
///
/// All fields are stored in separate vectors via `soa_derive` for cache-friendly
/// iteration and per-field access.
#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: Ty,
    /// Source location for diagnostics and LSP.
    pub span: SpanId,
    #[nested_soa]
    pub facts: ExprFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedSequenceLiteral {
    Elements(Vec<ExprId>),
    Repeat { seed: ExprId, count: NormalExpr },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiteralMaterializationPlan {
    FixedArray {
        declaration: Option<ast::ty::TypeId>,
        element: Ty,
        length: NormalExpr,
    },
    OwnedSequence {
        declaration: ast::ty::TypeId,
        element: Ty,
        empty: DefId,
        push: DefId,
    },
}

impl TypedExprVec {
    pub(crate) fn push_ownership_facts(
        &mut self,
        target_group: Option<ReferenceTargetSet>,
        result_target_groups: Vec<Option<ReferenceTargetSet>>,
        place: Option<PlaceId>,
        place_version: Option<PlaceVersionId>,
    ) {
        self.facts.ownership_facts.references.push(None);
        self.facts
            .ownership_facts
            .reference_reads
            .push(HashSet::new());
        self.facts.ownership_facts.target_group.push(target_group);
        self.facts
            .ownership_facts
            .result_target_groups
            .push(result_target_groups);
        self.facts.ownership_facts.place.push(place);
        self.facts.ownership_facts.place_version.push(place_version);
    }

    /// Append a row for `kind` with placeholder facts, returning its id.
    pub(crate) fn push_placeholder(&mut self, kind: TypedExprKind, ty: Ty, span: SpanId) -> ExprId {
        let id = ExprId(self.kind.len() as u32);
        self.kind.push(kind);
        self.ty.push(ty);
        self.span.push(span);
        self.facts.const_facts.const_value.push(None);
        self.facts.const_facts.const_object.push(None);
        self.facts.const_facts.sequence_literal.push(None);
        self.facts.const_facts.literal_materialization.push(None);
        self.facts.semantic_facts.integer_knowledge.push(None);
        self.facts.semantic_facts.result_evidence.push(None);
        self.facts
            .semantic_facts
            .projected_result_evidence
            .push(HashMap::new());
        self.facts
            .semantic_facts
            .availability
            .push(Availability::Unknown);
        self.push_ownership_facts(None, Vec::new(), None, None);
        self.facts.semantic_facts.flaws.push(Vec::new());
        id
    }

    pub(crate) fn set_ownership_facts(
        &mut self,
        index: usize,
        target_group: Option<ReferenceTargetSet>,
        result_target_groups: Vec<Option<ReferenceTargetSet>>,
        place: Option<PlaceId>,
        place_version: Option<PlaceVersionId>,
    ) {
        self.facts.ownership_facts.target_group[index] = target_group;
        self.facts.ownership_facts.result_target_groups[index] = result_target_groups;
        self.facts.ownership_facts.place[index] = place;
        self.facts.ownership_facts.place_version[index] = place_version;
    }

    #[inline]
    pub fn kind_of(&self, expr_id: ExprId) -> Option<&TypedExprKind> {
        self.kind.get(expr_id.index())
    }

    #[inline]
    pub fn ty_of(&self, expr_id: ExprId) -> Option<&Ty> {
        self.ty.get(expr_id.index())
    }

    #[inline]
    pub fn span_of(&self, expr_id: ExprId) -> Option<SpanId> {
        self.span.get(expr_id.index()).copied()
    }

    #[inline]
    pub fn const_value_of(&self, expr_id: ExprId) -> Option<&Option<ast::ConstValue>> {
        self.facts.const_facts.const_value.get(expr_id.index())
    }

    #[inline]
    pub fn integer_knowledge_of(
        &self,
        expr_id: ExprId,
    ) -> Option<&Option<ast::integer::IntegerKnowledge>> {
        self.facts
            .semantic_facts
            .integer_knowledge
            .get(expr_id.index())
    }

    #[inline]
    pub fn result_evidence_of(&self, expr_id: ExprId) -> Option<&ResultEvidence> {
        match self.value_evidence_of(expr_id)? {
            ValueEvidence::Alternatives(evidence) => Some(evidence),
            ValueEvidence::Proposition(_) => None,
        }
    }

    pub fn proposition_evidence_of(&self, expr_id: ExprId) -> Option<&EvidenceProposition> {
        match self.value_evidence_of(expr_id)? {
            ValueEvidence::Proposition(evidence) => Some(evidence),
            ValueEvidence::Alternatives(_) => None,
        }
    }

    pub fn value_evidence_of(&self, expr_id: ExprId) -> Option<&ValueEvidence> {
        self.facts
            .semantic_facts
            .result_evidence
            .get(expr_id.index())
            .and_then(|evidence| evidence.as_deref())
    }

    #[inline]
    pub fn projected_result_evidence_of(
        &self,
        expr_id: ExprId,
    ) -> Option<&ProjectedResultEvidence> {
        self.facts
            .semantic_facts
            .projected_result_evidence
            .get(expr_id.index())
    }

    #[inline]
    pub fn availability_of(&self, expr_id: ExprId) -> Option<&Availability> {
        self.facts.semantic_facts.availability.get(expr_id.index())
    }

    #[inline]
    pub fn references_of(&self, index: usize) -> &ReferenceValue {
        let value = match self
            .facts
            .ownership_facts
            .references
            .get(index)
            .and_then(|value| value.as_deref())
        {
            Some(value) => value,
            None => ReferenceValue::empty(),
        };
        crate::transform::semantic_reads::reference(ExprId(index as u32), value);
        value
    }

    #[inline]
    pub fn references_of_mut(&mut self, index: usize) -> &mut ReferenceValue {
        self.facts
            .ownership_facts
            .references
            .get_mut(index)
            .expect("expression index within arena")
            .get_or_insert_with(|| Box::new(ReferenceValue::default()))
            .as_mut()
    }

    #[inline]
    pub fn target_group_of(&self, expr_id: ExprId) -> Option<&Option<ReferenceTargetSet>> {
        self.facts.ownership_facts.target_group.get(expr_id.index())
    }

    #[inline]
    pub fn result_target_groups_of(
        &self,
        expr_id: ExprId,
    ) -> Option<&Vec<Option<ReferenceTargetSet>>> {
        self.facts
            .ownership_facts
            .result_target_groups
            .get(expr_id.index())
    }

    #[inline]
    pub fn place_of(&self, expr_id: ExprId) -> Option<Option<PlaceId>> {
        self.facts
            .ownership_facts
            .place
            .get(expr_id.index())
            .copied()
    }

    #[inline]
    pub fn place_version_of(&self, expr_id: ExprId) -> Option<Option<PlaceVersionId>> {
        self.facts
            .ownership_facts
            .place_version
            .get(expr_id.index())
            .copied()
    }

    pub fn place_entries(&self) -> impl Iterator<Item = (ExprId, Option<PlaceId>)> + '_ {
        self.facts
            .ownership_facts
            .place
            .iter()
            .copied()
            .enumerate()
            .map(|(index, place)| (ExprId(index as u32), place))
    }
}

/// Typed expression variant — post-resolution form of parse-time [`Expr`].
///
/// Key differences from parse-time `Expr`:
/// - `TypeNominal`, `TypeQualified`, `TypeGeneric` — removed (desugared to `Ty`)
/// - `AnonymousTag` — removed (merged into `TagCall` with `args: None`)
/// - `FnCall` — uses `DefId` instead of path
/// - `TagCall` — uses `VariantId` + discriminant
/// - `Cast` — `ty` is `Ty` not `Intern<String>`
/// - All `Box<Typed<Expr>>` → `ExprId`
/// - All `Vec<Typed<Expr>>` → `Vec<ExprId>`
///
/// Typed when-expression — like `WhenExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedWhenExpr {
    /// Subject expression for pattern matching (`None` for condition-based when).
    pub subject: Option<ExprId>,
    pub arms: Vec<TypedWhenArm>,
    pub place_joins: Vec<PlaceVersionId>,
    /// Covers from after the `when` keyword to end.
    /// The full expression span is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedWhenArm {
    Cond {
        condition: TypedCondition,
        coverage: Vec<ast::Pattern>,
        body: ExprId,
        owned_bindings: Vec<PlaceId>,
        /// Span of this arm (condition and body).
        arm_span: SubSpan,
    },
    Is {
        pattern: Box<ast::span::Spanned<ast::Pattern>>,
        body: ExprId,
        owned_bindings: Vec<PlaceId>,
        /// Span of this is-arm (pattern and body).
        arm_span: SubSpan,
    },
    Else(ExprId, SubSpan),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedCondition {
    Proposition {
        subject: ExprId,
    },
    Compare {
        left: ExprId,
        relation: ast::ProofRelation,
        right: ExprId,
    },
    Is {
        subject: ExprId,
        pattern: Box<ast::span::Spanned<ast::Pattern>>,
    },
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}

impl TypedCondition {
    fn walk<'s>(&'s self, visit: &mut impl FnMut(&'s Self)) {
        let mut pending = vec![self];
        while let Some(condition) = pending.pop() {
            visit(condition);
            match condition {
                Self::Proposition { .. } | Self::Compare { .. } | Self::Is { .. } => {}
                Self::Not(inner) => pending.push(inner),
                Self::And(left, right) | Self::Or(left, right) => {
                    pending.push(right);
                    pending.push(left);
                }
            }
        }
    }

    pub fn visit_subjects(&self, visit: &mut impl FnMut(ExprId)) {
        self.walk(&mut |condition| match condition {
            Self::Compare { left, right, .. } => {
                visit(*left);
                visit(*right);
            }
            Self::Proposition { subject } | Self::Is { subject, .. } => visit(*subject),
            Self::Not(_) | Self::And(..) | Self::Or(..) => {}
        });
    }

    pub fn pattern_subjects<'a>(
        &'a self,
        output: &mut Vec<(ExprId, &'a ast::span::Spanned<ast::Pattern>)>,
    ) {
        self.walk(&mut |condition| {
            if let Self::Is { subject, pattern } = condition {
                output.push((*subject, pattern.as_ref()));
            }
        });
    }
}

/// Typed if-expression — like `IfExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedIfExpr {
    pub condition: TypedCondition,
    pub stmts: Vec<ExprId>,
    pub ret: Option<ExprId>,
    pub postcondition: Option<TypedPostcondition>,
    pub owned_bindings: Vec<PlaceId>,
    pub place_joins: Vec<PlaceVersionId>,
    /// Covers from condition start to end (excludes the `if` keyword).
    /// The full expression span (including `if`) is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
}

/// Typed loop — like `Loop` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedLoop {
    pub kind: TypedLoopKind,
    pub place_phis: Vec<PlaceVersionId>,
    /// Span of the `for` or `while` keyword only.
    /// The full expression span is on the `TypedExpr` arena entry.
    pub keyword_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedLoopKind {
    While {
        condition: TypedCondition,
        stmts: Vec<ExprId>,
        owned_bindings: Vec<PlaceId>,
    },
    ForIn {
        iterable: ExprId,
        traversal: SequenceTraversal,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SequenceTraversal {
    IntegerRange {
        variable: Intern<String>,
        start: ExprId,
        end: ExprId,
        stmts: Vec<ExprId>,
    },
    RuntimeView {
        index: Intern<String>,
        length: ExprId,
        stmts: Vec<ExprId>,
    },
    Fixed {
        iterations: Vec<Vec<ExprId>>,
    },
    Unresolved,
}

impl TypedLoop {
    pub fn statement_batches(&self) -> impl DoubleEndedIterator<Item = &[ExprId]> {
        let (repeated, fixed) = match &self.kind {
            TypedLoopKind::While { stmts, .. }
            | TypedLoopKind::ForIn {
                traversal:
                    SequenceTraversal::IntegerRange { stmts, .. }
                    | SequenceTraversal::RuntimeView { stmts, .. },
                ..
            } => (Some(stmts.as_slice()), &[][..]),
            TypedLoopKind::ForIn {
                traversal: SequenceTraversal::Fixed { iterations },
                ..
            } => (None, iterations.as_slice()),
            TypedLoopKind::ForIn {
                traversal: SequenceTraversal::Unresolved,
                ..
            } => (None, &[][..]),
        };
        repeated.into_iter().chain(fixed.iter().map(Vec::as_slice))
    }

    pub fn statements(&self) -> impl Iterator<Item = &ExprId> {
        self.statement_batches().flatten()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExprKind {
    Block {
        statements: Vec<ExprId>,
        value: Option<ExprId>,
    },
    Return {
        value: Option<ExprId>,
        postcondition: Option<Box<TypedPostcondition>>,
    },
    Invalid,
    Todo,
    Emit {
        specification: ExprId,
        operands: Vec<(Intern<String>, ExprId)>,
        record: ExprId,
        state: crate::assembly::AssemblyState,
    },
    Lit(Literal),
    Domain(Box<ast::integer::IntegerDomain>),
    Binary {
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    InvalidOperator {
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        error: crate::operator::OperatorResolutionError,
    },
    FnCall {
        target: DefId,
        args: Option<Vec<ExprId>>,
        /// When the target has const-generic params, the return type after
        /// substituting type/const arguments from the call site.
        substituted_ty: Option<Ty>,
    },
    IntrinsicCall {
        op: IntrinsicOp,
        args: Vec<ExprId>,
    },
    TagCall {
        variant_id: VariantId,
        discriminant: usize,
        args: Option<Vec<ExprId>>,
        /// Original field names from the AST TagCall args (for named/positional
        /// args in shape literals). Empty for bare anonymous tags.
        field_names: Vec<Intern<String>>,
    },
    Bind {
        name: Intern<String>,
        stmts: Vec<ExprId>,
        body: ExprId,
        /// If true, this bind was declared with a type but no value (`name Type`).
        /// The variable starts in `Declared` state and must be assigned before use.
        unassigned: bool,
    },
    /// Reassign a value to a previously-declared variable.
    Reassign {
        name: Intern<String>,
        value: ExprId,
        operator: Option<BinOp>,
    },
    When(TypedWhenExpr),
    If(Box<TypedIfExpr>),
    Loop(TypedLoop),
    SelfRef {
        target: DefId,
    },
    Range {
        start: ExprId,
        end: ExprId,
    },
    /// Semantic construction of a record's positional storage fields.
    RecordValue(Vec<ExprId>),
    /// Semantic construction of a named callable result row.
    ResultRow(Vec<ExprId>),
    ArrayLit(Vec<ExprId>),
    List(Vec<ExprId>),

    UnionPayload {
        expr: ExprId,
        variant: usize,
    },
    Cast {
        expr: ExprId,
        ty: Ty,
    },
    TargetQuery {
        kind: ast::TargetQueryKind,
        operand: Ty,
    },
    Target,
    TargetField(Intern<String>),
    Rematerialize {
        source: ExprId,
        constant: ConstValue,
    },

    FixedArrayAlloc {
        init: ExprId,
        size: NormalExpr,
    },
    AggregateGet {
        base: ExprId,
        index: usize,
    },
    AggregateSet {
        base: ExprId,
        index: usize,
        value: ExprId,
        operator: Option<ResolvedCompoundOperator>,
    },
    /// Destructure bind: `Tag(field: bind, …) := expr`
    Destructure {
        tag_name: Intern<String>,
        value: ExprId,
        field_bindings: Vec<(Intern<String>, Intern<String>)>,
        places: Vec<PlaceId>,
    },

    /// Record field write: `base.field:: value`
    RecordSet {
        base: ExprId,
        field: Intern<String>,
        value: ExprId,
        operator: Option<ResolvedCompoundOperator>,
    },
    SliceGet {
        buf: ExprId,
        start: ExprId,
        length: ExprId,
    },
    BufGet {
        buf: ExprId,
        index: ExprId,
    },
    BufSet {
        buf: ExprId,
        index: ExprId,
        value: ExprId,
        operator: Option<ResolvedCompoundOperator>,
    },
    TakePtr(ExprId),
    /// A safe reference: `ref expr` or `mut expr`.
    Ref(ExprId),
    Deref(ExprId),

    Negate(ExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedCompoundOperator {
    Callable { target: DefId, return_ty: Ty },
    Intrinsic(IntrinsicOp),
    Invalid(crate::operator::OperatorResolutionError),
}

pub fn typed_walk_expr_children(
    kind: &TypedExprKind,
    visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
) -> std::ops::ControlFlow<()> {
    use std::ops::ControlFlow::Continue;

    match kind {
        TypedExprKind::Block { statements, value } => {
            for child in statements {
                visit(*child)?;
            }
            if let Some(child) = value {
                visit(*child)?;
            }
        }
        TypedExprKind::Return { value, .. } => {
            if let Some(child) = value {
                visit(*child)?;
            }
        }
        TypedExprKind::Invalid | TypedExprKind::Todo => {}
        TypedExprKind::Emit {
            specification,
            record,
            ..
        } => {
            visit(*specification)?;
            visit(*record)?;
        }
        TypedExprKind::Binary { lhs, rhs, .. }
        | TypedExprKind::InvalidOperator { lhs, rhs, .. } => {
            visit(*lhs)?;
            visit(*rhs)?;
        }
        TypedExprKind::IntrinsicCall { args, .. } => {
            for child in args {
                visit(*child)?;
            }
        }
        TypedExprKind::FnCall { args, .. } | TypedExprKind::TagCall { args, .. } => {
            if let Some(args) = args {
                for child in args {
                    visit(*child)?;
                }
            }
        }
        TypedExprKind::Bind {
            stmts,
            body,
            unassigned,
            ..
        } => {
            for child in stmts {
                visit(*child)?;
            }
            if !unassigned {
                visit(*body)?;
            }
        }
        TypedExprKind::Reassign { value, .. } | TypedExprKind::Destructure { value, .. } => {
            visit(*value)?
        }
        TypedExprKind::When(when_expr) => {
            if let Some(subject) = when_expr.subject {
                visit(subject)?;
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        let mut failure = None;
                        condition.visit_subjects(&mut |subject| {
                            if failure.is_none() {
                                failure = visit(subject).break_value();
                            }
                        });
                        if failure.is_some() {
                            return ControlFlow::Break(());
                        }
                        visit(*body)?;
                    }
                    TypedWhenArm::Is { body, .. } | TypedWhenArm::Else(body, _) => visit(*body)?,
                }
            }
        }
        TypedExprKind::If(if_expr) => {
            let mut failure = None;
            if_expr.condition.visit_subjects(&mut |subject| {
                if failure.is_none() {
                    failure = visit(subject).break_value();
                }
            });
            if failure.is_some() {
                return ControlFlow::Break(());
            }
            for child in &if_expr.stmts {
                visit(*child)?;
            }
            if let Some(child) = if_expr.ret {
                visit(child)?;
            }
        }
        TypedExprKind::Loop(loop_expr) => {
            match &loop_expr.kind {
                TypedLoopKind::While { condition, .. } => {
                    let mut failure = None;
                    condition.visit_subjects(&mut |subject| {
                        if failure.is_none() {
                            failure = visit(subject).break_value();
                        }
                    });
                    if failure.is_some() {
                        return ControlFlow::Break(());
                    }
                }
                TypedLoopKind::ForIn { iterable, .. } => visit(*iterable)?,
            }
            for child in loop_expr.statements() {
                visit(*child)?;
            }
        }
        TypedExprKind::Range { start, end } => {
            visit(*start)?;
            visit(*end)?;
        }
        TypedExprKind::RecordValue(children)
        | TypedExprKind::ResultRow(children)
        | TypedExprKind::ArrayLit(children)
        | TypedExprKind::List(children) => {
            for child in children {
                visit(*child)?;
            }
        }
        TypedExprKind::Cast { expr, .. }
        | TypedExprKind::UnionPayload { expr, .. }
        | TypedExprKind::Rematerialize { source: expr, .. }
        | TypedExprKind::FixedArrayAlloc { init: expr, .. }
        | TypedExprKind::AggregateGet { base: expr, .. }
        | TypedExprKind::TakePtr(expr)
        | TypedExprKind::Ref(expr)
        | TypedExprKind::Deref(expr)
        | TypedExprKind::Negate(expr) => visit(*expr)?,
        TypedExprKind::AggregateSet { base, value, .. }
        | TypedExprKind::RecordSet { base, value, .. } => {
            visit(*base)?;
            visit(*value)?;
        }
        TypedExprKind::SliceGet { buf, start, length } => {
            visit(*buf)?;
            visit(*start)?;
            visit(*length)?;
        }
        TypedExprKind::BufGet { buf, index } => {
            visit(*buf)?;
            visit(*index)?;
        }
        TypedExprKind::BufSet {
            buf, index, value, ..
        } => {
            visit(*buf)?;
            visit(*index)?;
            visit(*value)?;
        }
        TypedExprKind::Domain(_)
        | TypedExprKind::Lit(_)
        | TypedExprKind::SelfRef { .. }
        | TypedExprKind::TargetQuery { .. }
        | TypedExprKind::Target
        | TypedExprKind::TargetField(_) => {}
    }
    Continue(())
}

pub fn typed_walk_expr_children_of(
    typed: &TypedFileAst,
    expr_id: ExprId,
    visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
) -> std::ops::ControlFlow<()> {
    let Some(kind) = typed.exprs.kind_of(expr_id) else {
        return std::ops::ControlFlow::Continue(());
    };
    typed_walk_expr_children(kind, visit)
}

pub fn typed_walk_expr_preorder(
    typed: &TypedFileAst,
    root: ExprId,
    visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
) -> std::ops::ControlFlow<()> {
    let mut pending = vec![root];
    let mut visited = std::collections::HashSet::new();
    while let Some(expr_id) = pending.pop() {
        if !visited.insert(expr_id) {
            continue;
        }
        let Some(kind) = typed.exprs.kind.get(expr_id.as_usize()) else {
            continue;
        };
        visit(expr_id)?;
        let start = pending.len();
        typed_walk_expr_children(kind, &mut |child| {
            pending.push(child);
            std::ops::ControlFlow::Continue(())
        })?;
        pending[start..].reverse();
    }
    std::ops::ControlFlow::Continue(())
}

#[cfg(test)]
mod condition_traversal_tests {
    use super::*;
    use parser::query::SourceParseExt;

    fn check_on_small_stack(patterns: bool, test: &str) {
        const CHILD: &str = "GIN_TYPED_CONDITION_SMALL_STACK";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test, "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let parsed =
            "choose(flag is in 0 to 1):\n    when flag is 1 then\n        return 1\n    return 0\n"
                .parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = crate::transform::transform_file(parsed.ast, FileId(0));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let leaf = typed
            .exprs
            .kind
            .iter()
            .find_map(|kind| match kind {
                TypedExprKind::When(branch) => match &branch.arms[0] {
                    TypedWhenArm::Cond { condition, .. } => Some(condition.clone()),
                    TypedWhenArm::Is { pattern, .. } => Some(TypedCondition::Is {
                        subject: branch.subject.unwrap(),
                        pattern: pattern.clone(),
                    }),
                    _ => None,
                },
                _ => None,
            })
            .expect("condition fixture");
        let leaf = |index| {
            let mut leaf = leaf.clone();
            let TypedCondition::Is { subject, .. } = &mut leaf else {
                panic!("leaf condition")
            };
            *subject = ExprId(index);
            leaf
        };
        let count = 2000;
        let mut condition = leaf(0);
        for index in 1..count {
            let right = TypedCondition::Not(Box::new(leaf(index)));
            condition = if index % 2 == 0 {
                TypedCondition::And(Box::new(condition), Box::new(right))
            } else {
                TypedCondition::Or(Box::new(condition), Box::new(right))
            };
        }
        let subjects = std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn_scoped(scope, || {
                    if patterns {
                        let mut pairs = Vec::new();
                        condition.pattern_subjects(&mut pairs);
                        pairs
                            .into_iter()
                            .map(|(subject, _)| subject)
                            .collect::<Vec<_>>()
                    } else {
                        let mut subjects = Vec::new();
                        condition.visit_subjects(&mut |subject| subjects.push(subject));
                        subjects
                    }
                })
                .unwrap()
                .join()
                .unwrap()
        });
        assert_eq!(subjects, (0..count).map(ExprId).collect::<Vec<_>>());
    }

    #[test]
    fn deep_subject_visits_preserve_order_on_a_small_stack() {
        check_on_small_stack(
            false,
            "typed::typed_expr::condition_traversal_tests::deep_subject_visits_preserve_order_on_a_small_stack",
        );
    }

    #[test]
    fn deep_pattern_visits_preserve_order_on_a_small_stack() {
        check_on_small_stack(
            true,
            "typed::typed_expr::condition_traversal_tests::deep_pattern_visits_preserve_order_on_a_small_stack",
        );
    }
}
