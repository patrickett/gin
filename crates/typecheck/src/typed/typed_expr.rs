use super::*;

use soa_derive::StructOfArray;

#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct ConstFacts {
    pub const_value: Option<ast::ConstValue>,
    pub const_object: Option<crate::ConstObjectId>,
    pub sequence_literal: Option<TypedSequenceLiteral>,
    pub literal_materialization: Option<LiteralMaterializationPlan>,
}

#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct SemanticFacts {
    pub sequence_provenance: Option<SequenceProvenance>,
    pub integer_knowledge: Option<ast::integer::IntegerKnowledge>,
    pub result_evidence: Option<ResultEvidence>,
    pub projected_result_evidence: ProjectedResultEvidence,
    pub availability: Availability,
    pub flaws: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct OwnershipFacts {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceProvenance {
    Static(crate::ConstObjectId),
    Scoped,
}

impl SequenceProvenance {
    pub fn is_static(self) -> bool {
        matches!(self, Self::Static(_))
    }

    pub fn is_scoped(self) -> bool {
        matches!(self, Self::Scoped)
    }

    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Static(left), Self::Static(right)) if left == right => Self::Static(left),
            _ => Self::Scoped,
        }
    }
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
    BorrowedSequence {
        declaration: ast::ty::TypeId,
        element: Ty,
        backing: Option<crate::ConstObjectId>,
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
        self.facts.ownership_facts.target_group.push(target_group);
        self.facts
            .ownership_facts
            .result_target_groups
            .push(result_target_groups);
        self.facts.ownership_facts.place.push(place);
        self.facts.ownership_facts.place_version.push(place_version);
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
    pub fn result_evidence_of(&self, expr_id: ExprId) -> Option<&Option<ResultEvidence>> {
        self.facts
            .semantic_facts
            .result_evidence
            .get(expr_id.index())
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
    pub fn flaws_of(&self, expr_id: ExprId) -> Option<&Vec<Diagnostic>> {
        self.facts.semantic_facts.flaws.get(expr_id.index())
    }

    #[inline]
    pub fn availability_of(&self, expr_id: ExprId) -> Option<&Availability> {
        self.facts.semantic_facts.availability.get(expr_id.index())
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
        body: ExprId,
        /// Span of this arm (condition and body).
        arm_span: SubSpan,
    },
    Is {
        pattern: Box<ast::span::Spanned<ast::Pattern>>,
        body: ExprId,
        /// Span of this is-arm (pattern and body).
        arm_span: SubSpan,
    },
    Else(ExprId, SubSpan),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedCondition {
    Is {
        subject: ExprId,
        pattern: Box<ast::span::Spanned<ast::Pattern>>,
    },
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}

impl TypedCondition {
    pub fn visit_subjects(&self, visit: &mut impl FnMut(ExprId)) {
        match self {
            Self::Is { subject, .. } => visit(*subject),
            Self::Not(inner) => inner.visit_subjects(visit),
            Self::And(left, right) | Self::Or(left, right) => {
                left.visit_subjects(visit);
                right.visit_subjects(visit);
            }
        }
    }

    pub fn pattern_subjects<'a>(
        &'a self,
        output: &mut Vec<(ExprId, &'a ast::span::Spanned<ast::Pattern>)>,
    ) {
        match self {
            Self::Is { subject, pattern } => output.push((*subject, pattern)),
            Self::Not(inner) => inner.pattern_subjects(output),
            Self::And(left, right) | Self::Or(left, right) => {
                left.pattern_subjects(output);
                right.pattern_subjects(output);
            }
        }
    }
}

/// Typed if-expression — like `IfExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedIfExpr {
    pub condition: TypedCondition,
    pub stmts: Vec<ExprId>,
    pub ret: Option<ExprId>,
    pub place_joins: Vec<PlaceVersionId>,
    /// Covers from condition start to end (excludes the `if` keyword).
    /// The full expression span (including `if`) is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
}

/// Typed loop — like `Loop` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedLoop {
    pub kind: TypedLoopKind,
    pub stmts: Vec<ExprId>,
    pub place_phis: Vec<PlaceVersionId>,
    /// Span of the `loop` keyword only.
    /// The full expression span is on the `TypedExpr` arena entry.
    pub keyword_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedLoopKind {
    While {
        condition: TypedCondition,
    },
    ForIn {
        variable: Intern<String>,
        iterable: ExprId,
        plan: IterationPlan,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum IterationPlan {
    Range,
    FixedArray {
        element: Ty,
        length: ast::NormalExpr,
    },
    Unresolved,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExprKind {
    Invalid,
    Asm {
        specification: ExprId,
        operands: Vec<(Intern<String>, ExprId)>,
        state: crate::assembly::AssemblyState,
    },
    Register(ExprId),
    Lit(Literal),
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
    If(TypedIfExpr),
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
    },

    /// Record field write: `base.field:: value`
    RecordSet {
        base: ExprId,
        field: Intern<String>,
        value: ExprId,
        operator: Option<ResolvedCompoundOperator>,
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
        TypedExprKind::Register(inner) => {
            visit(*inner)?;
        }
        TypedExprKind::Invalid => {}
        TypedExprKind::Asm {
            specification,
            operands,
            ..
        } => {
            visit(*specification)?;
            for (_, operand) in operands {
                visit(*operand)?;
            }
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
            for child in &loop_expr.stmts {
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
        TypedExprKind::Lit(_)
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
    fn walk(
        typed: &TypedFileAst,
        expr_id: ExprId,
        visited: &mut std::collections::HashSet<ExprId>,
        visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
    ) -> std::ops::ControlFlow<()> {
        if !visited.insert(expr_id) {
            return std::ops::ControlFlow::Continue(());
        }
        let Some(kind) = typed.exprs.kind.get(expr_id.as_usize()) else {
            return std::ops::ControlFlow::Continue(());
        };
        visit(expr_id)?;
        typed_walk_expr_children(kind, &mut |child| walk(typed, child, visited, visit))
    }

    walk(typed, root, &mut std::collections::HashSet::new(), visit)
}
