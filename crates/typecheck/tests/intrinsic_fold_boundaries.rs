use ast::ConstValue;
use typecheck::intrinsic::{IntrinsicContract, IntrinsicOp, IntrinsicValidationError};

#[test]
fn scalar_intrinsic_folding_rejects_wrong_argument_counts() {
    for op in IntrinsicOp::ALL {
        let expected = match op.contract() {
            IntrinsicContract::Extension | IntrinsicContract::Truncation => 1,
            IntrinsicContract::SameWidthBinary | IntrinsicContract::Comparison => 2,
            _ => continue,
        };
        for actual in [0, expected + 1] {
            assert_eq!(
                op.fold(&vec![ConstValue::ONE; actual], 8, 8),
                Err(IntrinsicValidationError::WrongArity { expected, actual }),
                "{} with {actual} arguments",
                op.name()
            );
        }
    }
}

#[test]
fn scalar_intrinsic_folding_rejects_invalid_widths() {
    for (source, result, invalid) in [
        (0, 8, 0),
        (129, 8, 129),
        (u32::MAX, 8, u32::MAX),
        (8, 0, 0),
        (8, 129, 129),
        (8, u32::MAX, u32::MAX),
    ] {
        assert_eq!(
            IntrinsicOp::BitsAdd.fold(&[ConstValue::ONE, ConstValue::ONE], source, result),
            Err(IntrinsicValidationError::UnsupportedWidth { width: invalid })
        );
    }
}

#[test]
fn valid_width_boundaries_and_signed_extension_still_fold() {
    for (width, expected) in [(1, 0), (128, 2)] {
        assert_eq!(
            IntrinsicOp::BitsAdd.fold(&[ConstValue::ONE, ConstValue::ONE], width, width),
            Ok(Some(ConstValue::Int(expected.into())))
        );
    }
    assert_eq!(
        IntrinsicOp::BitsExtendSigned.fold(&[ConstValue::Int(255.into())], 8, 16),
        Ok(Some(ConstValue::Int(65535.into())))
    );
}

#[test]
fn unsupported_operations_and_values_remain_unfolded() {
    assert_eq!(IntrinsicOp::AddressLoad.fold(&[], 0, 0), Ok(None));
    assert_eq!(
        IntrinsicOp::BitsAdd.fold(&[ConstValue::Text("a".into()), ConstValue::ONE], 8, 8),
        Ok(None)
    );
}

#[test]
fn raw_bit_arithmetic_retains_its_declared_width_and_interpretation() {
    for (op, left, right, source, result, expected) in [
        (IntrinsicOp::BitsAdd, 250, 10, 8, 8, 4),
        (IntrinsicOp::BitsSubtract, 0, 1, 8, 8, 255),
        (IntrinsicOp::BitsMultiply, 3, 125, 4, 8, 39),
        (IntrinsicOp::BitsUnsignedDivide, 255, 2, 8, 8, 127),
        (IntrinsicOp::BitsSignedDivide, 255, 2, 8, 8, 0),
    ] {
        assert_eq!(
            op.fold(
                &[ConstValue::Int(left.into()), ConstValue::Int(right.into())],
                source,
                result,
            ),
            Ok(Some(ConstValue::Int(expected.into()))),
        );
    }
}
