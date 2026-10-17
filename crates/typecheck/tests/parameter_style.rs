mod support;

#[test]
fn parameter_names_repeating_their_type_are_not_type_errors() {
    let typed = support::check(
        "Allocation(element) has\n    value element\n    unwrap(self) element: return self.value\nkeep(element, allocation Allocation(element)) Allocation(element): return allocation\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
