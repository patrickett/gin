mod support;

use parser::query::SourceParseExt;
use support::{expect_flaws_with, flaw_codes};

const VALUES: &str = r#"
Byte is in 0 to 255
Byte has
    finish(self):
        return
Text has
    byte_count Byte
    finish(self):
        return
read(value ref Byte) Byte: return deref value
replace_byte(previous Byte, next Byte) Byte: return next
"#;

const VIEWER: &str = r#"
FileViewer has
    file_name ref Text
    contents ref Text
    finish(self):
        return
open_view(name ref Text, text ref Text) FileViewer:
    return FileViewer(file_name: name, contents: text)
render(view ref FileViewer) Byte: return view.contents.byte_count
"#;

const BUFFER: &str = r#"
DocumentBuffer has
    bytes @Byte
    length Byte
    finish(self):
        return
replace_storage(buffer mut DocumentBuffer, replacement @Byte):
    buffer.bytes:: replace_pointer(buffer.bytes, replacement)
    return
replace_pointer(previous @Byte, next @Byte) @Byte: return next
"#;

fn expect(fixture: &str, source: &str, expected: &[&str]) {
    expect_flaws_with(VALUES, &format!("{fixture}{source}"), expected);
}

#[test]
fn file_viewer_can_borrow_two_caller_owned_documents() {
    expect(
        VIEWER,
        r#"
preview(name ref Text, text ref Text) Byte:
    view: open_view(name, text)
    return render(view)
"#,
        &[],
    );
}

#[test]
fn file_viewer_can_borrow_the_same_document_twice() {
    expect(
        VIEWER,
        r#"
preview(text ref Text) Byte:
    view: open_view(text, text)
    return render(view)
"#,
        &[],
    );
}

#[test]
fn file_viewer_can_be_rendered_before_its_local_documents_are_consumed() {
    expect(
        VIEWER,
        r#"
preview:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    view: FileViewer(file_name: name, contents: text)
    render(view)
    name.finish
    text.finish
    view.finish
    return
"#,
        &[],
    );
}

#[test]
fn file_viewer_cannot_return_references_to_local_documents() {
    expect(
        VIEWER,
        r#"
load_preview FileViewer:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    return FileViewer(file_name: name, contents: text)
"#,
        &["type-reference-return-local-target"],
    );
}

#[test]
fn file_viewer_cannot_mix_caller_storage_with_escaping_local_storage() {
    expect(
        VIEWER,
        r#"
load_preview(name ref Text) FileViewer:
    text: Text(byte_count: 40)
    return FileViewer(file_name: name, contents: text)
"#,
        &["type-reference-return-local-target"],
    );
}

#[test]
fn file_viewer_factory_does_not_hide_escaping_local_storage() {
    expect(
        VIEWER,
        r#"
load_preview FileViewer:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    return open_view(name, text)
"#,
        &["type-reference-return-local-target"],
    );
}

#[test]
fn nesting_a_file_viewer_does_not_hide_escaping_local_storage() {
    expect(
        VIEWER,
        r#"
PreviewPanel has
    viewer FileViewer
    finish(self):
        self.viewer.finish
        return
load_panel PreviewPanel:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    view: FileViewer(file_name: name, contents: text)
    return PreviewPanel(viewer: view)
"#,
        &["type-reference-return-local-target"],
    );
}

#[test]
fn file_viewer_cannot_render_after_its_contents_are_consumed() {
    expect(
        VIEWER,
        r#"
preview:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    view: FileViewer(file_name: name, contents: text)
    text.finish
    return render(view)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn direct_document_reference_cannot_escape_local_storage() {
    expect(
        "",
        r#"
load_document(size Byte) ref Text:
    text: Text(byte_count: size)
    return ref text
"#,
        &["type-reference-return-local-target"],
    );
}

#[test]
fn selected_document_cannot_be_used_after_a_possible_target_is_consumed() {
    expect(
        "",
        r#"
select(left ref{documents} Text, right ref{documents} Text) ref{documents} Text: return left
size(text ref Text) Byte: return text.byte_count
preview:
    left: Text(byte_count: 8)
    right: Text(byte_count: 40)
    ref selected Text: select(left, right)
    right.finish
    return size(selected)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn consuming_an_unrelated_document_preserves_the_selection() {
    expect(
        "",
        r#"
select(left ref{documents} Text, right ref{documents} Text) ref{documents} Text: return left
size(text ref Text) Byte: return text.byte_count
preview:
    left: Text(byte_count: 8)
    right: Text(byte_count: 40)
    unrelated: Text(byte_count: 12)
    ref selected Text: select(left, right)
    unrelated.finish
    return size(selected)
"#,
        &[],
    );
}

#[test]
fn game_update_allows_two_mutable_aliases_in_one_entity_group() {
    expect(
        "",
        r#"
Entity has
    health Byte
    finish(self):
        return
attack(attacker mut{entities} Entity, defender mut{entities} Entity):
    attacker.health:: replace_byte(attacker.health, 90)
    defender.health:: replace_byte(defender.health, 80)
    return
turn:
    player: Entity(health: 100)
    attack(player, player)
    return
"#,
        &[],
    );
}

#[test]
fn copying_document_metadata_requires_isolation_from_the_source() {
    expect(
        "",
        r#"
copy_size(destination mut{output} Text, source ref{input} Text):
    destination.byte_count:: replace_byte(destination.byte_count, source.byte_count)
    return
update:
    text: Text(byte_count: 40)
    copy_size(text, text)
    return
"#,
        &["type-overlapping-target-group-arguments"],
    );
}

#[test]
fn copying_document_metadata_between_distinct_documents_is_allowed() {
    expect(
        "",
        r#"
copy_size(destination mut{output} Text, source ref{input} Text):
    destination.byte_count:: replace_byte(destination.byte_count, source.byte_count)
    return
update:
    source: Text(byte_count: 40)
    destination: Text(byte_count: 0)
    copy_size(destination, source)
    return
"#,
        &[],
    );
}

#[test]
fn replacing_document_storage_invalidates_a_selected_byte() {
    expect(
        BUFFER,
        r#"
reload(buffer mut DocumentBuffer, replacement @Byte) Byte:
    ref selected Byte: buffer.bytes.[0]
    replace_storage(buffer, replacement)
    return read(selected)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn replacing_document_storage_preserves_a_reference_to_inline_metadata() {
    expect(
        BUFFER,
        r#"
reload(buffer mut DocumentBuffer, replacement @Byte) Byte:
    ref length Byte: buffer.length
    replace_storage(buffer, replacement)
    return read(length)
"#,
        &[],
    );
}

#[test]
fn replacing_other_document_storage_preserves_a_selected_byte() {
    expect(
        BUFFER,
        r#"
reload(left mut DocumentBuffer, right mut DocumentBuffer, replacement @Byte) Byte:
    ref selected Byte: left.bytes.[0]
    replace_storage(right, replacement)
    return read(selected)
"#,
        &[],
    );
}

#[test]
fn early_return_cannot_use_a_selection_after_storage_replacement() {
    expect(
        BUFFER,
        r#"
reload(buffer mut DocumentBuffer, replacement @Byte, changed ref Byte) Byte:
    ref selected Byte: buffer.bytes.[0]
    when changed is 1 then
        replace_storage(buffer, replacement)
        return read(selected)
    return read(selected)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn factory_created_viewer_cannot_read_consumed_contents() {
    expect(
        VIEWER,
        r#"
preview:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    view: open_view(name, text)
    text.finish
    return render(view)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn moving_a_viewer_preserves_its_borrowed_dependencies() {
    expect(
        VIEWER,
        r#"
forward(view own FileViewer) FileViewer: return view
preview:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    original: open_view(name, text)
    forwarded: forward(original)
    text.finish
    return render(forwarded)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn rendering_contents_does_not_read_an_invalidated_file_name() {
    expect(
        VIEWER,
        r#"
preview:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    view: open_view(name, text)
    name.finish
    return render(view)
"#,
        &[],
    );
}

#[test]
fn nested_viewer_read_effects_reach_the_original_contents() {
    expect(
        VIEWER,
        r#"
PreviewPanel has
    viewer FileViewer
    finish(self):
        self.viewer.finish
        return
draw(panel ref PreviewPanel) Byte: return render(panel.viewer)
preview:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    view: open_view(name, text)
    panel: PreviewPanel(viewer: view)
    text.finish
    result: draw(panel)
    panel.finish
    return result
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn borrowed_record_early_return_cannot_escape_local_storage() {
    expect(
        VIEWER,
        r#"
load_preview(name ref Text, text ref Text, changed ref Byte) FileViewer:
    when changed is 1 then
        local: Text(byte_count: 40)
        return FileViewer(file_name: name, contents: local)
    return open_view(name, text)
"#,
        &["type-reference-return-local-target"],
    );
}

#[test]
fn selected_viewer_retains_dependencies_from_both_branches() {
    expect(
        VIEWER,
        r#"
preview(changed ref Byte):
    name: Text(byte_count: 8)
    left: Text(byte_count: 40)
    right: Text(byte_count: 50)
    view: when changed is 1 then open_view(name, left) else open_view(name, right)
    right.finish
    return render(view)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn caller_owned_viewer_can_be_forwarded_through_a_nested_record() {
    expect(
        VIEWER,
        r#"
PreviewPanel has
    viewer FileViewer
    finish(self):
        self.viewer.finish
        return
wrap(view own FileViewer) PreviewPanel: return PreviewPanel(viewer: view)
preview(name ref Text, text ref Text) Byte:
    view: open_view(name, text)
    panel: wrap(view)
    result: render(panel.viewer)
    panel.finish
    return result
"#,
        &[],
    );
}

#[test]
fn factory_reference_contracts_cross_files_in_either_order() {
    use typecheck::FileId;
    use typecheck::transform::{PackageTransformOptions, TransformCtx, transform_package};

    let library = format!("{VALUES}{VIEWER}");
    let client = r#"
load_preview FileViewer:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    return open_view(name, text)
"#;
    for sources in [[library.as_str(), client], [client, library.as_str()]] {
        let files: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let parsed = source.parse_source_full();
                assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
                (parsed.ast, FileId(index as u32))
            })
            .collect();
        let files = transform_package(&files, &TransformCtx::new(), PackageTransformOptions::FULL);
        let codes: Vec<_> = files.iter().flat_map(flaw_codes).collect();
        assert_eq!(codes, ["type-reference-return-local-target"]);
    }
}

#[test]
fn replacing_a_viewer_field_cannot_hide_escaping_local_storage() {
    expect(
        VIEWER,
        r#"
load_preview(name ref Text, original ref Text) FileViewer:
    text: Text(byte_count: 40)
    view: open_view(name, original)
    view.contents:: ref text
    return view
"#,
        &["type-reference-return-local-target"],
    );
}

#[test]
fn borrowing_a_field_through_a_viewer_checks_the_original_storage() {
    expect(
        VIEWER,
        r#"
inspect(view ref FileViewer) Byte: return read(view.contents.byte_count)
preview:
    name: Text(byte_count: 8)
    text: Text(byte_count: 40)
    view: open_view(name, text)
    text.finish
    return inspect(view)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn public_interface_changes_when_borrowed_field_origins_change() {
    use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

    let library = format!("{VALUES}{VIEWER}");
    let forward =
        "choose_view(left ref Text, right ref Text) FileViewer: return open_view(left, right)\n";
    let reverse =
        "choose_view(left ref Text, right ref Text) FileViewer: return open_view(right, left)\n";
    let fingerprint = |sources: [&str; 2]| {
        let files = sources
            .into_iter()
            .map(|source| {
                let parsed = source.parse_source_full();
                assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
                parsed.ast
            })
            .collect();
        let package = transform_package_with_shared_context(files, PackageTransformOptions::FULL);
        let codes: Vec<_> = package.typed_asts.iter().flat_map(flaw_codes).collect();
        assert!(codes.is_empty(), "{codes:?}");
        package.public_interface.semantic_surface_fingerprint
    };
    assert_ne!(
        fingerprint([&library, forward]),
        fingerprint([&library, reverse])
    );
}

#[test]
fn recursive_record_factory_converges_and_rejects_a_local_link() {
    expect(
        "",
        r#"
Node has
    next ref Node
    finish(self):
        return
build(seed ref Node, flag ref Byte) Node:
    when flag is 0 then
        return Node(next: seed)
    local: build(seed, flag)
    return Node(next: local)
"#,
        &["type-reference-return-local-target"],
    );
}
