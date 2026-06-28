use crate::doc_comment::DocComment;
use crate::expr::{Expr, Typed};
use crate::parameter::Parameters;
use crate::span::SpanId;
use crate::ty::{PredicateExpr, Ty};
use internment::Intern;
use std::hash::{Hash, Hasher};

use crate::AttributeItem;

use crate::TypeExpr;
use crate::WhenExpr;
use crate::prelude::*;
use crate::span::Spanned;
use i256::I256;
use indexmap::IndexMap;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct DeclareAttributes {
    /// Raw parsed attributes before semantic extraction.
    /// `None` means no `#[...]` block was present at all.
    /// `Some(vec![])` means an empty `#[]` was present.
    pub raw_attributes: Option<Vec<AttributeItem>>,
}

impl DeclareAttributes {
    /// Extract compiler-known intrinsic attributes from `raw_attributes` into typed fields.
    /// Should be called after parsing.
    pub fn extract_intrinsic_attributes(&mut self) {
        let Some(items) = &self.raw_attributes else {
            return;
        };
        if items.is_empty() {
            return;
        }

        for item in items {
            if let AttributeItem::Call { .. } = item {
            } else if let AttributeItem::Flag { .. } = item {
            }
        }
    }
}

/// A method signature in an interface body (`has (...)`).
///
/// Analogous to a function declaration without a body — the implementor
/// must provide a matching function.
///
/// ```gin
/// Allocator has (
///     allocate(ref self, l Layout) Slice(Byte) or AllocError,
/// )
/// ```
///
/// becomes:
/// ```ignore
/// InterfaceMember {
///     name: "allocate",
///     params: [ref self, l Layout],
///     return_ty: Some(Slice(Byte)),
///     error_ty: Some(AllocError),
///     doc_comment: Some("..."),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceMember {
    pub name: Intern<String>,
    pub name_span: SpanId,
    /// Method parameters (names → kinds).
    pub params: Parameters,
    /// Parameter convention metadata (ref/mut/eat) for each named param.
    pub conventions: IndexMap<Intern<String>, ParamConvention>,
    /// The return type, e.g. `Slice(Byte)` in `allocate(...) Slice(Byte)`.
    pub return_ty: Option<Box<Spanned<TypeExpr>>>,
    /// The error type after `or`, e.g. `AllocError` in `... Slice(Byte) or AllocError`.
    pub error_ty: Option<Box<Spanned<TypeExpr>>>,
    pub doc_comment: Option<DocComment>,
    /// Refinement predicate, e.g. `and < n` on a field.
    pub refinement: Option<PredicateExpr>,
}

impl std::fmt::Display for InterfaceMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name.as_str())?;
        if !self.params.is_empty() {
            write!(f, "(")?;
            let mut first = true;
            for (k, v) in &self.params {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                if let Some(conv) = self.conventions.get(k) {
                    match conv {
                        ParamConvention::Ref(false) => write!(f, "ref ")?,
                        ParamConvention::Ref(true) => write!(f, "mut ")?,
                        ParamConvention::Eat => write!(f, "eat ")?,
                        ParamConvention::Inferred => {}
                    }
                }
                write!(f, "{}", k.as_str())?;
                match v {
                    ParameterKind::Tagged(sp) => write!(f, " {:?}", sp.value)?,
                    ParameterKind::Default(expr) => write!(f, ": {:?}", expr)?,
                    ParameterKind::Generic => {}
                }
            }
            write!(f, ")")?;
        }
        if let Some(rt) = &self.return_ty {
            write!(f, " {:?}", rt.value)?;
        }
        if let Some(et) = &self.error_ty {
            write!(f, " or {:?}", et.value)?;
        }
        Ok(())
    }
}

impl Hash for InterfaceMember {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.name_span.hash(state);
        for (k, v) in &self.params {
            k.hash(state);
            v.hash(state);
        }
        for (k, conv) in &self.conventions {
            k.hash(state);
            conv.hash(state);
        }
        self.return_ty.hash(state);
        self.error_ty.hash(state);
        self.doc_comment.hash(state);
        self.refinement.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclareValue {
    Alias(Box<Spanned<TypeExpr>>),
    Union {
        variants: Vec<Variant>,
    },
    /// Type-level conditional: `PointerSize is when target.arch is … then …`.
    When(Box<WhenExpr>),
    Set(/* TODO */),
    Range(I256, I256),
    // DiceThrow is in 1...6 (element of range)
    InRange(I256, I256),
    /// Interface method signatures: `Allocator has (allocate(...), deallocate(...))`.
    Interface(Vec<InterfaceMember>),
}

impl std::fmt::Display for DeclareValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alias(sp) => write!(f, "{:?}", sp.value),
            Self::When(_) => write!(f, "when …"),
            Self::Union { variants } => {
                let all_literal = variants
                    .iter()
                    .all(|v| matches!(v.shape().value, TypeExpr::Literal(..)));
                if all_literal && !variants.is_empty() {
                    write!(f, "{}", variants[0])?;
                    for v in &variants[1..] {
                        write!(f, "\n             or {v}")?;
                    }
                    return Ok(());
                }
                let mut first = true;
                for v in variants {
                    if !first {
                        write!(f, " or ")?;
                    }
                    first = false;
                    write!(f, "{v}")?;
                }
                Ok(())
            }
            Self::Interface(members) => {
                write!(f, " (")?;
                let mut first = true;
                for m in members {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{m}")?;
                }
                write!(f, ")")
            }
            Self::Set() => write!(f, "set"),
            Self::Range(start, end) => write!(f, "{start}...{end}"),
            Self::InRange(start, end) => write!(f, "in {start}...{end}"),
        }
    }
}

impl Hash for DeclareValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Alias(sp) => sp.hash(state),
            Self::When(w) => w.hash(state),
            Self::Union { variants } => {
                for variant in variants {
                    variant.hash(state);
                }
            }
            Self::Interface(members) => {
                for m in members {
                    m.hash(state);
                }
            }
            Self::Set() => {}
            Self::Range(start, end) | Self::InRange(start, end) => {
                start.hash(state);
                end.hash(state);
            }
        }
    }
}

/// A trait implementation provided by a `Type.Trait(...)` declaration.
///
/// For example, in `Capacity.IsEmpty(is_empty: self.count > 0)`, this represents
/// the `IsEmpty(is_empty: self.count > 0)` part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvidedTrait {
    pub trait_name: Intern<String>,
    pub trait_name_span: SpanId,
    /// Field definitions for the provided trait.
    /// e.g. `is_empty: self.count > 0` → `(is_empty, <expr>)`.
    pub fields: Vec<(Intern<String>, Typed<Expr>)>,
}

impl Hash for ProvidedTrait {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.trait_name.hash(state);
        self.trait_name_span.hash(state);
        for (k, v) in &self.fields {
            k.hash(state);
            v.hash(state);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declare {
    pub doc_comment: Option<DocComment>,
    pub name: Intern<String>,
    pub name_span: SpanId,
    /// Full declaration site from the tag name through the `is`/`has` value.
    pub span: SpanId,
    pub params: Option<Parameters>,
    pub resolved_type: Option<Ty>,
    pub attributes: DeclareAttributes,
    pub value: DeclareValue,
    pub provided_traits: Vec<ProvidedTrait>,
}

impl Declare {
    pub fn new(name: Intern<String>, name_span: SpanId, value: DeclareValue) -> Self {
        Declare {
            doc_comment: None,
            name,
            name_span,
            span: name_span,
            params: None,
            resolved_type: None,
            attributes: DeclareAttributes::default(),
            value,
            provided_traits: Vec::new(),
        }
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_doc(mut self, doc: Option<DocComment>) -> Self {
        self.doc_comment = doc;
        self
    }

    pub fn with_attributes(mut self, attrs: DeclareAttributes) -> Self {
        self.attributes = attrs;
        self
    }

    pub fn with_provided_traits(mut self, traits: Vec<ProvidedTrait>) -> Self {
        self.provided_traits = traits;
        self
    }
}

impl Hash for Declare {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.name_span.hash(state);
        self.span.hash(state);
        match &self.params {
            None => 0u8.hash(state),
            Some(params) => {
                1u8.hash(state);
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
        self.value.hash(state);
        self.provided_traits.hash(state);
    }
}
