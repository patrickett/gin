use analysis::PackageCache;
use std::path::PathBuf;

#[test]
fn member_body_hover_resolves_dependent_param_loop_binding_and_method_call() {
    let source = r#"Byte is in 0 to 255
Length is in 0 to 256
Vector has
    push(self mut, value Byte):
        return

    copy(self ref) Self: return self

String has
    bytes Byte

    from_bytes(source [in Byte; count]) Length:
        buffer Vector
        for index in 0 to count
            buffer.push(source.[index] as Byte)
        return count as Length
"#;
    let path = PathBuf::from("/tmp/test_string_hover.gin");
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.set_contents(&path, source.to_string());

    let hover_at = |needle: &str, occurrence: usize| {
        let byte = source.match_indices(needle).nth(occurrence)?.0 as u32;
        engine.hover(&path, byte).map(|hover| hover.markdown)
    };
    let count_declaration = hover_at("count", 0).expect("count declaration hover");
    let count_use = hover_at("count", 1).expect("count use hover");
    let index_declaration = hover_at("index", 0).expect("index declaration hover");
    let index_use = hover_at("index", 1).expect("index use hover");
    let push_call = hover_at("push", 1).expect("push call hover");
    let self_type = hover_at("Self", 0).expect("Self type hover");

    assert_eq!(count_declaration.trim(), "```gin\ncount\n```");
    assert_eq!(count_use.trim(), "```gin\ncount\n```");
    assert_eq!(
        index_declaration.trim(),
        "```gin\nindex is in 0 to count\n```"
    );
    assert_eq!(index_use.trim(), "```gin\nindex is in 0 to count\n```");
    assert!(
        push_call.contains("push(self mut, value Byte)"),
        "{push_call}"
    );
    assert!(self_type.contains("```gin\nVector\n```"), "{self_type}");
}
