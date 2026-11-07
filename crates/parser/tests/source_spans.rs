use ast::{
    Annotation, Argument, Binding, Convention, DomainBase, Expr, Expression, GroupPath, Import,
    ImportSource, Item, Parameter, Refinement, RefinementPredicate, ReturnStatement,
    SourceFile, Span, SpanId, Spanned, Statement, TypeBody, TypeDeclaration, TypeDomain,
    TypeMember, TypePrefixKind,
};

enum Work<'a> {
    Annotation(SpanId, &'a Spanned<Annotation>),
    Predicate(SpanId, &'a Spanned<RefinementPredicate>),
    Domain(SpanId, &'a Spanned<TypeDomain>),
    Binding(SpanId, &'a Binding),
    Type(SpanId, &'a TypeDeclaration),
    Expression(SpanId, &'a Expression),
    Statement(SpanId, &'a Spanned<Statement>),
}

#[test]
fn casts_and_pointer_types_preserve_utf8_edit_prefixes_and_queries() {
    let source = concat!(
        "--- hé 🦀\nPointer is @Outer\n",
        "Box has address @Outer\n",
        "cast(address @Outer) result Outer: return address as Outer\n",
        "value: (read() as @Box(Outer).Member, 'hé🦀' as Text)\n",
        "generic: Box(value: identity((@Outer), address))\n",
        "later: 2\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
        let prefix = &source[..end];
        let output = parser::parse(prefix).unwrap();
        check_file(&output.file);
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
        for offset in 0..=prefix.len() {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.unwrap();
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            for ancestor in hit.ancestors {
                assert!(ancestor.span.start() <= hit.node.span.start());
                assert!(ancestor.span.end() >= hit.node.span.end());
            }
        }
    }
}

#[test]
fn sequences_and_numeric_projections_preserve_utf8_edit_prefixes() {
    let source = concat!(
        "--- hé 🦀\nPair is (First, Second)\n",
        "Holder has\n    #private\n    field (First, Second)\n",
        "    method() result (First,)\n",
        "swap(value (First, Second)) (Second, First):\n",
        "    local (First, Second): ('hé🦀', (), value.0.1, value.[0])\n",
        "    return (value.1, value.0)\nlater: 2\n",
        "bare(): return value.0,value.[1]\n",
        "nested():\n    while flag do\n        return value.0,value.1\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
        let prefix = &source[..end];
        let output = parser::parse(prefix).unwrap();
        check_file(&output.file);
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
        for offset in 0..=prefix.len() {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.unwrap();
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            for ancestor in hit.ancestors {
                assert!(ancestor.span.start() <= hit.node.span.start());
                assert!(ancestor.span.end() >= hit.node.span.end());
            }
        }
    }
}

#[test]
fn unfinished_sequence_elements_keep_contained_recovery_nodes() {
    for source in [
        "value: (a, b +)\nlater: 2\n",
        "value: (a, b.0 +)\nlater: 2\n",
        "value: [a\nb +, c]\nlater: 2\n",
        "f(x (T,U +)): return x\nlater: 2\n",
        "f(): return a,b +\nlater: 2\n",
        "f(): return a,\nlater: 2\n",
        "f():\n    while flag do\n        return a,\nlater: 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        check_file(&output.file);
        for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
            let output = parser::parse(&source[..end]).unwrap();
            check_file(&output.file);
        }
    }
}

#[test]
fn private_fields_and_patterns_preserve_spans_through_edit_prefixes() {
    let source = concat!(
        "--- hé 🦀\nParcel(element) has\n",
        "    #private\n    item element\n",
        "    #private\n    #private\n    bound in 0 to 16\n",
        "take(value Parcel(element)) element:\n",
        "    Parcel(item, bound) := value\n    return item\n",
        "empty(value Marker):\n    Marker() := value\n    return\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
        let output = parser::parse(&source[..end]).unwrap();
        check_file(&output.file);
    }
}

#[test]
fn end_headers_preserve_spans_through_every_utf8_edit_prefix() {
    let source = concat!(
        "--- hé 🦀\nforward(tx end Transaction) Transaction: return tx\n",
        "root end Transaction: make()\n",
        "work():\n    local end: make()\n",
        "Thing has method(self end): return\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).unwrap();
        check_file(&output.file);
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
        for offset in 0..=end {
            if prefix.is_char_boundary(offset) {
                let hit = output.file.node_at_offset(offset).unwrap();
                assert!(prefix.get(hit.node.span.to_range()).is_some());
                for ancestor in &hit.ancestors {
                    assert!(ancestor.span.start() <= hit.node.span.start());
                    assert!(ancestor.span.end() >= hit.node.span.end());
                }
            }
        }
    }
}

#[test]
fn single_header_contracts_preserve_utf8_edit_prefixes() {
    let source = concat!(
        "--- hé 🦀\nmake(value Int) result Int where (result >= value or result = 0):\n",
        "    return value\n",
        "Thing has\n    method() named Int where named > 0\n    field Int\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).unwrap();
        check_file(&output.file);
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
        for offset in 0..=end {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.unwrap();
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            assert!(hit.node.span.start() <= offset && offset <= hit.node.span.end());
        }
    }
}

#[test]
fn header_claims_and_transitions_preserve_utf8_edit_prefixes_and_query_containment() {
    let source = concat!(
        "--- hé 🦀\nf(value Int) Int where (value >= 0 or value = 0), value :: value + 1:\n",
        "    before: value\n",
        "    return check(value)\n",
        "g() Int where >= 10: return 10\n",
        "later := 2\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).unwrap();
        check_file(&output.file);
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
        for offset in 0..=end {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.unwrap();
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            assert!(hit.node.span.start() <= offset && offset <= hit.node.span.end());
            let mut child = hit.node.span;
            for ancestor in hit.ancestors.iter().rev() {
                assert!(ancestor.span.start() <= child.start());
                assert!(child.end() <= ancestor.span.end());
                child = ancestor.span;
            }
        }
    }
}

#[test]
fn every_source_span_is_utf8_aligned_and_encloses_its_children() {
    let source = "\
--| module
use core.string.utf8 validate_utf8, folder.Name as renamed
use '../café/文.gin' as module
use Str as text, Str, Byte
use self.primitive
    Bool, Bool as Flag
Choice is Ready(Int) or Failed
Box(element Type, count: 4) has value element, read(self) element: return self.value
Box(element) has
    value element
    read(self) element:
        local := self.value
        return local
main(limit: 4, value Box(Int)) Box(Int):
    local(x Int) Int:
        return (x + 1)
    result := local(value.read(limit))
    return result
private
done := #todo
text := 'hé\\n'
ratio := 1.00
";
    let file = parser::parse_strict(source).expect("supported syntax with nested bodies");
    assert_eq!(file.items.len(), 12);
    check_file(&file);
}

#[test]
fn recovered_nodes_and_insertion_points_stay_inside_their_parent_spans() {
    for source in [
        "value := f(right:)    \nnext := 2\n",
        "value := f(1 +) + 2\n",
        "main:\n    value := f(\nnext := 2\n",
        "value := receiver.\n",
        "value := (",
        "value :=\nnext := 2\n",
        "main:\n    when x is\n        Unsupported if guard then 1\n    return 2\n",
        "use math\n    add as\nnext := 2\n",
        "Thing has\n    first Int\n    @bad\n    second Int\n",
        "Thing has\n    read(self):\n        value := receiver.\n        return value\n",
        "Alias is\nnext := 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        check_file(&output.file);
    }
}

#[test]
fn every_edit_prefix_keeps_navigable_utf8_aligned_syntax() {
    let source = "\
use '../café/文.gin' as module
use self.primitive Bool, folder.Name as renamed
use Str as text, Byte
Choice is Ready(Int) or Failed
Box(element Type, count: 4) has value element, other Int
Box(element) has
    value element
    read(self) element:
        local := self.value
        return local
main:
    local(x Int) Int:
        return (x + 1)
    value := f(local(1), right: receiver.field)
    text := 'hé\\n'
    return value
next := 2
";
    for end in 0..=source.len() {
        if source.is_char_boundary(end) {
            let prefix = &source[..end];
            let output = parser::parse(prefix).expect("normal edit fits the budgets");
            assert_eq!(output.file.text.as_ref(), prefix);
            check_file(&output.file);
            for diagnostic in output.diagnostics {
                assert!(
                    prefix.get(diagnostic.span.to_range()).is_some(),
                    "{diagnostic:?}"
                );
            }
        }
    }
}

#[test]
fn multiline_union_edit_prefixes_preserve_spans_and_query_containment() {
    let source = "\
--- hé 🦀
Choice(element) is
    First(element) or
        Box(Second)
    or Third
Choice has
    value Int
next := 2
";
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).unwrap();
        check_file(&output.file);
        for offset in 0..=end {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.unwrap();
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            for ancestor in hit.ancestors {
                assert!(ancestor.span.start() <= hit.node.span.start());
                assert!(hit.node.span.end() <= ancestor.span.end());
            }
        }
        for diagnostic in output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
    }
}

#[test]
fn domain_spans_survive_valid_recovered_and_every_utf8_edit_prefix() {
    for source in [
        "Signed is in -128 to 127 and (self >= -8 and self < 8)\n",
        "Bare is -limit to limit and < limit and >= -limit\n",
        "Named is in core.Size and self % 8 = 0",
        "T is in 0 to limit and = 0 and <= limit and > -limit\n",
        concat!(
            "--- hé 🦀\nT is\n    in -limit to\n        limit\n",
            "    and (self > 0)\n    and < limit\nnext := 2\n",
        ),
    ] {
        let file = parser::parse_strict(source).expect(source);
        check_file(&file);
        for end in 0..=source.len() {
            if !source.is_char_boundary(end) {
                continue;
            }
            let prefix = &source[..end];
            let output = parser::parse(prefix).expect(prefix);
            assert_eq!(output.file.text.as_ref(), prefix);
            check_file(&output.file);
            for offset in 0..=end {
                let hit = output.file.node_at_offset(offset);
                if !prefix.is_char_boundary(offset) {
                    assert!(hit.is_none());
                    continue;
                }
                let hit = hit.unwrap();
                assert!(prefix.get(hit.node.span.to_range()).is_some());
                for ancestor in hit.ancestors {
                    assert!(ancestor.span.start() <= hit.node.span.start());
                    assert!(hit.node.span.end() <= ancestor.span.end());
                }
            }
            for diagnostic in output.diagnostics {
                assert!(prefix.get(diagnostic.span.to_range()).is_some());
            }
        }
    }
    for incomplete in [
        "T is in",
        "T is in -",
        "T is in 0 to",
        "T is in 0 to -",
        "T is in core.Size and",
        "T is in 0 to limit and <",
        "T is in 0 to limit and (self >",
        "T is in 0 to limit\n    and <",
    ] {
        for suffix in ["", "\nNext has\n    value Int\nlast := 2\n"] {
            let source = format!("{incomplete}{suffix}");
            let output = parser::parse(&source).unwrap();
            assert!(output.has_errors(), "{source}");
            check_file(&output.file);
            let names: Vec<_> = output
                .file
                .declaration_outline()
                .map(|entry| entry.name)
                .collect();
            let expected = if suffix.is_empty() {
                vec!["T"]
            } else {
                vec!["T", "Next", "last"]
            };
            assert_eq!(names, expected, "{source}");
        }
    }
}

#[test]
fn annotation_edit_prefixes_preserve_spans_order_and_query_containment() {
    for source in [
        "--- hé 🦀\nvalue in 0 to max and < max: 1\nlater := 2\n",
        "--- hé 🦀\nf(x is >= 0 and < max, y Int and y > 0) in Count: return x\n",
        "Generic(element in Type, size in 0 to max) is Ready\n",
        "main:\n    local Int and (local > 0): 1\n    return local\n",
        "Thing has\n    method(self, value in Byte) Int: return value\n",
    ] {
        check_file(&parser::parse_strict(source).expect(source));
        for end in 0..=source.len() {
            if !source.is_char_boundary(end) {
                continue;
            }
            let prefix = &source[..end];
            let output = parser::parse(prefix).expect(prefix);
            check_file(&output.file);
            for diagnostic in &output.diagnostics {
                assert!(prefix.get(diagnostic.span.to_range()).is_some());
            }
            for offset in 0..=end {
                let hit = output.file.node_at_offset(offset);
                if !prefix.is_char_boundary(offset) {
                    assert!(hit.is_none());
                    continue;
                }
                let hit = hit.unwrap();
                assert!(prefix.get(hit.node.span.to_range()).is_some());
                assert!(hit.node.span.start() <= offset);
                assert!(offset <= hit.node.span.end());
                let mut child = hit.node.span;
                for ancestor in hit.ancestors.iter().rev() {
                    assert!(ancestor.span.start() <= child.start());
                    assert!(child.end() <= ancestor.span.end());
                    child = ancestor.span;
                }
            }
        }
    }
}

#[test]
fn convention_and_member_edit_prefixes_preserve_order_and_utf8_containment() {
    for source in [
        concat!(
            "--- hé 🦀\nThing(element own Type) has\n",
            "    field own Box(Int)\n",
            "    read(self ref in Byte, value mut Int and > 0) is >= 0:\n",
            "        local(x own): return x\n",
            "        return local(value)\nnext := 2\n",
        ),
        "Thing has field own in 0 to max and < max, read(self mut is >= 0) in Byte\n",
        "Thing has field own\nnext := 2\n",
        "Thing has value and > 0, method(self mut and self.count > 0) and > 0\n",
        "Thing has value own and > 0\nnext := 2\n",
        "Thing has method(self ref mut Byte): return self\nnext := 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        check_file(&output.file);
        for end in 0..=source.len() {
            if !source.is_char_boundary(end) {
                continue;
            }
            let prefix = &source[..end];
            let output = parser::parse(prefix).expect("bounded edit prefix");
            check_file(&output.file);
            for diagnostic in &output.diagnostics {
                assert!(prefix.get(diagnostic.span.to_range()).is_some());
            }
            for offset in 0..=end {
                let hit = output.file.node_at_offset(offset);
                if !prefix.is_char_boundary(offset) {
                    assert!(hit.is_none());
                    continue;
                }
                let hit = hit.unwrap();
                assert!(prefix.get(hit.node.span.to_range()).is_some());
                let mut child = hit.node.span;
                for parent in hit.ancestors.iter().rev() {
                    assert!(parent.span.start() <= child.start());
                    assert!(child.end() <= parent.span.end());
                    child = parent.span;
                }
            }
        }
    }
}

#[test]
fn borrowed_headers_and_groups_preserve_every_utf8_edit_prefix() {
    for source in [
        "--- hé 🦀\nAlias is ref{arena.Items} Box(mut{arena.inner} Item)\n",
        "--- hé 🦀\nf(x ref{arena.Items} mut{other} Item) ref{arena} (mut Item)\n",
        "Thing has\n    value ref{arena} Item\n    read(self mut{arena}) ref Item\n",
        "--- hé 🦀\nAlias is own Box(own Item)\n",
        concat!(
            "--- hé 🦀\nf(x own own Item) own own (ref Item):\n",
            "    local own own Item: x\n    return local\n",
        ),
        "--- hé 🦀\nThing has field own Box(own Item), read(self) own own Item\n",
    ] {
        check_file(&parser::parse_strict(source).expect(source));
        check_borrowed_prefixes(source);
    }
    for source in [
        "Alias is ref",
        "Alias is mut{arena}",
        "Alias is ref{",
        "Alias is ref{} Item",
        "Alias is ref{arena.",
        "Alias is ref{arena.Items",
        "Alias is ref{arena..items} Item",
        "Alias is ref{arena.if} Item",
        "Alias is ref{arena.mut} Item",
        "f(x ref{arena",
        "f(x ref mut)",
        "f(x own{arena} Item)",
        "Alias is own",
        "Alias is own{arena} Item",
        "f(x own own)",
    ] {
        let output = parser::parse(source).expect(source);
        assert!(output.has_errors(), "{source}");
        check_file(&output.file);
        check_borrowed_prefixes(source);
    }
}

#[test]
fn binding_conventions_preserve_every_utf8_edit_prefix() {
    for source in [
        concat!(
            "--- hé 🦀\nmake own Transaction:\n",
            "    tx own: Transaction.new('hé')\n    return tx\n",
            "Thing has\n    field own Item\n    method() own own Item: return field\n",
            "qualifier own\nfunction() own\nlater := 2\n",
        ),
        "value own own",
        "method() own own\nlater := 2\n",
        "Thing has method() own own, field own Item\nlater := 2\n",
        "value own{arena} Item\nlater := 2\n",
    ] {
        check_borrowed_prefixes(source);
    }
}

fn check_borrowed_prefixes(source: &str) {
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).expect(prefix);
        assert_eq!(output.file.text.as_ref(), prefix);
        check_file(&output.file);
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
        for offset in 0..=end {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.expect("every UTF8 boundary is navigable");
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            assert!(hit.node.span.start() <= offset);
            assert!(offset <= hit.node.span.end());
            let mut child = hit.node.span;
            for ancestor in hit.ancestors.iter().rev() {
                assert!(ancestor.span.start() <= child.start());
                assert!(child.end() <= ancestor.span.end());
                child = ancestor.span;
            }
        }
    }
}

#[test]
fn case_edit_prefixes_and_delimiter_mutations_remain_contained_and_queryable() {
    let source = concat!(
        "--- hé 🦀\nSlot(element) is Empty or Full(element)\n",
        "inspect(slot Slot(Int)):\n",
        "    result := when slot is\n",
        "        Empty then None\n",
        "        Full(value) then\n",
        "            Empty := slot\n",
        "            return when value is\n",
        "                Empty() then None\n",
        "                Full(inner) then Found(value: inner)\n",
        "    return result\nlater := 2\n",
    );
    check_borrowed_prefixes(source);
    for (position, character) in source.char_indices() {
        if matches!(character, '(' | ')' | ',') {
            let mut edited = source.to_owned();
            edited.remove(position);
            let output = parser::parse(&edited).unwrap();
            check_file(&output.file);
            for offset in 0..=edited.len() {
                if edited.is_char_boundary(offset) {
                    output.file.node_at_offset(offset).unwrap();
                }
            }
        }
    }
}

#[test]
fn integer_case_edit_prefixes_keep_exact_utf8_containment_and_queries() {
    let source = concat!(
        "--- hé 🦀\nchoose(index in 0 to 1) Result:\n",
        "    local := when index is\n",
        "        00_000 then Zero\n",
        "        - 9_999_999_999_999_999_999_999_999_999 then One\n",
        "    return when index is\n",
        "        0 then Zero\n",
        "        1 then local\n",
        "later := 2\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    check_borrowed_prefixes(source);
    for pattern in ["-", "--0", "+0", "-name", "0 + 1", "0 to 1", ")", ",", "0."] {
        let source =
            format!("--- hé 🦀\nchoose: when index is\n    {pattern} then Zero\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        check_file(&output.file);
    }
}

#[test]
fn collection_and_loop_edit_prefixes_and_mutations_keep_contained_utf8_queries() {
    let source = concat!(
        "--- hé 🦀\nVector is [ref T; count]\n",
        "scan(input [T; count]) [T; count]:\n",
        "    values := ['hé🦀', [1, 2], (0; count)]\n",
        "    for item in 0 to count - 1 do\n",
        "        while item < count do\n",
        "            local := input.[item; count]\n",
        "            return local\n",
        "    return values.[0]\nlater := 2\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    check_borrowed_prefixes(source);
    for (position, character) in source.char_indices() {
        if matches!(character, '(' | ')' | '[' | ']' | ',' | ';' | '.') {
            for replacement in ["", "(", ")", "[", "]", ",", ";", "."] {
                let mut edited = source.to_owned();
                edited.replace_range(position..position + character.len_utf8(), replacement);
                let output = parser::parse(&edited).unwrap();
                check_file(&output.file);
                for offset in 0..=edited.len() {
                    if edited.is_char_boundary(offset) {
                        output.file.node_at_offset(offset).unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn do_keyword_mutations_preserve_loop_spans_queries_and_following_items() {
    let source = concat!(
        "--- hé 🦀\nmain:\n",
        "    for item in items do\n",
        "        while item < count do\n",
        "            return item\n",
        "later := 2\n",
    );
    check_file(&parser::parse_strict(source).unwrap());
    check_borrowed_prefixes(source);
    for (position, _) in source.match_indices("do") {
        for replacement in ["", "d", "do do"] {
            let mut edited = source.to_owned();
            edited.replace_range(position..position + 2, replacement);
            let output = parser::parse(&edited).unwrap();
            assert!(output.has_errors(), "{edited}");
            check_file(&output.file);
            assert_eq!(
                output.file.declaration_outline().last().unwrap().name,
                "later"
            );
            for offset in 0..=edited.len() {
                if edited.is_char_boundary(offset) {
                    output.file.node_at_offset(offset).unwrap();
                }
            }
        }
    }
}

#[test]
fn missing_loop_case_subjects_keep_containment_after_consuming_their_newline() {
    for source in [
        "--- hé 🦀\nbad := when while flag do\nlater := 2\n",
        "--- hé 🦀\nbad := when for item in items\nlater := 2\n",
        "--- hé 🦀\nbad := when for item in items do\nlater := 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        check_file(&output.file);
        check_borrowed_prefixes(source);
        for offset in 0..=source.len() {
            if source.is_char_boundary(offset) {
                output.file.node_at_offset(offset).unwrap();
            }
        }
    }
}

#[test]
fn borrowed_recovery_retains_missing_targets_and_written_group_components() {
    for (source, expected_components, closed) in [
        ("Alias is ref", None, false),
        ("Alias is mut{arena}", Some(vec!["arena"]), true),
        ("Alias is ref{", Some(vec![]), false),
        (
            "Alias is ref{arena.Items",
            Some(vec!["arena", "Items"]),
            false,
        ),
        ("Alias is ref{arena.", Some(vec!["arena"]), false),
    ] {
        let output = parser::parse(source).expect(source);
        assert!(output.has_errors());
        check_file(&output.file);
        let Item::Type(declaration) = &output.file.items[0].value else {
            panic!("expected recovered alias: {source}");
        };
        let TypeBody::Is(expression) = &declaration.body.value else {
            panic!("expected recovered alias expression: {source}");
        };
        let Expr::PrefixedType(borrowed) = &expression.value else {
            panic!("expected retained borrow prefix: {source}");
        };
        assert!(matches!(borrowed.target.value, Expr::Missing), "{source}");
        assert!(check_span(&output.file, borrowed.target.span_id).is_empty());
        match (expected_components, &borrowed.group) {
            (None, None) => {}
            (Some(expected), Some(group)) => {
                assert_eq!(group.value.close.is_some(), closed);
                let actual: Vec<_> = group
                    .value
                    .components
                    .iter()
                    .map(|name| name.value.as_str())
                    .collect();
                assert_eq!(actual, expected, "{source}");
            }
            _ => panic!("group presence differs from written source: {source}"),
        }
    }
}

fn check_file(file: &SourceFile) {
    let source = file.text.as_ref();
    let mut work = Vec::new();
    ordered(file, &file.items);
    ordered(file, &file.comments);
    for item in &file.items {
        check_span(file, item.span_id);
        match &item.value {
            Item::Binding(binding) => {
                work.push(Work::Binding(item.span_id, binding));
            }
            Item::Expression(expression) => work.push(Work::Expression(item.span_id, expression)),
            Item::Private => assert_eq!(check_span(file, item.span_id).extract(source), "private"),
            Item::Import(import) => check_import(file, item.span_id, import),
            Item::Type(declaration) => work.push(Work::Type(item.span_id, declaration)),
            Item::Entry => assert_eq!(check_span(file, item.span_id).extract(source), "#entry"),
            Item::Wrap => assert_eq!(check_span(file, item.span_id).extract(source), "#wrap"),
            Item::Load => assert_eq!(check_span(file, item.span_id).extract(source), "#load"),
            Item::Store => assert_eq!(check_span(file, item.span_id).extract(source), "#store"),
            Item::Instruction { text } => {
                contained(file, item.span_id, *text);
            }
            Item::Unrestricted => {
                assert_eq!(
                    check_span(file, item.span_id).extract(source),
                    "#unrestricted"
                );
            }
            Item::Error => {}
        }
    }
    for comment in &file.comments {
        check_span(file, comment.span_id);
    }
    for _ in 0..10_000 {
        let Some(node) = work.pop() else {
            return;
        };
        match node {
            Work::Annotation(parent, annotation) => {
                check_annotation(file, parent, annotation, &mut work);
            }
            Work::Predicate(parent, predicate) => {
                check_predicate(file, parent, predicate, &mut work);
            }
            Work::Domain(parent, domain) => check_domain(file, parent, domain, &mut work),
            Work::Type(span, declaration) => {
                check_type(file, span, declaration, &mut work);
            }
            Work::Binding(span, binding) => check_binding(file, span, binding, &mut work),
            Work::Expression(parent, expression) => {
                check_expression(file, parent, expression, &mut work);
            }
            Work::Statement(parent, statement) => {
                contained(file, parent, statement.span_id);
                match &statement.value {
                    Statement::Destructure(pattern) => {
                        contained(file, statement.span_id, pattern.pattern.span_id);
                        check_pattern(file, pattern.pattern.span_id, &pattern.pattern.value);
                        contained(file, statement.span_id, pattern.operator.span_id);
                        work.push(Work::Expression(statement.span_id, &pattern.value));
                    }
                    Statement::Assign(assign) => {
                        contained(file, statement.span_id, assign.place.span_id);
                        contained(file, statement.span_id, assign.operator);
                        assert_eq!(check_span(file, assign.operator).extract(&file.text), "::");
                        ordered_ids(
                            file,
                            &[assign.place.span_id, assign.operator, assign.value.span_id],
                        );
                        work.push(Work::Expression(statement.span_id, &assign.place));
                        work.push(Work::Expression(statement.span_id, &assign.value));
                    }
                    Statement::Binding(binding) => {
                        work.push(Work::Binding(statement.span_id, binding));
                    }
                    Statement::Expression(expression) => {
                        work.push(Work::Expression(statement.span_id, expression));
                    }
                    Statement::Return(returned) => {
                        check_return(file, statement.span_id, returned, &mut work);
                    }
                    Statement::When(when) => {
                        contained(file, statement.span_id, when.condition.span_id);
                        work.push(Work::Expression(statement.span_id, &when.condition));
                        let otherwise = when.else_branch.iter().map(|branch| &branch.block);
                        for block in std::iter::once(&when.then_block).chain(otherwise) {
                            contained(file, statement.span_id, block.span_id);
                            for inner in &block.value.statements {
                                work.push(Work::Statement(block.span_id, inner));
                            }
                        }
                    }
                    Statement::WhenIs(when) => {
                        contained(file, statement.span_id, when.subject.span_id);
                        contained(file, statement.span_id, when.pattern.span_id);
                        work.push(Work::Expression(statement.span_id, &when.subject));
                        let otherwise = when.else_branch.iter().map(|branch| &branch.block);
                        for block in std::iter::once(&when.then_block).chain(otherwise) {
                            contained(file, statement.span_id, block.span_id);
                            for inner in &block.value.statements {
                                work.push(Work::Statement(block.span_id, inner));
                            }
                        }
                    }
                    Statement::Error => {}
                }
            }
        }
    }
    panic!("span traversal exceeded the fixture's node budget");
}

fn check_return<'a>(
    file: &SourceFile,
    span: SpanId,
    returned: &'a ReturnStatement,
    work: &mut Vec<Work<'a>>,
) {
    let siblings: Vec<_> = returned.value.iter().map(|value| value.span_id).collect();
    if let Some(value) = &returned.value {
        work.push(Work::Expression(span, value));
    }
    ordered_ids(file, &siblings);
}

fn check_contract<'a>(
    file: &SourceFile,
    parent: SpanId,
    contract: &'a Spanned<ast::Contract>,
    work: &mut Vec<Work<'a>>,
) {
    use ast::ClauseItem;
    contained(file, parent, contract.span_id);
    check_keyword(file, contract.span_id, contract.value.where_keyword, "where");
    assert_eq!(
        contract.value.joiners.len(),
        contract.value.items.len().saturating_sub(1)
    );
    let mut siblings = vec![contract.value.where_keyword];
    for (index, item) in contract.value.items.iter().enumerate() {
        contained(file, contract.span_id, item.span_id);
        if index > 0 {
            let joiner = contract.value.joiners[index - 1];
            let spelling = check_span(file, joiner).extract(&file.text);
            assert!(spelling == "," || spelling == "and", "{spelling:?}");
            contained(file, contract.span_id, joiner);
            siblings.push(joiner);
        }
        siblings.push(item.span_id);
        match &item.value {
            ClauseItem::Transition(transition) => {
                check_keyword(file, item.span_id, transition.operator, "::");
                ordered_ids(
                    file,
                    &[
                        transition.place.span_id,
                        transition.operator,
                        transition.value.span_id,
                    ],
                );
                work.push(Work::Expression(item.span_id, &transition.place));
                work.push(Work::Expression(item.span_id, &transition.value));
            }
            ClauseItem::Claim(RefinementPredicate::Expression(expression)) => {
                work.push(Work::Expression(item.span_id, expression));
            }
            ClauseItem::Claim(RefinementPredicate::ImplicitComparison { operator, right }) => {
                contained(file, item.span_id, operator.span_id);
                ordered_ids(file, &[operator.span_id, right.span_id]);
                work.push(Work::Expression(item.span_id, right));
            }
            ClauseItem::Claim(RefinementPredicate::ImplicitDomain(domain)) => {
                contained(file, item.span_id, domain.span_id);
            }
        }
    }
    ordered_ids(file, &siblings);
}

fn check_binding<'a>(
    file: &SourceFile,
    span: SpanId,
    binding: &'a Binding,
    work: &mut Vec<Work<'a>>,
) {
    for attribute in &binding.attributes {
        contained(file, span, attribute.span_id);
        match attribute.value {
            ast::FieldAttribute::Private {
                hash,
                private_keyword,
            } => {
                contained(file, attribute.span_id, hash);
                contained(file, attribute.span_id, private_keyword);
            }
            ast::FieldAttribute::Register {
                hash,
                keyword,
                open,
                name,
                close,
            } => {
                for part in [hash, keyword, open, name, close] {
                    contained(file, attribute.span_id, part);
                }
            }
            ast::FieldAttribute::Infix {
                hash,
                keyword,
                open,
                operator,
                close,
            } => {
                for part in [hash, keyword, open, operator, close] {
                    contained(file, attribute.span_id, part);
                }
            }
            ast::FieldAttribute::Inline { hash, keyword } => {
                contained(file, attribute.span_id, hash);
                contained(file, attribute.span_id, keyword);
            }
        }
    }
    contained(file, span, binding.name.span_id);
    assert_eq!(
        check_span(file, binding.name.span_id).extract(&file.text),
        binding.name.value
    );
    if let Some(annotation) = &binding.annotation {
        work.push(Work::Annotation(span, annotation));
    }
    if let Some(contract) = &binding.contract {
        check_contract(file, span, contract, work);
    }
    if let Some(value) = &binding.value {
        contained(file, span, value.operator.span_id);
        work.push(Work::Expression(span, &value.expression));
    }
    if let Some(parameters) = &binding.parameters {
        check_parameters(file, span, parameters, work);
    }
    let mut siblings = vec![binding.name.span_id];
    if let Some(parameters) = &binding.parameters {
        siblings.push(parameters.span_id);
    }
    if let Some(convention) = &binding.convention {
        let spelling = match convention.value {
            ast::BindingConvention::Own => "own",
            ast::BindingConvention::End => "end",
        };
        check_keyword(file, span, convention.span_id, spelling);
        siblings.push(convention.span_id);
    }
    if let Some(name) = &binding.result_binder {
        check_name(file, span, name);
        siblings.push(name.span_id);
    }
    if let Some(annotation) = &binding.annotation {
        siblings.push(annotation.span_id);
    }
    if let Some(contract) = &binding.contract {
        siblings.push(contract.span_id);
    }
    if let Some(value) = &binding.value {
        siblings.push(value.operator.span_id);
        siblings.push(value.expression.span_id);
    }
    ordered_ids(file, &siblings);
}

fn check_parameters<'a>(
    file: &SourceFile,
    parent: SpanId,
    parameters: &'a Spanned<Vec<Spanned<Parameter>>>,
    work: &mut Vec<Work<'a>>,
) {
    contained(file, parent, parameters.span_id);
    ordered(file, &parameters.value);
    for parameter in &parameters.value {
        contained(file, parameters.span_id, parameter.span_id);
        check_name(file, parameter.span_id, &parameter.value.name);
        let mut siblings = vec![parameter.value.name.span_id];
        if let Some(convention) = &parameter.value.convention {
            contained(file, parameter.span_id, convention.span_id);
            let keyword = &convention.value.keyword;
            let spelling = match keyword.value {
                Convention::Ref => "ref",
                Convention::Mut => "mut",
                Convention::Own => "own",
                Convention::End => "end",
            };
            check_keyword(file, convention.span_id, keyword.span_id, spelling);
            if let Some(group) = &convention.value.group {
                check_group(file, convention.span_id, group);
                ordered_ids(file, &[keyword.span_id, group.span_id]);
            }
            siblings.push(convention.span_id);
        }
        if let Some(annotation) = &parameter.value.annotation {
            siblings.push(annotation.span_id);
            work.push(Work::Annotation(parameter.span_id, annotation));
        }
        if let Some(default) = &parameter.value.default {
            siblings.push(default.span_id);
            work.push(Work::Expression(parameter.span_id, default));
        }
        ordered_ids(file, &siblings);
    }
}

fn check_type<'a>(
    file: &SourceFile,
    span: SpanId,
    declaration: &'a TypeDeclaration,
    work: &mut Vec<Work<'a>>,
) {
    check_name(file, span, &declaration.name);
    let mut siblings = vec![declaration.name.span_id];
    if let Some(parameters) = &declaration.parameters {
        check_parameters(file, span, parameters, work);
        siblings.push(parameters.span_id);
    }
    siblings.push(declaration.body.span_id);
    ordered_ids(file, &siblings);
    contained(file, span, declaration.body.span_id);
    match &declaration.body.value {
        TypeBody::Domain(domain) => {
            work.push(Work::Domain(declaration.body.span_id, domain));
        }
        TypeBody::Is(expression) => {
            work.push(Work::Expression(declaration.body.span_id, expression));
        }
        TypeBody::Has(members) => {
            ordered(file, members);
            for member in members {
                contained(file, declaration.body.span_id, member.span_id);
                match &member.value {
                    TypeMember::Binding(binding) => {
                        work.push(Work::Binding(member.span_id, binding));
                    }
                    TypeMember::Provision(_) | TypeMember::Error => {}
                    TypeMember::ProvisionApplied(expression) => {
                        work.push(Work::Expression(member.span_id, expression));
                    }
                }
            }
        }
    }
}

fn check_domain<'a>(
    file: &SourceFile,
    parent: SpanId,
    domain: &'a Spanned<TypeDomain>,
    work: &mut Vec<Work<'a>>,
) {
    contained(file, parent, domain.span_id);
    let value = &domain.value;
    let mut siblings = Vec::new();
    if let Some(keyword) = value.in_keyword {
        check_keyword(file, domain.span_id, keyword, "in");
        siblings.push(keyword);
    }
    contained(file, domain.span_id, value.base.span_id);
    siblings.push(value.base.span_id);
    match &value.base.value {
        DomainBase::Expression(expression) => {
            work.push(Work::Expression(value.base.span_id, expression));
        }
        DomainBase::Range {
            lower,
            to_keyword,
            upper,
        } => {
            check_keyword(file, value.base.span_id, *to_keyword, "to");
            ordered_ids(file, &[lower.span_id, *to_keyword, upper.span_id]);
            work.push(Work::Expression(value.base.span_id, lower));
            work.push(Work::Expression(value.base.span_id, upper));
        }
    }
    check_refinements(file, domain.span_id, &value.refinements, work);
    for refinement in &value.refinements {
        siblings.push(refinement.span_id);
    }
    ordered_ids(file, &siblings);
}

fn check_refinements<'a>(
    file: &SourceFile,
    parent: SpanId,
    refinements: &'a [Spanned<Refinement>],
    work: &mut Vec<Work<'a>>,
) {
    ordered(file, refinements);
    for refinement in refinements {
        contained(file, parent, refinement.span_id);
        let clause = &refinement.value;
        check_keyword(file, refinement.span_id, clause.and_keyword, "and");
        ordered_ids(file, &[clause.and_keyword, clause.predicate.span_id]);
        work.push(Work::Predicate(refinement.span_id, &clause.predicate));
    }
}

fn check_predicate<'a>(
    file: &SourceFile,
    parent: SpanId,
    predicate: &'a Spanned<RefinementPredicate>,
    work: &mut Vec<Work<'a>>,
) {
    contained(file, parent, predicate.span_id);
    match &predicate.value {
        RefinementPredicate::Expression(expression) => {
            work.push(Work::Expression(predicate.span_id, expression));
        }
        RefinementPredicate::ImplicitComparison { operator, right } => {
            contained(file, predicate.span_id, operator.span_id);
            let spelling = match operator.value {
                ast::BinaryOperator::Equal => "=",
                ast::BinaryOperator::Less => "<",
                ast::BinaryOperator::LessOrEqual => "<=",
                ast::BinaryOperator::Greater => ">",
                ast::BinaryOperator::GreaterOrEqual => ">=",
                other => panic!("not a shorthand comparison: {other:?}"),
            };
            check_keyword(file, predicate.span_id, operator.span_id, spelling);
            ordered_ids(file, &[operator.span_id, right.span_id]);
            work.push(Work::Expression(predicate.span_id, right));
        }
        RefinementPredicate::ImplicitDomain(domain) => {
            contained(file, predicate.span_id, domain.span_id);
            contained(file, predicate.span_id, domain.base.span_id);
            if let Some(keyword) = domain.in_keyword {
                contained(file, predicate.span_id, keyword);
            }
        }
    }
}

fn check_annotation<'a>(
    file: &SourceFile,
    parent: SpanId,
    annotation: &'a Spanned<Annotation>,
    work: &mut Vec<Work<'a>>,
) {
    let span = annotation.span_id;
    contained(file, parent, span);
    let (head, refinements) = match &annotation.value {
        Annotation::Expression(expression) => {
            work.push(Work::Expression(span, expression));
            return;
        }
        Annotation::Domain { is_keyword, domain } => {
            if let Some(keyword) = is_keyword {
                check_keyword(file, span, *keyword, "is");
                ordered_ids(file, &[*keyword, domain.span_id]);
            }
            work.push(Work::Domain(span, domain));
            return;
        }
        Annotation::Refined { base, refinements } => {
            if let Some(base) = base {
                work.push(Work::Expression(span, base));
                if let Some(first) = refinements.first() {
                    ordered_ids(file, &[base.span_id, first.span_id]);
                }
            }
            check_refinements(file, span, refinements, work);
            return;
        }
        Annotation::Bound { has_keyword, bound } => {
            check_keyword(file, span, *has_keyword, "has");
            ordered_ids(file, &[*has_keyword, bound.span_id]);
            work.push(Work::Expression(span, bound));
            return;
        }
        Annotation::Predicate {
            is_keyword,
            predicate,
            refinements,
        } => {
            check_keyword(file, span, *is_keyword, "is");
            ordered_ids(file, &[*is_keyword, predicate.span_id]);
            work.push(Work::Predicate(span, predicate));
            (predicate.span_id, refinements)
        }
    };
    check_refinements(file, span, refinements, work);
    if let Some(first) = refinements.first() {
        ordered_ids(file, &[head, first.span_id]);
    }
}

fn check_keyword(file: &SourceFile, parent: SpanId, keyword: SpanId, spelling: &str) {
    contained(file, parent, keyword);
    assert_eq!(check_span(file, keyword).extract(&file.text), spelling);
}

fn check_import(file: &SourceFile, span: SpanId, import: &Import) {
    ordered(file, &import.entries);
    for entry in &import.entries {
        contained(file, span, entry.span_id);
        let value = &entry.value;
        contained(file, entry.span_id, value.source.span_id);
        match &value.source.value {
            ImportSource::Path => {
                let spelling = check_span(file, value.source.span_id).extract(&file.text);
                assert!(spelling.starts_with('\''));
                // Recovery can retain an unterminated quoted path at an edit prefix.
            }
            ImportSource::Module(path) => check_path(file, value.source.span_id, path),
            ImportSource::Current(name) => check_name(file, value.source.span_id, name),
        }
        ordered(file, &value.members);
        let mut previous = value.source.span_id;
        for member in &value.members {
            contained(file, entry.span_id, member.span_id);
            ordered_ids(file, &[previous, member.span_id]);
            previous = member.span_id;
            check_path(file, member.span_id, &member.value.path);
            if let Some(alias) = &member.value.alias {
                check_name(file, member.span_id, alias);
                ordered_ids(
                    file,
                    &[member.value.path.last().unwrap().span_id, alias.span_id],
                );
            }
        }
        if let Some(alias) = &value.alias {
            check_name(file, entry.span_id, alias);
            ordered_ids(file, &[previous, alias.span_id]);
        }
    }
}

fn check_path(file: &SourceFile, parent: SpanId, path: &[ast::Name]) {
    assert!(!path.is_empty());
    ordered(file, path);
    for name in path {
        check_name(file, parent, name);
    }
}

fn check_name(file: &SourceFile, parent: SpanId, name: &ast::Name) {
    contained(file, parent, name.span_id);
    assert!(
        !name.value.is_empty(),
        "missing names must not be synthesized"
    );
    assert_eq!(
        check_span(file, name.span_id).extract(&file.text),
        name.value
    );
}

fn ordered<T>(file: &SourceFile, nodes: &[Spanned<T>]) {
    for pair in nodes.windows(2) {
        ordered_ids(file, &[pair[0].span_id, pair[1].span_id]);
    }
}

fn ordered_ids(file: &SourceFile, ids: &[SpanId]) {
    for pair in ids.windows(2) {
        let left = check_span(file, pair[0]);
        let right = check_span(file, pair[1]);
        assert!(left.end() <= right.start(), "{left:?} precedes {right:?}");
    }
}

fn check_expression<'a>(
    file: &SourceFile,
    parent: SpanId,
    expression: &'a Expression,
    work: &mut Vec<Work<'a>>,
) {
    let span = expression.span_id;
    contained(file, parent, span);
    match &expression.value {
        Expr::Sequence(sequence) => {
            if let ast::SequenceDelimiters::Parenthesized { open, close } =
                sequence.delimiters
            {
                contained(file, span, open);
                contained(file, span, close);
            } else {
                assert!(sequence.elements.len() >= 2);
                assert_eq!(sequence.separators.len() + 1, sequence.elements.len());
            }
            ordered(file, &sequence.elements);
            for element in &sequence.elements {
                work.push(Work::Expression(span, element));
            }
            for separator in &sequence.separators {
                contained(file, span, *separator);
                assert_eq!(check_span(file, *separator).extract(&file.text), ",");
            }
        }
        Expr::NumericSelect(select) => {
            contained(file, span, select.dot);
            assert_eq!(check_span(file, select.dot).extract(&file.text), ".");
            ordered_ids(
                file,
                &[select.base.span_id, select.dot, select.index.span_id],
            );
            work.push(Work::Expression(span, &select.base));
            work.push(Work::Expression(span, &select.index));
        }
        Expr::Array(array) => {
            contained(file, span, array.open);
            contained(file, span, array.close);
            ordered(file, &array.elements);
            for element in &array.elements {
                work.push(Work::Expression(span, element));
            }
            for separator in &array.separators {
                contained(file, span, *separator);
            }
            if let Some(length) = &array.length {
                contained(file, span, length.separator);
                work.push(Work::Expression(span, &length.value));
            }
        }
        Expr::Repeat(repeat) => {
            contained(file, span, repeat.open);
            contained(file, span, repeat.close);
            contained(file, span, repeat.separator);
            ordered_ids(
                file,
                &[
                    repeat.value.span_id,
                    repeat.separator,
                    repeat.length.span_id,
                ],
            );
            work.push(Work::Expression(span, &repeat.value));
            work.push(Work::Expression(span, &repeat.length));
        }
        Expr::Select(select) => {
            contained(file, span, select.open);
            contained(file, span, select.close);
            contained(file, span, select.dot);
            work.push(Work::Expression(span, &select.base));
            work.push(Work::Expression(span, &select.index));
            if let Some(length) = &select.length {
                contained(file, span, length.separator);
                work.push(Work::Expression(span, &length.value));
            }
        }
        Expr::Range {
            start,
            to_keyword,
            end,
        } => {
            contained(file, span, *to_keyword);
            ordered_ids(file, &[start.span_id, *to_keyword, end.span_id]);
            work.push(Work::Expression(span, start));
            work.push(Work::Expression(span, end));
        }
        Expr::Loop(loop_expression) => match loop_expression.as_ref() {
            ast::LoopExpression::For {
                for_keyword,
                binder,
                in_keyword,
                iterable,
                invariant,
                do_keyword,
                body,
            } => {
                contained(file, span, *for_keyword);
                contained(file, span, *in_keyword);
                contained(file, span, *do_keyword);
                check_name(file, span, binder);
                if let Some(invariant) = invariant {
                    contained(file, span, invariant.where_keyword);
                    work.push(Work::Expression(span, &invariant.condition));
                }
                work.push(Work::Expression(span, iterable));
                work.push(Work::Expression(span, body));
            }
            ast::LoopExpression::While {
                while_keyword,
                condition,
                invariant,
                do_keyword,
                body,
            } => {
                contained(file, span, *while_keyword);
                contained(file, span, *do_keyword);
                if let Some(invariant) = invariant {
                    contained(file, span, invariant.where_keyword);
                    work.push(Work::Expression(span, &invariant.condition));
                }
                work.push(Work::Expression(span, condition));
                work.push(Work::Expression(span, body));
            }
        },
        Expr::PrefixedType(prefix) => check_type_prefix(file, span, prefix, work),
        Expr::Cast {
            value,
            as_keyword,
            target,
        } => {
            check_keyword(file, span, *as_keyword, "as");
            ordered_ids(file, &[value.span_id, *as_keyword, target.span_id]);
            work.push(Work::Expression(span, value));
            work.push(Work::Expression(span, target));
        }
        Expr::Query { hash, name } => {
            check_keyword(file, span, *hash, "#");
            contained(file, span, name.span_id);
            ordered_ids(file, &[*hash, name.span_id]);
        }
        Expr::IsTest(test) => {
            check_keyword(file, span, test.is_keyword, "is");
            contained(file, span, test.pattern.span_id);
            ordered_ids(
                file,
                &[test.subject.span_id, test.is_keyword, test.pattern.span_id],
            );
            work.push(Work::Expression(span, &test.subject));
        }
        Expr::Unary { operator, operand } => {
            contained(file, span, operator.span_id);
            let written = match operator.value {
                ast::UnaryOperator::Negate => "-",
                ast::UnaryOperator::Deref => "deref",
                ast::UnaryOperator::Not => "not",
                ast::UnaryOperator::DomainOf => "domain of",
            };
            assert_eq!(
                check_span(file, operator.span_id).extract(&file.text),
                written
            );
            ordered_ids(file, &[operator.span_id, operand.span_id]);
            work.push(Work::Expression(span, operand));
        }
        Expr::Name(name) => assert_eq!(check_span(file, span).extract(&file.text), name),
        Expr::Literal(_) | Expr::Hole => {}
        Expr::Missing => assert!(check_span(file, span).is_empty()),
        Expr::IncompleteMember { base } => work.push(Work::Expression(span, base)),
        Expr::Member { base, name } => {
            check_name(file, span, name);
            ordered_ids(file, &[base.span_id, name.span_id]);
            work.push(Work::Expression(span, base));
        }
        Expr::Call { callee, arguments } => {
            work.push(Work::Expression(span, callee));
            contained(file, span, arguments.span_id);
            ordered_ids(file, &[callee.span_id, arguments.span_id]);
            ordered(file, &arguments.value);
            for argument in &arguments.value {
                contained(file, arguments.span_id, argument.span_id);
                match &argument.value {
                    Argument::Positional(value) => {
                        work.push(Work::Expression(argument.span_id, value));
                    }
                    Argument::Named { name, value } => {
                        check_name(file, argument.span_id, name);
                        ordered_ids(file, &[name.span_id, value.span_id]);
                        work.push(Work::Expression(argument.span_id, value));
                    }
                    Argument::Typed { name, annotation } => {
                        check_name(file, argument.span_id, name);
                        ordered_ids(file, &[name.span_id, annotation.span_id]);
                        work.push(Work::Expression(argument.span_id, annotation));
                    }
                }
            }
        }
        Expr::Binary {
            left,
            operator,
            right,
        } => {
            contained(file, span, operator.span_id);
            ordered_ids(file, &[left.span_id, operator.span_id, right.span_id]);
            work.push(Work::Expression(span, left));
            work.push(Work::Expression(span, right));
        }
        Expr::Group(inner) => work.push(Work::Expression(span, inner)),
        Expr::Bind(binding) => work.push(Work::Binding(span, binding)),
        Expr::Conditional(case) => {
            for keyword in [case.when_keyword, case.then_keyword, case.else_keyword] {
                contained(file, span, keyword);
            }
            // Source order: when, condition, then, value, else, value.
            let ordered_spans = [
                case.when_keyword,
                case.condition.span_id,
                case.then_keyword,
                case.then_branch.span_id,
                case.else_keyword,
                case.else_branch.span_id,
            ];
            let mut previous = 0;
            for id in ordered_spans {
                let part = file.span_table.get(id);
                assert!(
                    part.start() >= previous,
                    "conditional parts are in source order"
                );
                previous = part.end();
            }
            for part in [&case.condition, &case.then_branch, &case.else_branch] {
                work.push(Work::Expression(span, part));
            }
        }
        Expr::Chain(chain) => {
            contained(file, span, chain.when_keyword);
            contained(file, span, chain.else_keyword);
            ordered(file, &chain.arms);
            let mut previous = file.span_table.get(chain.when_keyword).end();
            for arm in &chain.arms {
                contained(file, span, arm.span_id);
                contained(file, arm.span_id, arm.value.then_keyword);
                for id in [
                    arm.value.condition.span_id,
                    arm.value.then_keyword,
                    arm.value.value.span_id,
                ] {
                    let part = file.span_table.get(id);
                    assert!(part.start() >= previous, "chain parts are in source order");
                    previous = part.end();
                }
                work.push(Work::Expression(arm.span_id, &arm.value.condition));
                work.push(Work::Expression(arm.span_id, &arm.value.value));
            }
            work.push(Work::Expression(span, &chain.otherwise));
        }
        Expr::Match(case) => {
            contained(file, span, case.when_keyword);
            contained(file, span, case.is_keyword);
            contained(file, span, case.arms.span_id);
            work.push(Work::Expression(span, &case.subject));
            ordered(file, &case.arms.value);
            for arm in &case.arms.value {
                contained(file, case.arms.span_id, arm.span_id);
                let pattern = &arm.value.pattern;
                contained(file, arm.span_id, pattern.span_id);
                match &pattern.value {
                    ast::MatchPattern::Variant(variant) => {
                        check_pattern(file, pattern.span_id, variant);
                    }
                    ast::MatchPattern::Integer(expression) => {
                        assert_eq!(pattern.span_id, expression.span_id);
                        match &expression.value {
                            Expr::Literal(ast::LiteralKind::Integer) => {}
                            Expr::Unary { operand, .. } => assert!(matches!(
                                operand.value,
                                Expr::Literal(ast::LiteralKind::Integer)
                            )),
                            _ => panic!("integer pattern must retain literal syntax"),
                        }
                        work.push(Work::Expression(pattern.span_id, expression));
                    }
                    ast::MatchPattern::Domain { in_keyword, domain } => {
                        check_keyword(file, pattern.span_id, *in_keyword, "in");
                        work.push(Work::Expression(pattern.span_id, domain));
                    }
                    ast::MatchPattern::Text(expression) => {
                        assert_eq!(pattern.span_id, expression.span_id);
                        work.push(Work::Expression(pattern.span_id, expression));
                    }
                    ast::MatchPattern::Else { else_keyword } => {
                        check_keyword(file, pattern.span_id, *else_keyword, "else");
                    }
                }
                contained(file, arm.span_id, arm.value.then_keyword);
                ordered_ids(
                    file,
                    &[
                        pattern.span_id,
                        arm.value.then_keyword,
                        arm.value.body.span_id,
                    ],
                );
                work.push(Work::Expression(arm.span_id, &arm.value.body));
            }
        }
        Expr::Block(block) => {
            ordered(file, &block.statements);
            for statement in &block.statements {
                work.push(Work::Statement(span, statement));
            }
        }
    }
}

fn check_pattern(file: &SourceFile, span: SpanId, pattern: &ast::VariantPattern) {
    contained(file, span, pattern.constructor.span_id);
    if let Some(fields) = &pattern.fields {
        contained(file, span, fields.binders.span_id);
        contained(file, fields.binders.span_id, fields.open);
        contained(file, fields.binders.span_id, fields.close);
        for binder in &fields.binders.value {
            contained(file, fields.binders.span_id, binder.span_id);
        }
    }
}

fn check_type_prefix<'a>(
    file: &SourceFile,
    span: SpanId,
    prefix: &'a ast::PrefixedType,
    work: &mut Vec<Work<'a>>,
) {
    let spelling = match prefix.keyword.value {
        TypePrefixKind::Ref => "ref",
        TypePrefixKind::Mut => "mut",
        TypePrefixKind::Own => "own",
        TypePrefixKind::Pointer => "@",
    };
    check_keyword(file, span, prefix.keyword.span_id, spelling);
    let mut siblings = vec![prefix.keyword.span_id];
    if let Some(group) = &prefix.group {
        check_group(file, span, group);
        siblings.push(group.span_id);
    }
    siblings.push(prefix.target.span_id);
    ordered_ids(file, &siblings);
    work.push(Work::Expression(span, &prefix.target));
}

fn check_group(file: &SourceFile, parent: SpanId, group: &Spanned<GroupPath>) {
    contained(file, parent, group.span_id);
    check_keyword(file, group.span_id, group.value.open, "{");
    let mut siblings = vec![group.value.open];
    for component in &group.value.components {
        check_name(file, group.span_id, component);
        siblings.push(component.span_id);
    }
    for pair in group.value.components.windows(2) {
        let left = check_span(file, pair[0].span_id);
        let right = check_span(file, pair[1].span_id);
        let punctuation = file.text[left.end()..right.start()].trim();
        assert!(!punctuation.is_empty());
        assert!(
            punctuation
                .chars()
                .all(|ch| ch == '.' || ch.is_whitespace())
        );
    }
    if let Some(close) = group.value.close {
        check_keyword(file, group.span_id, close, "}");
        siblings.push(close);
        assert_eq!(
            check_span(file, group.span_id).end(),
            check_span(file, close).end()
        );
    }
    ordered_ids(file, &siblings);
    assert_eq!(
        check_span(file, group.span_id).start(),
        check_span(file, group.value.open).start()
    );
}

fn check_span(file: &SourceFile, id: SpanId) -> Span {
    assert!(id.is_valid());
    let span = file.span_table.get(id);
    assert!(file.text.get(span.to_range()).is_some(), "{span:?}");
    span
}

fn contained(file: &SourceFile, parent: SpanId, child: SpanId) {
    let parent = check_span(file, parent);
    let child = check_span(file, child);
    assert!(parent.start() <= child.start(), "{parent:?} / {child:?}");
    assert!(child.end() <= parent.end(), "{parent:?} / {child:?}");
}
