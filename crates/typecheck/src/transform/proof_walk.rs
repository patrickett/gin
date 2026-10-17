//! Traversals over the proof expressions, shared by the transform collectors.
//!
//! One macro generates the read-only (`read`) and mutating (`write`) families so
//! their shapes stay in sync. Overriding a `visit_*` hook replaces the default
//! traversal for that node, so hooks that only observe leaves must call the
//! matching `walk_*` helper to keep visiting children.

macro_rules! proof_walks {
    (
        $module:ident,
        $visitor:ident,
        $normal:ty,
        $term:ty,
        $proposition:ty,
        $predicate:ty,
        $reference:ty
    ) => {
        pub(crate) mod $module {
            use std::ops::ControlFlow;

            use ast::{NormalExpr, PredicateExpr, ProofProposition, ProofTerm, TypeReference};

            pub(crate) trait $visitor {
                fn visit_normal(&mut self, expr: $normal) -> ControlFlow<()> {
                    walk_normal(self, expr)
                }

                fn visit_term(&mut self, term: $term) -> ControlFlow<()> {
                    walk_term(self, term)
                }

                fn visit_proposition(&mut self, proposition: $proposition) -> ControlFlow<()> {
                    walk_proposition(self, proposition)
                }

                fn visit_predicate(&mut self, predicate: $predicate) -> ControlFlow<()> {
                    walk_predicate(self, predicate)
                }

                fn visit_reference(&mut self, reference: $reference) -> ControlFlow<()> {
                    walk_reference(self, reference)
                }
            }

            pub(crate) fn walk_normal(
                visitor: &mut (impl $visitor + ?Sized),
                expr: $normal,
            ) -> ControlFlow<()> {
                match expr {
                    NormalExpr::Add(left, right)
                    | NormalExpr::Sub(left, right)
                    | NormalExpr::Mul(left, right) => {
                        visitor.visit_normal(left)?;
                        visitor.visit_normal(right)?;
                    }
                    NormalExpr::TargetQuery { operand, .. } => visitor.visit_reference(operand)?,
                    NormalExpr::Value(_) | NormalExpr::Var(_) | NormalExpr::Inferred(_) => {}
                }
                ControlFlow::Continue(())
            }

            pub(crate) fn walk_term(
                visitor: &mut (impl $visitor + ?Sized),
                term: $term,
            ) -> ControlFlow<()> {
                match term {
                    ProofTerm::Add(left, right)
                    | ProofTerm::Sub(left, right)
                    | ProofTerm::Mul(left, right)
                    | ProofTerm::Remainder(left, right) => {
                        visitor.visit_term(left)?;
                        visitor.visit_term(right)?;
                    }
                    ProofTerm::PowerOfTwo(inner) => visitor.visit_term(inner)?,
                    ProofTerm::TargetQuery { operand, .. } => visitor.visit_reference(operand)?,
                    ProofTerm::Value(_) | ProofTerm::Name(_) => {}
                }
                ControlFlow::Continue(())
            }

            pub(crate) fn walk_proposition(
                visitor: &mut (impl $visitor + ?Sized),
                proposition: $proposition,
            ) -> ControlFlow<()> {
                match proposition {
                    ProofProposition::Compare { left, right, .. } => {
                        visitor.visit_term(left)?;
                        visitor.visit_term(right)?;
                    }
                    ProofProposition::InRange { value, start, end } => {
                        visitor.visit_term(value)?;
                        visitor.visit_term(start)?;
                        visitor.visit_term(end)?;
                    }
                    ProofProposition::Not(inner) => visitor.visit_proposition(inner)?,
                    ProofProposition::And(left, right) | ProofProposition::Or(left, right) => {
                        visitor.visit_proposition(left)?;
                        visitor.visit_proposition(right)?;
                    }
                }
                ControlFlow::Continue(())
            }

            pub(crate) fn walk_predicate(
                visitor: &mut (impl $visitor + ?Sized),
                predicate: $predicate,
            ) -> ControlFlow<()> {
                match predicate {
                    PredicateExpr::Lt(expr)
                    | PredicateExpr::Gt(expr)
                    | PredicateExpr::Le(expr)
                    | PredicateExpr::Ge(expr)
                    | PredicateExpr::Eq(expr)
                    | PredicateExpr::Ne(expr) => visitor.visit_normal(expr)?,
                    PredicateExpr::And(predicates) => {
                        for predicate in predicates {
                            visitor.visit_predicate(predicate)?;
                        }
                    }
                    PredicateExpr::Proposition(proposition) => {
                        visitor.visit_proposition(proposition)?;
                    }
                }
                ControlFlow::Continue(())
            }

            pub(crate) fn walk_reference(
                visitor: &mut (impl $visitor + ?Sized),
                reference: $reference,
            ) -> ControlFlow<()> {
                match reference {
                    TypeReference::Application { arguments, .. } => {
                        for argument in arguments {
                            visitor.visit_reference(argument)?;
                        }
                    }
                    TypeReference::Pointer(inner) => visitor.visit_reference(inner)?,
                    TypeReference::Unit | TypeReference::Nominal(_) => {}
                }
                ControlFlow::Continue(())
            }
        }
    };
}

proof_walks!(
    read,
    ProofVisitor,
    &NormalExpr,
    &ProofTerm,
    &ProofProposition,
    &PredicateExpr,
    &TypeReference
);
proof_walks!(
    write,
    ProofVisitorMut,
    &mut NormalExpr,
    &mut ProofTerm,
    &mut ProofProposition,
    &mut PredicateExpr,
    &mut TypeReference
);
