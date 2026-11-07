use crate::{Annotation, AnnotationKeyword};

use super::{Kind, Lookup};

impl<'syntax> Lookup<'syntax> {
    pub(super) fn annotation_children(&mut self, index: usize, annotation: &'syntax Annotation) {
        match annotation {
            Annotation::Expression(expression) => self.expression(index, expression),
            Annotation::Domain { is_keyword, domain } => {
                if let Some(keyword) = is_keyword {
                    self.add(
                        index,
                        Kind::AnnotationKeyword(AnnotationKeyword::Is),
                        *keyword,
                    );
                }
                self.add(index, Kind::TypeDomain(domain), domain.span_id);
            }
            Annotation::Refined { base, refinements } => {
                if let Some(base) = base {
                    self.expression(index, base);
                }
                for refinement in refinements {
                    self.add(index, Kind::Refinement(refinement), refinement.span_id);
                }
            }
            Annotation::Bound { has_keyword, bound } => {
                self.add(
                    index,
                    Kind::AnnotationKeyword(AnnotationKeyword::Has),
                    *has_keyword,
                );
                self.expression(index, bound);
            }
            Annotation::Predicate {
                is_keyword,
                predicate,
                refinements,
            } => {
                self.add(
                    index,
                    Kind::AnnotationKeyword(AnnotationKeyword::Is),
                    *is_keyword,
                );
                self.add(
                    index,
                    Kind::RefinementPredicate(predicate),
                    predicate.span_id,
                );
                for refinement in refinements {
                    self.add(index, Kind::Refinement(refinement), refinement.span_id);
                }
            }
        }
    }
}
