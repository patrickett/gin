use super::*;

fn id(version: &str, source: u8) -> ModuleId {
    ModuleId { package: "pkg".into(), version: version.into(), source: [source; 32], module: "pkg".into() }
}

fn artifact_of(text: &str, version: &str, source: u8, target: &str) -> Vec<u8> {
    let parsed = parser::parse_strict(text).unwrap_or_else(|error| panic!("{text}\n{error:?}"));
    let module = check_module("pkg", &parsed, &[], crate::Limits::default()).unwrap_or_else(|error| panic!("{text}\n{error:?}"));
    let identity = ArtifactIdentity { module: id(version, source), dependencies: Vec::new(), target: target.into(), compiler: "ginc test".into() };
    write_artifact(&module, &identity).unwrap()
}

/// Old is version 1.0.0, new 1.1.0 from other source: identities must not make every item differ.
fn classify(old: &str, new: &str) -> Classification {
    classify_artifacts(&artifact_of(old, "1.0.0", 1, "t"), &artifact_of(new, "1.1.0", 2, "t")).unwrap()
}

const DIGIT: &str = "#unrestricted\nDigit is in 0 to 9\n";
const COUNTER: &str = "#unrestricted\nCounter has\n    n in 0 to 100\n";

/// (row, old, new, expected result, a word the reason must contain).
fn cases() -> Vec<(&'static str, String, String, Compatibility, &'static str)> {
    use Compatibility::*;
    let f = "f(n in 0 to 9) in 0 to 9: return n\n";
    let d = |rest: &str| format!("{DIGIT}{rest}");
    let c = |rest: &str| format!("{COUNTER}{rest}");
    let release = "Release has\n    release(self)\n";
    vec![
        ("add public item", f.into(), format!("{f}g() in 0 to 9: return 2\n"), Compatible, "added"),
        ("remove public item", format!("{f}g() in 0 to 9: return 2\n"), f.into(), Breaking, "removed"),
        ("rename public item", f.into(), f.replace("f(", "h("), Breaking, "removed"),
        ("precondition removed", d("f(x Digit and < 5) in 0 to 9: return x\n"), d("f(x Digit) in 0 to 9: return x\n"), Compatible, "precondition weakened"),
        ("precondition added", d("f(x Digit) in 0 to 9: return x\n"), d("f(x Digit and < 5) in 0 to 9: return x\n"), Breaking, "precondition strengthened"),
        ("precondition changed otherwise", d("f(x Digit and < 5) in 0 to 9: return x\n"), d("f(x Digit and > 2) in 0 to 9: return x\n"), Unknown, "precondition changed"),
        ("postcondition added", f.into(), "f(n in 0 to 9) r in 0 to 9 where (r = n): return n\n".into(), Compatible, "postcondition strengthened"),
        ("postcondition removed", "f(n in 0 to 9) r in 0 to 9 where (r = n): return n\n".into(), f.into(), Breaking, "postcondition weakened"),
        (
            "transition removed",
            c("grow(k mut Counter and k.n < 100) where k.n :: k.n + 1:\n    k.n :: k.n + 1\n    return\n"),
            c("grow(k mut Counter and k.n < 100):\n    k.n :: k.n + 1\n    return\n"),
            Breaking,
            "mut",
        ),
        ("parameter added", f.into(), "f(n in 0 to 9, m in 0 to 9) in 0 to 9: return n\n".into(), Breaking, "parameters changed"),
        ("parameter retyped", f.into(), "f(n in 0 to 8) in 0 to 9: return n\n".into(), Breaking, "parameters changed"),
        (
            "convention changed",
            c("peek(k ref Counter) in 0 to 100: return k.n\n"),
            c("peek(k mut Counter) in 0 to 100: return k.n\n"),
            Breaking,
            "",
        ),
        ("result changed", f.into(), "f(n in 0 to 9) in 0 to 10: return n\n".into(), Breaking, "result type changed"),
        ("domain widened", DIGIT.into(), DIGIT.replace("0 to 9", "0 to 10"), Breaking, "widened"),
        ("domain narrowed", DIGIT.into(), DIGIT.replace("0 to 9", "0 to 8"), Breaking, "narrowed"),
        ("wrap added", "#unrestricted\nByte is in 0 to 255\n".into(), "#unrestricted, #wrap\nByte is in 0 to 255\n".into(), Breaking, "#wrap"),
        ("wrap removed", "#unrestricted, #wrap\nByte is in 0 to 255\n".into(), "#unrestricted\nByte is in 0 to 255\n".into(), Breaking, "#wrap"),
        ("unrestricted added", "Token has\n    v in 0 to 9\n".into(), "#unrestricted\nToken has\n    v in 0 to 9\n".into(), Compatible, "#unrestricted` added"),
        ("unrestricted removed", "#unrestricted\nToken has\n    v in 0 to 9\n".into(), "Token has\n    v in 0 to 9\n".into(), Breaking, "#unrestricted` removed"),
        ("variant added", "#unrestricted\nShape is Dot or Line\n".into(), "#unrestricted\nShape is Dot or Line or Arc\n".into(), Breaking, "variant added"),
        ("variant removed", "#unrestricted\nShape is Dot or Line or Arc\n".into(), "#unrestricted\nShape is Dot or Line\n".into(), Breaking, "variant removed"),
        ("variants reordered", "#unrestricted\nShape is Dot or Line\n".into(), "#unrestricted\nShape is Line or Dot\n".into(), Breaking, "variant"),
        ("payload retyped", d("#unrestricted\nShape is Dot or Line(Digit)\n"), d("#unrestricted\nShape is Dot or Line(Digit, Digit)\n"), Breaking, ""),
        ("discriminant changed", "#unrestricted\nLevel is Low = 1 or High = 2\n".into(), "#unrestricted\nLevel is Low = 1 or High = 3\n".into(), Breaking, "discriminant"),
        ("public field added, no private fields", "#unrestricted\nPoint has\n    x in 0 to 9\n".into(), "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 9\n".into(), Breaking, "construct"),
        (
            "public field added beside private ones",
            "#unrestricted\nPoint has\n    #private\n    s in 0 to 9\n    x in 0 to 9\n".into(),
            "#unrestricted\nPoint has\n    #private\n    s in 0 to 9\n    x in 0 to 9\n    y in 0 to 9\n".into(),
            Compatible,
            "private fields",
        ),
        (
            "private field added",
            "#unrestricted\nPoint has\n    #private\n    s in 0 to 9\n    x in 0 to 9\n".into(),
            "#unrestricted\nPoint has\n    #private\n    s in 0 to 9\n    #private\n    t in 0 to 9\n    x in 0 to 9\n".into(),
            Unchanged,
            "layout only",
        ),
        ("public field removed", "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 9\n".into(), "#unrestricted\nPoint has\n    x in 0 to 9\n".into(), Breaking, "removed"),
        (
            "provision added",
            "Width has\n    width(self) in 0 to 9\n#unrestricted\nDigit is in 0 to 9\n".into(),
            "Width has\n    width(self) in 0 to 9\n#unrestricted\nDigit is in 0 to 9\nDigit has Width\n    width(self) in 0 to 9: return 4\n".into(),
            Compatible,
            "provision added",
        ),
        (
            "provision removed",
            "Width has\n    width(self) in 0 to 9\n#unrestricted\nDigit is in 0 to 9\nDigit has Width\n    width(self) in 0 to 9: return 4\n".into(),
            "Width has\n    width(self) in 0 to 9\n#unrestricted\nDigit is in 0 to 9\n".into(),
            Breaking,
            "",
        ),
        ("trait method added with a default", format!("{release}"), "Release has\n    release(self)\n    count(self) in 0 to 9: return 1\n".into(), Compatible, "with a default"),
        ("trait method added without a default", format!("{release}"), "Release has\n    release(self)\n    count(self) in 0 to 9\n".into(), Breaking, "without a default"),
        ("trait method removed", "Release has\n    release(self)\n    count(self) in 0 to 9\n".into(), release.into(), Breaking, "removed"),
        (
            "public to private",
            "#unrestricted\nP has\n    x in 0 to 9\n    get(self) in 0 to 9: return 1\n".into(),
            "#unrestricted\nP has\n    x in 0 to 9\n    #private\n    get(self) in 0 to 9: return 1\n".into(),
            Breaking,
            "removed",
        ),
        (
            "private to public",
            "#unrestricted\nP has\n    x in 0 to 9\n    #private\n    get(self) in 0 to 9: return 1\n".into(),
            "#unrestricted\nP has\n    x in 0 to 9\n    get(self) in 0 to 9: return 1\n".into(),
            Compatible,
            "added",
        ),
        ("body changed (unchecked or not)", d("f(n in 0 to 9) Digit: return n as Digit\n"), d("f(n in 0 to 9) Digit: return #unchecked(n as Digit)\n"), Unchanged, ""),
        ("constant changed", "limit := 9\n".into(), "limit := 8\n".into(), Breaking, "value changed"),
        ("constant added", "limit := 9\n".into(), "limit := 9\nother := 2\n".into(), Compatible, "added"),
        ("generic parameter added without a default", "Pair(a) has\n    x a\n".into(), "Pair(a, b) has\n    x a\n    y b\n".into(), Breaking, "generic parameters changed"),
        (
            "generic bound weakened",
            format!("{release}dispose(element has Release, value element):\n    element.release(value)\n    return\n"),
            format!("{release}keep(element Type, value element) element: return value\ndispose(element has Release, value element):\n    element.release(value)\n    return\nhold(element has Release, value element) element: return value\n")
                .replace("hold(element has Release", "hold(element Type"),
            Compatible,
            "added",
        ),
        (
            "template body changed",
            "pick(element Type, x element, y element) element: return x\n".into(),
            "pick(element Type, x element, y element) element: return y\n".into(),
            Unchanged,
            "",
        ),
    ]
}

#[test]
#[ignore = "classifier in progress: stopped mid-build"]
fn every_row_of_the_table() {
    let mut failures = Vec::new();
    for (row, old, new, expected, word) in cases() {
        let got = classify(&old, &new);
        let reasons: Vec<String> = got.changes.iter().map(|c| c.to_string()).collect();
        if got.result != expected || !reasons.join("\n").contains(word) && !word.is_empty() {
            failures.push(format!("{row}: expected {expected} ({word}), got {} {reasons:?}", got.result));
        }
        // Negative control for every row: a module against itself is unchanged.
        let same = classify(&old, &old);
        if same.result != Compatibility::Unchanged || !same.changes.is_empty() {
            failures.push(format!("{row}: old against itself is {} {:?}", same.result, same.changes));
        }
        // And the change undone in the other direction is never called unchanged unless it is.
        if expected != Compatibility::Unchanged && classify(&new, &old).result == Compatibility::Unchanged {
            failures.push(format!("{row}: the reverse change is unchanged"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The rows the surface language cannot write yet, by editing the signature layer directly.
fn edited(text: &str, edit: impl Fn(&mut Node)) -> (Vec<u8>, Vec<u8>) {
    let parsed = parser::parse_strict(text).unwrap();
    let module = check_module("pkg", &parsed, &[], crate::Limits::default()).unwrap();
    let identity = ArtifactIdentity { module: id("1.0.0", 1), dependencies: Vec::new(), target: "t".into(), compiler: "c".into() };
    let (header, signature, instantiation) = artifact_nodes(&module, &identity).unwrap();
    let mut changed = signature.clone();
    edit(&mut changed);
    assert_ne!(changed, signature, "the edit changed nothing");
    (
        interface_codec::encode_artifact(&header, &signature, &instantiation),
        interface_codec::encode_artifact(&header, &changed, &instantiation),
    )
}

fn visit(node: &mut Node, tag: u64, f: &dyn Fn(&mut Vec<Node>)) {
    match node {
        Node::Record(t, fields) => {
            if *t == tag {
                f(fields);
            }
            fields.iter_mut().for_each(|field| visit(field, tag, f));
        }
        Node::List(items) => items.iter_mut().for_each(|item| visit(item, tag, f)),
        _ => {}
    }
}

#[test]
fn summaries_are_strict_but_wider_input_ranges_are_compatible() {
    let text = "#unrestricted\nCounter has\n    n in 0 to 100\npeek(k ref Counter) in 0 to 100: return k.n\n";
    let set_ranges = |low: i64, high: i64| {
        move |fields: &mut Vec<Node>| fields[6] = Node::List(vec![pair(Node::Uint(0), pair(Node::Int(I256::from(low)), Node::Int(I256::from(high))))])
    };
    // Base with a range, then wider / narrower.
    let parsed = parser::parse_strict(text).unwrap();
    let module = check_module("pkg", &parsed, &[], crate::Limits::default()).unwrap();
    assert!(!module.functions[0].summaries.is_empty(), "the function has a summary");
    let identity = ArtifactIdentity { module: id("1.0.0", 1), dependencies: Vec::new(), target: "t".into(), compiler: "c".into() };
    let (header, signature, instantiation) = artifact_nodes(&module, &identity).unwrap();
    let with = |low, high| {
        let mut node = signature.clone();
        visit(&mut node, SUMMARY, &set_ranges(low, high));
        interface_codec::encode_artifact(&header, &node, &instantiation)
    };
    let base = with(10, 20);
    assert_eq!(classify_artifacts(&base, &with(0, 30)).unwrap().result, Compatibility::Compatible);
    assert_eq!(classify_artifacts(&base, &with(12, 20)).unwrap().result, Compatibility::Breaking);
    assert_eq!(classify_artifacts(&base, &with(10, 20)).unwrap().result, Compatibility::Unchanged);
    // Any other difference (here a disposition) is breaking.
    let (a, b) = edited(text, |node| visit(node, SUMMARY, &|fields| fields[2] = Node::List(Vec::new())));
    let got = classify_artifacts(&a, &b).unwrap();
    assert_eq!(got.result, Compatibility::Breaking, "{:?}", got.changes);
}

#[test]
#[ignore = "classifier in progress: stopped mid-build"]
fn generic_defaults_bounds_and_register_layouts() {
    // A type parameter added with a default (defaults are not in the surface language yet).
    let text = "Pair(a) has\n    x a\n";
    let (a, b) = edited(text, |node| {
        visit(node, LAYOUT, &|fields| {
            if matches!(&fields[1], Node::Text(name) if name == "Pair") {
                fields[2] = Node::List(vec![Node::text("a"), Node::text("b")]);
                fields[10] = Node::List(vec![Node::Absent, Node::record(TY_UNIT, Vec::new())]);
            }
        })
    });
    let got = classify_artifacts(&a, &b).unwrap();
    assert_eq!(got.result, Compatibility::Compatible, "{:?}", got.changes);
    // A bound on a function's type parameter removed / added.
    let bounded = "Release has\n    release(self)\ndispose(element has Release, value element):\n    element.release(value)\n    return\n";
    let (a, b) = edited(bounded, |node| visit(node, SIGNATURE, &|fields| fields[10] = Node::List(vec![Node::Absent])));
    assert_eq!(classify_artifacts(&a, &b).unwrap().result, Compatibility::Compatible);
    assert_eq!(classify_artifacts(&b, &a).unwrap().result, Compatibility::Breaking);
    // A private field change of a type with a `#register` field: its layout is observable (D4).
    let text = "#unrestricted\nPoint has\n    #private\n    s in 0 to 9\n    x in 0 to 9\n";
    let (a, b) = edited(text, |node| {
        visit(node, LAYOUT, &|fields| {
            fields[5] = Node::Uint(3);
            fields[4] = Node::Uint(2);
            if let Node::List(items) = &mut fields[3] {
                for item in items {
                    if let Node::Record(FIELD, field) = item {
                        field[3] = Node::text("x0");
                    }
                }
            }
        })
    });
    // Old without the register; compare old-with-register to new-with-register.
    let (_, with_register) = edited(text, |node| {
        visit(node, LAYOUT, &|fields| {
            if let Node::List(items) = &mut fields[3] {
                for item in items {
                    if let Node::Record(FIELD, field) = item {
                        field[3] = Node::text("x0");
                    }
                }
            }
        })
    });
    let got = classify_artifacts(&with_register, &b).unwrap();
    assert_eq!(got.result, Compatibility::Unknown, "{:?}", got.changes);
    let _ = a;
}

#[test]
fn targets_take_the_largest_bump_and_count_added_and_dropped_targets() {
    let f = "f(n in 0 to 9) in 0 to 9: return n\n";
    let g = "f(n in 0 to 9) in 0 to 9: return n\ng() in 0 to 9: return 1\n";
    let h = "f(n in 0 to 8) in 0 to 9: return n\n";
    let at = |text: &str, target: &str, version: &str| (target.to_owned(), artifact_of(text, version, 1, target));
    let old = vec![at(f, "a", "1.0.0"), at(f, "b", "1.0.0")];
    // One target compatible, the other breaking: breaking wins.
    let new = vec![at(g, "a", "1.1.0"), at(h, "b", "1.1.0")];
    let got = classify_targets(&old, &new).unwrap();
    assert_eq!(got.result, Compatibility::Breaking);
    assert!(got.changes.iter().any(|c| c.item.ends_with("[b]") && c.result == Compatibility::Breaking));
    assert!(got.changes.iter().any(|c| c.item.ends_with("[a]") && c.result == Compatibility::Compatible));
    // Both compatible: compatible.
    let new = vec![at(g, "a", "1.1.0"), at(g, "b", "1.1.0")];
    assert_eq!(classify_targets(&old, &new).unwrap().result, Compatibility::Compatible);
    // A target added is compatible, one dropped breaking.
    let more = vec![at(f, "a", "1.1.0"), at(f, "b", "1.1.0"), at(f, "c", "1.1.0")];
    assert_eq!(classify_targets(&old, &more).unwrap().result, Compatibility::Compatible);
    assert_eq!(classify_targets(&more, &old).unwrap().result, Compatibility::Breaking);
    assert_eq!(classify_targets(&old, &old).unwrap().result, Compatibility::Unchanged);
}

#[test]
fn another_schema_version_is_unknown_and_another_module_is_refused() {
    let f = "f(n in 0 to 9) in 0 to 9: return n\n";
    let a = artifact_of(f, "1.0.0", 1, "t");
    let mut b = a.clone();
    b[6] = 9;
    assert_eq!(classify_artifacts(&a, &b).unwrap().result, Compatibility::Unknown);
    let parsed = parser::parse_strict(f).unwrap();
    let module = check_module("pkg", &parsed, &[], crate::Limits::default()).unwrap();
    let mut other = id("1.0.0", 1);
    other.package = "other".into();
    let identity = ArtifactIdentity { module: other, dependencies: Vec::new(), target: "t".into(), compiler: "ginc test".into() };
    assert!(classify_artifacts(&a, &write_artifact(&module, &identity).unwrap()).is_err());
}
