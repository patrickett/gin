use tree_sitter::{Parser, Query, QueryCursor};

const REFLECT_TYPE_DECL: &str = r#"Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Tuple(elems List(Type))
     or Ptr(inner Type)
     or Ref(inner Type, mutable Bool)
     or Array(elem Type, size BigInt)
     or Opaque(name String)
"#;

fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&super::language())
        .expect("Error loading gin language");
    parser
}

#[test]
fn test_can_load_grammar() {
    let _ = parser();
}

#[test]
fn numbered_unions_accept_single_variants_and_signed_discriminants() {
    let mut parser = parser();
    for source in [
        "Code is Foo := 3 or Bar := 2 or Baz := 1\n",
        "Code is Foo := 3\n     or Bar := -2\n",
        "Code is Only := 7\n",
        "#bits(16)\nCode is Foo := 300 or Bar := 2\n",
    ] {
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
    }
}

#[test]
fn private_blocks_group_members_and_end_on_dedent() {
    let mut parser = parser();
    for grants in ["", "(client, other.module)"] {
        let source = format!(
            "Record has\n    visible Word\n    #private{grants}\n        secret Word\n        hidden(self ref) Word:\n            when self.secret = 0 then\n                return 1\n            return self.secret\n    read(self ref) Word: self.secret\nOther has value Word\n"
        );
        let tree = parser.parse(&source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
        let declaration = tree.root_node().named_child(0).unwrap();
        let members = declaration.named_child(1).unwrap();
        assert_eq!(members.kind(), "decl_member_list");
        assert_eq!(members.named_child_count(), 3);
        assert_eq!(
            members.named_child(1).unwrap().kind(),
            "private_member_block"
        );
        assert_eq!(members.named_child(2).unwrap().kind(), "record_signature");
        assert_eq!(tree.root_node().named_child_count(), 2);
    }
}

#[test]
fn single_member_privacy_does_not_create_a_block() {
    let mut parser = parser();
    let source = "Record has\n    #private(client)\n    secret Word\n    visible Word\n";
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    assert!(!tree.root_node().to_sexp().contains("private_member_block"));
}

#[test]
fn inferred_value_parameters_accept_trait_constraints() {
    let mut parser = parser();
    for source in [
        "reclaim(self, a mut has Allocator): return a.reclaim(self)\n",
        "inspect(a ref has Convert(Word) and Copy): return a.convert\n",
        include_str!("../../../../../modules/core/alloc/allocator.gin"),
    ] {
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
    }
}

#[test]
fn dot_bracket_indexing_is_the_only_element_access_form() {
    let mut parser = parser();
    for source in [
        "read: values.[index]\n",
        "read: values .[index]\n",
        "read: values . [index]\n",
        "read: values.[index].[next]\n",
        "write:\n    self.values.[index]:: value\n    return\n",
        "values[Byte; 2]: [1, 2]\n",
        "values [Byte; 2]: [1, 2]\n",
    ] {
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
    }
    for source in [
        "read: values.(index)\n",
        "write:\n    values.(index):: value\n    return\n",
    ] {
        let tree = parser.parse(source, None).unwrap();
        assert!(
            tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
    }
    for source in ["read: values[0]\n", "read: make(0)[index]\n"] {
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().to_sexp().contains("index_expression"),
            "{source}"
        );
    }
}

#[test]
fn ranges_use_to_in_declarations_expressions_and_proofs() {
    let mut parser = parser();
    for source in [
        "Byte is in 0 to 255\n",
        "Window is -10 to 10\n",
        "values := low to max\n",
        "check(value) True thus value in low to max: return True\n",
    ] {
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error(), "{source}");
        let legacy = source.replace(" to ", "...");
        let tree = parser.parse(legacy, None).unwrap();
        assert!(
            tree.root_node().has_error(),
            "accepted obsolete range: {source}"
        );
    }
}

#[test]
fn simple_decl_no_newline() {
    let mut p = parser();
    let result = p.parse("Bool is True", None);
    let tree = result.unwrap();
    assert!(
        !tree.root_node().has_error(),
        "simple decl (no newline): return {}",
        tree.root_node().to_sexp()
    );
}

#[test]
fn simple_decl_with_newline() {
    let mut p = parser();
    let result = p.parse("Bool is True\n", None);
    let tree = result.unwrap();
    assert!(
        !tree.root_node().has_error(),
        "simple decl (with newline): return {}",
        tree.root_node().to_sexp()
    );
}

#[test]
fn simple_union_decl_parses() {
    let mut p = parser();
    let result = p.parse("Bool is True or False\n", None);
    let tree = result.unwrap();
    assert!(
        !tree.root_node().has_error(),
        "simple union: {}",
        tree.root_node().to_sexp()
    );
}

#[test]
fn constructor_union_variants_parse_without_errors() {
    let mut parser = parser();
    let tree = parser
        .parse(REFLECT_TYPE_DECL, None)
        .expect("tree-sitter should parse source");

    assert!(
        !tree.root_node().has_error(),
        "constructor-style union variants should parse without errors:\n{}",
        tree.root_node().to_sexp()
    );
}

#[test]
fn js_style_list_rest_patterns_parse_without_errors() {
    let source = r#"check(xs List(Int)) Bool := return when xs is
    case [] then True
    case [head, ...tail] then False
"#;
    let mut parser = parser();
    let tree = parser
        .parse(source, None)
        .expect("tree-sitter should parse source");

    assert!(
        !tree.root_node().has_error(),
        "JS-style list rest patterns should parse without errors:\n{}",
        tree.root_node().to_sexp()
    );
}

#[test]
fn every_or_in_constructor_union_highlights_as_keyword_operator() {
    let mut parser = parser();
    let tree = parser
        .parse(REFLECT_TYPE_DECL, None)
        .expect("tree-sitter should parse source");
    assert!(
        !tree.root_node().has_error(),
        "highlight regression input must parse cleanly:\n{}",
        tree.root_node().to_sexp()
    );

    let query = Query::new(&super::language(), super::HIGHLIGHTS_QUERY)
        .expect("highlights query should compile");
    let keyword_operator_index = query
        .capture_names()
        .iter()
        .position(|name| *name == "keyword.operator")
        .expect("highlights query should define @keyword.operator")
        as u32;

    use tree_sitter::StreamingIterator;
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(&query, tree.root_node(), REFLECT_TYPE_DECL.as_bytes());
    let mut highlighted_ors = 0;
    while let Some((query_match, capture_index)) = captures.next() {
        let capture = query_match.captures[*capture_index];
        if capture.index == keyword_operator_index
            && capture.node.utf8_text(REFLECT_TYPE_DECL.as_bytes()) == Ok("or")
        {
            highlighted_ors += 1;
        }
    }

    assert_eq!(
        highlighted_ors, 7,
        "all seven union `or` separators should be @keyword.operator"
    );
}

#[test]
fn emission_replaces_assembly_expressions() {
    let mut parser = parser();
    let emission = "read(value Word) Word: return #emit(code, [value: value])\n";
    assert!(
        !parser
            .parse(emission, None)
            .unwrap()
            .root_node()
            .has_error()
    );
    assert!(
        parser
            .parse("write(): return #asm('spec')\n", None)
            .unwrap()
            .root_node()
            .has_error()
    );
}

#[test]
fn indented_returns_and_dedented_loops_preserve_sibling_functions() {
    let mut parser = parser();
    for source in [
        "first Int:\n    return 1\nsecond Int:\n    return 2\n",
        "run:\n    for index in 0 to 3\n        observe(index)\n    after\n    return\nnext: 1\n",
        "run:\n    while ready do\n        when done then\n            return 1\n        observe\n    return 0\nnext: 1\n",
        "run:\n    when ready then\n        when done then\n            return 1\n        return 2\n    return 3\n",
        "run Int:\n    when ready then\n        observe\n    return 1\n",
        "run:\n\twhile ready do\n\t\tobserve\n\treturn 0\nnext:\n\treturn 1\n",
        "run:\n    -- before loop\n    while ready do\n        -- before statement\n        observe\n\n    -- after loop\n    return 0\n\nnext:\n    return 1\n",
    ] {
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
        let root = tree.root_node();
        let expected = if source.contains("next:") || source.contains("second Int:") {
            2
        } else {
            1
        };
        assert_eq!(
            root.named_child_count(),
            expected,
            "{source}: {}",
            root.to_sexp()
        );
    }
}

#[test]
fn while_requires_do_before_its_body() {
    let mut parser = parser();
    for delimiter in ["", "then", "do do"] {
        let source = format!("run: while ready {delimiter}\n    observe\n");
        let tree = parser.parse(&source, None).unwrap();
        assert!(
            tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
    }
    let source = "run: while index < limit and ready or not done do\n    observe\n";
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
}

#[test]
fn aligned_returns_are_rejected_by_the_editor_grammar() {
    let mut parser = parser();
    for source in [
        "run Int:\n    value: 1\nreturn value\n",
    ] {
        let tree = parser.parse(source, None).unwrap();
        assert!(
            tree.root_node().has_error(),
            "{source}: {}",
            tree.root_node().to_sexp()
        );
    }
}
