use super::*;

#[test]
fn build_add_import_edits_replaces_existing_use_line() {
    let source = "use '../primitive/'.List\n\nString has (bytes List(Byte))\n";
    let insert_pos = Backend::use_insert_position(source);
    let edits = Backend::build_add_import_edits(
        source,
        "use '../primitive/'.(Byte, List)",
        Some(0),
        &[],
        insert_pos,
        false,
    );
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "use '../primitive/'.(Byte, List)\n");
    assert_eq!(edits[0].range.start.line, 0);
    assert_eq!(edits[0].range.start.character, 0);
    // End spans through the line's trailing newline.
    assert!(edits[0].range.end.line <= 1);
}

#[test]
fn build_add_import_edits_replaces_sibling_use_line() {
    let source = "use Marker\n\nCopy is Marker(Unit)\n";
    let insert_pos = Backend::use_insert_position(source);
    let edits = Backend::build_add_import_edits(
        source,
        "use Infection, Marker",
        Some(0),
        &[],
        insert_pos,
        false,
    );
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "use Infection, Marker\n");
    assert_eq!(edits[0].range.start.line, 0);
}

#[test]
fn build_add_import_edits_inserts_when_no_replace_line() {
    let source = "main:\nreturn\n";
    let insert_pos = Backend::use_insert_position(source);
    let edits =
        Backend::build_add_import_edits(source, "use core.Int", None, &[], insert_pos, false);
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].range.start, insert_pos);
    assert!(edits[0].new_text.contains("use core.Int"));
}
