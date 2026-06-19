//! Quantified compile-time trait implementations: `x.Trait(...)`.

use internment::Intern;

use crate::Expr;
use crate::declare::ProvidedTrait;
use crate::expr::Typed;

/// A blanket trait impl applying to any type unless a concrete impl exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlanketImpl {
    pub trait_name: Intern<String>,
    pub type_var: Intern<String>,
    pub fields: Vec<(Intern<String>, Typed<Expr>)>,
}

impl BlanketImpl {
    pub fn to_provided_trait(&self) -> ProvidedTrait {
        ProvidedTrait {
            trait_name: self.trait_name,
            trait_name_span: crate::span::SpanId::INVALID,
            fields: self.fields.clone(),
        }
    }
}
