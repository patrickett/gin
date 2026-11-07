use crate::{DomainBase, Refinement, RefinementPredicate, TypeBody, TypeDomain};

use super::{Kind, Lookup};
use crate::DomainKeyword;

impl<'syntax> Lookup<'syntax> {
    pub(super) fn type_body_children(&mut self, index: usize, body: &'syntax TypeBody) {
        match body {
            TypeBody::Is(expression) => self.expression(index, expression),
            TypeBody::Domain(domain) => {
                self.add(index, Kind::TypeDomain(domain), domain.span_id);
            }
            TypeBody::Has(members) => {
                for member in members {
                    self.add(index, Kind::TypeMember(member), member.span_id);
                }
            }
        }
    }

    pub(super) fn domain_children(&mut self, index: usize, domain: &'syntax TypeDomain) {
        if let Some(keyword) = domain.in_keyword {
            self.add(index, Kind::DomainKeyword(DomainKeyword::In), keyword);
        }
        self.add(index, Kind::DomainBase(&domain.base), domain.base.span_id);
        for refinement in &domain.refinements {
            self.add(index, Kind::Refinement(refinement), refinement.span_id);
        }
    }

    pub(super) fn domain_base_children(&mut self, index: usize, base: &'syntax DomainBase) {
        match base {
            DomainBase::Expression(expression) => self.expression(index, expression),
            DomainBase::Range {
                lower,
                to_keyword,
                upper,
            } => {
                self.expression(index, lower);
                self.add(index, Kind::DomainKeyword(DomainKeyword::To), *to_keyword);
                self.expression(index, upper);
            }
        }
    }

    pub(super) fn refinement_children(&mut self, index: usize, refinement: &'syntax Refinement) {
        self.add(
            index,
            Kind::DomainKeyword(DomainKeyword::And),
            refinement.and_keyword,
        );
        self.add(
            index,
            Kind::RefinementPredicate(&refinement.predicate),
            refinement.predicate.span_id,
        );
    }

    pub(super) fn predicate_children(
        &mut self,
        index: usize,
        predicate: &'syntax RefinementPredicate,
    ) {
        match predicate {
            RefinementPredicate::Expression(expression) => self.expression(index, expression),
            RefinementPredicate::ImplicitComparison { operator, right } => {
                self.add(index, Kind::BinaryOperator(operator), operator.span_id);
                self.expression(index, right);
            }
            RefinementPredicate::ImplicitDomain(domain) => {
                self.add(index, Kind::TypeDomain(domain), domain.span_id);
            }
        }
    }
}
