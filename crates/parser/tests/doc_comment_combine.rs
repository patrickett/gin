//! Tests for combined pre+postfix doc comments on declares and union variants.

use ast::{DeclareValue, HasMember, Variant};
use diagnostic::Category;
use internment::Intern;
use parser::query::SourceParseExt;

fn intern(s: &str) -> Intern<String> {
    Intern::new(s.to_owned())
}

fn variant_doc(v: &Variant) -> Option<&str> {
    match v {
        Variant::Local {
            doc_comment: Some(d),
            ..
        } => Some(d.value.as_str()),
        Variant::Local {
            doc_comment: None, ..
        }
        | Variant::External { .. } => None,
    }
}

#[test]
fn doc_combine_prefix_and_suffix_on_variant() {
    let src = "\
Maybe(x) is
    --- Before the variant
    Some(x) or --- After the variant: has some value
    None
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let declare = out.ast.tags.get(&intern("Maybe")).expect("Maybe");
    let DeclareValue::Union { variants } = &declare.value else {
        panic!("expected union");
    };

    assert_eq!(variants.len(), 2);

    // Some - should have both docs combined
    assert_eq!(
        variant_doc(&variants[0]),
        Some("Before the variant\n\nAfter the variant: has some value")
    );

    // None - no doc
    assert_eq!(variant_doc(&variants[1]), None);
}

#[test]
fn doc_combine_prefix_and_suffix_on_declare() {
    let src = "\
--- Doc before the tag
Maybe(x) is    --- Doc after is
    Some(x) or --- Has some value `x`
    None       --- Has no value
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let declare = out.ast.tags.get(&intern("Maybe")).expect("Maybe");

    // Top-level declare doc should combine prefix + suffix
    assert_eq!(
        declare.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("Doc before the tag\n\nDoc after is")
    );
}

#[test]
fn doc_combine_prefix_suffix_and_variant_docs_declare() {
    let src = "\
--- Top-level prefix only
Maybe(x) is    --- Top-level suffix
    Some(x) or --- Some variant doc
    None       --- None variant doc
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let declare = out.ast.tags.get(&intern("Maybe")).expect("Maybe");

    // Top-level: prefix + suffix
    assert_eq!(
        declare.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("Top-level prefix only\n\nTop-level suffix")
    );

    let DeclareValue::Union { variants } = &declare.value else {
        panic!("expected union");
    };

    // Variant docs (suffix only)
    assert_eq!(variant_doc(&variants[0]), Some("Some variant doc"));
    assert_eq!(variant_doc(&variants[1]), Some("None variant doc"));
}

#[test]
fn doc_prefix_only_on_variant() {
    let src = "\
Maybe(x) is
    --- Doc prefix
    Some(x) or None
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let declare = out.ast.tags.get(&intern("Maybe")).expect("Maybe");
    let DeclareValue::Union { variants } = &declare.value else {
        panic!("expected union");
    };

    // Prefix-only doc
    assert_eq!(variant_doc(&variants[0]), Some("Doc prefix"));
    assert_eq!(variant_doc(&variants[1]), None);
}

#[test]
fn doc_suffix_only_on_variant() {
    let src = "\
Maybe(x) is
    Some(x) or --- Doc suffix
    None
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let declare = out.ast.tags.get(&intern("Maybe")).expect("Maybe");
    let DeclareValue::Union { variants } = &declare.value else {
        panic!("expected union");
    };

    // Suffix-only doc
    assert_eq!(variant_doc(&variants[0]), Some("Doc suffix"));
    assert_eq!(variant_doc(&variants[1]), None);
}

/// Multi-line doc comment before a union declare whose variants are on the same
/// line as `is` (e.g. `Comparison is Less\n           or Equal`).
#[test]
fn doc_multiline_before_union_on_same_line_as_is() {
    let src = "\
--- Result of comparing two values that may not be totally ordered.
---
--- `Ordered(ordering)` means comparison succeeded.
Comparison is Less
           or Equal
           or Incomparable
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let declare = out.ast.tags.get(&intern("Comparison")).expect("Comparison");
    let doc = declare
        .doc_comment
        .as_ref()
        .expect("Comparison should have a doc comment");
    assert_eq!(
        doc.value,
        "Result of comparing two values that may not be totally ordered.\n\n`Ordered(ordering)` means comparison succeeded.",
        "Comparison doc"
    );

    let DeclareValue::Union { variants } = &declare.value else {
        panic!("expected union");
    };
    assert_eq!(variants.len(), 3);
}

/// Hover tests for module docs appear in `ast/tests/hover_classify_tests.rs`.
/// This test validates that a `declare` followed by a different top-level element
/// (e.g. `x has Trait(...)` each keep their own doc comments.
#[test]
fn doc_comment_stays_with_own_element_across_multiple_elements() {
    // Two separate declares with docs
    let src = "\
--- First tag doc.
Foo is 1
--- Second tag doc.
Bar is 2
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let foo = out.ast.tags.get(&intern("Foo")).expect("Foo");
    assert_eq!(
        foo.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("First tag doc."),
        "Foo should get its own doc"
    );

    let bar = out.ast.tags.get(&intern("Bar")).expect("Bar");
    assert_eq!(
        bar.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("Second tag doc."),
        "Bar should get its own doc, not Foo's"
    );
}

/// A tag with a multi-line doc comment (blank separator line) before it,
/// followed by another tag with its own doc. Both must keep their docs.
#[test]
fn doc_multiline_before_two_declares() {
    let src = "\
--- Results of comparing values.
---
--- This is the second paragraph.
Comparison is Less
           or Equal

--- Another tag.
Other is 42
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let comparison = out.ast.tags.get(&intern("Comparison")).expect("Comparison");
    let doc = comparison
        .doc_comment
        .as_ref()
        .expect("Comparison should have doc");
    assert_eq!(
        doc.value, "Results of comparing values.\n\nThis is the second paragraph.",
        "Comparison doc"
    );

    let other = out.ast.tags.get(&intern("Other")).expect("Other");
    assert_eq!(
        other.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("Another tag."),
        "Other should get its own doc"
    );
}

/// A declare with a doc comment followed by a bind (not a declare).
/// The bind should NOT steal the declare's doc.
#[test]
fn doc_before_declare_not_stolen_by_bind() {
    let src = "\
--- Doc for the tag.
MyTag is 42

--- Doc for the function.
my_fn() Int := 99
";
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let my_tag = out.ast.tags.get(&intern("MyTag")).expect("MyTag");
    assert_eq!(
        my_tag.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("Doc for the tag."),
        "MyTag should get its doc"
    );

    let my_fn = out.ast.defs.get(&intern("my_fn")).expect("my_fn");
    assert_eq!(
        my_fn.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("Doc for the function."),
        "my_fn should get its own doc"
    );
}

#[test]
fn doc_on_single_has_member() {
    let src = "\
MyType has
    --- allocate doc
    allocate(ref self)
";
    let out = src.parse_source_full();
    let flaws: Vec<_> = out
        .symptoms
        .iter()
        .filter(|d| matches!(d.category, Category::Flaw))
        .collect();
    assert!(
        flaws.is_empty(),
        "parse flaws: {:?}",
        flaws.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let DeclareValue::Has(members) = &out.ast.tags.get(&intern("MyType")).expect("MyType").value
    else {
        panic!("expected Has");
    };
    assert_eq!(members.len(), 1);
    let HasMember::Function(allocate) = &members[0] else {
        panic!("allocate should be a function member");
    };
    assert_eq!(allocate.name.as_str(), "allocate");
    assert_eq!(
        allocate.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("allocate doc"),
        "allocate should have its doc comment"
    );
}

#[test]
fn doc_on_has_members() {
    let src = "\
--- Doc for the type.
MyType has
    --- Multi-line doc for the allocate method.
    ---
    --- Returns the allocated bytes on success, or `AllocError` on failure.
    allocate(ref self) Slice(Byte) or AllocError

    --- Single-line doc for deallocate.
    deallocate(ref self)
";
    let out = src.parse_source_full();
    let flaws: Vec<_> = out
        .symptoms
        .iter()
        .filter(|d| matches!(d.category, Category::Flaw))
        .collect();
    assert!(
        flaws.is_empty(),
        "parse flaws: {:?}",
        flaws.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let my_type = out.ast.tags.get(&intern("MyType")).expect("MyType");
    assert_eq!(
        my_type.doc_comment.as_ref().map(|d| d.value.as_str()),
        Some("Doc for the type."),
        "MyType should get its own doc"
    );

    let DeclareValue::Has(members) = &my_type.value else {
        panic!("expected Has");
    };
    assert_eq!(members.len(), 2, "should have 2 has members");

    let HasMember::Function(allocate) = &members[0] else {
        panic!("allocate should be a function member");
    };
    assert_eq!(allocate.name.as_str(), "allocate");
    let doc = allocate
        .doc_comment
        .as_ref()
        .expect("allocate should have a doc comment");
    assert_eq!(
        doc.value,
        "Multi-line doc for the allocate method.\n\nReturns the allocated bytes on success, or `AllocError` on failure.",
        "allocate doc"
    );

    let HasMember::Function(deallocate) = &members[1] else {
        panic!("deallocate should be a function member");
    };
    assert_eq!(deallocate.name.as_str(), "deallocate");
    match &deallocate.doc_comment {
        Some(d) => assert_eq!(
            d.value.as_str(),
            "Single-line doc for deallocate.",
            "deallocate doc mismatch"
        ),
        None => panic!("deallocate should have a doc comment"),
    }
}
