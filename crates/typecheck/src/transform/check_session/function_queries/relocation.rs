use std::collections::BTreeSet;
use std::ops::ControlFlow;

use crate::typed::*;

#[derive(Clone, Copy, PartialEq)]
pub(in crate::transform) struct Relocation {
    expression: i64,
    place: i64,
}

pub(super) fn optional<T, U>(
    value: Option<T>,
    map: impl FnOnce(T) -> Option<U>,
) -> Option<Option<U>> {
    match value {
        Some(value) => Some(Some(map(value)?)),
        None => Some(None),
    }
}

impl Relocation {
    pub(super) fn new(expression: i64, place: i64) -> Self {
        Self { expression, place }
    }

    pub(super) fn place_offset(self) -> i64 {
        self.place
    }

    pub fn relative_to(self, other: Self) -> (i64, i64) {
        (self.expression - other.expression, self.place - other.place)
    }
    pub fn for_function(
        file: &TypedFileAst,
        bind: &TypedBind,
    ) -> Option<(Vec<ExprId>, Vec<PlaceId>, Self)> {
        let mut pending = match &bind.body {
            BindBody::Expr(id) => vec![*id],
            BindBody::Body { exprs, ret } => exprs.iter().chain(ret).copied().collect(),
            BindBody::Unlowered => Vec::new(),
        };
        let mut expressions = BTreeSet::new();
        let mut places = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !expressions.insert(id.0) {
                continue;
            }
            if expressions.len() > 8192 {
                return None;
            }
            let kind = file.exprs.kind_of(id)?;
            let mut owned = Vec::new();
            Self::additional_children(kind, &mut pending, &mut owned);
            let _ = walk_expr_children(kind, &mut |child| {
                pending.push(child);
                ControlFlow::Continue(())
            });
            owned.extend(file.exprs.place_of(id).flatten());
            for mut place in owned {
                for _ in 0..=file.places.len() {
                    if !places.insert(place.0) {
                        break;
                    }
                    match file.places.get(place.0 as usize)?.parent {
                        Some(parent) => place = parent,
                        None => break,
                    }
                }
            }
        }
        let relocation = Self {
            expression: i64::from(expressions.first().copied().unwrap_or(0)),
            place: i64::from(places.first().copied().unwrap_or(0)),
        };
        Some((
            expressions.into_iter().map(ExprId).collect(),
            places.into_iter().map(PlaceId).collect(),
            relocation,
        ))
    }

    fn additional_children(
        kind: &TypedExprKind,
        pending: &mut Vec<ExprId>,
        owned: &mut Vec<PlaceId>,
    ) {
        match kind {
            TypedExprKind::Emit { operands, .. } => {
                pending.extend(operands.iter().map(|(_, id)| *id))
            }
            TypedExprKind::Destructure { places, .. } => owned.extend(places),
            TypedExprKind::If(branch) => owned.extend(&branch.owned_bindings),
            TypedExprKind::When(branch) => {
                for arm in &branch.arms {
                    match arm {
                        TypedWhenArm::Cond { owned_bindings, .. }
                        | TypedWhenArm::Is { owned_bindings, .. } => owned.extend(owned_bindings),
                        TypedWhenArm::Else(_, _) => {}
                    }
                }
            }
            TypedExprKind::Loop(branch) => match &branch.kind {
                TypedLoopKind::While { owned_bindings, .. } => owned.extend(owned_bindings),
                TypedLoopKind::ForIn { traversal, .. } => match traversal {
                    SequenceTraversal::IntegerRange { start, end, .. } => {
                        pending.extend([*start, *end])
                    }
                    SequenceTraversal::RuntimeView { length, .. } => pending.push(*length),
                    SequenceTraversal::Fixed { .. } | SequenceTraversal::Unresolved => {}
                },
            },
            _ => {}
        }
    }

    pub fn inverse(self) -> Self {
        Self {
            expression: -self.expression,
            place: -self.place,
        }
    }

    pub fn expression(self, id: ExprId) -> Option<ExprId> {
        Some(ExprId(
            u32::try_from(i64::from(id.0) - self.expression).ok()?,
        ))
    }

    pub fn place(self, id: PlaceId) -> Option<PlaceId> {
        Some(PlaceId(u32::try_from(i64::from(id.0) - self.place).ok()?))
    }

    fn expressions(self, ids: &mut [ExprId]) -> Option<()> {
        for id in ids {
            *id = self.expression(*id)?;
        }
        Some(())
    }

    fn places(self, ids: &mut [PlaceId]) -> Option<()> {
        for id in ids {
            *id = self.place(*id)?;
        }
        Some(())
    }

    fn condition(self, condition: &mut TypedCondition) -> Option<()> {
        let mut pending = vec![condition];
        while let Some(condition) = pending.pop() {
            match condition {
                TypedCondition::Proposition { subject } => *subject = self.expression(*subject)?,
                TypedCondition::Compare { left, right, .. } => {
                    *left = self.expression(*left)?;
                    *right = self.expression(*right)?;
                }
                TypedCondition::Is { subject, pattern } => {
                    *subject = self.expression(*subject)?;
                    pattern.span_id = ast::span::SpanId::INVALID;
                }
                TypedCondition::Not(inner) => pending.push(inner),
                TypedCondition::And(left, right) | TypedCondition::Or(left, right) => {
                    pending.push(left);
                    pending.push(right);
                }
            }
        }
        Some(())
    }

    pub fn body(self, body: &mut BindBody) -> Option<()> {
        match body {
            BindBody::Expr(id) => *id = self.expression(*id)?,
            BindBody::Body { exprs, ret } => {
                self.expressions(exprs)?;
                *ret = optional(*ret, |id| self.expression(id))?;
            }
            BindBody::Unlowered => {}
        }
        Some(())
    }

    pub fn kind(self, kind: &mut TypedExprKind) -> Option<()> {
        match kind {
            TypedExprKind::FnCall { args, .. } | TypedExprKind::TagCall { args, .. } => {
                if let Some(args) = args {
                    self.expressions(args)?;
                }
            }
            TypedExprKind::Bind {
                stmts,
                body,
                unassigned,
                ..
            } => {
                self.expressions(stmts)?;
                *body = if *unassigned {
                    ExprId(0)
                } else {
                    self.expression(*body)?
                };
            }
            TypedExprKind::Emit {
                specification,
                operands,
                record,
                ..
            } => {
                self.expression_slots([specification, record])?;
                for (_, id) in operands {
                    *id = self.expression(*id)?;
                }
            }
            TypedExprKind::Destructure { value, places, .. } => {
                *value = self.expression(*value)?;
                self.places(places)?;
            }
            TypedExprKind::Range { start, end } => self.expression_slots([start, end])?,
            TypedExprKind::If(branch) => self.branch(branch)?,
            TypedExprKind::When(branch) => self.when(branch)?,
            TypedExprKind::Loop(branch) => self.loop_body(branch)?,
            _ => self.operand_kind(kind)?,
        }
        Some(())
    }

    fn operand_kind(self, kind: &mut TypedExprKind) -> Option<()> {
        match kind {
            TypedExprKind::Block { statements, value } => {
                for child in statements {
                    *child = self.expression(*child)?;
                }
                if let Some(child) = value {
                    *child = self.expression(*child)?;
                }
            }
            TypedExprKind::Return { value, .. } => {
                if let Some(child) = value {
                    *child = self.expression(*child)?;
                }
            }
            TypedExprKind::Invalid
            | TypedExprKind::Todo
            | TypedExprKind::Lit(_)
            | TypedExprKind::Domain(_)
            | TypedExprKind::SelfRef { .. }
            | TypedExprKind::TargetQuery { .. }
            | TypedExprKind::Target
            | TypedExprKind::TargetField(_) => {}
            TypedExprKind::Binary {
                lhs: left,
                rhs: right,
                ..
            }
            | TypedExprKind::InvalidOperator {
                lhs: left,
                rhs: right,
                ..
            }
            | TypedExprKind::AggregateSet {
                base: left,
                value: right,
                ..
            }
            | TypedExprKind::RecordSet {
                base: left,
                value: right,
                ..
            }
            | TypedExprKind::BufGet {
                buf: left,
                index: right,
            } => self.expression_slots([left, right])?,
            TypedExprKind::IntrinsicCall { args, .. }
            | TypedExprKind::RecordValue(args)
            | TypedExprKind::ResultRow(args)
            | TypedExprKind::ArrayLit(args)
            | TypedExprKind::List(args) => self.expressions(args)?,
            TypedExprKind::Reassign { value: expr, .. }
            | TypedExprKind::Cast { expr, .. }
            | TypedExprKind::UnionPayload { expr, .. }
            | TypedExprKind::Rematerialize { source: expr, .. }
            | TypedExprKind::FixedArrayAlloc { init: expr, .. }
            | TypedExprKind::AggregateGet { base: expr, .. }
            | TypedExprKind::TakePtr(expr)
            | TypedExprKind::Ref(expr)
            | TypedExprKind::Deref(expr)
            | TypedExprKind::Negate(expr) => *expr = self.expression(*expr)?,
            TypedExprKind::SliceGet {
                buf,
                start: index,
                length: value,
            }
            | TypedExprKind::BufSet {
                buf, index, value, ..
            } => self.expression_slots([buf, index, value])?,
            TypedExprKind::Range { .. }
            | TypedExprKind::FnCall { .. }
            | TypedExprKind::TagCall { .. }
            | TypedExprKind::Bind { .. }
            | TypedExprKind::Emit { .. }
            | TypedExprKind::Destructure { .. }
            | TypedExprKind::If(_)
            | TypedExprKind::When(_)
            | TypedExprKind::Loop(_) => unreachable!(),
        }
        Some(())
    }

    fn expression_slots<const N: usize>(self, ids: [&mut ExprId; N]) -> Option<()> {
        for id in ids {
            *id = self.expression(*id)?;
        }
        Some(())
    }

    fn branch(self, branch: &mut TypedIfExpr) -> Option<()> {
        self.condition(&mut branch.condition)?;
        self.expressions(&mut branch.stmts)?;
        branch.ret = optional(branch.ret, |id| self.expression(id))?;
        self.places(&mut branch.owned_bindings)?;
        branch.place_joins.clear();
        branch.body_span = ast::span::SubSpan::new(ast::span::SpanId::INVALID);
        if let Some(proof) = &mut branch.postcondition {
            proof.span = ast::span::SpanId::INVALID;
        }

        Some(())
    }

    fn when(self, branch: &mut TypedWhenExpr) -> Option<()> {
        branch.subject = optional(branch.subject, |id| self.expression(id))?;
        branch.place_joins.clear();
        branch.body_span = ast::span::SubSpan::new(ast::span::SpanId::INVALID);
        for arm in &mut branch.arms {
            match arm {
                TypedWhenArm::Cond {
                    condition,
                    body,
                    owned_bindings,
                    arm_span,
                    ..
                } => {
                    self.condition(condition)?;
                    *body = self.expression(*body)?;
                    self.places(owned_bindings)?;
                    *arm_span = ast::span::SubSpan::new(ast::span::SpanId::INVALID);
                }
                TypedWhenArm::Is {
                    pattern,
                    body,
                    owned_bindings,
                    arm_span,
                } => {
                    pattern.span_id = ast::span::SpanId::INVALID;
                    *body = self.expression(*body)?;
                    self.places(owned_bindings)?;
                    *arm_span = ast::span::SubSpan::new(ast::span::SpanId::INVALID);
                }
                TypedWhenArm::Else(body, span) => {
                    *body = self.expression(*body)?;
                    *span = ast::span::SubSpan::new(ast::span::SpanId::INVALID);
                }
            }
        }

        Some(())
    }

    fn loop_body(self, branch: &mut TypedLoop) -> Option<()> {
        branch.place_phis.clear();
        branch.keyword_span = ast::span::SubSpan::new(ast::span::SpanId::INVALID);
        match &mut branch.kind {
            TypedLoopKind::While {
                condition,
                stmts,
                owned_bindings,
            } => {
                self.condition(condition)?;
                self.expressions(stmts)?;
                self.places(owned_bindings)?;
            }
            TypedLoopKind::ForIn {
                iterable,
                traversal,
            } => {
                *iterable = self.expression(*iterable)?;
                match traversal {
                    SequenceTraversal::IntegerRange {
                        start, end, stmts, ..
                    } => {
                        *start = self.expression(*start)?;
                        *end = self.expression(*end)?;
                        self.expressions(stmts)?;
                    }
                    SequenceTraversal::RuntimeView { length, stmts, .. } => {
                        *length = self.expression(*length)?;
                        self.expressions(stmts)?;
                    }
                    SequenceTraversal::Fixed { iterations } => {
                        for iteration in iterations {
                            self.expressions(iteration)?;
                        }
                    }
                    SequenceTraversal::Unresolved => {}
                }
            }
        }

        Some(())
    }

    pub fn target(self, target: &mut ReferenceTargetGroup) -> Option<()> {
        let mut current = target;
        loop {
            match current {
                ReferenceTargetGroup::Local(id) => {
                    *id = self.place(*id)?;
                    break;
                }
                ReferenceTargetGroup::Field { base, .. }
                | ReferenceTargetGroup::Variant { base, .. }
                | ReferenceTargetGroup::Deref(base) => current = base,
                ReferenceTargetGroup::ItemRegion { base, index } => {
                    self.index(index)?;
                    current = base;
                }
                ReferenceTargetGroup::ItemRange { base, start, end } => {
                    self.index(start)?;
                    self.index(end)?;
                    current = base;
                }
                ReferenceTargetGroup::Static | ReferenceTargetGroup::Param(_) => break,
            }
        }
        Some(())
    }

    pub fn normal(self, value: &mut ast::NormalExpr) -> Option<()> {
        let mut pending = vec![value];
        while let Some(value) = pending.pop() {
            match value {
                ast::NormalExpr::Var(name) => {
                    if let Some(suffix) = name.as_str().strip_prefix("$call") {
                        let end = suffix.find('.').unwrap_or(suffix.len());
                        if let Ok(id) = suffix[..end].parse::<u32>() {
                            let id = self.expression(ExprId(id))?;
                            *name =
                                internment::Intern::new(format!("$call{}{}", id.0, &suffix[end..]));
                        }
                    }
                }
                ast::NormalExpr::Add(left, right)
                | ast::NormalExpr::Sub(left, right)
                | ast::NormalExpr::Mul(left, right) => {
                    pending.push(left);
                    pending.push(right);
                }
                ast::NormalExpr::Value(_)
                | ast::NormalExpr::Inferred(_)
                | ast::NormalExpr::TargetQuery { .. } => {}
            }
        }
        Some(())
    }

    fn index(self, index: &mut TargetIndex) -> Option<()> {
        if let TargetIndex::Symbolic(value) = index {
            self.normal(value)?;
        }
        Some(())
    }

    pub fn effects(self, effects: &mut FunctionEffects) -> Option<()> {
        for targets in [
            &mut effects.reads,
            &mut effects.writes,
            &mut effects.invalidates,
            &mut effects.consumes,
            &mut effects.raw_exposes,
        ] {
            let mut relocated = std::collections::HashSet::new();
            for mut target in targets.drain() {
                let mut pending = vec![&mut target];
                while let Some(target) = pending.pop() {
                    match target {
                        EffectTarget::Field { base, .. }
                        | EffectTarget::Variant { base, .. }
                        | EffectTarget::Deref(base)
                        | EffectTarget::Descendants(base) => pending.push(base),
                        EffectTarget::ItemRegion { base, index } => {
                            self.index(index)?;
                            pending.push(base);
                        }
                        EffectTarget::ItemRange { base, start, end } => {
                            self.index(start)?;
                            self.index(end)?;
                            pending.push(base);
                        }
                        EffectTarget::Group(_) => {}
                    }
                }
                relocated.insert(target);
            }
            *targets = relocated;
        }
        if let Some(value) = &mut effects.scalar_result {
            self.normal(value)?;
        }
        for field in &mut effects.scalar_fields {
            self.normal(&mut field.value)?;
        }
        Some(())
    }

    pub fn targets(self, targets: &ReferenceTargetSet) -> Option<ReferenceTargetSet> {
        let mut valid = true;
        let output = targets.projected(|target| {
            let mut target = target.clone();
            valid &= self.target(&mut target).is_some();
            target
        });
        valid.then_some(output)
    }

    pub fn loans(self, loans: &mut std::collections::HashSet<ReferenceLoan>) -> Option<()> {
        *loans = loans
            .iter()
            .cloned()
            .map(|mut loan| {
                loan.expression = self.expression(loan.expression)?;
                Some(loan)
            })
            .collect::<Option<_>>()?;
        Some(())
    }

    pub fn reference(self, reference: &mut ReferenceValue) -> Option<()> {
        let mut pending = vec![reference];
        while let Some(value) = pending.pop() {
            value.sources = value
                .sources
                .iter()
                .cloned()
                .map(|mut source| {
                    if let ReferenceSource::Target(target) = &mut source {
                        self.target(target)?;
                    }
                    Some(source)
                })
                .collect::<Option<_>>()?;
            self.loans(&mut value.loans)?;
            pending.extend(value.fields.iter_mut());
            pending.extend(value.array_element.as_deref_mut());
        }
        Some(())
    }

    pub fn summary(self, summary: &mut ReferenceSummary) -> Option<()> {
        self.reference(&mut summary.result)?;
        for value in summary
            .writes
            .values_mut()
            .chain(summary.unknown_slot_writes.values_mut())
        {
            self.reference(value)?;
        }
        Some(())
    }
}
