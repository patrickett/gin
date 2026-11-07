use ast::{
    Annotation, Convention, Expr, Item, SourceFile, SyntaxNodeKind, TypeBody, TypeMember,
    TypePrefixKind,
};

const STORAGE: &str =
    include_str!("../../../editors/tree-sitter-gin/test/corpus/owning_storage.txt");
const METHODS: &str =
    include_str!("../../../editors/tree-sitter-gin/test/corpus/record_methods.txt");
const CORE_BOX: &str = include_str!("../../test-fixtures/fixtures/box_storage_core_box.gin");
const GATHER: &str = include_str!("../../test-fixtures/fixtures/validated_gather_plan.gin");
const CURRENT: &str =
    include_str!("../../../editors/tree-sitter-gin/test/corpus/current_syntax.txt");

fn original_line<'a>(fixture: &'a str, expected: &str) -> &'a str {
    let line = fixture
        .split_inclusive('\n')
        .find(|line| line.trim_end() == expected)
        .expect("exact existing fixture line");
    assert_eq!(line.trim_end(), expected);
    line
}

fn first_member(file: &SourceFile) -> &ast::Binding {
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("snippet wrapper");
    };
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("snippet members");
    };
    let TypeMember::Binding(binding) = &members[0].value else {
        panic!("original member");
    };
    binding
}

#[test]
fn exact_owned_storage_snippet_preserves_field_and_passing_keyword_roles() {
    // An exact source block from the tree-sitter corpus, not its expected CST.
    // This does not claim the complete corpus file is Gin source.
    let source = concat!(
        "Allocation(x) has\n",
        "    address own x\n",
        "    capacity Count(x)\n",
        "    transfer(value own x) x: return value\n",
    );
    assert!(STORAGE.contains(source));
    let file = parser::parse_strict(source).unwrap();
    let field = first_member(&file);
    assert!(field.parameters.is_none());
    let Annotation::Expression(expression) = &field.annotation.as_ref().unwrap().value else {
        panic!("written field annotation");
    };
    let Expr::PrefixedType(prefix) = &expression.value else {
        panic!("own type prefix");
    };
    assert_eq!(prefix.keyword.value, TypePrefixKind::Own);
    assert_eq!(prefix.target.value, Expr::Name("x".into()));
    let own = prefix.keyword.span_id;
    assert_eq!(file.span_table.get(own).extract(source), "own");
    let hit = file.node_at_offset(source.find("own").unwrap()).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::TypePrefixKeyword(_)
    ));
    let hit = file.node_at_offset(source.rfind("own").unwrap()).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::Convention(convention) if convention.value == Convention::Own
    ));
    assert_eq!(hit.declarations.last().unwrap().name, "transfer");
}

#[test]
fn source_ownership_markers_and_initialization_metadata_remain_separate() {
    // Supported source snippets, not a lifecycle check or the complete allocator.
    for count in ["0", "2"] {
        let source = format!(
            concat!(
                "Allocation(x) has\n",
                "    address own x\n",
                "    capacity Count(x)\n",
                "    initialized Count(x) and <= capacity: {}\n",
            ),
            count
        );
        let file = parser::parse_strict(&source).unwrap();
        let address = first_member(&file);
        assert!(address.value.is_none());
        let Annotation::Expression(target) = &address.annotation.as_ref().unwrap().value else {
            panic!("written storage element type");
        };
        let Expr::PrefixedType(prefix) = &target.value else {
            panic!("own type prefix");
        };
        assert_eq!(prefix.keyword.value, TypePrefixKind::Own);
        assert_eq!(prefix.target.value, Expr::Name("x".into()));
        let Item::Type(declaration) = &file.items[0].value else {
            panic!("allocation");
        };
        let TypeBody::Has(members) = &declaration.body.value else {
            panic!("members");
        };
        let TypeMember::Binding(initialized) = &members[2].value else {
            panic!("written initialization metadata");
        };
        assert_eq!(initialized.name.value, "initialized");
        assert!(matches!(
            initialized.annotation.as_ref().unwrap().value,
            Annotation::Refined { .. }
        ));
        let value = &initialized.value.as_ref().unwrap().expression;
        assert_eq!(file.span_table.get(value.span_id).extract(&source), count);
        let own = file.node_at_offset(source.find("own").unwrap()).unwrap();
        assert!(matches!(
            own.node.kind,
            SyntaxNodeKind::TypePrefixKeyword(_)
        ));
        assert_eq!(file.text.as_ref(), source);
    }
}

#[test]
fn exact_method_lines_keep_ref_mut_and_plain_receivers() {
    // Only selected exact member lines are placed in a supported `Snippet has` wrapper.
    // The original files also contain deferred traits, attributes and body syntax.
    for (fixture, expected, convention) in [
        (
            METHODS,
            "    copy(self ref) Self: return Self(values: self.values.copy)",
            Some(Convention::Ref),
        ),
        (
            METHODS,
            "    append(self mut, other own Self): return self.values.append(other.values)",
            Some(Convention::Mut),
        ),
        (
            CORE_BOX,
            concat!(
                "    reserve(self mut, capacity Count(x)) AllocationResult(x): ",
                "return self.inner.reserve(capacity)",
            ),
            Some(Convention::Mut),
        ),
        (CORE_BOX, "    release(self):", None),
    ] {
        let line = original_line(fixture, expected);
        // A signature-only snippet intentionally omits the original indented body.
        let source = if expected.ends_with(':') {
            format!("Snippet has\n{}\n", line.trim_end().trim_end_matches(':'))
        } else {
            format!("Snippet has\n{line}")
        };
        let file = parser::parse_strict(&source).expect(&source);
        let member = first_member(&file);
        let receiver = &member.parameters.as_ref().unwrap().value[0].value;
        assert_eq!(receiver.name.value, "self");
        assert_eq!(
            receiver
                .convention
                .as_ref()
                .map(|mode| mode.value.keyword.value),
            convention
        );
        assert!(receiver.annotation.is_none());
    }
}

#[test]
fn exact_ordinary_parameter_lines_keep_each_explicit_convention() {
    for (expected, convention) in [
        (
            "read(value ref Resource) Byte: return value.value",
            Convention::Ref,
        ),
        ("write(value mut Resource, next Byte):", Convention::Mut),
        (
            "move_box(value own Box(Resource)) Box(Resource): return value",
            Convention::Own,
        ),
    ] {
        let line = original_line(CORE_BOX, expected);
        // Keep `write`'s colon: removing it would turn the header into an ambiguous
        // bare call. Its reassignment body is deferred, so recover the missing body.
        let source = line.trim_end();
        let output = parser::parse(source).unwrap();
        assert_eq!(output.has_errors(), source.ends_with(':'));
        let file = output.file;
        let Item::Binding(binding) = &file.items[0].value else {
            panic!("original function");
        };
        let parameter = &binding.parameters.as_ref().unwrap().value[0].value;
        assert_eq!(
            parameter.convention.as_ref().unwrap().value.keyword.value,
            convention
        );
        assert!(parameter.annotation.is_some());
    }
}

#[test]
fn exact_legacy_receiver_return_header_recovers_noncanonical_domain_spelling() {
    // Header-only snippet: the original body also contains a legacy local annotation.
    // We deliberately retain `is in`, which is recovery syntax for member headers.
    let line = original_line(
        GATHER,
        "    as_num(self ref) is in Self: return (number is in Self: self)",
    );
    let header = line.split_once(": return").unwrap().0;
    let source = format!("Numeric has\n{header}\n");
    let output = parser::parse(&source).unwrap();
    assert!(output.has_errors());
    assert!(parser::parse_strict(&source).is_err());
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.span.extract(&source) == "is"
            && diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("in"))
    }));
    let member = first_member(&output.file);
    let Annotation::Domain { is_keyword, .. } = &member.annotation.as_ref().unwrap().value else {
        panic!("retained legacy domain");
    };
    assert_eq!(
        output
            .file
            .span_table
            .get(is_keyword.unwrap())
            .extract(&source),
        "is"
    );
    assert_eq!(
        member.parameters.as_ref().unwrap().value[0]
            .value
            .convention
            .as_ref()
            .unwrap()
            .value
            .keyword
            .value,
        Convention::Ref
    );
}

#[test]
fn exact_current_receiver_constraint_header_has_no_synthetic_base() {
    // The `thus` return expression in the original method body is deferred.
    let line = original_line(
        CURRENT,
        concat!(
            "    initialize(self mut and self.initialized < self.capacity, ",
            "index Count(x) and index = self.initialized, value own x): ",
            "return thus self.initialized = index + 1",
        ),
    );
    let header = line.split_once(": return").unwrap().0;
    let source = format!("Storage(x) has\n{header}\n");
    let file = parser::parse_strict(&source).unwrap();
    let member = first_member(&file);
    let receiver = &member.parameters.as_ref().unwrap().value[0].value;
    assert_eq!(
        receiver.convention.as_ref().unwrap().value.keyword.value,
        Convention::Mut
    );
    let Annotation::Refined { base, refinements } = &receiver.annotation.as_ref().unwrap().value
    else {
        panic!("original bare-and receiver");
    };
    assert!(base.is_none());
    assert_eq!(refinements.len(), 1);
    let hit = file
        .node_at_offset(source.find("initialized").unwrap())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("initialized")));
    assert!(!matches!(
        hit.node.kind,
        SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
    ));
}
