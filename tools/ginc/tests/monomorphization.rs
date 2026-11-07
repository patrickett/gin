//! Generic functions and methods of generic types are lowered by monomorphization: one machine
//! function per concrete instantiation, found from the module that uses it. The exit status of
//! `start()` is what each program observes.
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "mod.flask",
        "#settings\nmodule: \"mono\"\nversion: 0.1.0\n",
    );
    write(directory.path(), "main.gin", main);
    directory
}

fn ginc(arguments: &[&str], package: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(arguments)
        .arg(package)
        .output()
        .unwrap()
}

fn mlir(main: &str) -> String {
    let output = ginc(&["--emit", "mlir"], package(main).path());
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn fails(main: &str) -> String {
    let output = ginc(&["--emit", "mlir"], package(main).path());
    assert!(!output.status.success(), "{output:?}");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn build_and_run(directory: &Path, root: &Path) -> Option<i32> {
    let program = directory.join("program");
    let built = ginc(
        &["--emit", "exe", "--output", program.to_str().unwrap()],
        root,
    );
    assert!(built.status.success(), "{built:?}");
    Command::new(&program).status().unwrap().code()
}

fn run(main: &str) -> Option<i32> {
    let directory = package(main);
    build_and_run(directory.path(), directory.path())
}

/// How many functions the MLIR defines whose symbol contains `needle`.
fn definitions(text: &str, needle: &str) -> usize {
    text.lines()
        .filter(|line| line.contains("func.func @") && line.contains(needle))
        .count()
}

const WORD: &str = "#unrestricted\nWord has value in 0 to 255\n\n";

#[test]
fn a_generic_identity_returns_what_it_is_given() {
    let source = format!(
        "{WORD}id(element, x element) element: return x\n\n#entry\nstart() in 0 to 255:\n    w := id(Word, Word(value: 7))\n    return w.value\n"
    );
    assert_eq!(run(&source), Some(7));
    let text = mlir(&source);
    assert_eq!(definitions(&text, "mono.id<mono.Word>"), 1, "{text}");
    // The template itself has no machine form.
    assert_eq!(definitions(&text, "\"mono.id\""), 0, "{text}");
}

#[test]
fn one_instance_serves_every_call_with_the_same_arguments() {
    let source = format!(
        "{WORD}id(element, x element) element: return x\n\n#entry\nstart() in 0 to 255:\n    a := id(Word, Word(value: 7))\n    b := id(Word, Word(value: 9))\n    return a.value + b.value\n"
    );
    assert_eq!(run(&source), Some(16));
    assert_eq!(definitions(&mlir(&source), "mono.id<mono.Word>"), 1);
}

#[test]
fn instances_are_linkonce_so_that_every_module_may_emit_them() {
    let source = format!(
        "{WORD}id(element, x element) element: return x\n\n#entry\nstart() in 0 to 255:\n    w := id(Word, Word(value: 7))\n    return w.value\n"
    );
    assert!(mlir(&source).contains("llvm.linkage = #llvm.linkage<linkonce_odr>"));
}

#[test]
fn methods_of_a_generic_record_swap_its_fields_for_each_element_type() {
    let source = format!(
        "{WORD}#unrestricted\nByte has value in 0 to 15\n\nPair(element) has\n    left element\n    right element\n\n    swap(self) Pair(element):\n        Pair(left, right) := self\n        Pair(left: right, right: left)\n\n#entry\nstart() in 0 to 255:\n    p := Pair(left: Word(value: 3), right: Word(value: 9))\n    Pair(left, right) := p.swap()\n    q := Pair(left: Byte(value: 1), right: Byte(value: 2))\n    Pair(a, b) := q.swap()\n    return left.value * 10 + a.value\n"
    );
    assert_eq!(run(&source), Some(92));
    let text = mlir(&source);
    assert_eq!(definitions(&text, "Pair.swap<mono.Word>"), 1, "{text}");
    assert_eq!(definitions(&text, "Pair.swap<mono.Byte>"), 1, "{text}");
}

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

#[test]
fn a_bounded_call_reaches_the_method_of_the_type_given() {
    let source = format!(
        "{MEASURE}#entry\nstart() in 0 to 255:\n    s Small := 4\n    b Big := 4\n    x in 0 to 100: weigh(Small, s)\n    y in 0 to 100: weigh(Big, b)\n    return x + y\n"
    );
    assert_eq!(run(&source), Some(55));
    let text = mlir(&source);
    assert_eq!(definitions(&text, "mono.weigh<mono.Small>"), 1, "{text}");
    assert_eq!(definitions(&text, "mono.weigh<mono.Big>"), 1, "{text}");
    assert!(text.contains("call @\"mono.Small.measure\""), "{text}");
    assert!(text.contains("call @\"mono.Big.measure\""), "{text}");
}

#[test]
fn a_type_that_does_not_provide_the_bound_is_refused_where_it_is_given() {
    let source = format!(
        "{MEASURE}#unrestricted\nPlain is in 0 to 9\n\n#entry\nstart() in 0 to 255:\n    p Plain := 1\n    x in 0 to 100: weigh(Plain, p)\n    return x\n"
    );
    let report = fails(&source);
    assert!(report.contains("source-check-type"), "{report}");
}

#[test]
fn a_generic_entry_is_refused() {
    let report = fails("#entry\nstart(element) in 0 to 255: return 1\n");
    assert!(report.contains("source-check-type"), "{report}");
}

#[test]
fn a_generic_union_is_built_and_matched_in_generic_functions() {
    let source = format!(
        "{MEASURE}Step(element) is Yield(element) or Done\n\nwrap(element, value element) Step(element): Yield(value)\n\nrewrap(element, step Step(element)) Step(element):\n    return when step is\n        Yield(value) then Yield(value)\n        Done then Done\n\nmeasure_step(element has Measure, step Step(element)) in 0 to 100:\n    return when step is\n        Yield(value) then element.measure(value)\n        Done then 3\n\n#entry\nstart() in 0 to 255:\n    b Big := 4\n    s := wrap(Big, b)\n    t := rewrap(Big, s)\n    return measure_step(Big, t)\n"
    );
    assert_eq!(run(&source), Some(50));
    let text = mlir(&source);
    for name in ["wrap", "rewrap", "measure_step"] {
        assert_eq!(
            definitions(&text, &format!("mono.{name}<mono.Big>")),
            1,
            "{text}"
        );
    }
}

#[test]
fn a_generic_that_calls_another_with_a_different_instantiation() {
    let source = format!(
        "{WORD}#unrestricted\nPair(element) has\n    left element\n    right element\n\ninner(shape, p shape) shape: return p\n\nouter(element, a element, b element) Pair(element):\n    return inner(Pair(element), Pair(left: a, right: b))\n\n#entry\nstart() in 0 to 255:\n    r := outer(Word, Word(value: 4), Word(value: 6))\n    Pair(l, m) := r\n    return m.value\n"
    );
    assert_eq!(run(&source), Some(6));
    let text = mlir(&source);
    assert_eq!(definitions(&text, "mono.outer<mono.Word>"), 1, "{text}");
    assert_eq!(
        definitions(&text, "mono.inner<mono.Pair<mono.Word>>"),
        1,
        "{text}"
    );
}

/// Changed: this expected a refusal ("needs 8 registers") for `Pair(Wide)`, the 4-register-limit
/// behaviour. The limit is now 16 and a record over it is passed by address, so neither the
/// 8-register instance nor an 18-register one (nine fields twice) is refused. The test now holds
/// the wide instance to its run: it is lowered through memory (an `llvm.alloca`, no 18-register
/// signature) and the program exits with the field it read. The refusal that remains for
/// non-instance signatures is in `lower_next.rs` (`passes a value of N registers`).
#[test]
fn an_instance_wider_than_the_registers_is_passed_by_address() {
    let source = "#unrestricted\nWide has\n    a in 0 to 9\n    b in 0 to 9\n    c in 0 to 9\n    d in 0 to 9\n    e in 0 to 9\n    f in 0 to 9\n    g in 0 to 9\n    h in 0 to 9\n    i in 0 to 9\n\nPair(element) has\n    left element\n    right element\n\nid(element, x element) element: return x\n\nmake() in 0 to 9:\n    p := id(Pair(Wide), Pair(left: Wide(a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9), right: Wide(a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9)))\n    Pair(l, r) := p\n    return l.a\n\n#entry\nstart() in 0 to 255: return make()\n";
    let text = mlir(source);
    assert!(text.contains("id<mono.Pair<mono.Wide>>"), "{text}");
    assert!(text.contains("llvm.alloca"), "{text}");
    assert!(!text.contains("%arg0_17"), "{text}");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(source), Some(1));
}

#[test]
fn an_instance_of_eight_registers_is_within_the_limit() {
    let source = "#unrestricted\nWide has\n    a in 0 to 9\n    b in 0 to 9\n    c in 0 to 9\n    d in 0 to 9\n\nPair(element) has\n    left element\n    right element\n\nid(element, x element) element: return x\n\nmake() in 0 to 9:\n    p := id(Pair(Wide), Pair(left: Wide(a: 1, b: 2, c: 3, d: 4), right: Wide(a: 1, b: 2, c: 3, d: 4)))\n    Pair(l, r) := p\n    return l.a\n\n#entry\nstart() in 0 to 255: return make()\n";
    assert!(mlir(source).contains("id<mono.Pair<mono.Wide>>"));
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert_eq!(run(source), Some(1));
}

#[test]
fn lowering_twice_gives_the_same_text() {
    let source = format!(
        "{MEASURE}#entry\nstart() in 0 to 255:\n    s Small := 4\n    b Big := 4\n    x in 0 to 100: weigh(Small, s)\n    y in 0 to 100: weigh(Big, b)\n    return x + y\n"
    );
    assert_eq!(mlir(&source), mlir(&source));
}

fn two_packages(library: &str, app: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "lib/mod.flask",
        "#settings\nmodule: \"lib\"\nversion: 0.1.0\n",
    );
    write(root, "lib/util/util.gin", library);
    write(
        root,
        "app/mod.flask",
        "#settings\nmodule: \"app\"\nversion: 0.1.0\n\n#dependencies\nlib: \"../lib\"\n",
    );
    write(root, "app/main.gin", app);
    directory
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
fn a_generic_of_another_module_is_instantiated_with_a_type_of_this_one() {
    let app = "use lib.util id\nuse lib.util Pair\n\n#unrestricted\nWord has value in 0 to 255\n\n#entry\nstart() in 0 to 255:\n    w := id(Word, Word(value: 7))\n    p := Pair(left: Word(value: 1), right: Word(value: 2))\n    Pair(l, r) := p.swap()\n    return w.value + l.value * 10\n";
    let directory = two_packages(LIBRARY, app);
    let root = directory.path().join("app");
    assert_eq!(build_and_run(directory.path(), &root), Some(27));
    let output = ginc(&["--emit", "mlir"], &root);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("@\"lib.util.id<app.Word>\""), "{text}");
    assert!(text.contains("@\"lib.util.Pair.swap<app.Word>\""), "{text}");
}

#[test]
fn a_bound_of_another_module_resolves_to_the_provider_in_its_home() {
    let app = "use lib.util id\nuse lib.util Item\nuse lib.util Tag\nuse lib.util weigh\nuse lib.util local_tag\n\n#entry\nstart() in 0 to 255:\n    t := id(Tag, Tag(value: 7))\n    i Item := 1\n    k in 0 to 100: weigh(Item, i)\n    return t.value + k + local_tag()\n";
    let directory = two_packages(LIBRARY, app);
    let root = directory.path().join("app");
    assert_eq!(build_and_run(directory.path(), &root), Some(45));
}

/// Both modules instantiate `id` with the library's `Tag`: each emits it, and the executable
/// holds one.
#[test]
fn an_instance_both_modules_need_links_to_one_copy() {
    let app = "use lib.util id\nuse lib.util Tag\nuse lib.util local_tag\n\n#entry\nstart() in 0 to 255:\n    t := id(Tag, Tag(value: 7))\n    return t.value + local_tag()\n";
    let directory = two_packages(LIBRARY, app);
    let root = directory.path().join("app");
    assert_eq!(build_and_run(directory.path(), &root), Some(12));
    let symbols = Command::new("nm")
        .arg(directory.path().join("program"))
        .output()
        .unwrap();
    let symbols = String::from_utf8(symbols.stdout).unwrap();
    assert_eq!(
        symbols.matches("lib.util.id<lib.util.Tag>").count(),
        1,
        "{symbols}"
    );
}

#[test]
fn an_inferred_free_call_requests_the_same_instance_as_a_written_one() {
    let source = format!(
        "{WORD}id(element, x element) element: return x\n\n#entry\nstart() in 0 to 255:\n    a := id(Word(value: 7))\n    b := id(Word, Word(value: 9))\n    return a.value + b.value\n"
    );
    assert_eq!(run(&source), Some(16));
    let text = mlir(&source);
    assert_eq!(definitions(&text, "mono.id<mono.Word>"), 1, "{text}");
}

#[test]
fn a_function_of_a_generic_type_called_through_the_type_infers_its_arguments() {
    let source = format!(
        "{WORD}#unrestricted\nPair(element) has\n    left element\n    right element\n\n    make(left element, right element) Pair(element): return Pair(left: left, right: right)\n\n#entry\nstart() in 0 to 255:\n    p := Pair.make(Word(value: 3), Word(value: 9))\n    q := Pair(Word).make(Word(value: 1), Word(value: 2))\n    Pair(a, b) := p\n    Pair(c, d) := q\n    return b.value * 10 + c.value\n"
    );
    assert_eq!(run(&source), Some(91));
    let text = mlir(&source);
    assert_eq!(definitions(&text, "Pair.make<mono.Word>"), 1, "{text}");
}

#[test]
fn self_builds_the_receiver_type_inside_a_generic_method() {
    let source = format!(
        "{WORD}#unrestricted\nPair(element) has\n    left element\n    right element\n\n    flipped(self) Self:\n        Pair(left, right) := self\n        return Self(left: right, right: left)\n\n#entry\nstart() in 0 to 255:\n    p := Pair(left: Word(value: 3), right: Word(value: 9))\n    q := p.flipped()\n    return q.left.value\n"
    );
    assert_eq!(run(&source), Some(9));
}
