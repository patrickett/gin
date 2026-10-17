use super::*;
use crate::typed::{PlaceProjection, TargetIndex};
use diagnostic::code::{SequenceCode, TerminalCode};

fn ownership_path(typed: &TypedFileAst, mut place: PlaceId) -> (ReferenceTargetGroup, usize) {
    let mut projections = Vec::new();
    for _ in 0..typed.places.len() {
        if !typed.owns_projection(place) {
            break;
        }
        let info = &typed.places[place.0 as usize];
        projections.push(
            info.projection
                .clone()
                .expect("owned projection has a path"),
        );
        place = info.parent.expect("owned projection has a parent");
    }
    assert!(!typed.owns_projection(place));
    let depth = projections.len();
    let mut target = ReferenceTargetGroup::Local(place);
    for projection in projections.into_iter().rev() {
        target = match projection {
            PlaceProjection::Field(index) => ReferenceTargetGroup::Field {
                base: Box::new(target),
                index,
            },
            PlaceProjection::Item(index) => ReferenceTargetGroup::ItemRegion {
                base: Box::new(target),
                index: stable_index(typed, index),
            },
            PlaceProjection::Deref => unreachable!("dereferences do not own their referents"),
        };
    }
    (target, depth)
}

fn stable_index(typed: &TypedFileAst, index: TargetIndex) -> TargetIndex {
    let TargetIndex::Symbolic(expression) = &index else {
        return index;
    };
    let mut names = HashSet::new();
    super::super::scalar_mutations::variables(expression, &mut names);
    for name in names {
        let mut stable = false;
        for place in typed.places.iter().filter(|place| place.name == name) {
            if place.mutable || is_reference_type(typed, &place.ty) {
                return TargetIndex::Unknown;
            }
            stable |= place.parent.is_none();
        }
        if !stable {
            return TargetIndex::Unknown;
        }
    }
    index
}

pub(super) fn ownership_state(
    typed: &TypedFileAst,
    flow: &FlowContext,
    place: PlaceId,
    whole: bool,
) -> Option<VarState> {
    let mut ancestor = place;
    for _ in 0..typed.places.len() {
        if let Some(state @ (VarState::Consumed | VarState::MaybeConsumed)) =
            flow.place_state(ancestor)
        {
            return Some(state);
        }
        if !typed.owns_projection(ancestor) {
            break;
        }
        ancestor = typed.places[ancestor.0 as usize].parent.unwrap();
    }
    let (target, depth) = ownership_path(typed, place);
    let contents = whole && !is_reference_type(typed, &typed.places[place.0 as usize].ty);
    if depth == 0 && !contents {
        return flow.place_state(place);
    }
    for index in 0..typed.places.len() {
        let consumed = PlaceId(index as u32);
        let Some(state @ (VarState::Consumed | VarState::MaybeConsumed)) =
            flow.place_state(consumed)
        else {
            continue;
        };
        let (source, source_depth) = ownership_path(typed, consumed);
        if !contents && source_depth > depth {
            continue;
        }
        match target.overlap(&source, &flow.constraint_env) {
            Overlap::Equal => return Some(state),
            Overlap::Partial | Overlap::Unknown => return Some(VarState::MaybeConsumed),
            Overlap::Disjoint => {}
        }
    }
    flow.place_state(place)
}

impl WalkExprCtx<'_> {
    pub(super) fn initialize_temporary_owner(&mut self, expression: ExprId, place: PlaceId) {
        assert_eq!(self.typed.temporary_owner(expression), Some(place));
        assert!(self.typed.places[place.0 as usize].parent.is_none());
        if matches!(
            self.flow_ctx.place_state(place),
            Some(VarState::Alive | VarState::MaybeConsumed)
        ) && self
            .pending_terminal_obligation(&self.typed.places[place.0 as usize].ty, Some(place))
        {
            // type-terminal-value-not-consumed
            let diagnostic = Diagnostic::new(
                TerminalCode::ValueNotConsumed,
                "evaluating this temporary again would discard pending ownership",
            )
            .at_span_id(
                self.typed.exprs.span[expression.index()],
                &self.typed.span_table,
            );
            self.typed.exprs.facts.semantic_facts.flaws[expression.index()].push(diagnostic);
        }
        let mut descendants = HashSet::from([place]);
        self.flow_ctx.set_place_state(place, VarState::Alive);
        for (index, info) in self
            .typed
            .places
            .iter()
            .enumerate()
            .skip(place.0 as usize + 1)
        {
            if info
                .parent
                .is_some_and(|parent| descendants.contains(&parent))
            {
                let child = PlaceId(index as u32);
                descendants.insert(child);
                self.flow_ctx.set_place_state(child, VarState::Alive);
            }
        }
    }

    fn repeat_seed_is_copyable(&self, seed: ExprId) -> bool {
        let ty = &self.typed.exprs.ty[seed.index()];
        if is_reference_type(self.typed, ty)
            || (!self.typed.borrowed_value_needs_ownership(ty)
                && self
                    .typed
                    .type_registry
                    .resolved_definition_for_type(ty)
                    .is_structurally_discardable())
            || expr_is_rematerializable(self.typed, seed)
        {
            return true;
        }
        let TypedExprKind::IntrinsicCall { op, .. } = &self.typed.exprs.kind[seed.index()] else {
            return false;
        };
        use crate::intrinsic::IntrinsicContract;
        matches!(
            op.contract(),
            IntrinsicContract::SameWidthBinary
                | IntrinsicContract::Extension
                | IntrinsicContract::Truncation
                | IntrinsicContract::Comparison
        )
    }

    pub(super) fn walk_repeated_array(&mut self, site: ExprId, seed: ExprId, count: &NormalExpr) {
        self.walk(seed);
        if count.as_const_int() == Some(0) {
            self.validate_discarded_result(seed);
            return;
        }
        if count.as_const_int() != Some(1) && !self.repeat_seed_is_copyable(seed) {
            // type-sequence-repeat-seed-not-copyable
            let diagnostic = Diagnostic::new(
                SequenceCode::RepeatSeedNotCopyable,
                "repeated array initialization cannot duplicate an owned value",
            )
            .at_span_id(self.typed.exprs.span[site.index()], &self.typed.span_table);
            self.typed.exprs.facts.semantic_facts.flaws[site.index()].push(diagnostic);
        }
        mark_consumed_if_local_ref(self.typed, seed, self.flow_ctx);
    }
}
