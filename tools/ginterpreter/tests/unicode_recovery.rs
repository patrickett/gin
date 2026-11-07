use ginterpreter::check::check_text;

#[test]
fn independent_flaws_keep_their_original_offsets_after_unicode_is_removed() {
    for comment in ["-- ordinary", "-- hé 😀😀😀"] {
        let source = format!(
            "first() in 0 to 9:\n    {comment}\n    return 10\n\
             second() in 0 to 9:\n    {comment}\n    return 10\n\
             third() in 0 to 9:\n    {comment}\n    return 10\n"
        );
        let checked = check_text("<edit>", &source);
        assert_eq!(checked.flaws.len(), 3, "{:?}", checked.flaws);
        for (index, name) in ["first", "second", "third"].into_iter().enumerate() {
            let flaw = &checked.flaws[index];
            let construct = flaw.construct.as_ref().expect("the original declaration");
            assert_eq!(construct.name, name);
            let expected = source.find(&format!("{name}()")).unwrap();
            assert_eq!(construct.span.byte_start, expected);
            let culprit = flaw.culprit.as_ref().expect("the invalid returned value");
            assert_eq!(&source[culprit.byte_start..culprit.byte_end], "10");
        }
    }
}
