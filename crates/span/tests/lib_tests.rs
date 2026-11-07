use span::{Span, SpanId, SpanTable, Spanned};

#[test]
fn span_id_operations() {
    let id = SpanId::new(42);
    assert!(id.is_valid());
    assert_eq!(id.into_inner(), 42);
    assert!(!SpanId::INVALID.is_valid());
    assert_eq!(SpanId::default(), SpanId::INVALID);
}

#[test]
fn span_operations() {
    let span = Span::new(10, 20);
    assert_eq!(span.len(), 10);
    assert!(!span.is_empty());
    assert!(span.contains(15));
    assert!(!span.contains(5));
    assert!(!span.contains(25));
    assert_eq!(span.to_range(), 10..20);
}

#[test]
fn span_merge() {
    let a = Span::new(10, 20);
    let b = Span::new(15, 30);
    let merged = a.merge(b);
    assert_eq!(merged.start(), 10);
    assert_eq!(merged.end(), 30);
}

#[test]
fn span_table() {
    let mut table = SpanTable::new();
    let id1 = table.insert_range(10..20);
    let id2 = table.insert_range(30..40);
    assert_eq!(table.get(id1), Span::new(10, 20));
    assert_eq!(table.get(id2), Span::new(30, 40));
    assert!(table.contains(id1, 15));
    assert!(!table.contains(id1, 25));
}

#[test]
fn spanned_operations() {
    let mut table = SpanTable::new();
    let spanned = Spanned::with_range(&mut table, "hello", 5..10);
    assert_eq!(spanned.value(), &"hello");
    assert!(spanned.span_id().is_valid());
    assert_eq!(spanned.resolve_span(&table), Span::new(5, 10));
    let span_id = spanned.span_id();
    let uppercased = spanned.map(|s| s.to_uppercase());
    assert_eq!(uppercased.value(), &"HELLO");
    assert_eq!(uppercased.span_id(), span_id);
}

#[test]
fn extract_source() {
    let source = "fn main() { return 42; }";
    let mut table = SpanTable::new();
    let spanned = Spanned::with_range(
        &mut table,
        "return",
        source.find("return").unwrap()..source.find("return").unwrap() + 6,
    );
    assert_eq!(spanned.extract_source(&table, source), "return");
}

#[test]
fn file_tables_issue_module_unique_ids_that_name_their_file() {
    let mut first = SpanTable::for_file(0);
    let mut second = SpanTable::for_file(2);
    let a = first.insert(Span::new(1, 2));
    let b = second.insert(Span::new(3, 4));
    assert_ne!(a, b);
    assert_eq!(a.file(), 0);
    assert_eq!(b.file(), 2);
    assert_eq!(first.get(a), Span::new(1, 2));
    assert_eq!(second.get(b), Span::new(3, 4));
}

#[test]
fn a_file_table_does_not_resolve_another_files_ids() {
    let mut first = SpanTable::for_file(0);
    let mut second = SpanTable::for_file(1);
    first.insert(Span::new(1, 2));
    let foreign = second.insert(Span::new(3, 4));
    assert_eq!(first.get(foreign), Span::new(0, 0));
    assert_eq!(second.get(first.insert(Span::new(5, 6))), Span::new(0, 0));
}

#[test]
fn merge_stays_inside_the_file_table() {
    let mut table = SpanTable::for_file(3);
    let a = table.insert(Span::new(1, 2));
    let b = table.insert(Span::new(5, 9));
    let merged = table.merge(a, b);
    assert_eq!(merged.file(), 3);
    assert_eq!(table.get(merged), Span::new(1, 9));
}

#[test]
fn plain_tables_keep_their_unrestricted_ids() {
    let mut table = SpanTable::new();
    let id = table.insert(Span::new(0, 1));
    assert_eq!(id.into_inner(), 0);
    assert_eq!(table.get(id), Span::new(0, 1));
}

#[test]
#[should_panic]
fn file_index_past_the_limit_is_rejected() {
    SpanTable::for_file(SpanId::FILES_MAX);
}

#[test]
fn a_full_file_table_flags_overflow_instead_of_spilling_into_the_next_file() {
    let mut table = SpanTable::for_file(0);
    for _ in 0..SpanId::SPANS_PER_FILE {
        table.insert(Span::new(0, 0));
    }
    assert!(!table.overflowed());
    let last = table.insert(Span::new(7, 8));
    assert!(table.overflowed());
    assert_eq!(last.file(), 0, "an overflowed id still names this file");
    assert!(last.is_valid());
}
