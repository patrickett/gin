use diagnostic::code::IntegerCode;
use diagnostic::{Diagnostic, DiagnosticCode};
use i256::I256;

use crate::representation::Repr;
use crate::ty::Ty;
use crate::typed::{ExprId, TypedFileAst};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegerLiteralMaterializationError {
    NonBitsExpected,
    InvalidRepresentation { code: DiagnosticCode },
    MissingIntegerValidity,
    OutsideSymbolicValidity,
    NegativeRejected { min: I256, max: I256 },
    OutsideValidity { min: I256, max: I256 },
}

pub fn materialize(
    typed: &mut TypedFileAst,
    expression: ExprId,
    expected: &Ty,
) -> Result<(), IntegerLiteralMaterializationError> {
    let Some(ast::ConstValue::Int(value)) =
        typed.exprs.facts.const_facts.const_value[expression.as_usize()]
    else {
        return Ok(());
    };
    validate_with_registry(value, expected, &typed.type_registry)?;
    typed.exprs.ty[expression.as_usize()] = expected.clone();
    Ok(())
}

pub fn validate_with_registry(
    value: I256,
    expected: &Ty,
    registry: &crate::TypeRegistry,
) -> Result<(), IntegerLiteralMaterializationError> {
    if registry.is_target_dependent_integer(expected)
        && let Some(validity) = registry.integer_validity_for_type(expected)
    {
        return match validity.domain().membership(value) {
            ast::integer::ContainmentProof::Disproven => {
                Err(IntegerLiteralMaterializationError::OutsideSymbolicValidity)
            }
            ast::integer::ContainmentProof::Proven | ast::integer::ContainmentProof::Unknown => {
                Ok(())
            }
        };
    }
    match Repr::derive(expected, Some(registry)) {
        Ok(_) => {}
        Err(crate::representation::RepresentationError::InvalidInteger { code, .. }) => {
            return Err(IntegerLiteralMaterializationError::InvalidRepresentation { code });
        }
        Err(_) => return Err(IntegerLiteralMaterializationError::NonBitsExpected),
    }
    let validity = registry
        .integer_validity_for_type(expected)
        .or_else(|| expected.anonymous_integer_validity())
        .ok_or(IntegerLiteralMaterializationError::MissingIntegerValidity)?;
    let hull = validity
        .domain()
        .storage_hull()
        .ok_or(IntegerLiteralMaterializationError::MissingIntegerValidity)?;
    if value < I256::from(0) && hull.min() >= I256::from(0) {
        return Err(IntegerLiteralMaterializationError::NegativeRejected {
            min: hull.min(),
            max: hull.max(),
        });
    }
    if !validity.domain().contains(value) {
        return Err(IntegerLiteralMaterializationError::OutsideValidity {
            min: hull.min(),
            max: hull.max(),
        });
    }
    Ok(())
}

pub fn diagnostic(
    value: I256,
    expected: &Ty,
    error: &IntegerLiteralMaterializationError,
) -> Diagnostic {
    match error {
        // type-integer-literal-expected-non-bits
        IntegerLiteralMaterializationError::NonBitsExpected => Diagnostic::new(
            IntegerCode::LiteralExpectedNonBits,
            format!(
                "integer literal `{value}` cannot materialize as `{}`",
                expected.format_for_hover()
            ),
        ),
        IntegerLiteralMaterializationError::InvalidRepresentation { code } => Diagnostic::new(
            *code,
            format!(
                "integer literal `{value}` cannot materialize using `{}` representation",
                expected.format_for_hover()
            ),
        ),
        // type-integer-literal-out-of-range
        IntegerLiteralMaterializationError::OutsideSymbolicValidity => Diagnostic::new(
            IntegerCode::LiteralOutOfRange,
            format!(
                "integer literal `{value}` is outside `{}` validity",
                expected.format_for_hover()
            ),
        ),
        // type-integer-literal-missing-validity
        IntegerLiteralMaterializationError::MissingIntegerValidity => Diagnostic::new(
            IntegerCode::LiteralMissingValidity,
            format!(
                "`{}` has bits representation but no integer validity",
                expected.format_for_hover()
            ),
        ),
        // type-negative-integer-literal-rejected
        IntegerLiteralMaterializationError::NegativeRejected { min, max } => Diagnostic::new(
            IntegerCode::LiteralNegativeRejected,
            format!(
                "negative integer literal `{value}` is outside `{}` bounds {min} to {max}",
                expected.format_for_hover()
            ),
        )
        .with_arg("min", min.to_string())
        .with_arg("max", max.to_string()),
        // type-integer-literal-out-of-range
        IntegerLiteralMaterializationError::OutsideValidity { min, max } => Diagnostic::new(
            IntegerCode::LiteralOutOfRange,
            format!(
                "integer literal `{value}` is outside `{}` bounds {min} to {max}",
                expected.format_for_hover()
            ),
        )
        .with_arg("min", min.to_string())
        .with_arg("max", max.to_string()),
    }
    .with_arg("value", value.to_string())
    .with_arg("expected_type", expected.format_for_hover())
}
