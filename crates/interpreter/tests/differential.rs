//! Differential tests: the same program through the interpreter and through the compiler.
//!
//! Programs and oracles are the ones `tools/ginc/tests` already runs under `lli` or as a native
//! executable (`guard_lowering.rs`, `chain_lowering.rs`, `monomorphization.rs`,
//! `payload_variants.rs`). Each case is checked three ways:
//!
//! 1. the interpreter against the oracle the compiler's test states (always);
//! 2. the interpreter against the compiled program (when `ginc` and the LLVM tools exist; set
//!    `INTERPRETER_NEXT_REQUIRE_COMPILED=1` to make their absence a failure);
//! 3. so, transitively, the compiled program against that oracle.
mod common;
use common::*;
use interpreter::Error;

/// A function of one integer `n in 0 to 9` and what it must return for each `n`.
struct Numeric {
    name: &'static str,
    source: &'static str,
    function: &'static str,
    oracle: fn(i32) -> i32,
    /// The largest input the function's domain admits; the interpreter must refuse the rest.
    max: i32,
}

const MODES: &str = "\
#unrestricted
Mode is Quiet or Loud or Silent

pick(n in 0 to 9) Mode:
    when n < 3 then return Mode.Quiet
    when n < 6 then return Mode.Loud
    return Mode.Silent
";

fn numeric_cases() -> Vec<Numeric> {
    let mut cases = numeric_cases_unbounded();
    for case in &mut cases {
        case.max = match case.function {
            "spread" => 8,
            "f" if case.name.starts_with("nested for") => 4,
            "f" => 6,
            "go" => 5,
            _ => 9,
        };
    }
    cases
}

fn numeric_cases_unbounded() -> Vec<Numeric> {
    vec![
        Numeric {
            name: "clamp: guards that return",
            source: "clamp(n in 0 to 9) in 0 to 7:\n    when n < 2 then return 0\n    when n > 8 then return 7\n    return n - 2\n",
            function: "clamp",
            oracle: |n| if n < 2 { 0 } else if n > 8 { 7 } else { n - 2 },
            max: 9,
        },
        Numeric {
            name: "or guard",
            source: "edge(n in 0 to 9) in 0 to 5:\n    when n < 2 or n > 7 then return 0\n    return n - 2\n",
            function: "edge",
            oracle: |n| if n < 2 || n > 7 { 0 } else { n - 2 },
            max: 9,
        },
        Numeric {
            name: "and guard",
            source: "middle(n in 0 to 9) in 0 to 9:\n    when n > 2 and n < 6 then return n - 3\n    return 9\n",
            function: "middle",
            oracle: |n| if n > 2 && n < 6 { n - 3 } else { 9 },
            max: 9,
        },
        Numeric {
            name: "joined assignments run in order",
            source: "chain(n in 0 to 9) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 3 then m :: 2\n    when n > 6 then m :: 6\n    return m\n",
            function: "chain",
            oracle: |n| if n > 6 { 6 } else if n < 3 { 2 } else { 0 },
            max: 9,
        },
        Numeric {
            name: "both branches of a join",
            source: "spread(n in 0 to 8) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then\n        m :: n + 1\n    else\n        m :: n\n    return m\n",
            function: "spread",
            oracle: |n| if n < 5 { n + 1 } else { n },
            max: 9,
        },
        Numeric {
            name: "counting while",
            source: "count(n in 0 to 9) in 0 to 10:\n    i in 0 to 10: 0\n    while i < n do\n        i :: i + 1\n    return i\n",
            function: "count",
            oracle: |n| n,
            max: 9,
        },
        Numeric {
            name: "while carrying two locals with a join inside",
            source: "drain(n in 0 to 9) in 0 to 10:\n    i in 0 to 10: 0\n    j in 0 to 10: 10\n    while i < n do\n        i :: i + 1\n        when j > 0 then\n            j :: j - 1\n    return j\n",
            function: "drain",
            oracle: |n| 10 - n,
            max: 9,
        },
        Numeric {
            name: "while inside while",
            source: "nested(n in 0 to 9) in 0 to 10:\n    i in 0 to 10: 0\n    while i < n do\n        k in 0 to 10: 0\n        while k < i do\n            k :: k + 1\n        i :: i + 1\n    return i\n",
            function: "nested",
            oracle: |n| n,
            max: 9,
        },
        Numeric {
            name: "inclusive for runs end + 1 times",
            source: "tally(n in 0 to 9) in 0 to 100:\n    c in 0 to 100: 0\n    for k in 0 to n do\n        when c < 100 then\n            c :: c + 1\n    return c\n",
            function: "tally",
            oracle: |n| n + 1,
            max: 9,
        },
        Numeric {
            name: "return inside a while ends the function",
            source: "first(n in 0 to 9) in 0 to 20:\n    i in 0 to 10: 0\n    while i < 10 do\n        when i * i > n then return i\n        i :: i + 1\n    return 20\n",
            function: "first",
            oracle: |n| (0..10).find(|i| i * i > n).unwrap_or(20),
            max: 9,
        },
        Numeric {
            name: "the first return in a pass wins",
            source: "pick(n in 0 to 9) in 0 to 99:\n    i in 0 to 10: 0\n    while i < 10 do\n        when i > 2 then return 11\n        when i > 0 then return 22\n        i :: i + 1\n    return 99\n",
            function: "pick",
            oracle: |_| 22,
            max: 9,
        },
        Numeric {
            name: "a loop whose return never holds falls through",
            source: "scan(n in 0 to 9) in 0 to 50:\n    i in 0 to 10: 0\n    while i < n do\n        when i = 15 then return 1\n        i :: i + 1\n    return i + 30\n",
            function: "scan",
            oracle: |n| n + 30,
            max: 9,
        },
        Numeric {
            name: "negated while condition",
            source: "upto(n in 0 to 9) in 0 to 10:\n    i in 0 to 10: 0\n    while not i >= n do\n        i :: i + 1\n    return i\n",
            function: "upto",
            oracle: |n| n,
            max: 9,
        },
        Numeric {
            name: "membership value conditional",
            source: "inside(n in 0 to 9) in 0 to 1: return when n is in 3 to 6 then 1 else 0\n",
            function: "inside",
            oracle: |n| i32::from((3..=6).contains(&n)),
            max: 9,
        },
        Numeric {
            name: "membership guard",
            source: "swap(n in 0 to 9) in 0 to 9:\n    when n is in 3 to 6 then return 7\n    return n\n",
            function: "swap",
            oracle: |n| if (3..=6).contains(&n) { 7 } else { n },
            max: 9,
        },
        Numeric {
            name: "negated membership",
            source: "outside(n in 0 to 9) in 0 to 9:\n    when not n is in 3 to 6 then return 0\n    return n\n",
            function: "outside",
            oracle: |n| if (3..=6).contains(&n) { n } else { 0 },
            max: 9,
        },
        Numeric {
            name: "one-armed case dispatches on a tag",
            source: "#unrestricted\nMode is Quiet or Loud or Silent\n\npick(n in 0 to 9) Mode:\n    when n < 3 then return Mode.Quiet\n    when n < 6 then return Mode.Loud\n    return Mode.Silent\n\nlevel(n in 0 to 9) in 0 to 9:\n    when pick(n) is Loud then return 5\n    when pick(n) is Silent then return 9\n    return 1\n",
            function: "level",
            oracle: |n| if n < 3 { 1 } else if n < 6 { 5 } else { 9 },
            max: 9,
        },
        Numeric {
            name: "a one-armed case ends a loop",
            source: "#unrestricted\nMode is Quiet or Loud\n\nclassify(k in 0 to 9) Mode:\n    when k > 6 then return Mode.Loud\n    return Mode.Quiet\n\nfirst_loud(n in 0 to 9) in 0 to 10:\n    i in 0 to 10: n\n    while i < 10 do\n        when classify(i) is Loud then return i\n        i :: i + 1\n    return 10\n",
            function: "first_loud",
            oracle: |n| n.max(7),
            max: 9,
        },
        Numeric {
            name: "chain: grade",
            source: "grade(n in 0 to 9) in 0 to 3:\n    return when\n        n < 3 then 0\n        n < 6 then 1\n        n < 8 then 2\n        else 3\n",
            function: "grade",
            oracle: |n| match n { 0..=2 => 0, 3..=5 => 1, 6..=7 => 2, _ => 3 },
            max: 9,
        },
        Numeric {
            name: "chain over different operands",
            source: "pick(n in 0 to 9) in 0 to 9:\n    return when\n        n = 4 then 9\n        n > 6 then n - 7\n        n >= 2 then n\n        else 5\n",
            function: "pick",
            oracle: |n| if n == 4 { 9 } else if n > 6 { n - 7 } else if n >= 2 { n } else { 5 },
            max: 9,
        },
        Numeric {
            name: "later arm uses the failed conditions",
            source: "shift(n in 0 to 9) in 0 to 6:\n    return when\n        n < 3 then 0\n        else n - 3\n",
            function: "shift",
            oracle: |n| if n < 3 { 0 } else { n - 3 },
            max: 9,
        },
        Numeric {
            name: "chain in a local before a return",
            source: "band(n in 0 to 9) in 0 to 4:\n    base in 0 to 2: when\n        n < 3 then 0\n        n < 6 then 1\n        else 2\n    return base + base\n",
            function: "band",
            oracle: |n| { let base = if n < 3 { 0 } else if n < 6 { 1 } else { 2 }; base + base },
            max: 9,
        },
        Numeric {
            name: "reassignment and a loop of known count",
            source: "total(a in 0 to 9) in 0 to 99:\n  s in 0 to 99: a\n  for i in 1 to 3 do\n    s +: i\n  s *: 2\n  return s\n",
            function: "total",
            oracle: |a| (a + 6) * 2,
            max: 9,
        },
        Numeric {
            name: "for of unknown count carrying a local",
            source: "total(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\n",
            function: "total",
            oracle: |n| n * (n + 1) / 2,
            max: 9,
        },
        Numeric {
            name: "nested for with two carried values",
            source: "f(n in 0 to 4) in 0 to 999:\n  a in 0 to 400: 0\n  b in 0 to 400: 0\n  for i in 1 to n do\n    for j in 1 to i do\n      a +: 1\n    b +: i\n  return a + b\n",
            function: "f",
            oracle: |n| { let (mut a, mut b) = (0, 0); for i in 1..=n { for _ in 1..=i { a += 1; } b += i; } a + b },
            max: 9,
        },
        Numeric {
            name: "variant conditional",
            source: Box::leak(format!("{MODES}\nvolume(n in 0 to 9) in 0 to 9: return when pick(n) is Loud then 5 else 1\n").into_boxed_str()),
            function: "volume",
            oracle: |n| if (3..6).contains(&n) { 5 } else { 1 },
            max: 9,
        },
        Numeric {
            name: "negated variant conditional",
            source: Box::leak(format!("{MODES}\nquiet(n in 0 to 9) in 0 to 9: return when not pick(n) is Loud then 1 else 5\n").into_boxed_str()),
            function: "quiet",
            oracle: |n| if (3..6).contains(&n) { 5 } else { 1 },
            max: 9,
        },
        Numeric {
            name: "mut number handed back",
            source: "bump(x mut in 0 to 9, by in 0 to 3) in 0 to 9:\n  x :: when x < 6 then x + by else x\n  return by\nf(a in 0 to 6) in 0 to 18:\n  n in 0 to 9: a\n  r: bump(n, 2)\n  return n + r\n",
            function: "f",
            oracle: |a| (if a < 6 { a + 2 } else { a }) + 2,
            max: 9,
        },
        Numeric {
            name: "associated function of a type",
            source: "Point has\n  x in 0 to 9\n  add(a in 0 to 5, b in 0 to 5) in 0 to 10: return a + b\ngo(a in 0 to 5) in 0 to 10: return Point.add(a, 2)\n",
            function: "go",
            oracle: |a| if a <= 5 { a + 2 } else { 0 },
            max: 9,
        },
    ]
}

#[test]
fn the_interpreter_agrees_with_the_oracles_of_the_compiler_tests() {
    for case in numeric_cases() {
        for (n, result) in interpreted_inputs(case.source, case.function).into_iter().enumerate() {
            let n = n as i32;
            if n > case.max {
                // Negative control: an argument outside the parameter's domain is refused,
                // not computed with.
                assert!(
                    matches!(result, Err(Error::ArgumentOutOfDomain { .. })),
                    "{}: {}({n}) should be out of domain, got {result:?}",
                    case.name,
                    case.function
                );
                continue;
            }
            let value = result.unwrap_or_else(|e| panic!("{}: {}({n}): {e}", case.name, case.function));
            assert_eq!(value, i128::from((case.oracle)(n)), "{}: {}({n})", case.name, case.function);
        }
    }
}

#[test]
fn the_interpreter_agrees_with_the_compiled_program_for_every_input() {
    let Some(ginc) = compiler_or_skip("numeric differential") else { return };
    for case in numeric_cases() {
        let compiled = compiled_inputs(&ginc, case.source, case.function);
        let interpreted = interpreted_inputs(case.source, case.function);
        for n in 0..=case.max as usize {
            let value = interpreted[n].clone().unwrap_or_else(|e| panic!("{}({n}): {e}", case.name));
            assert_eq!(value, i128::from(compiled[n]), "{}: {}({n}) interpreted vs lli", case.name, case.function);
        }
    }
}

/// An `#entry` program whose exit status is its result.
struct Entry {
    name: &'static str,
    source: String,
    expected: i32,
}

const WORD: &str = "#unrestricted\nWord has value in 0 to 255\n\n";
const MEASURE: &str = "Measure has
    measure(self) in 0 to 100

#unrestricted
Small is in 0 to 9
Small has Measure
    measure(self) in 0 to 100: return 5

#unrestricted
Big is in 0 to 9
Big has Measure
    measure(self) in 0 to 100: return 50

weigh(element has Measure, item element) in 0 to 100:
    return element.measure(item)

";

fn entry_cases() -> Vec<Entry> {
    let case = |name, source: String, expected| Entry { name, source, expected };
    vec![
        case(
            "generic identity",
            format!("{WORD}id(element, x element) element: return x\n\n#entry\nstart() in 0 to 255:\n    w := id(Word, Word(value: 7))\n    return w.value\n"),
            7,
        ),
        case(
            "one instance serves every call",
            format!("{WORD}id(element, x element) element: return x\n\n#entry\nstart() in 0 to 255:\n    a := id(Word, Word(value: 7))\n    b := id(Word, Word(value: 9))\n    return a.value + b.value\n"),
            16,
        ),
        case(
            "methods of a generic record",
            format!("{WORD}#unrestricted\nByte has value in 0 to 15\n\nPair(element) has\n    left element\n    right element\n\n    swap(self) Pair(element):\n        Pair(left, right) := self\n        Pair(left: right, right: left)\n\n#entry\nstart() in 0 to 255:\n    p := Pair(left: Word(value: 3), right: Word(value: 9))\n    Pair(left, right) := p.swap()\n    q := Pair(left: Byte(value: 1), right: Byte(value: 2))\n    Pair(a, b) := q.swap()\n    return left.value * 10 + a.value\n"),
            92,
        ),
        case(
            "bounded call reaches the method of the type given",
            format!("{MEASURE}#entry\nstart() in 0 to 255:\n    s Small := 4\n    b Big := 4\n    x in 0 to 100: weigh(Small, s)\n    y in 0 to 100: weigh(Big, b)\n    return x + y\n"),
            55,
        ),
        case(
            "generic union built and matched in generic functions",
            format!("{MEASURE}Step(element) is Yield(element) or Done\n\nwrap(element, value element) Step(element): Yield(value)\n\nrewrap(element, step Step(element)) Step(element):\n    return when step is\n        Yield(value) then Yield(value)\n        Done then Done\n\nmeasure_step(element has Measure, step Step(element)) in 0 to 100:\n    return when step is\n        Yield(value) then element.measure(value)\n        Done then 3\n\n#entry\nstart() in 0 to 255:\n    b Big := 4\n    s := wrap(Big, b)\n    t := rewrap(Big, s)\n    return measure_step(Big, t)\n"),
            50,
        ),
        case(
            "generic calling another with a different instantiation",
            format!("{WORD}#unrestricted\nPair(element) has\n    left element\n    right element\n\ninner(shape, p shape) shape: return p\n\nouter(element, a element, b element) Pair(element):\n    return inner(Pair(element), Pair(left: a, right: b))\n\n#entry\nstart() in 0 to 255:\n    r := outer(Word, Word(value: 4), Word(value: 6))\n    Pair(l, m) := r\n    return m.value\n"),
            6,
        ),
        case(
            "inferred free call",
            format!("{WORD}id(element, x element) element: return x\n\n#entry\nstart() in 0 to 255:\n    a := id(Word(value: 7))\n    b := id(Word, Word(value: 9))\n    return a.value + b.value\n"),
            16,
        ),
        case(
            "function of a generic type called through the type",
            format!("{WORD}#unrestricted\nPair(element) has\n    left element\n    right element\n\n    make(left element, right element) Pair(element): return Pair(left: left, right: right)\n\n#entry\nstart() in 0 to 255:\n    p := Pair.make(Word(value: 3), Word(value: 9))\n    q := Pair(Word).make(Word(value: 1), Word(value: 2))\n    Pair(a, b) := p\n    Pair(c, d) := q\n    return b.value * 10 + c.value\n"),
            91,
        ),
        case(
            "Self inside a generic method",
            format!("{WORD}#unrestricted\nPair(element) has\n    left element\n    right element\n\n    flipped(self) Self:\n        Pair(left, right) := self\n        return Self(left: right, right: left)\n\n#entry\nstart() in 0 to 255:\n    p := Pair(left: Word(value: 3), right: Word(value: 9))\n    q := p.flipped()\n    return q.left.value\n"),
            9,
        ),
        case(
            "sum with payloads, same width",
            "#unrestricted\nAmount has value in 0 to 100\n#unrestricted\nNumber is Small(Amount) or Big(Amount)\n\nweigh(number Number) in 0 to 200:\n    return when number is\n        Small(amount) then amount.value\n        Big(amount) then amount.value + 100\n\npick(n in 0 to 100) Number:\n    return when n < 50 then Small(Amount(value: n)) else Big(Amount(value: n))\n\nrelay(number Number) Number:\n    return number\n\n#entry\nstart() in 0 to 255: return weigh(relay(pick(70)))\n".to_owned(),
            170,
        ),
        case(
            "sum with payloads, mixed width",
            "#unrestricted\nAmount has value in 0 to 100\n#unrestricted\nShape is Single(value Amount) or Pair(first Amount, second Amount)\n\ntotal(shape Shape) in 0 to 200:\n    return when shape is\n        Single(amount) then amount.value\n        Pair(first, second) then first.value + second.value\n\nmake(n in 0 to 100) Shape:\n    return when n < 50 then Single(Amount(value: n)) else Pair(first: Amount(value: n), second: Amount(value: 3))\n\n#entry\nstart() in 0 to 255: return total(make(70))\n".to_owned(),
            73,
        ),
    ]
}

#[test]
fn entry_programs_agree_with_the_compiler_tests_expected_exit_status() {
    for case in entry_cases() {
        let directory = package(&case.source);
        let value = interpreted_entry(directory.path()).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert_eq!(value, i128::from(case.expected), "{}", case.name);
    }
}

#[test]
fn entry_programs_agree_with_the_native_executable() {
    let Some(ginc) = compiler_or_skip("entry differential") else { return };
    for case in entry_cases() {
        let directory = package(&case.source);
        let value = interpreted_entry(directory.path()).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        let compiled = compiled_entry(&ginc, directory.path());
        assert_eq!(Some((value & 0xff) as i32), compiled, "{}: interpreted vs native", case.name);
    }
}

const LIBRARY: &str = "id(element, x element) element: return x

Pair(element) has
    left element
    right element

    swap(self) Pair(element):
        Pair(left, right) := self
        Pair(left: right, right: left)

Measure has
    measure(self) in 0 to 100

#unrestricted
Item is in 0 to 9
Item has Measure
    measure(self) in 0 to 100: return 33

#unrestricted
Tag has value in 0 to 255

weigh(element has Measure, item element) in 0 to 100:
    return element.measure(item)

local_tag() in 0 to 255:
    t := id(Tag, Tag(value: 5))
    return t.value
";

#[test]
fn generics_run_across_modules() {
    let cases = [
        (
            "use lib.util id\nuse lib.util Pair\n\n#unrestricted\nWord has value in 0 to 255\n\n#entry\nstart() in 0 to 255:\n    w := id(Word, Word(value: 7))\n    p := Pair(left: Word(value: 1), right: Word(value: 2))\n    Pair(l, r) := p.swap()\n    return w.value + l.value * 10\n",
            27,
        ),
        (
            "use lib.util id\nuse lib.util Item\nuse lib.util Tag\nuse lib.util weigh\nuse lib.util local_tag\n\n#entry\nstart() in 0 to 255:\n    t := id(Tag, Tag(value: 7))\n    i Item := 1\n    k in 0 to 100: weigh(Item, i)\n    return t.value + k + local_tag()\n",
            45,
        ),
        (
            "use lib.util id\nuse lib.util Tag\nuse lib.util local_tag\n\n#entry\nstart() in 0 to 255:\n    t := id(Tag, Tag(value: 7))\n    return t.value + local_tag()\n",
            12,
        ),
    ];
    let ginc = compiler_or_skip("cross-module differential");
    for (app, expected) in cases {
        let directory = two_packages(LIBRARY, app);
        let root = directory.path().join("app");
        assert_eq!(interpreted_entry(&root).unwrap(), expected);
        if let Some(ginc) = &ginc {
            assert_eq!(compiled_entry(ginc, &root), Some(expected as i32));
        }
    }
}
