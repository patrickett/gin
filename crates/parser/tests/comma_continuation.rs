//! A line may end in a comma inside a `use` list or a `where` clause, and the next line, indented
//! deeper than the line that began the list, continues it: one import member or claim per line.

use ast::{Item, TypeBody, TypeMember};

fn one_line_and_broken(single: &str, broken: &str) {
    let first = parser::parse_strict(single).unwrap_or_else(|d| panic!("{single:?}: {d:?}"));
    let second = parser::parse_strict(broken).unwrap_or_else(|d| panic!("{broken:?}: {d:?}"));
    assert_eq!(first.items.len(), second.items.len());
    // The same tree apart from where things sit.
    let shape = |file: &ast::SourceFile| format!("{:?}", file.items).split("SpanId(").count();
    assert_eq!(shape(&first), shape(&second));
}

fn rejected(source: &str) {
    assert!(parser::parse_strict(source).is_err(), "must stay a flaw: {source:?}");
}

#[test]
fn use_members_continue_after_a_comma() {
    one_line_and_broken("use core.x A, B, C\n", "use core.x A,\n    B,\n    C\n");
    one_line_and_broken("use core.x A, B\nuse core.y D\n", "use core.x A,\n    B\nuse core.y D\n");
    let file = parser::parse_strict("use core.x A,\n    B,\n\n    C\n").unwrap();
    let Item::Import(import) = &file.items[0].value else { panic!("import") };
    assert_eq!(import.entries[0].value.members.len(), 3);
}

#[test]
fn where_claims_continue_after_a_comma() {
    one_line_and_broken(
        "clear(self mut) where self.length :: 0, self.start :: 0:\n    return\n",
        "clear(self mut) where self.length :: 0,\n    self.start :: 0:\n    return\n",
    );
    one_line_and_broken(
        "f(a Int) Int where a > 0, a < 9:\n    return a\n",
        "f(a Int) Int where a > 0,\n        a < 9,\n        a /= 5:\n    return a\n"
            .replace(",\n        a /= 5", "")
            .as_str(),
    );
}

#[test]
fn method_header_claims_continue_inside_a_type() {
    let source = "\
Thing has
    length Int
    start Int

    clear(self mut) where self.length :: 0,
        self.start :: 0:
        return

    other(self mut) where self.length :: 1:
        return
";
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(declaration) = &file.items[0].value else { panic!("type") };
    let TypeBody::Has(members) = &declaration.body.value else { panic!("has") };
    let TypeMember::Binding(clear) = &members[2].value else { panic!("method") };
    assert_eq!(clear.contract.as_ref().unwrap().value.items.len(), 2);
    assert_eq!(members.len(), 4);
}

#[test]
fn transitions_and_result_variants_continue() {
    one_line_and_broken(
        "swap(a mut, b mut) where a :: b, b :: a:\n    return\n",
        "swap(a mut, b mut) where a :: b,\n    b :: a:\n    return\n",
    );
    // Parenthesised variant claims, already free to break inside the parentheses, and a trailing
    // `or` that continues onto the next variant.
    one_line_and_broken(
        "f(self mut) Ok where (self.a >= 1, self.b >= 2) or\n    Bad where (self :: self):\n    return\n",
        "f(self mut) Ok where (self.a >= 1,\n        self.b >= 2) or\n    Bad where (self :: self):\n    return\n",
    );
}

#[test]
fn a_break_before_the_comma_is_still_a_flaw() {
    rejected("use core.x A\n    , B\n");
    rejected("clear(self mut) where self.length :: 0\n    , self.start :: 0:\n    return\n");
    rejected("f(self mut) Ok where (self.a >= 1\n    , self.b >= 2):\n    return\n");
}

#[test]
fn a_line_that_would_start_a_new_statement_is_not_a_continuation() {
    // Same indentation as the line that began the list: the comma is dangling, as before.
    rejected("use core.x A,\nrun: 1\n");
    rejected("clear(self mut) where self.length :: 0,\nrun: 1\n");
}

#[test]
fn a_dangling_comma_is_still_a_flaw() {
    rejected("use core.x A,\n");
    rejected("use core.x A,\n\n");
    rejected("clear(self mut) where self.length :: 0,\n");
}

#[test]
fn commas_elsewhere_do_not_continue() {
    rejected("x: 1,\n    y: 2\n");
    rejected("run:\n    x: f(a),\n        b\n    return x\n");
}
