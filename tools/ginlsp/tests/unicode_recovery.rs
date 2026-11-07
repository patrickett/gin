use ginlsp::hover::{HoverState, hover};
use ginlsp::position::LineIndex;

#[test]
fn hover_keeps_good_declarations_after_several_unicode_flaws() {
    let source = "\
first() in 0 to 9:
    -- 😀😀😀
    return 10
second() in 0 to 9:
    -- hé 🦀
    return 10
#unrestricted
Byte is in 0 to 255
after() in 0 to 9: return 3
";
    let state = HoverState::new("<edit>", source);
    assert_eq!(state.original.flaws.len(), 2);
    let checked = state.inspectable();
    assert!(checked.module.is_some(), "{:?}", checked.flaws);
    assert_eq!(checked.source.len(), source.len());
    for (before, after) in source.bytes().zip(checked.source.bytes()) {
        assert_eq!(before == b'\n', after == b'\n');
    }
    let offset = source.find("Byte is").unwrap();
    let index = LineIndex::new(source);
    let found = hover(&state, source, &index, offset + 1).expect("hover on the valid type");
    assert_eq!(found.range, Some(index.range(source, offset, offset + 4)));
}
