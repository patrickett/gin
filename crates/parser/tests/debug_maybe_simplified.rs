use parser::parse_source_full;

#[test]
fn debug_maybe_union_parse() {
    let src = "Maybe[x] is Some(x) or None\n";
    let out = parse_source_full(src);
    println!("{:#?}", out.symptoms);
    assert!(out.symptoms.is_empty(), "should parse without errors");
}
