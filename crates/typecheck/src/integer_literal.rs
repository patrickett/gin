use diagnostic::Diagnostic;
use i256::I256;

use crate::representation::Repr;
use crate::ty::Ty;
use crate::typed::{ExprId, TypedFileAst};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegerLiteralMaterializationError {
    NonBitsExpected,
    InvalidRepresentation { code: &'static str },
    MissingIntegerValidity,
    NegativeRejected { min: I256, max: I256 },
    OutsideValidity { min: I256, max: I256 },
}

pub fn materialize(
    typed: &mut TypedFileAst,
    expression: ExprId,
    expected: &Ty,
) -> Result<(), IntegerLiteralMaterializationError> {
    let Some(ast::ConstValue::Int(value)) = typed.exprs.const_value[expression.as_usize()] else {
        return Ok(());
    };
    validate_with_registry(value, expected, &typed.type_registry)?;
    typed.exprs.ty[expression.as_usize()] = expected.clone();
    Ok(())
}

pub fn validate(value: I256, expected: &Ty) -> Result<(), IntegerLiteralMaterializationError> {
    validate_with_optional_registry(value, expected, None)
}

pub fn validate_with_registry(
    value: I256,
    expected: &Ty,
    registry: &crate::TypeRegistry,
) -> Result<(), IntegerLiteralMaterializationError> {
    validate_with_optional_registry(value, expected, Some(registry))
}

fn validate_with_optional_registry(
    value: I256,
    expected: &Ty,
    registry: Option<&crate::TypeRegistry>,
) -> Result<(), IntegerLiteralMaterializationError> {
    match Repr::derive(expected, registry) {
        Ok(_) => {}
        Err(crate::representation::RepresentationError::InvalidInteger { code, .. }) => {
            return Err(IntegerLiteralMaterializationError::InvalidRepresentation { code });
        }
        Err(_) => return Err(IntegerLiteralMaterializationError::NonBitsExpected),
    }
    let validity = registry
        .and_then(|registry| registry.integer_validity_for_type(expected))
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
        IntegerLiteralMaterializationError::NonBitsExpected => Diagnostic::new(
            "type-integer-literal-expected-non-bits",
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
        IntegerLiteralMaterializationError::MissingIntegerValidity => Diagnostic::new(
            "type-integer-literal-missing-validity",
            format!(
                "`{}` has bits representation but no integer validity",
                expected.format_for_hover()
            ),
        ),
        IntegerLiteralMaterializationError::NegativeRejected { min, max } => Diagnostic::new(
            "type-negative-integer-literal-rejected",
            format!(
                "negative integer literal `{value}` is outside `{}` bounds {min}...{max}",
                expected.format_for_hover()
            ),
        )
        .with_arg("min", min.to_string())
        .with_arg("max", max.to_string()),
        IntegerLiteralMaterializationError::OutsideValidity { min, max } => Diagnostic::new(
            "type-integer-literal-out-of-range",
            format!(
                "integer literal `{value}` is outside `{}` bounds {min}...{max}",
                expected.format_for_hover()
            ),
        )
        .with_arg("min", min.to_string())
        .with_arg("max", max.to_string()),
    }
    .with_arg("value", value.to_string())
    .with_arg("expected_type", expected.format_for_hover())
}
