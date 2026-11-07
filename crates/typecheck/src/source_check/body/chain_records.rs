//! A condition chain as a value graph: a nest of selects, the first arm outermost. Arm `i`'s
//! value is recorded in the `then` region of its select and the rest of the chain in the `else`
//! region, so an arm's instructions run only when its condition is the first that holds.
//!
//! An arm whose condition is one comparison becomes a `Select`; one that is one variant test a
//! `Match` on the subject's member. A condition that combines tests with `and`/`or` is not
//! recorded: its failing side would share the rest of the chain between several selects.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn record_chain(
        &mut self,
        expression: &'a Expression,
        index: usize,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let Expr::Chain(chain) = &expression.value else {
            return None;
        };
        let Some(arm) = chain.arms.get(index) else {
            return self.record_expr(&chain.otherwise, parameter_count, depth + 1, arena, scope);
        };
        let span = expression.span_id;
        if let Some((negated, test)) = super::super::guards::variant_test(&arm.condition) {
            return self.record_chain_variant(
                expression,
                index,
                negated,
                test,
                parameter_count,
                depth,
                arena,
                scope,
            );
        }
        let (connective, leaves) = super::super::guards::conditions(&arm.condition)?;
        if leaves.len() > 1 {
            return self.record_joined_chain_arm(expression, index, connective, &leaves, parameter_count, depth, arena, scope);
        }
        let [leaf] = leaves[..] else {
            return None;
        };
        let test = self.record_leaf(&leaf, parameter_count, depth, arena, scope)?;
        // Each side knows what the test told of its numbers, as the checker did: the arm that it
        // holds, the rest of the chain that it fails (`rhs > 0` then `rhs / ...` fits a register).
        // The narrowed reads are made inside each side's own run of nodes.
        let then_start = arena.len() as u32;
        let mut inside = scope.clone();
        self.narrow_by_test(&leaf, &test, true, parameter_count, arena, &mut inside);
        let then = self.record_expr(&arm.value.value, parameter_count, depth + 1, arena, &mut inside)?;
        let else_start = arena.len() as u32;
        let mut rest = scope.clone();
        self.narrow_by_test(&leaf, &test, false, parameter_count, arena, &mut rest);
        let other =
            self.record_chain(expression, index + 1, parameter_count, depth + 1, arena, &mut rest)?;
        let region = SelectRegion {
            then_start,
            else_start,
            exclusive: true,
        };
        self.select_node(&test, then, other, span, Some(region), arena)
    }

    /// Arm `index` whose condition joins comparisons (`a = x and b = y`): the tests made one
    /// comparison of a count, so the arm and the rest of the chain each own their run of nodes.
    /// With `and` the arm knows every comparison holds; the rest knows nothing more.
    #[allow(clippy::too_many_arguments)]
    fn record_joined_chain_arm(
        &mut self,
        expression: &'a Expression,
        index: usize,
        connective: super::super::guards::Connective,
        leaves: &[super::super::guards::Leaf<'a>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let Expr::Chain(chain) = &expression.value else {
            return None;
        };
        let arm = chain.arms.get(index)?;
        let span = expression.span_id;
        let mut tests = Vec::new();
        for leaf in leaves {
            tests.push(self.record_leaf(leaf, parameter_count, depth, arena, scope)?);
        }
        let narrowing = tests.clone();
        let combined = self.combine_tests(tests, connective, span, arena)?;
        let [test] = &combined[..] else {
            return None;
        };
        let then_start = arena.len() as u32;
        let mut inside = scope.clone();
        if connective == super::super::guards::Connective::And {
            for (leaf, test) in leaves.iter().zip(&narrowing) {
                self.narrow_by_test(leaf, test, true, parameter_count, arena, &mut inside);
            }
        }
        let then = self.record_expr(&arm.value.value, parameter_count, depth + 1, arena, &mut inside)?;
        let else_start = arena.len() as u32;
        let other = self.record_chain(expression, index + 1, parameter_count, depth + 1, arena, scope)?;
        let region = SelectRegion { then_start, else_start, exclusive: true };
        self.select_node(test, then, other, span, Some(region), arena)
    }

    /// Arm `index` tests the member of a subject: a `Match` whose arm for the pattern's member
    /// is the arm's value (with the binders in scope) and whose every other arm is the rest of
    /// the chain. A subject whose member is known picks its side and nothing else is recorded.
    #[allow(clippy::too_many_arguments)]
    fn record_chain_variant(
        &mut self,
        expression: &'a Expression,
        index: usize,
        negated: bool,
        test: &'a IsTest,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let Expr::Chain(chain) = &expression.value else {
            return None;
        };
        let arm = &chain.arms[index];
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
        // The side taken where the member is the pattern's, and where it is not. A negated test
        // binds nothing, and its value is the side for the other members.
        let locals = scope.len();
        let member_start = arena.len() as u32;
        let (member_value, other_value, other_start);
        if negated {
            if pattern.fields.as_ref().is_some_and(|f| !f.binders.value.is_empty()) {
                return None;
            }
            member_value = self.record_chain(
                expression,
                index + 1,
                parameter_count,
                depth + 1,
                arena,
                scope,
            )?;
            if known == Some(schema) {
                return Some(member_value);
            }
            other_start = arena.len() as u32;
            other_value =
                self.record_expr(&arm.value.value, parameter_count, depth + 1, arena, scope)?;
        } else {
            self.bind_fields(subject, schema, pattern.fields.as_ref(), arena, scope)?;
            member_value =
                self.record_expr(&arm.value.value, parameter_count, depth + 1, arena, scope)?;
            scope.truncate(locals);
            if known == Some(schema) {
                return Some(member_value);
            }
            other_start = arena.len() as u32;
            other_value = self.record_chain(
                expression,
                index + 1,
                parameter_count,
                depth + 1,
                arena,
                scope,
            )?;
        }
        scope.truncate(locals);
        if known.is_some() {
            return Some(other_value);
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
            expression.span_id,
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
}
