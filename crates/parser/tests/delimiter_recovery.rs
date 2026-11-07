use ast::{Argument, Expr, Item, SourceFile, SyntaxNodeKind};

fn outline(file: &SourceFile) -> Vec<&str> {
    file.declaration_outline().map(|entry| entry.name).collect()
}

#[test]
fn end_headers_sever_expression_lists_before_a_stray_close() {
    for header in ["next end: 2", "next end Item: 2"] {
        let source = format!("value := f(\n{header}\n)\nlater := 3\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert_eq!(outline(&output.file), ["value", "next", "later"]);
        let hit = output
            .file
            .node_at_offset(source.find("end").unwrap())
            .unwrap();
        assert!(matches!(
            hit.node.kind, SyntaxNodeKind::BindingConvention(keyword)
                if keyword.value == ast::BindingConvention::End
        ));
        assert_eq!(hit.declarations[0].name, "next");
        assert!(parser::parse_strict(&source).is_err());
    }
}

#[test]
fn missing_close_preserves_all_supported_following_headers() {
    for header in [
        "next: 2",
        "next Int: 2",
        "next in Byte: 2",
        "next(value in Byte) is >= 0: return value",
        "next(x) Int: return 2",
        "next(\n    x Int\n) Int: return 2",
        "Next is Ready or Failed",
        "Next(element) has value element",
    ] {
        let source = format!("value := f(\n{header}\nlast := 3\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        let expected = if header.starts_with("Next") {
            "Next"
        } else {
            "next"
        };
        assert_eq!(
            outline(&output.file),
            ["value", expected, "last"],
            "{source}"
        );
        let offset = source
            .find(header.split(['(', ' ', ':']).next().unwrap())
            .unwrap();
        let hit = output.file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name(name) if name == expected));
        assert_eq!(hit.declarations[0].name, expected);
        let first = output.file.span_table.get(output.file.items[0].span_id);
        assert!(first.end() <= offset);
        assert!(parser::parse_strict(&source).is_err());
    }
}

#[test]
fn recovery_restores_body_and_member_scopes_before_the_next_header() {
    let source = "\
Thing has
    read(self):
        value := f(
        next: 2
        return next
    field Int
main:
    value := g(
    local(x) Int: return x
    return local(1)
last := 4
";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(outline(&output.file), ["Thing", "main", "last"]);
    for (needle, expected) in [
        ("next:", vec!["Thing", "read", "next"]),
        ("field Int", vec!["Thing", "field"]),
        ("local(x)", vec!["main", "local"]),
    ] {
        let hit = output
            .file
            .node_at_offset(source.find(needle).unwrap())
            .unwrap();
        let owners: Vec<_> = hit.declarations.iter().map(|entry| entry.name).collect();
        assert_eq!(owners, expected);
    }
}

#[test]
fn lower_indented_headers_close_the_old_scope_before_parsing() {
    for source in [
        "main:\n    value := f(\nnext: 2\nlast := 3\n",
        "main:\n    value := f(\n        arg Int\nnext: 2\nlast := 3\n",
        "Thing has\n    read(self):\n        value := f(\n    field Int\nlast := 3\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let needle = if source.starts_with("Thing") {
            "field"
        } else {
            "next"
        };
        let hit = output
            .file
            .node_at_offset(source.find(needle).unwrap())
            .unwrap();
        assert_eq!(hit.declarations.last().unwrap().name, needle);
        let owners: Vec<_> = hit.declarations.iter().map(|entry| entry.name).collect();
        if needle == "field" {
            assert_eq!(owners, ["Thing", "field"]);
        } else {
            assert_eq!(owners, ["next"]);
        }
        assert_eq!(outline(&output.file).last(), Some(&"last"));
    }
}

#[test]
fn balanced_multiline_named_arguments_win_even_at_the_statement_indentation() {
    for source in [
        "value := f(\nlabel: 2\n)\nlast := 3\n",
        "main:\n    value := f(\n    label: 2\n    )\n    return value\nlast := 3\n",
        "value := f(\n    nested(\nlabel: 2\n    )\n)\nlast := 3\n",
        "f(\nname: 2\n)\n",
        "fn(\nname: 2\n): return name\n",
    ] {
        let file = parser::parse_strict(source).expect(source);
        assert!(
            file.node_at_offset(
                source
                    .find("name:")
                    .or_else(|| source.find("label:"))
                    .unwrap()
            )
            .unwrap()
            .declarations
            .iter()
            .all(|entry| !matches!(entry.name, "name" | "label"))
        );
    }
}

#[test]
fn deeper_named_arguments_stay_inside_an_unfinished_call() {
    let source = "value := f(\n    label: 2\n    other: nested(\n        name: 3\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(outline(&output.file), ["value"]);
    let Item::Binding(binding) = &output.file.items[0].value else {
        panic!("binding survives");
    };
    let Expr::Call { arguments, .. } = &binding.value.as_ref().unwrap().expression.value else {
        panic!("call survives");
    };
    assert_eq!(arguments.value.len(), 2);
    assert!(
        matches!(&arguments.value[0].value, Argument::Named { name, .. } if name.value == "label")
    );
}

#[test]
fn tabs_and_comment_only_lines_do_not_change_the_recovery_owner_column() {
    let source =
        "main:\n\tvalue := f(\n        -- 🦀 comment\n\tnext: 2\n\treturn next\nlast := 3\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(outline(&output.file), ["main", "last"]);
    let hit = output
        .file
        .node_at_offset(source.find("next:").unwrap())
        .unwrap();
    assert_eq!(hit.declarations.last().unwrap().name, "next");
}

#[test]
fn many_unclosed_calls_and_colon_headers_do_not_accumulate_nesting_depth() {
    let source: String = (0..2_000)
        .map(|index| format!("f(\nx{index}: 2\n"))
        .collect();
    let options = parser::ParseOptions {
        limits: parser::ParseLimits {
            diagnostics_max: 3_000,
            ..Default::default()
        },
        ..Default::default()
    };
    let output = parser::parse_with_options(&source, options).unwrap();
    assert!(output.has_errors());
    assert_eq!(output.file.declaration_outline().count(), 2_000);
}

#[test]
fn a_stray_close_after_a_hard_boundary_cannot_hide_an_earlier_declaration() {
    for hard_start in ["last := 2", "Last is Ready", "use core"] {
        let source = format!("value := f(\nnext: 1\n{hard_start}\n)\nafter := 3\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        let declarations = outline(&output.file);
        assert_eq!(&declarations[..2], ["value", "next"], "{source}");
        assert_eq!(declarations.last(), Some(&"after"));
        let offset = source.find("next:").unwrap();
        let hit = output.file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("next")));
        assert_eq!(hit.declarations[0].name, "next");
        assert!(
            output
                .file
                .span_table
                .get(output.file.items[0].span_id)
                .end()
                <= offset
        );
    }
}

#[test]
fn long_blank_runs_preserve_balanced_and_recovered_call_context() {
    let blank_lines = "\n".repeat(50_000);
    let complete = format!("value := f({blank_lines}label: 2\n)\nlast := 3\n");
    let file = parser::parse_strict(&complete).expect("balanced list with blank lines");
    assert_eq!(outline(&file), ["value", "last"]);
    let incomplete = format!("value := f(g({blank_lines}next: 2\nlast := 3\n");
    let output = parser::parse(&incomplete).unwrap();
    assert!(output.has_errors());
    assert_eq!(outline(&output.file), ["value", "next", "last"]);
    let next = output
        .file
        .node_at_offset(incomplete.find("next:").unwrap())
        .unwrap();
    assert_eq!(next.declarations.last().unwrap().name, "next");
}

/// A `name(...) where ...` header ends an unclosed expression list.
#[test]
fn missing_close_preserves_a_following_where_header() {
    let source = "value := f(\nnext() where 2 = 2: return 2\nlast := 3\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(outline(&output.file), ["value", "next", "last"]);
}
