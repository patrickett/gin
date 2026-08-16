use ast::ty::Ty;
use i256::I256;

#[test]
fn bounded_assignability_uses_validity() {
    let narrow = Ty::bounded_int(I256::from_i128(2), I256::from_i128(4));
    let wide = Ty::bounded_int(I256::from_i128(1), I256::from_i128(7));

    assert!(narrow.int_assignable_to(&wide));
    assert!(!wide.int_assignable_to(&narrow));
    assert!(wide.check_int_in_bounds(7).is_ok());
    assert!(wide.check_int_in_bounds(8).is_err());
}

#[test]
fn bounded_int_widths_match_expected_values() {
    let values = [
        (0, 1, 1),
        (0, 2, 2),
        (0, 255, 8),
        (0, 256, 9),
        (250, 260, 9),
        (-1, 0, 1),
        (-1, 1, 2),
        (-1, 127, 8),
        (-1, 128, 9),
        (-1, 255, 9),
        (-1, 256, 10),
    ];

    for (min, max, width) in values {
        let ty = Ty::bounded_int(I256::from_i128(min), I256::from_i128(max));
        assert!(matches!(&ty, Ty::AnonymousInteger { .. }));
        assert_eq!(
            ty.anonymous_integer_width(),
            Some(width),
            "width for {min}...{max}"
        );
    }
}

#[test]
fn bounded_int_does_not_project_wide_i256_validity_through_i128() {
    let max = (I256::from(1) << 199) - I256::from(1);
    let ty = Ty::bounded_int(I256::from(0), max);
    let validity = ty.anonymous_integer_validity().unwrap();

    assert_eq!(validity.domain().storage_hull().unwrap().max(), max);
    assert_eq!(ty.int_bounds(), None);
}
