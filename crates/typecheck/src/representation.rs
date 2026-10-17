use crate::normal_expr::Normalize;
use crate::{TypeRegistry, ty::Ty};
use ast::NormalExpr;
use ast::ty::{TyArg, TypeId};
use diagnostic::code::{LayoutCode, RepresentationCode};
use diagnostic::{Diagnostic, DiagnosticCode};
use std::collections::HashMap;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RecursionKey {
    Declaration(ast::ty::DeclarationKey),
    Instance(ast::ty::NamedTypeInstance),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Repr {
    Bits {
        width: u32,
    },
    Address {
        address_space: u32,
    },
    Product {
        fields: Vec<Repr>,
    },
    Sum {
        variants: Vec<Repr>,
        discriminant_width: Option<u32>,
    },
    Array {
        element: Box<Repr>,
        length: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrayLength {
    Symbolic(NormalExpr),
    Concrete(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixedArrayLengthArgument {
    Concrete(u64),
    Dependent(NormalExpr),
    WrongKind,
    Missing,
    Invalid(ast::ConstValue),
}

fn fixed_array_length_argument(argument: Option<&ast::TyArg>) -> FixedArrayLengthArgument {
    match argument {
        None => FixedArrayLengthArgument::Missing,
        Some(ast::TyArg::Type(_)) => FixedArrayLengthArgument::WrongKind,
        Some(ast::TyArg::Const(length)) => match length.normalize_to_value() {
            Some(value) => match value {
                ast::ConstValue::Int(value) => ast::integer::to_u64(value)
                    .map(FixedArrayLengthArgument::Concrete)
                    .unwrap_or(FixedArrayLengthArgument::Invalid(ast::ConstValue::Int(
                        value,
                    ))),
                _ => FixedArrayLengthArgument::Invalid(value),
            },
            None => FixedArrayLengthArgument::Dependent(length.normalize()),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepresentationError {
    #[error("integer type `{type_name}` requires a concrete target for layout")]
    TargetDependentInteger { type_name: String },
    #[error("type `{type_name}` has no supported physical representation")]
    Unsupported { type_name: String },
    #[error("recursive representation at `{type_id:?}`")]
    RecursiveRepresentation { type_id: TypeId },
    #[error("fixed-array length for `{type_name}` is not a nonnegative integer")]
    InvalidArrayLength { type_name: String },
    #[error("integer type `{type_name}` has invalid representation")]
    InvalidInteger {
        type_name: String,
        code: DiagnosticCode,
    },
}

impl RepresentationError {
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::TargetDependentInteger { .. } => {
                // type-target-dependent-layout
                DiagnosticCode::Layout(LayoutCode::TargetDependent)
            }
            Self::Unsupported { .. } => {
                // unsupported-type-representation
                DiagnosticCode::Representation(RepresentationCode::Unsupported)
            }
            Self::RecursiveRepresentation { .. } => {
                // recursive-type-representation
                DiagnosticCode::Representation(RepresentationCode::Recursive)
            }
            Self::InvalidArrayLength { .. } => {
                // invalid-fixed-array-length
                DiagnosticCode::Representation(RepresentationCode::FixedArrayLengthInvalid)
            }
            Self::InvalidInteger { code, .. } => *code,
        }
    }

    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic::new(self.code(), self.to_string())
    }
}

pub struct RepresentationContext<'a> {
    registry: Option<&'a TypeRegistry>,
}

impl Repr {
    pub fn derive(ty: &Ty, registry: Option<&TypeRegistry>) -> Result<Self, RepresentationError> {
        RepresentationContext { registry }.derive(ty)
    }

    pub fn array_parts(ty: &Ty, registry: Option<&TypeRegistry>) -> Option<(Ty, u64)> {
        match ty {
            Ty::Array { elem, size } => {
                let ArrayLength::Concrete(length) = array_length(size) else {
                    return None;
                };
                Some(((**elem).clone(), length))
            }
            Ty::Sequence(fields) => {
                let first = fields.first()?;
                fields
                    .iter()
                    .all(|field| field.ty == first.ty)
                    .then(|| ((*first.ty).clone(), fields.len() as u64))
            }
            Ty::Named { instance, .. } => {
                let former = registry?
                    .declaration_for_instance(instance)?
                    .sequence_literal
                    .as_ref()?;
                let length_parameter = former.fixed_array_length_parameter()?;
                let element = instance.arguments.iter().find_map(|(parameter, argument)| {
                    (*parameter == former.element_parameter).then_some(argument)
                });
                let length = instance.arguments.iter().find_map(|(parameter, argument)| {
                    (*parameter == length_parameter).then_some(argument)
                });
                let TyArg::Type(element) = element? else {
                    return None;
                };
                let length = match fixed_array_length_argument(length) {
                    FixedArrayLengthArgument::Concrete(length) => length,
                    _ => return None,
                };
                Some(((**element).clone(), length))
            }
            _ => None,
        }
    }
}

pub fn has_symbolic_fixed_array_length(ty: &Ty, registry: Option<&TypeRegistry>) -> bool {
    RepresentationContext::new(registry).has_symbolic_fixed_array_length(ty)
}

impl<'a> RepresentationContext<'a> {
    pub fn new(registry: Option<&'a TypeRegistry>) -> Self {
        Self { registry }
    }

    pub fn has_symbolic_fixed_array_length(&self, ty: &Ty) -> bool {
        self.has_symbolic_fixed_array_length_inner(ty, &mut HashSet::new())
    }

    fn has_symbolic_fixed_array_length_inner(&self, ty: &Ty, active: &mut HashSet<TypeId>) -> bool {
        match ty {
            Ty::Array { size, .. } => !matches!(array_length(size), ArrayLength::Concrete(_)),
            Ty::Named { instance, .. } => {
                let Some(declaration) = self
                    .registry
                    .and_then(|registry| registry.declaration_for_instance(instance))
                else {
                    return false;
                };
                if let Some(former) = &declaration.sequence_literal {
                    let Some(length_parameter) = former.fixed_array_length_parameter() else {
                        return false;
                    };
                    let element = instance.arguments.iter().find_map(|(parameter, argument)| {
                        (*parameter == former.element_parameter).then_some(argument)
                    });
                    if !matches!(element, Some(TyArg::Type(_))) {
                        return false;
                    }
                    let length = instance.arguments.iter().find_map(|(parameter, argument)| {
                        (*parameter == length_parameter).then_some(argument)
                    });
                    return !matches!(
                        fixed_array_length_argument(length),
                        FixedArrayLengthArgument::Concrete(_)
                    );
                }
                if !active.insert(instance.declaration) {
                    return false;
                }
                let symbolic =
                    self.has_symbolic_fixed_array_length_inner(&declaration.definition, active);
                active.remove(&instance.declaration);
                symbolic
            }
            Ty::Ref { inner, .. }
            | Ty::Ptr { inner, .. }
            | Ty::OwnedStorage { inner }
            | Ty::Address { pointee: inner, .. } => {
                self.has_symbolic_fixed_array_length_inner(inner, active)
            }
            Ty::Record { fields, .. } => fields
                .iter()
                .any(|(_, field)| self.has_symbolic_fixed_array_length_inner(field, active)),
            Ty::Sequence(fields) => fields
                .iter()
                .any(|field| self.has_symbolic_fixed_array_length_inner(&field.ty, active)),
            Ty::Union { variants, .. } => variants.iter().any(|variant| {
                variant
                    .fields
                    .iter()
                    .any(|(_, field)| self.has_symbolic_fixed_array_length_inner(field, active))
            }),
            _ => false,
        }
    }

    pub fn derive(&self, ty: &Ty) -> Result<Repr, RepresentationError> {
        self.derive_inner(ty, &mut HashSet::new())
    }

    fn derive_inner(
        &self,
        ty: &Ty,
        active: &mut HashSet<RecursionKey>,
    ) -> Result<Repr, RepresentationError> {
        if self
            .registry
            .is_some_and(|registry| registry.reference_former_for_type(ty).is_some())
        {
            return Ok(Repr::Address { address_space: 0 });
        }

        match ty {
            Ty::Proposition(_) => Ok(Repr::Bits { width: 1 }),
            Ty::Ref { .. } => Ok(Repr::Address { address_space: 0 }),
            Ty::Named { instance, name } => {
                let recursion_key = match self
                    .registry
                    .and_then(|registry| registry.declaration_key(instance))
                {
                    Some(key) => RecursionKey::Declaration(key.clone()),
                    None => RecursionKey::Instance(instance.clone()),
                };
                if !active.insert(recursion_key.clone()) {
                    return Err(RepresentationError::RecursiveRepresentation {
                        type_id: TypeId {
                            file: 0,
                            name: *name,
                        },
                    });
                }
                if let Some(declaration) = self
                    .registry
                    .and_then(|registry| registry.declaration_for_instance(instance))
                    && declaration.reference.is_some()
                {
                    active.remove(&recursion_key);
                    return Ok(Repr::Address { address_space: 0 });
                }
                if let Some(integer) = self
                    .registry
                    .and_then(|registry| registry.declaration_for_instance(instance))
                    .and_then(|declaration| declaration.integer.as_ref())
                {
                    if self
                        .registry
                        .is_some_and(|registry| registry.is_target_dependent_integer(ty))
                    {
                        return Err(RepresentationError::TargetDependentInteger {
                            type_name: name.to_string(),
                        });
                    }
                    let representation = integer
                        .validity
                        .resolve_runtime_representation(&integer.representation_rule)
                        .map_err(|error| RepresentationError::InvalidInteger {
                            type_name: name.to_string(),
                            code: error.diagnostic_code(),
                        })?;
                    active.remove(&recursion_key);
                    return Ok(Repr::Bits {
                        width: representation.width().get(),
                    });
                }
                if let Some(discriminant) = self
                    .registry
                    .and_then(|registry| registry.discriminant_semantics_for_type(ty))
                {
                    let width = discriminant
                        .validity
                        .resolve_runtime_representation(&discriminant.representation_rule)
                        .map_err(|error| RepresentationError::InvalidInteger {
                            type_name: name.to_string(),
                            code: error.diagnostic_code(),
                        })?
                        .width()
                        .get();
                    let definition = self.registry.unwrap().resolved_definition_for_type(ty);
                    let mut repr = self.derive_inner(&definition, active)?;
                    if let Repr::Sum {
                        discriminant_width, ..
                    } = &mut repr
                    {
                        *discriminant_width = Some(width);
                    }
                    active.remove(&recursion_key);
                    return Ok(repr);
                }
                let definition = self
                    .registry
                    .and_then(|registry| registry.declaration_for_instance(instance))
                    .map(|declaration| &declaration.definition)
                    .ok_or_else(|| RepresentationError::Unsupported {
                        type_name: name.to_string(),
                    })?;
                if let Ty::Opaque(actual_name) = definition
                    && actual_name == name
                {
                    return Err(RepresentationError::RecursiveRepresentation {
                        type_id: instance.declaration,
                    });
                }
                if let Some(former) = self
                    .registry
                    .and_then(|registry| registry.declaration_for_instance(instance))
                    .and_then(|declaration| declaration.sequence_literal.as_ref())
                {
                    let Some(length_parameter) = former.fixed_array_length_parameter() else {
                        return Err(RepresentationError::Unsupported {
                            type_name: name.to_string(),
                        });
                    };
                    let element = instance.arguments.iter().find_map(|(parameter, argument)| {
                        (*parameter == former.element_parameter).then_some(argument)
                    });
                    let length = instance.arguments.iter().find_map(|(parameter, argument)| {
                        (*parameter == length_parameter).then_some(argument)
                    });
                    let Some(TyArg::Type(element)) = element else {
                        return Err(RepresentationError::InvalidArrayLength {
                            type_name: name.to_string(),
                        });
                    };
                    let element = self.derive_inner(element, active)?;
                    let result = match fixed_array_length_argument(length) {
                        FixedArrayLengthArgument::Concrete(length) => Ok(Repr::Array {
                            element: Box::new(element),
                            length,
                        }),
                        FixedArrayLengthArgument::Dependent(_)
                        | FixedArrayLengthArgument::WrongKind
                        | FixedArrayLengthArgument::Missing
                        | FixedArrayLengthArgument::Invalid(_) => {
                            Err(RepresentationError::InvalidArrayLength {
                                type_name: name.to_string(),
                            })
                        }
                    };
                    active.remove(&recursion_key);
                    return result;
                }
                let type_substitution = instance
                    .arguments
                    .iter()
                    .filter_map(|(parameter, argument)| match argument {
                        TyArg::Type(ty) => Some((*parameter, (**ty).clone())),
                        TyArg::Const(_) => None,
                    })
                    .collect::<HashMap<_, _>>();
                let substituted = definition.substitute(&type_substitution);
                let result = self.derive_inner(&substituted, active);
                active.remove(&recursion_key);
                result.map_err(|error| match error {
                    RepresentationError::Unsupported { .. } => RepresentationError::Unsupported {
                        type_name: name.to_string(),
                    },
                    other => other,
                })
            }
            Ty::AnonymousInteger { validity } => {
                let representation = validity
                    .resolve_runtime_representation(
                        &ast::integer::IntegerRepresentationRule::InferFromValidity,
                    )
                    .map_err(|error| RepresentationError::InvalidInteger {
                        type_name: ty.format_for_hover(),
                        code: error.diagnostic_code(),
                    })?;
                Ok(Repr::Bits {
                    width: representation.width().get(),
                })
            }
            Ty::ResultFamily { alternatives, .. } => Ok(Repr::Sum {
                discriminant_width: None,
                variants: alternatives
                    .iter()
                    .map(|_| Repr::Product { fields: Vec::new() })
                    .collect(),
            }),
            Ty::Ptr { .. } | Ty::OwnedStorage { .. } | Ty::Address { .. } => Ok(Repr::Address {
                address_space: ty.address_space().unwrap_or(0),
            }),
            Ty::Record { fields, .. } => fields
                .iter()
                .map(|(_, field)| self.derive_inner(field, active))
                .collect::<Result<Vec<_>, _>>()
                .map(|fields| Repr::Product { fields }),
            Ty::Sequence(fields) => {
                if let Some((element, length)) = Repr::array_parts(ty, self.registry) {
                    Ok(Repr::Array {
                        element: Box::new(self.derive_inner(&element, active)?),
                        length,
                    })
                } else {
                    fields
                        .iter()
                        .map(|field| self.derive_inner(&field.ty, active))
                        .collect::<Result<Vec<_>, _>>()
                        .map(|fields| Repr::Product { fields })
                }
            }
            Ty::Unit => Ok(Repr::Product { fields: Vec::new() }),
            Ty::Union { variants, .. } => {
                let discriminant_width = ty
                    .union_discriminant_domain()
                    .map(|domain| {
                        ast::integer::IntegerValidity::new(domain)
                            .resolve_runtime_representation(
                                &ast::integer::IntegerRepresentationRule::InferFromValidity,
                            )
                            .map(|representation| representation.width().get())
                            .map_err(|error| RepresentationError::InvalidInteger {
                                type_name: ty.format_for_hover(),
                                code: error.diagnostic_code(),
                            })
                    })
                    .transpose()?;
                let variants = variants
                    .iter()
                    .map(|variant| {
                        let fields = variant
                            .fields
                            .iter()
                            .map(|(_, field)| self.derive_inner(field, active))
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(Repr::Product { fields })
                    })
                    .collect::<Result<Vec<_>, RepresentationError>>()?;
                Ok(Repr::Sum {
                    variants,
                    discriminant_width,
                })
            }
            Ty::Array { elem, size } => {
                let ArrayLength::Concrete(length) = array_length(size) else {
                    return Err(RepresentationError::InvalidArrayLength {
                        type_name: ty.format_for_hover(),
                    });
                };
                Ok(Repr::Array {
                    element: Box::new(self.derive_inner(elem, active)?),
                    length,
                })
            }
            Ty::UnresolvedLiteral(_)
            | Ty::UnresolvedSequence { .. }
            | Ty::Float { .. }
            | Ty::Opaque(_)
            | Ty::Literal(_) => Err(RepresentationError::Unsupported {
                type_name: ty.format_for_hover(),
            }),
        }
    }
}

fn array_length(expr: &NormalExpr) -> ArrayLength {
    let normalized = expr.normalize();
    match concrete_array_length_i128(&normalized) {
        Some(value) if value >= 0 => u64::try_from(value)
            .map(ArrayLength::Concrete)
            .unwrap_or(ArrayLength::Symbolic(normalized)),
        _ => ArrayLength::Symbolic(normalized),
    }
}

fn concrete_array_length_i128(expr: &NormalExpr) -> Option<i128> {
    match expr {
        NormalExpr::Value(ast::ConstValue::Int(value))
            if *value >= i256::I256::from(i128::MIN) && *value <= i256::I256::from(i128::MAX) =>
        {
            Some(value.as_i128())
        }
        NormalExpr::Add(lhs, rhs) => checked_i128_add(
            concrete_array_length_i128(lhs),
            concrete_array_length_i128(rhs),
        ),
        NormalExpr::Sub(lhs, rhs) => checked_i128_sub(
            concrete_array_length_i128(lhs),
            concrete_array_length_i128(rhs),
        ),
        NormalExpr::Mul(lhs, rhs) => checked_i128_mul(
            concrete_array_length_i128(lhs),
            concrete_array_length_i128(rhs),
        ),
        NormalExpr::Value(_)
        | NormalExpr::Var(_)
        | NormalExpr::Inferred(_)
        | NormalExpr::TargetQuery { .. } => None,
    }
}

fn checked_i128_add(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_add(right),
        _ => None,
    }
}

fn checked_i128_sub(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_sub(right),
        _ => None,
    }
}

fn checked_i128_mul(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_mul(right),
        _ => None,
    }
}
