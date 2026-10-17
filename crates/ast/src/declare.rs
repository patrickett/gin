use crate::doc_comment::DocComment;
use crate::expr::{BindValue, Expr, Typed};

use crate::parameter::Parameters;
use crate::span::SpanId;
use crate::ty::{PredicateExpr, Ty};
use internment::Intern;
use std::hash::{Hash, Hasher};

use crate::AttributeItem;

use crate::Pattern;
use crate::WhenExpr;
use crate::prelude::*;
use crate::span::Spanned;
use i256::I256;
use indexmap::IndexMap;

fn attribute_identifier(arg: &Typed<Expr>) -> Option<Intern<String>> {
    match &arg.value {
        Expr::AnonymousTag(name) => Some(*name),
        Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty() => {
            Some(call.path.value.root)
        }
        _ => None,
    }
}

/// How a `has` member body is declared: overrideable (`:`) or final (`:=`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HasMemberBody {
    /// `name Type: body` — may be overridden by impls.
    Overrideable(BindValue),
    /// `name Type := body` — final, cannot be overridden.
    Final(BindValue),
}

impl HasMemberBody {
    pub fn as_expr(&self) -> Option<&Typed<Expr>> {
        match self {
            Self::Overrideable(BindValue::Expr(expr)) | Self::Final(BindValue::Expr(expr)) => {
                Some(expr)
            }
            _ => None,
        }
    }
}

/// Whether a function member is an instance method or associated method.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HasFunctionKind {
    Instance,
    Associated,
}

/// A property (stored field or computed projection) in a `has` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HasProperty {
    pub qualifier: Option<HasMemberQualifier>,
    pub name: Intern<String>,
    pub name_span: SpanId,
    pub ty: Option<Box<Spanned<Expr>>>,
    pub owned_storage: bool,
    pub attributes: crate::expr::BindAttributes,
    pub body: Option<HasMemberBody>,
    pub doc_comment: Option<DocComment>,
    pub refinement: Option<PredicateExpr>,
}

impl Hash for HasProperty {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.qualifier.hash(state);
        self.name.hash(state);
        self.name_span.hash(state);
        self.ty.hash(state);
        self.owned_storage.hash(state);
        self.attributes.hash(state);
        self.body.hash(state);
        self.doc_comment.hash(state);
        self.refinement.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HasMemberQualifier {
    pub name: Intern<String>,
    pub span: SpanId,
}

/// A function member (instance or associated method) in a `has` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HasFunction {
    pub qualifier: Option<HasMemberQualifier>,
    pub name: Intern<String>,
    pub name_span: SpanId,
    pub params: Box<Parameters>,
    pub conventions: Box<IndexMap<Intern<String>, ParamConvention>>,
    pub param_groups: Box<IndexMap<Intern<String>, crate::GroupPath>>,
    pub param_refinements: Box<IndexMap<Intern<String>, PredicateExpr>>,
    pub return_ty: Option<Box<Spanned<Expr>>>,
    pub anonymous_result_alternatives: Vec<Spanned<crate::ResultAlternative>>,
    pub payload_postconditions: Vec<crate::PayloadPostcondition>,
    pub body: Option<HasMemberBody>,
    pub doc_comment: Option<DocComment>,
    pub refinement: Option<PredicateExpr>,
    pub kind: HasFunctionKind,
    pub attributes: crate::expr::BindAttributes,
}

impl HasFunction {
    pub fn implementation(&self, receiver: Spanned<Expr>) -> Option<crate::Bind> {
        let Some(body) = &self.body else {
            return None;
        };
        let value = match body {
            crate::HasMemberBody::Overrideable(value) | crate::HasMemberBody::Final(value) => {
                value.clone()
            }
        };
        let bind_name = self.qualifier.as_ref().map_or(self.name, |qualifier| {
            Intern::new(format!(
                "{}.{}",
                qualifier.name.as_str(),
                self.name.as_str()
            ))
        });
        let mut bind = crate::Bind::new(bind_name, self.name_span, value)
            .with_params(Some(self.params.as_ref().clone()))
            .with_receiver_type(Some(Box::new(receiver)))
            .with_doc(self.doc_comment.clone())
            .with_attributes(self.attributes.clone());
        bind.param_conventions = self.conventions.as_ref().clone();
        bind.param_groups = self.param_groups.as_ref().clone();
        bind.param_refinements = self.param_refinements.as_ref().clone();
        bind.return_tag = self.return_ty.clone();
        bind.anonymous_result_alternatives = self.anonymous_result_alternatives.clone();
        bind.payload_postconditions = self.payload_postconditions.clone();
        bind.operator = if matches!(body, crate::HasMemberBody::Final(_)) {
            crate::BindOperator::FreshImmutable
        } else {
            crate::BindOperator::FreshMutable
        };
        Some(bind)
    }
}

impl Hash for HasFunction {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.qualifier.hash(state);
        self.name.hash(state);
        self.name_span.hash(state);
        for (k, v) in self.params.iter() {
            k.hash(state);
            v.hash(state);
        }
        for (k, conv) in self.conventions.iter() {
            k.hash(state);
            conv.hash(state);
        }
        for (k, group) in self.param_groups.iter() {
            k.hash(state);
            group.hash(state);
        }
        for (k, refinement) in self.param_refinements.iter() {
            k.hash(state);
            refinement.hash(state);
        }
        self.return_ty.hash(state);
        self.anonymous_result_alternatives.hash(state);
        self.payload_postconditions.hash(state);
        self.body.hash(state);
        self.doc_comment.hash(state);
        self.refinement.hash(state);
        self.kind.hash(state);
        self.attributes.hash(state);
    }
}

/// A member of a `has` body — either a property/field or a function (method).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasMember {
    Property(Box<HasProperty>),
    Function(Box<HasFunction>),
}

impl Hash for HasMember {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            HasMember::Property(p) => p.hash(state),
            HasMember::Function(f) => f.hash(state),
        }
    }
}

impl std::fmt::Display for HasProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(qualifier) = &self.qualifier {
            write!(f, "{}.", qualifier.name.as_str())?;
        }
        write!(f, "{}", self.name.as_str())?;
        if self.owned_storage {
            write!(f, " own")?;
        }
        if let Some(ty) = &self.ty {
            write!(f, " {:?}", ty.value)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for HasFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(qualifier) = &self.qualifier {
            write!(f, "{}.", qualifier.name.as_str())?;
        }
        if self.params.is_empty() {
            write!(f, "{}", self.name.as_str())?;
        } else {
            write!(f, "{}(", self.name.as_str())?;
            let mut first = true;
            for (k, v) in self.params.iter() {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}", k.as_str())?;
                match &v.kind {
                    ParameterKind::Tagged(sp) => write!(f, " {:?}", sp.value)?,
                    ParameterKind::ValueParam { ty } => write!(f, " {:?} (value)", ty.value)?,
                    ParameterKind::Inferred { ty } => write!(f, " {:?}: ?", ty.value)?,
                    ParameterKind::Default(expr) => write!(f, ": {:?}", expr)?,
                    ParameterKind::Generic => {}
                    ParameterKind::Proposition(predicate) => write!(f, " is {predicate}")?,
                }
                if let Some(conv) = self.conventions.get(k) {
                    match conv {
                        ParamConvention::Observe => write!(f, " ref")?,
                        ParamConvention::Mutate => write!(f, " mut")?,
                        ParamConvention::Own => write!(f, " own")?,
                        ParamConvention::Terminal => {}
                    }
                }
                if !v.constraints.is_empty() {
                    write!(f, " has {}", v.constraints_surface())?;
                }
            }
            write!(f, ")")?;
        }
        for (index, alternative) in self.anonymous_result_alternatives.iter().enumerate() {
            write!(
                f,
                "{}{}",
                if index == 0 { " " } else { " or " },
                alternative.value.label
            )?;
            if let Some(proposition) = &alternative.value.proposition {
                write!(f, " thus {proposition}")?;
            }
        }
        if let Some(rt) = &self.return_ty {
            write!(f, " {:?}", rt.value)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for HasMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HasMember::Property(p) => write!(f, "{p}"),
            HasMember::Function(fn_m) => write!(f, "{fn_m}"),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct DeclareAttributes {
    pub fixed_array: Option<Box<FixedArrayContract>>,
    pub invalid_fixed_array: Option<Intern<String>>,
    pub sequence_builder: Option<Box<SequenceBuilderContract>>,
    pub invalid_sequence_builder: Option<Intern<String>>,
    pub bits: Option<Box<Typed<Expr>>>,
    pub bits_span: Option<SpanId>,
    pub raw_attributes: Option<Vec<AttributeItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FixedArrayContract {
    pub element_parameter: Intern<String>,
    pub length_parameter: Intern<String>,
    pub span: SpanId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SequenceBuilderContract {
    pub element_parameter: Intern<String>,
    pub empty_member: Intern<String>,
    pub push_member: Intern<String>,
    pub span: SpanId,
}

impl DeclareAttributes {
    pub fn phantom_parameters(&self) -> Vec<Intern<String>> {
        self.raw_attributes
            .iter()
            .flatten()
            .filter_map(|attribute| match attribute {
                AttributeItem::Call { name, args, .. } if name.as_str() == "phantom" => Some(args),
                _ => None,
            })
            .flatten()
            .filter_map(attribute_identifier)
            .collect()
    }

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
            match item {
                AttributeItem::Call {
                    name,
                    name_span,
                    args,
                } if name.as_str() == "fixed_array" => {
                    match args
                        .as_slice()
                        .first()
                        .and_then(attribute_identifier)
                        .zip(args.as_slice().get(1).and_then(attribute_identifier))
                    {
                        Some((element_parameter, length_parameter)) => {
                            self.fixed_array = Some(Box::new(FixedArrayContract {
                                element_parameter,
                                length_parameter,
                                span: *name_span,
                            }));
                        }
                        _ => self.invalid_fixed_array = Some(*name),
                    }
                }
                AttributeItem::Call {
                    name,
                    name_span,
                    args,
                } if name.as_str() == "sequence_builder" => {
                    match (
                        args.first().and_then(attribute_identifier),
                        args.get(1).and_then(attribute_identifier),
                        args.get(2).and_then(attribute_identifier),
                    ) {
                        (Some(element_parameter), Some(empty_member), Some(push_member)) => {
                            self.sequence_builder = Some(Box::new(SequenceBuilderContract {
                                element_parameter,
                                empty_member,
                                push_member,
                                span: *name_span,
                            }));
                        }
                        _ => self.invalid_sequence_builder = Some(*name),
                    }
                }
                AttributeItem::Call {
                    name,
                    name_span,
                    args,
                } if name.as_str() == "bits" => {
                    self.bits = args.first().cloned().map(Box::new);
                    self.bits_span = Some(*name_span);
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclareValue {
    Alias(Box<Spanned<Expr>>),
    Union {
        variants: Vec<Variant>,
    },
    When(Box<WhenExpr>),
    Set(/* TODO */),
    Range(I256, I256),
    // DiceThrow is in 1 to 6 (element of range)
    InRange(I256, I256),
    Refinement(Box<crate::ty::PredicateExpr>),
    /// A bond: a type's data, behavior, and requirements declared together.
    Has(Vec<HasMember>),
}

impl std::fmt::Display for DeclareValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alias(sp) => write!(f, "{:?}", sp.value),
            Self::When(_) => write!(f, "when …"),
            Self::Union { variants } => {
                let all_literal = variants
                    .iter()
                    .all(|v| matches!(v.shape().value, Pattern::Literal(..)));
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
            Self::Has(members) => {
                write!(f, " ")?;
                let mut first = true;
                for m in members {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{m}")?;
                }
                Ok(())
            }
            Self::Set() => write!(f, "set"),
            Self::Range(start, end) => write!(f, "{start} to {end}"),
            Self::InRange(start, end) => write!(f, "in {start} to {end}"),
            Self::Refinement(predicate) => write!(f, "{predicate:?}"),
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
            Self::Has(members) => {
                for m in members {
                    m.hash(state);
                }
            }
            Self::Set() => {}
            Self::Range(start, end) | Self::InRange(start, end) => {
                start.hash(state);
                end.hash(state);
            }
            Self::Refinement(predicate) => predicate.hash(state),
        }
    }
}

/// Nominal trait membership and any qualified property implementations collected
/// from the containing type declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvidedTrait {
    pub trait_name: Intern<String>,
    pub trait_name_span: SpanId,
    pub args: Vec<Typed<Expr>>,
    /// Field definitions for the provided trait.
    /// e.g. `is_empty: self.count > 0` → `(is_empty, <expr>)`.
    pub fields: Vec<(Intern<String>, Typed<Expr>)>,
}

impl Hash for ProvidedTrait {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.trait_name.hash(state);
        self.trait_name_span.hash(state);
        self.args.hash(state);
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
    pub keyword_span: SpanId,
    /// Full declaration site from the tag name through the `is`/`has` value.
    pub span: SpanId,
    pub params: Option<Parameters>,
    pub resolved_type: Option<Ty>,
    pub attributes: DeclareAttributes,
    pub value: DeclareValue,
    pub provided_traits: Vec<ProvidedTrait>,
    pub provided_traits_span: Option<SpanId>,
    pub extensions: Vec<Declare>,
}

impl Declare {
    pub fn receiver_surface(&self) -> Spanned<Expr> {
        let receiver = if let Some(params) = &self.params {
            Expr::TagCall(crate::TagCall {
                name: self.name,
                qual_path: None,
                args: params
                    .iter()
                    .map(|(name, parameter)| {
                        Typed::infer(Expr::AnonymousTag(*name), parameter.name_span)
                    })
                    .collect(),
            })
        } else {
            Expr::AnonymousTag(self.name)
        };
        Spanned::new(receiver, self.span)
    }

    pub fn new(name: Intern<String>, name_span: SpanId, value: DeclareValue) -> Self {
        Declare {
            doc_comment: None,
            name,
            name_span,
            keyword_span: name_span,
            span: name_span,
            params: None,
            resolved_type: None,
            attributes: DeclareAttributes::default(),
            value,
            provided_traits: Vec::new(),
            provided_traits_span: None,
            extensions: Vec::new(),
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
        self.provided_traits_span.hash(state);
        self.extensions.hash(state);
    }
}
