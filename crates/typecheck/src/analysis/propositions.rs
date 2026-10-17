use ast::{BinOp, Binary, Expr, Literal, ProofProposition, ProofRelation, ProofTerm, Ty};
use ast_format::type_expr::ExprFormatExt;
use internment::Intern;

pub(crate) fn comparison_type(binary: &Binary) -> Option<Ty> {
    Some(Ty::Proposition(Box::new(ProofProposition::Compare {
        left: operand_term(&binary.lhs.value),
        relation: ProofRelation::for_bin_op(&binary.op)?,
        right: operand_term(&binary.rhs.value),
    })))
}

fn operand_term(expr: &Expr) -> ProofTerm {
    enum Task<'a> {
        Visit(&'a Expr),
        Binary(&'a BinOp),
        Negate,
    }
    let mut pending = vec![Task::Visit(expr)];
    let mut terms = Vec::new();
    while let Some(task) = pending.pop() {
        match task {
            Task::Visit(Expr::Lit(Literal::Int(value))) => terms.push(ProofTerm::Value(*value)),
            Task::Visit(Expr::Binary(binary))
                if matches!(
                    binary.op,
                    BinOp::Add | BinOp::Subtract | BinOp::Multiply | BinOp::Modulo
                ) =>
            {
                pending.push(Task::Binary(&binary.op));
                pending.push(Task::Visit(&binary.rhs.value));
                pending.push(Task::Visit(&binary.lhs.value));
            }
            Task::Visit(Expr::Negate(inner)) => {
                pending.push(Task::Negate);
                pending.push(Task::Visit(&inner.value));
            }
            Task::Visit(expr) => terms.push(ProofTerm::Name(Intern::new(expr.format_surface()))),
            Task::Binary(op) => {
                let right = Box::new(terms.pop().expect("right operand"));
                let left = Box::new(terms.pop().expect("left operand"));
                terms.push(match op {
                    BinOp::Add => ProofTerm::Add(left, right),
                    BinOp::Subtract => ProofTerm::Sub(left, right),
                    BinOp::Multiply => ProofTerm::Mul(left, right),
                    BinOp::Modulo => ProofTerm::Remainder(left, right),
                    _ => unreachable!(),
                });
            }
            Task::Negate => {
                let right = Box::new(terms.pop().expect("negated operand"));
                terms.push(ProofTerm::Sub(Box::new(ProofTerm::Value(0.into())), right));
            }
        }
    }
    assert_eq!(terms.len(), 1);
    terms.pop().expect("comparison operand")
}

pub(crate) fn substitute_names(
    predicate: &ProofProposition,
    mut substitute: impl FnMut(Intern<String>) -> ast::NormalExpr,
) -> ProofProposition {
    enum Node<'a> {
        Predicate(&'a mut ProofProposition),
        Term(&'a mut ProofTerm),
    }
    let mut predicate = predicate.clone();
    let mut pending = vec![Node::Predicate(&mut predicate)];
    while let Some(node) = pending.pop() {
        match node {
            Node::Predicate(ProofProposition::Compare { left, right, .. }) => {
                pending.push(Node::Term(left));
                pending.push(Node::Term(right));
            }
            Node::Predicate(ProofProposition::InRange { value, start, end }) => {
                pending.push(Node::Term(value));
                pending.push(Node::Term(start));
                pending.push(Node::Term(end));
            }
            Node::Predicate(
                ProofProposition::And(left, right) | ProofProposition::Or(left, right),
            ) => {
                pending.push(Node::Predicate(left));
                pending.push(Node::Predicate(right));
            }
            Node::Predicate(ProofProposition::Not(inner)) => pending.push(Node::Predicate(inner)),
            Node::Term(term @ ProofTerm::Name(_)) => {
                let ProofTerm::Name(name) = term else {
                    unreachable!()
                };
                if let Some(replacement) = normal_term(&substitute(*name)) {
                    *term = replacement;
                }
            }
            Node::Term(
                ProofTerm::Add(left, right)
                | ProofTerm::Sub(left, right)
                | ProofTerm::Mul(left, right)
                | ProofTerm::Remainder(left, right),
            ) => {
                pending.push(Node::Term(left));
                pending.push(Node::Term(right));
            }
            Node::Term(ProofTerm::PowerOfTwo(inner)) => pending.push(Node::Term(inner)),
            Node::Term(ProofTerm::Value(_) | ProofTerm::TargetQuery { .. }) => {}
        }
    }
    predicate
}

fn normal_term(expr: &ast::NormalExpr) -> Option<ProofTerm> {
    use ast::NormalExpr;
    enum Task<'a> {
        Visit(&'a NormalExpr),
        Combine(u8),
    }
    let mut pending = vec![Task::Visit(expr)];
    let mut terms = Vec::new();
    while let Some(task) = pending.pop() {
        match task {
            Task::Visit(NormalExpr::Value(ast::ConstValue::Int(value))) => {
                terms.push(ProofTerm::Value(*value))
            }
            Task::Visit(NormalExpr::Var(name)) => terms.push(ProofTerm::Name(*name)),
            Task::Visit(
                expr @ (NormalExpr::Add(left, right)
                | NormalExpr::Sub(left, right)
                | NormalExpr::Mul(left, right)),
            ) => {
                pending.push(Task::Combine(match expr {
                    NormalExpr::Add(..) => 0,
                    NormalExpr::Sub(..) => 1,
                    _ => 2,
                }));
                pending.push(Task::Visit(right));
                pending.push(Task::Visit(left));
            }
            Task::Visit(NormalExpr::TargetQuery { kind, operand }) => {
                terms.push(ProofTerm::TargetQuery {
                    kind: *kind,
                    operand: operand.clone(),
                })
            }
            Task::Visit(_) => return None,
            Task::Combine(op) => {
                let right = Box::new(terms.pop()?);
                let left = Box::new(terms.pop()?);
                terms.push(match op {
                    0 => ProofTerm::Add(left, right),
                    1 => ProofTerm::Sub(left, right),
                    _ => ProofTerm::Mul(left, right),
                });
            }
        }
    }
    assert_eq!(terms.len(), 1);
    terms.pop()
}
