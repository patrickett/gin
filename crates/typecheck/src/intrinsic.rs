use ast::ConstValue;
use diagnostic::Diagnostic;
use i256::I256;

use crate::representation::Repr;
use crate::ty::Ty;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComparisonPredicate {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegerComparison {
    Signed,
    Unsigned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicOp {
    BitsAdd,
    BitsSubtract,
    BitsMultiply,
    BitsSignedDivide,
    BitsUnsignedDivide,
    BitsSignedRemainder,
    BitsUnsignedRemainder,
    BitsAnd,
    BitsOr,
    BitsXor,
    BitsShiftLeft,
    BitsShiftRightLogical,
    BitsShiftRightArithmetic,
    BitsExtendSigned,
    BitsExtendUnsigned,
    BitsTruncate,
    BitsCompare {
        interpretation: IntegerComparison,
        predicate: ComparisonPredicate,
    },
    AddressLoad,
    AddressStore,
    AddressOffset,
    AddressToBits,
    BitsToAddress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrinsicContract {
    SameWidthBinary,
    Extension,
    Truncation,
    Comparison,
    AddressLoad,
    AddressStore,
    AddressOffset,
    AddressConversion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrinsicRuntimePrecondition {
    None,
    NonZeroDivisor,
    NonZeroDivisorAndNoSignedOverflow,
    RawAddressValidity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntrinsicValidationError {
    WrongArity { expected: usize, actual: usize },
    NonBitsOperand { index: usize },
    NonBitsResult,
    MismatchedBinaryWidths { operands: Vec<u32>, result: u32 },
    MismatchedOperandWidths { operands: Vec<u32> },
    InvalidComparisonResult,
    InvalidExtension { source: u32, result: u32 },
    InvalidTruncation { source: u32, result: u32 },
    UnsupportedWidth { width: u32 },
    DivisionByZero,
    SignedDivisionOverflow,
    NonAddressOperand { index: usize },
    NonAddressResult,
    PointeeMismatch,
    InvalidAddressOffsetIndex,
    InvalidAddressConversionWidth { width: u32 },
    UnsupportedAddressSpace { address_space: u32 },
    PointerTypeMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntrinsicResolutionError {
    pub name: String,
}

impl IntrinsicResolutionError {
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic::new(
            "type-unknown-intrinsic",
            format!("unknown intrinsic `{}`", self.name),
        )
        .with_arg("name", self.name.clone())
    }
}

const MAX_SUPPORTED_ADDRESS_SPACE: u32 = u8::MAX as u32;

fn validate_address_space(ty: &Ty) -> Result<(), IntrinsicValidationError> {
    let Some(address_space) = ty.address_space() else {
        return Ok(());
    };
    if address_space > MAX_SUPPORTED_ADDRESS_SPACE {
        return Err(IntrinsicValidationError::UnsupportedAddressSpace { address_space });
    }
    Ok(())
}

impl IntrinsicOp {
    pub const ALL: [Self; 31] = [
        Self::BitsAdd,
        Self::BitsSubtract,
        Self::BitsMultiply,
        Self::BitsSignedDivide,
        Self::BitsUnsignedDivide,
        Self::BitsSignedRemainder,
        Self::BitsUnsignedRemainder,
        Self::BitsAnd,
        Self::BitsOr,
        Self::BitsXor,
        Self::BitsShiftLeft,
        Self::BitsShiftRightLogical,
        Self::BitsShiftRightArithmetic,
        Self::BitsExtendSigned,
        Self::BitsExtendUnsigned,
        Self::BitsTruncate,
        Self::BitsCompare {
            interpretation: IntegerComparison::Signed,
            predicate: ComparisonPredicate::Less,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Signed,
            predicate: ComparisonPredicate::LessOrEqual,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Signed,
            predicate: ComparisonPredicate::Greater,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Signed,
            predicate: ComparisonPredicate::GreaterOrEqual,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Signed,
            predicate: ComparisonPredicate::Equal,
        },
        Self::AddressLoad,
        Self::AddressStore,
        Self::AddressOffset,
        Self::AddressToBits,
        Self::BitsToAddress,
        Self::BitsCompare {
            interpretation: IntegerComparison::Unsigned,
            predicate: ComparisonPredicate::Less,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Unsigned,
            predicate: ComparisonPredicate::LessOrEqual,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Unsigned,
            predicate: ComparisonPredicate::Greater,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Unsigned,
            predicate: ComparisonPredicate::GreaterOrEqual,
        },
        Self::BitsCompare {
            interpretation: IntegerComparison::Unsigned,
            predicate: ComparisonPredicate::Equal,
        },
    ];

    pub fn resolve(name: &str) -> Result<Self, IntrinsicResolutionError> {
        Self::ALL
            .into_iter()
            .find(|op| op.name() == name)
            .ok_or_else(|| IntrinsicResolutionError {
                name: name.to_string(),
            })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::BitsAdd => "BitsAdd",
            Self::BitsSubtract => "BitsSubtract",
            Self::BitsMultiply => "BitsMultiply",
            Self::BitsSignedDivide => "BitsSignedDivide",
            Self::BitsUnsignedDivide => "BitsUnsignedDivide",
            Self::BitsSignedRemainder => "BitsSignedRemainder",
            Self::BitsUnsignedRemainder => "BitsUnsignedRemainder",
            Self::BitsAnd => "BitsAnd",
            Self::BitsOr => "BitsOr",
            Self::BitsXor => "BitsXor",
            Self::BitsShiftLeft => "BitsShiftLeft",
            Self::BitsShiftRightLogical => "BitsShiftRightLogical",
            Self::BitsShiftRightArithmetic => "BitsShiftRightArithmetic",
            Self::BitsExtendSigned => "BitsExtendSigned",
            Self::BitsExtendUnsigned => "BitsExtendUnsigned",
            Self::BitsTruncate => "BitsTruncate",
            Self::BitsCompare {
                interpretation,
                predicate,
            } => match (interpretation, predicate) {
                (IntegerComparison::Signed, ComparisonPredicate::Less) => "BitsSignedLess",
                (IntegerComparison::Signed, ComparisonPredicate::LessOrEqual) => {
                    "BitsSignedLessOrEqual"
                }
                (IntegerComparison::Signed, ComparisonPredicate::Greater) => "BitsSignedGreater",
                (IntegerComparison::Signed, ComparisonPredicate::GreaterOrEqual) => {
                    "BitsSignedGreaterOrEqual"
                }
                (IntegerComparison::Signed, ComparisonPredicate::Equal) => "BitsSignedEqual",
                (IntegerComparison::Unsigned, ComparisonPredicate::Less) => "BitsUnsignedLess",
                (IntegerComparison::Unsigned, ComparisonPredicate::LessOrEqual) => {
                    "BitsUnsignedLessOrEqual"
                }
                (IntegerComparison::Unsigned, ComparisonPredicate::Greater) => {
                    "BitsUnsignedGreater"
                }
                (IntegerComparison::Unsigned, ComparisonPredicate::GreaterOrEqual) => {
                    "BitsUnsignedGreaterOrEqual"
                }
                (IntegerComparison::Unsigned, ComparisonPredicate::Equal) => "BitsUnsignedEqual",
            },
            Self::AddressLoad => "AddressLoad",
            Self::AddressStore => "AddressStore",
            Self::AddressOffset => "AddressOffset",
            Self::AddressToBits => "AddressToBits",
            Self::BitsToAddress => "BitsToAddress",
        }
    }

    pub const fn contract(self) -> IntrinsicContract {
        match self {
            Self::BitsExtendSigned | Self::BitsExtendUnsigned => IntrinsicContract::Extension,
            Self::BitsTruncate => IntrinsicContract::Truncation,
            Self::BitsCompare { .. } => IntrinsicContract::Comparison,
            Self::AddressLoad => IntrinsicContract::AddressLoad,
            Self::AddressStore => IntrinsicContract::AddressStore,
            Self::AddressOffset => IntrinsicContract::AddressOffset,
            Self::AddressToBits | Self::BitsToAddress => IntrinsicContract::AddressConversion,
            _ => IntrinsicContract::SameWidthBinary,
        }
    }

    pub const fn runtime_precondition(self) -> IntrinsicRuntimePrecondition {
        match self {
            Self::BitsSignedDivide | Self::BitsSignedRemainder => {
                IntrinsicRuntimePrecondition::NonZeroDivisorAndNoSignedOverflow
            }
            Self::BitsUnsignedDivide | Self::BitsUnsignedRemainder => {
                IntrinsicRuntimePrecondition::NonZeroDivisor
            }
            Self::AddressLoad | Self::AddressStore | Self::AddressOffset => {
                IntrinsicRuntimePrecondition::RawAddressValidity
            }
            _ => IntrinsicRuntimePrecondition::None,
        }
    }

    pub fn validate_signature(
        self,
        operands: &[Ty],
        result: &Ty,
        registry: Option<&crate::TypeRegistry>,
    ) -> Result<(), IntrinsicValidationError> {
        let expected = match self.contract() {
            IntrinsicContract::SameWidthBinary | IntrinsicContract::Comparison => 2,
            IntrinsicContract::Extension | IntrinsicContract::Truncation => 1,
            IntrinsicContract::AddressLoad | IntrinsicContract::AddressConversion => 1,
            IntrinsicContract::AddressStore | IntrinsicContract::AddressOffset => 2,
        };
        if operands.len() != expected {
            return Err(IntrinsicValidationError::WrongArity {
                expected,
                actual: operands.len(),
            });
        }
        if matches!(
            self.contract(),
            IntrinsicContract::AddressLoad
                | IntrinsicContract::AddressStore
                | IntrinsicContract::AddressOffset
                | IntrinsicContract::AddressConversion
        ) {
            return self.validate_address_signature(operands, result, registry);
        }
        let widths: Vec<u32> = operands
            .iter()
            .enumerate()
            .map(|(index, ty)| {
                Self::width(ty, registry).ok_or(IntrinsicValidationError::NonBitsOperand { index })
            })
            .collect::<Result<_, _>>()?;
        for width in widths.iter().copied() {
            if !(1..=128).contains(&width) {
                return Err(IntrinsicValidationError::UnsupportedWidth { width });
            }
        }
        if matches!(self.contract(), IntrinsicContract::Comparison) {
            if widths[0] != widths[1] {
                return Err(IntrinsicValidationError::MismatchedOperandWidths { operands: widths });
            }
            let integer_truth_domain = registry
                .and_then(|registry| registry.integer_validity_for_type(result))
                .or_else(|| result.anonymous_integer_validity())
                .is_some_and(|validity| {
                    validity.domain().contains(I256::from(0))
                        && validity.domain().contains(I256::from(1))
                });
            let structural_truth_domain = matches!(
                result,
                Ty::ResultFamily { alternatives, .. }
                    if alternatives.as_slice()
                        == [
                            ast::ResultAlternative {
                                label: internment::Intern::from_ref("False"),
                                proposition: None,
                            },
                            ast::ResultAlternative {
                                label: internment::Intern::from_ref("True"),
                                proposition: None,
                            },
                        ]
            );
            if !integer_truth_domain && !structural_truth_domain {
                return Err(IntrinsicValidationError::InvalidComparisonResult);
            }
            return Ok(());
        }
        let result_width =
            Self::width(result, registry).ok_or(IntrinsicValidationError::NonBitsResult)?;
        if !(1..=128).contains(&result_width) {
            return Err(IntrinsicValidationError::UnsupportedWidth {
                width: result_width,
            });
        }
        match self.contract() {
            IntrinsicContract::SameWidthBinary
                if widths.iter().any(|width| *width != result_width) =>
            {
                Err(IntrinsicValidationError::MismatchedBinaryWidths {
                    operands: widths,
                    result: result_width,
                })
            }
            IntrinsicContract::Extension if result_width <= widths[0] => {
                Err(IntrinsicValidationError::InvalidExtension {
                    source: widths[0],
                    result: result_width,
                })
            }
            IntrinsicContract::Truncation if result_width >= widths[0] => {
                Err(IntrinsicValidationError::InvalidTruncation {
                    source: widths[0],
                    result: result_width,
                })
            }
            _ => Ok(()),
        }
    }

    fn validate_address_signature(
        self,
        operands: &[Ty],
        result: &Ty,
        registry: Option<&crate::TypeRegistry>,
    ) -> Result<(), IntrinsicValidationError> {
        match self {
            Self::AddressLoad => {
                validate_address_space(&operands[0])?;
                let pointee = operands[0]
                    .pointee_ty()
                    .ok_or(IntrinsicValidationError::NonAddressOperand { index: 0 })?;
                if result != pointee {
                    return Err(IntrinsicValidationError::PointeeMismatch);
                }
            }
            Self::AddressStore => {
                validate_address_space(&operands[0])?;
                let pointee = operands[0]
                    .pointee_ty()
                    .ok_or(IntrinsicValidationError::NonAddressOperand { index: 0 })?;
                if operands[1] != *pointee {
                    return Err(IntrinsicValidationError::PointeeMismatch);
                }
            }
            Self::AddressOffset => {
                validate_address_space(&operands[0])?;
                validate_address_space(result)?;
                if operands[0].pointee_ty().is_none() {
                    return Err(IntrinsicValidationError::NonAddressOperand { index: 0 });
                }
                if Self::width(&operands[1], registry).is_none() {
                    return Err(IntrinsicValidationError::InvalidAddressOffsetIndex);
                }
                if result != &operands[0] {
                    return Err(IntrinsicValidationError::PointerTypeMismatch);
                }
            }
            Self::AddressToBits => {
                validate_address_space(&operands[0])?;
                if operands[0].address_space().is_none() {
                    return Err(IntrinsicValidationError::NonAddressOperand { index: 0 });
                }
                let width =
                    Self::width(result, registry).ok_or(IntrinsicValidationError::NonBitsResult)?;
                if !matches!(width, 32 | 64) {
                    return Err(IntrinsicValidationError::InvalidAddressConversionWidth { width });
                }
            }
            Self::BitsToAddress => {
                validate_address_space(result)?;
                let width = Self::width(&operands[0], registry)
                    .ok_or(IntrinsicValidationError::NonBitsOperand { index: 0 })?;
                if !matches!(width, 32 | 64) {
                    return Err(IntrinsicValidationError::InvalidAddressConversionWidth { width });
                }
                if result.address_space().is_none() {
                    return Err(IntrinsicValidationError::NonAddressResult);
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn diagnostic(self, error: &IntrinsicValidationError) -> Diagnostic {
        let code = match error {
            IntrinsicValidationError::WrongArity { .. } => "type-intrinsic-wrong-arity",
            IntrinsicValidationError::NonBitsOperand { .. }
            | IntrinsicValidationError::NonBitsResult => "type-intrinsic-non-bits",
            IntrinsicValidationError::MismatchedBinaryWidths { .. } => {
                "type-intrinsic-mismatched-widths"
            }
            IntrinsicValidationError::MismatchedOperandWidths { .. } => {
                "type-intrinsic-mismatched-widths"
            }
            IntrinsicValidationError::InvalidComparisonResult => {
                "type-intrinsic-invalid-comparison-result"
            }
            IntrinsicValidationError::InvalidExtension { .. } => "type-intrinsic-invalid-extension",
            IntrinsicValidationError::InvalidTruncation { .. } => {
                "type-intrinsic-invalid-truncation"
            }
            IntrinsicValidationError::UnsupportedWidth { .. } => "type-intrinsic-unsupported-width",
            IntrinsicValidationError::DivisionByZero => "type-intrinsic-division-by-zero",
            IntrinsicValidationError::SignedDivisionOverflow => {
                "type-intrinsic-signed-division-overflow"
            }
            IntrinsicValidationError::NonAddressOperand { .. }
            | IntrinsicValidationError::NonAddressResult => "type-intrinsic-non-address",
            IntrinsicValidationError::PointeeMismatch => "type-raw-address-pointee-mismatch",
            IntrinsicValidationError::InvalidAddressOffsetIndex => {
                "type-invalid-pointer-offset-index"
            }
            IntrinsicValidationError::InvalidAddressConversionWidth { .. } => {
                "type-invalid-address-conversion-width"
            }
            IntrinsicValidationError::UnsupportedAddressSpace { .. } => {
                "type-unsupported-address-space"
            }
            IntrinsicValidationError::PointerTypeMismatch => "type-pointer-type-mismatch",
        };
        let diagnostic = Diagnostic::new(
            code,
            format!("invalid {} intrinsic: {error:?}", self.name()),
        )
        .with_arg("intrinsic", self.name());
        match error {
            IntrinsicValidationError::WrongArity { expected, actual } => diagnostic
                .with_arg("expected_arity", expected.to_string())
                .with_arg("actual_arity", actual.to_string()),
            IntrinsicValidationError::NonBitsOperand { index } => {
                diagnostic.with_arg("operand", index.to_string())
            }
            IntrinsicValidationError::MismatchedBinaryWidths { operands, result } => diagnostic
                .with_arg("operand_widths", format!("{operands:?}"))
                .with_arg("result_width", result.to_string()),
            IntrinsicValidationError::MismatchedOperandWidths { operands } => {
                diagnostic.with_arg("operand_widths", format!("{operands:?}"))
            }
            IntrinsicValidationError::InvalidExtension { source, result }
            | IntrinsicValidationError::InvalidTruncation { source, result } => diagnostic
                .with_arg("source_width", source.to_string())
                .with_arg("result_width", result.to_string()),
            IntrinsicValidationError::UnsupportedWidth { width } => {
                diagnostic.with_arg("width", width.to_string())
            }
            IntrinsicValidationError::NonBitsResult
            | IntrinsicValidationError::InvalidComparisonResult
            | IntrinsicValidationError::DivisionByZero
            | IntrinsicValidationError::SignedDivisionOverflow
            | IntrinsicValidationError::NonAddressResult
            | IntrinsicValidationError::PointeeMismatch
            | IntrinsicValidationError::InvalidAddressOffsetIndex
            | IntrinsicValidationError::PointerTypeMismatch => diagnostic,
            IntrinsicValidationError::NonAddressOperand { index } => {
                diagnostic.with_arg("operand", index.to_string())
            }
            IntrinsicValidationError::InvalidAddressConversionWidth { width } => {
                diagnostic.with_arg("width", width.to_string())
            }
            IntrinsicValidationError::UnsupportedAddressSpace { address_space } => {
                diagnostic.with_arg("address_space", address_space.to_string())
            }
        }
    }

    pub fn fold(
        self,
        args: &[ConstValue],
        source_width: u32,
        result_width: u32,
    ) -> Result<Option<ConstValue>, IntrinsicValidationError> {
        if !matches!(
            self,
            Self::BitsAdd
                | Self::BitsSubtract
                | Self::BitsMultiply
                | Self::BitsSignedDivide
                | Self::BitsUnsignedDivide
                | Self::BitsSignedRemainder
                | Self::BitsUnsignedRemainder
                | Self::BitsAnd
                | Self::BitsOr
                | Self::BitsXor
                | Self::BitsShiftLeft
                | Self::BitsShiftRightLogical
                | Self::BitsShiftRightArithmetic
                | Self::BitsExtendSigned
                | Self::BitsExtendUnsigned
                | Self::BitsTruncate
                | Self::BitsCompare { .. }
        ) {
            return Ok(None);
        }
        let values: Vec<u128> = args
            .iter()
            .map(|value| match value {
                ConstValue::Int(value) => {
                    ast::integer::to_u128(*value).map(|value| value & Self::mask(source_width))
                }
                _ => None,
            })
            .collect::<Option<_>>()
            .unwrap_or_default();
        if values.len() != args.len() {
            return Ok(None);
        }
        let mask = Self::mask(result_width);
        let binary = |f: fn(u128, u128) -> u128| (f(values[0], values[1]) & mask) as i128;
        let value = match self {
            Self::BitsAdd => binary(u128::wrapping_add),
            Self::BitsSubtract => binary(u128::wrapping_sub),
            Self::BitsMultiply => binary(u128::wrapping_mul),
            Self::BitsAnd => binary(|a, b| a & b),
            Self::BitsOr => binary(|a, b| a | b),
            Self::BitsXor => binary(|a, b| a ^ b),
            Self::BitsShiftLeft => {
                ((values[0] << (values[1] % u128::from(source_width))) & mask) as i128
            }
            Self::BitsShiftRightLogical => {
                (values[0] >> (values[1] % u128::from(source_width))) as i128
            }
            Self::BitsShiftRightArithmetic => {
                let shift = (values[1] % u128::from(source_width)) as u32;
                (Self::signed(values[0], source_width) >> shift) as u128 as i128
            }
            Self::BitsUnsignedDivide | Self::BitsUnsignedRemainder => {
                if values[1] == 0 {
                    return Err(IntrinsicValidationError::DivisionByZero);
                }
                if matches!(self, Self::BitsUnsignedDivide) {
                    (values[0] / values[1]) as i128
                } else {
                    (values[0] % values[1]) as i128
                }
            }
            Self::BitsSignedDivide | Self::BitsSignedRemainder => {
                let lhs = Self::signed(values[0], source_width);
                let rhs = Self::signed(values[1], source_width);
                if rhs == 0 {
                    return Err(IntrinsicValidationError::DivisionByZero);
                }
                let minimum = if source_width == 128 {
                    i128::MIN
                } else {
                    -(1_i128 << (source_width - 1))
                };
                if lhs == minimum && rhs == -1 {
                    return Err(IntrinsicValidationError::SignedDivisionOverflow);
                }
                if matches!(self, Self::BitsSignedDivide) {
                    (lhs / rhs) as u128 as i128
                } else {
                    (lhs % rhs) as u128 as i128
                }
            }
            Self::BitsExtendSigned => Self::signed(values[0], source_width) as u128 as i128,
            Self::BitsExtendUnsigned | Self::BitsTruncate => (values[0] & mask) as i128,
            Self::BitsCompare {
                interpretation,
                predicate,
            } => {
                let compared = match interpretation {
                    IntegerComparison::Signed => {
                        let lhs = Self::signed(values[0], source_width);
                        let rhs = Self::signed(values[1], source_width);
                        predicate.compare(lhs, rhs)
                    }
                    IntegerComparison::Unsigned => predicate.compare(values[0], values[1]),
                };
                i128::from(compared)
            }
            Self::AddressLoad
            | Self::AddressStore
            | Self::AddressOffset
            | Self::AddressToBits
            | Self::BitsToAddress => return Ok(None),
        };
        Ok(Some(ConstValue::Int(I256::from(value as u128 & mask))))
    }

    fn width(ty: &Ty, registry: Option<&crate::TypeRegistry>) -> Option<u32> {
        match Repr::derive(ty, registry) {
            Ok(Repr::Bits { width }) => Some(width),
            _ => None,
        }
    }

    const fn mask(width: u32) -> u128 {
        if width == 128 {
            u128::MAX
        } else {
            (1_u128 << width) - 1
        }
    }

    fn signed(bits: u128, width: u32) -> i128 {
        if width == 128 {
            bits as i128
        } else {
            let shift = 128 - width;
            ((bits << shift) as i128) >> shift
        }
    }
}

impl ComparisonPredicate {
    fn compare<T: Ord>(self, lhs: T, rhs: T) -> bool {
        match self {
            Self::Less => lhs < rhs,
            Self::LessOrEqual => lhs <= rhs,
            Self::Greater => lhs > rhs,
            Self::GreaterOrEqual => lhs >= rhs,
            Self::Equal => lhs == rhs,
        }
    }
}
