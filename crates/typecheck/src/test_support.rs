//! Shared fixtures for unit tests.

use crate::ExprId;
use crate::ty::Ty;
use crate::typed::{
    Availability, ConstFacts, ExprFacts, OwnershipFacts, SemanticFacts, TypedExpr, TypedExprKind,
    TypedFileAst,
};
use ast::SpanId;

/// Append an expression to `typed`'s arena with the given type and empty facts.
pub(crate) fn push_expr_typed(typed: &mut TypedFileAst, kind: TypedExprKind, ty: Ty) -> ExprId {
    let id = ExprId(u32::try_from(typed.exprs.kind.len()).unwrap());
    typed.exprs.push(TypedExpr {
        kind,
        ty,
        span: SpanId::INVALID,
        facts: ExprFacts {
            const_facts: ConstFacts {
                const_value: None,
                const_object: None,
                sequence_literal: None,
                literal_materialization: None,
            },
            semantic_facts: SemanticFacts {
                integer_knowledge: None,
                result_evidence: None,
                projected_result_evidence: Default::default(),
                availability: Availability::Unknown,
                flaws: vec![],
            },
            ownership_facts: OwnershipFacts {
                references: Default::default(),
                reference_reads: Default::default(),
                target_group: None,
                result_target_groups: vec![],
                place: None,
                place_version: None,
            },
        },
    });
    id
}

/// Append a `Unit`-typed expression, the default placeholder type for tests.
pub(crate) fn push_expr(typed: &mut TypedFileAst, kind: TypedExprKind) -> ExprId {
    push_expr_typed(typed, kind, Ty::Unit)
}

/// Append a byte-typed expression, the expected type in byte-flow tests.
pub(crate) fn push_byte_expr(typed: &mut TypedFileAst, kind: TypedExprKind) -> ExprId {
    push_expr_typed(typed, kind, Ty::bounded_int(0.into(), 0.into()))
}
