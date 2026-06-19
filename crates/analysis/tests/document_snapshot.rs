//! Tests for `PackageCache::document_snapshot` — verifying that the returned
//! source, parse output, and line index are consistent and usable.

use analysis::{CacheEngine, QueryEngine};
use ast::source::SourceExt;
use crossbeam_channel::unbounded;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a unique temporary directory per call so parallel tests don't collide.
fn unique_temp_dir(name: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("ginlsp_snapshot_{name}_{id}"))
}

fn make_test_file(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn document_snapshot_returns_all_three_fields() {
    let dir = unique_temp_dir("three_fields");
    let path = make_test_file(&dir, "main.gin", "hello is Unit\nworld is Unit\n");

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let snapshot = engine.snapshot();
    let doc = snapshot
        .document_snapshot(&path)
        .expect("document should exist");

    assert_eq!(doc.path, path);
    assert_eq!(doc.source.as_str(), "hello is Unit\nworld is Unit\n");
    // "hello is Unit\nworld is Unit\n" has 3 line starts: [0, 15, 30]
    assert_eq!(doc.line_index.line_count(), 3);

    // The line index should produce correct positions.
    // "hello is Unit\n" = h(0) e1 l2 l3 o4 ' '5 i6 s7 ' '8 U9 n10 i11 t12 \n13
    // "world is Unit\n" = w14 o15 r16 l17 d18 ' '19 i20 s21 ' '22 U23 n24 i25 t26 \n27
    assert_eq!(doc.line_index.byte_to_position(&doc.source, 0), (0, 0));
    assert_eq!(doc.line_index.byte_to_position(&doc.source, 12), (0, 12));
    assert_eq!(doc.line_index.byte_to_position(&doc.source, 14), (1, 0));
    assert_eq!(doc.line_index.byte_to_position(&doc.source, 15), (1, 1));
}

#[test]
fn document_snapshot_uses_cached_line_index() {
    let dir = unique_temp_dir("cached");
    let path = make_test_file(&dir, "data.gin", "a\nb\nc\n");

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // First snapshot — "a\nb\nc\n" has 4 line starts: [0, 2, 4, 6]
    // bytes: a0 \n1 b2 \n3 c4 \n5
    let snap1 = engine.snapshot();
    let doc1 = snap1.document_snapshot(&path).unwrap();
    assert_eq!(doc1.line_index.line_count(), 4);
    assert_eq!(doc1.line_index.byte_to_position(&doc1.source, 0), (0, 0));
    assert_eq!(doc1.line_index.byte_to_position(&doc1.source, 2), (1, 0));
    assert_eq!(doc1.line_index.byte_to_position(&doc1.source, 3), (1, 1));

    // Update contents
    engine.set_contents(&path, "a\nb\nc\nd\ne\nf\n".to_string());

    // Second snapshot — "a\nb\nc\nd\ne\nf\n": 7 lines, [0, 2, 4, 6, 8, 10, 12]
    // bytes: a0 \n1 b2 \n3 c4 \n5 d6 \n7 e8 \n9 f10 \n11
    let snap2 = engine.snapshot();
    let doc2 = snap2.document_snapshot(&path).unwrap();
    assert_eq!(doc2.line_index.line_count(), 7);
    assert_eq!(doc2.line_index.byte_to_position(&doc2.source, 3), (1, 1));
    assert_eq!(doc2.line_index.byte_to_position(&doc2.source, 9), (4, 1));
}

#[test]
fn document_snapshot_line_index_consistent_with_sourceext() {
    let dir = unique_temp_dir("consistency");
    let content = "use core.true\n\nmain:\n    core.true\n    check(core.true)\nreturn\n";
    let path = make_test_file(&dir, "main.gin", content);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let snapshot = engine.snapshot();
    let doc = snapshot.document_snapshot(&path).unwrap();

    // byte_to_position must match SourceExt for all valid char-boundary bytes.
    for byte in 0..content.len() {
        if !content.is_char_boundary(byte) {
            continue;
        }
        let expected = content.byte_offset_to_position(byte);
        let actual = doc.line_index.byte_to_position(content, byte);
        assert_eq!(expected, actual, "mismatch at byte {byte}");
    }

    // position_to_byte: only check columns that are within the line's UTF-16 length.
    // LineIndex is stricter than SourceExt (which scans past newlines),
    // so we compare only at valid columns.
    for line in 0..doc.line_index.line_count() as u32 {
        // Determine valid column range by probing.
        for col in 0..=30 {
            let idx = doc.line_index.position_to_byte(content, line, col);
            if idx.is_some() {
                let ext = content.position_to_byte_offset(line, col);
                assert_eq!(
                    ext, idx,
                    "mismatch at line {line}, col {col} (ext={ext:?}, idx={idx:?})"
                );
            }
        }
    }
}

#[test]
fn document_snapshot_returns_none_for_unknown_file() {
    let (tx, _rx) = unbounded();
    let engine = CacheEngine::new(tx);
    let snapshot = engine.snapshot();

    assert!(
        snapshot
            .document_snapshot(&PathBuf::from("/nonexistent/file.gin"))
            .is_none()
    );
}

#[test]
fn document_snapshot_fields_are_coherent() {
    let dir = unique_temp_dir("coherent");
    let path = make_test_file(&dir, "test.gin", "x is Unit\n");

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let snapshot = engine.snapshot();
    let doc = snapshot.document_snapshot(&path).unwrap();

    assert!(doc.line_index.line_count() >= 1);
    assert!(!doc.source.is_empty());

    // parse should have produced an AST with a span table.
    let _ = &doc.parse.ast.span_table;

    // For "x is Unit\n", line 1 should exist.
    let byte = doc.line_index.position_to_byte(&doc.source, 1, 0);
    assert!(byte.is_some(), "line 1 should exist");
}
