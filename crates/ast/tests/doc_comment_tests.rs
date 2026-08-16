use ast::DocComment;

#[test]
fn combine_both_none() {
    assert_eq!(DocComment::combine(None, None), None);
}

#[test]
fn combine_left_only() {
    let a = DocComment::new("Doc A");
    let result = DocComment::combine(Some(a.clone()), None);
    assert_eq!(result, Some(a));
}

#[test]
fn combine_right_only() {
    let b = DocComment::new("Doc B");
    let result = DocComment::combine(None, Some(b.clone()));
    assert_eq!(result, Some(b));
}

#[test]
fn combine_both_present() {
    let a = DocComment::new("Doc A");
    let b = DocComment::new("Doc B");
    let result = DocComment::combine(Some(a), Some(b));
    assert_eq!(result.unwrap().value, "Doc A\n\nDoc B");
}

#[test]
fn combine_both_empty() {
    assert_eq!(DocComment::combine(Some(DocComment::new("")), None), None);
    assert_eq!(DocComment::combine(None, Some(DocComment::new(""))), None);
    assert_eq!(
        DocComment::combine(Some(DocComment::new("")), Some(DocComment::new(""))),
        None
    );
}

#[test]
fn combine_doc_comment_pre_and_post_variant() {
    // Simulates: --- Doc B
    //            Some(x) or --- Doc C
    let b = DocComment::new("Doc B");
    let c = DocComment::new("Doc C");
    let result = DocComment::combine(Some(b), Some(c));
    assert_eq!(result.unwrap().value, "Doc B\n\nDoc C");
}

#[test]
fn combine_doc_comment_triple() {
    // Simulates:
    //   --- Doc A
    //   Maybe(x) is
    //       --- Doc B
    //       Some(x) or --- Doc C
    let a = DocComment::new("Doc A");
    let b = DocComment::new("Doc B");
    let c = DocComment::new("Doc C");
    let ab = DocComment::combine(Some(a), Some(b));
    let abc = DocComment::combine(ab, Some(c));
    assert_eq!(abc.unwrap().value, "Doc A\n\nDoc B\n\nDoc C");
}
