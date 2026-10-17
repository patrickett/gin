//! Bridge compile-time [`ConstValue`]/[`ast::NormalExpr`] values back into surface [`Expr`] trees.

use internment::Intern;

use ast::expr::{Binary, Expr, FnCall, Literal, SequenceLiteral, TagCall, Typed};
use ast::span::SpanId;
use ast::{ConstValue, HashFloat};

pub fn const_value_to_typed_expr(cv: &ConstValue, span_id: SpanId) -> Typed<Expr> {
    Typed {
        value: const_value_expr(cv, SpanId::INVALID, true),
        ty: ast::ty_state::TyState::Infer,
        const_value: Some(Box::new(cv.clone())),
        span_id,
    }
}

pub fn const_value_to_expr(cv: &ConstValue) -> Expr {
    const_value_expr(cv, SpanId::INVALID, true)
}

pub(crate) fn const_value_of_literal(literal: &Literal) -> ConstValue {
    match literal {
        Literal::Int(n) => ConstValue::Int(*n),
        Literal::Float(HashFloat(f)) => ConstValue::Float(HashFloat(*f)),
        Literal::String(s) => ConstValue::Text(s.clone()),
    }
}

/// Like [`const_value_to_expr`], but threads `span_id` through the tree and
/// omits per-node `const_value` hints for callers that propagate them later
/// (`prepare_target` materialization).
pub(crate) fn const_value_to_inferred_expr(cv: &ConstValue, span_id: SpanId) -> Expr {
    const_value_expr(cv, span_id, false)
}

fn typed_child(cv: &ConstValue, span_id: SpanId, hints: bool) -> Typed<Expr> {
    Typed {
        value: const_value_expr(cv, span_id, hints),
        ty: ast::ty_state::TyState::Infer,
        const_value: hints.then(|| Box::new(cv.clone())),
        span_id,
    }
}

fn const_value_expr(cv: &ConstValue, span_id: SpanId, hints: bool) -> Expr {
    match cv {
        ConstValue::Proposition(holds) => Expr::Binary(Binary::new(
            Typed::infer(
                Expr::Lit(Literal::Int(i256::I256::from(i128::from(*holds)))),
                SpanId::INVALID,
            ),
            ast::BinOp::Equal,
            Typed::infer(
                Expr::Lit(Literal::Int(i256::I256::from(1))),
                SpanId::INVALID,
            ),
        )),
        ConstValue::Domain(domain) => Expr::DomainValue(domain.clone()),
        ConstValue::ResultAlternative { label, .. } => Expr::AnonymousTag(*label),
        ConstValue::Text(s) => Expr::Lit(Literal::String(s.clone())),
        ConstValue::Int(n) => Expr::Lit(Literal::Int(*n)),
        ConstValue::Float(HashFloat(f)) => Expr::Lit(Literal::Float(HashFloat(*f))),
        ConstValue::Tag {
            name,
            qual_path,
            args,
        } if args.is_empty() && qual_path.is_none() => Expr::AnonymousTag(*name),
        ConstValue::Tag {
            name,
            qual_path,
            args,
        } => Expr::TagCall(TagCall {
            name: *name,
            qual_path: qual_path.as_ref().map(|s| {
                let parts: Vec<_> = s.split('.').map(Intern::from_ref).collect();
                let root = parts[0];
                let segments = parts.get(1..).unwrap_or(&[]).to_vec();
                ast::Spanned::new(ast::ModPath::new(root, segments), span_id)
            }),
            args: args
                .iter()
                .map(|a| typed_child(a, span_id, hints))
                .collect(),
        }),
        ConstValue::Record { fields } => Expr::RecordLit(
            fields
                .iter()
                .map(|(name, v)| (*name, typed_child(v, span_id, hints)))
                .collect(),
        ),
        ConstValue::ResultRow(items) => Expr::ResultValues(
            items
                .iter()
                .map(|item| typed_child(item, span_id, hints))
                .collect(),
        ),
        ConstValue::FixedArray(items) => Expr::ArrayLit(
            items
                .iter()
                .map(|item| typed_child(item, span_id, hints))
                .collect(),
        ),
        ConstValue::Sequence(items) => Expr::SequenceLiteral(SequenceLiteral::Elements(
            items
                .iter()
                .map(|item| typed_child(item, span_id, hints))
                .map(ast::SequenceElement::new)
                .collect(),
        )),
        ConstValue::SequenceRepeat { seed, count } => {
            Expr::SequenceLiteral(SequenceLiteral::Repeat {
                seed: Box::new(typed_child(seed, span_id, hints)),
                count: Box::new(Typed::infer(normal_expr_to_expr(count), span_id)),
            })
        }
    }
}

pub(crate) fn normal_expr_to_expr(normal: &ast::NormalExpr) -> Expr {
    match normal {
        ast::NormalExpr::Value(value) => const_value_to_expr(value),
        ast::NormalExpr::Var(name) => Expr::FnCall(FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: ast::Spanned::new(ast::ModPath::new(*name, Vec::new()), SpanId::INVALID),
            args: None,
        }),
        ast::NormalExpr::Inferred(id) => {
            let name = Intern::new(format!("_dependent_{id:?}"));
            Expr::FnCall(FnCall {
                syntax: ast::CallSyntax::Path,
                definition: None,
                path: ast::Spanned::new(ast::ModPath::new(name, Vec::new()), SpanId::INVALID),
                args: None,
            })
        }
        ast::NormalExpr::Add(lhs, rhs)
        | ast::NormalExpr::Sub(lhs, rhs)
        | ast::NormalExpr::Mul(lhs, rhs) => {
            let op = match normal {
                ast::NormalExpr::Add(_, _) => ast::BinOp::Add,
                ast::NormalExpr::Sub(_, _) => ast::BinOp::Subtract,
                ast::NormalExpr::Mul(_, _) => ast::BinOp::Multiply,
                _ => unreachable!(),
            };
            Expr::Binary(Binary::new(
                Typed::infer(normal_expr_to_expr(lhs), SpanId::INVALID),
                op,
                Typed::infer(normal_expr_to_expr(rhs), SpanId::INVALID),
            ))
        }
        ast::NormalExpr::TargetQuery { kind, operand } => Expr::TargetQuery {
            kind: *kind,
            operand: operand.clone(),
        },
    }
}
