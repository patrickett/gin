//! Regions of a recorded body: which nodes run only when a loop pass or a chosen arm runs.
//!
//! One definition, used by the lowering (which puts a region in a branch) and by the evaluator
//! (which runs the effects of a region when the region runs). A node is *guarded* when running
//! it where its operands are not the ones it was checked for is not harmless: an instruction, a
//! call that may reach one, a division, a wide shift. The caller says which nodes those are.
use std::ops::Range;

use super::body::{CheckedSourceBody, CheckedSourceBodyKind};

/// The two arms of a choice that must not both run, as node ranges, when an arm holds a guarded
/// node and its nodes belong to it alone (an exclusive `Select` region, or a `Match` region).
pub fn lazy_arms(
    body: &CheckedSourceBody,
    id: usize,
    guarded: &dyn Fn(&CheckedSourceBodyKind) -> bool,
) -> Option<(Range<usize>, Range<usize>)> {
    let any_guarded = |range: Range<usize>| range.into_iter().any(|node| guarded(&body.expressions[node].kind));
    match &body.expressions[id].kind {
        CheckedSourceBodyKind::Select { region: Some(region), .. }
            if region.exclusive && any_guarded(region.then_start as usize..id) =>
        {
            Some((region.then_start as usize..region.else_start as usize, region.else_start as usize..id))
        }
        CheckedSourceBodyKind::Match { region: Some(region), .. } if any_guarded(region.member_start as usize..id) => {
            Some((region.member_start as usize..region.other_start as usize, region.other_start as usize..id))
        }
        _ => None,
    }
}

/// How many loop bodies and lazy arms each node sits in. A node at depth 0 runs whenever the body
/// does.
pub fn nesting(body: &CheckedSourceBody, guarded: &dyn Fn(&CheckedSourceBodyKind) -> bool) -> Vec<u32> {
    let mut nesting = vec![0u32; body.expressions.len()];
    for (id, expr) in body.expressions.iter().enumerate() {
        if let CheckedSourceBodyKind::Loop { body_start, .. } | CheckedSourceBodyKind::While { body_start, .. } = &expr.kind {
            for inside in *body_start as usize..id {
                nesting[inside] += 1;
            }
        }
        if let Some((first, second)) = lazy_arms(body, id, guarded) {
            for inside in first.chain(second) {
                nesting[inside] += 1;
            }
        }
    }
    nesting
}
