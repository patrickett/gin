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
    assert_eq!(table.len(), 2);
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
