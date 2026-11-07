//! The session image: `inspect` for shape and size, `image` for dependencies, incremental state.
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn session(requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .arg("--jsonl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn declare(source: &str) -> Value {
    json!({"id":"d","op":"$declare","source":source})
}

fn inspect(name: &str) -> Value {
    json!({"id":"i","op":"$inspect","name":name})
}

const POINT: &str = "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 300\n";
const SHAPES: &str = "#unrestricted\nAmount is in 0 to 100\n#unrestricted\nShape is Single(value Amount) or Pair(first Amount, second Amount)\n";

#[test]
fn a_record_has_fields_in_order_a_checker_size_and_offsets() {
    let r = session(&[declare(POINT), inspect("Point")]);
    let d = &r[1]["data"];
    assert_eq!(d["kind"], "type");
    assert_eq!(d["type_kind"], "record");
    assert_eq!(d["linearity"], "unrestricted");
    assert_eq!(d["fields"][0]["name"], "x");
    assert_eq!(d["fields"][1]["type"], "0 to 300");
    // The checker's own `#size` and `#alignment`: 1 byte, then 2 bytes aligned to 2.
    assert_eq!(d["layout"]["size"], json!({"available":true,"value":4,"unit":"bytes"}));
    assert_eq!(d["layout"]["alignment"]["value"], 2);
    let offsets = &d["layout"]["field_offsets"];
    assert_eq!(offsets["fields"][0]["offset"], 0);
    assert_eq!(offsets["fields"][1]["offset"], 2);
    assert_eq!(offsets["consistent_with_size"], true);
    // The lowering's view: two numbers in registers.
    assert_eq!(d["layout"]["registers"]["registers"], 2);
    assert_eq!(d["layout"]["registers"]["limit"], 16);
}

#[test]
fn a_record_without_the_marker_is_linear() {
    let r = session(&[declare("Token has value in 0 to 9\n"), inspect("Token")]);
    assert_eq!(r[1]["data"]["linearity"], "linear");
}

#[test]
fn a_sum_lists_variants_with_payloads_and_its_register_shape() {
    let r = session(&[declare(SHAPES), inspect("Shape")]);
    let d = &r[1]["data"];
    assert_eq!(d["type_kind"], "sum");
    assert_eq!(d["variants"][0], json!({"name":"Single","qualified":"Shape.Single","payload":[{"name":"value","type":"Amount"}]}));
    assert_eq!(d["variants"][1]["payload"].as_array().unwrap().len(), 2);
    let sum = &d["layout"]["registers"]["sum"];
    assert_eq!(sum["tag_registers"], 1);
    assert_eq!(sum["widest_payload"], 2);
    assert_eq!(sum["total_registers"], 3);
    assert_eq!(sum["members"][0]["padding"], 1);
    assert_eq!(sum["members"][1]["padding"], 0);
    // Negative control: the checker defines no byte size for a sum, and says so rather than
    // inventing one from the registers.
    assert_eq!(d["layout"]["size"]["available"], false);
    assert_eq!(d["layout"]["size"]["reason"]["kind"], "not_defined_yet");
}

fn ginc() -> Option<PathBuf> {
    let path = std::env::var_os("GINC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ginc"));
    path.exists().then_some(path)
}

#[test]
fn the_register_shape_of_a_payload_sum_is_what_the_lowering_emits() {
    let source = format!("{SHAPES}\nweigh(shape Shape) in 0 to 200:\n    return when shape is\n        Single(_) then 1\n        Pair(_, _) then 2\n");
    let reply = session(&[declare(&source), inspect("Shape")]);
    let total = reply[1]["data"]["layout"]["registers"]["sum"]["total_registers"].as_u64().unwrap() as usize;
    let Some(ginc) = ginc() else {
        assert!(std::env::var_os("INTERPRETER_NEXT_REQUIRE_COMPILED").is_none(), "no ginc binary");
        eprintln!("SKIPPED: no ginc binary; the layout was checked against the register shape only");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, &source).unwrap();
    let output = Command::new(ginc).args(["--emit", "mlir"]).arg(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let mlir = String::from_utf8(output.stdout).unwrap();
    let line = mlir.lines().find(|l| l.contains("func.func @weigh")).unwrap();
    let params = line.matches("%arg0_").count();
    assert_eq!(params, total, "{line}");
}

#[test]
fn a_generic_type_is_symbolic_until_it_is_applied() {
    let source = "#unrestricted\nWord has value in 0 to 255\n\n#unrestricted\nPair(element) has\n    left element\n    right element\n\nmake(w Word) Pair(Word): return Pair(left: w, right: w)\n";
    let r = session(&[declare(source), inspect("Pair"), inspect("Pair(Word)")]);
    let generic = &r[1]["data"];
    assert_eq!(generic["type_kind"], "generic");
    assert_eq!(generic["parameters"][0]["name"], "element");
    assert_eq!(generic["layout"]["size"]["available"], false);
    assert_eq!(generic["layout"]["size"]["reason"]["kind"], "symbolic");
    assert!(generic["layout"]["size"]["reason"]["detail"].as_str().unwrap().contains("element"));
    // The application the session makes has a size of its own.
    assert_eq!(generic["layout"]["applications"][0]["type"], "Pair(Word)");
    assert_eq!(generic["layout"]["applications"][0]["size"]["value"], 2);
    let applied = &r[2]["data"];
    assert_eq!(applied["layout"]["size"]["value"], 2);
    assert_eq!(applied["layout"]["registers"]["registers"], 2);
}

#[test]
fn domains_report_bounds_wrapping_and_claims() {
    let r = session(&[declare("#unrestricted\nByte is in 0 to 255\n#unrestricted\n#wrap\nBit is in 0 to 1\n"), inspect("Byte"), inspect("Bit")]);
    assert_eq!(r[1]["data"]["type_kind"], "domain");
    assert_eq!(r[1]["data"]["domain"]["bounds"], json!({"lower":"0","upper":"255"}));
    assert_eq!(r[1]["data"]["domain"]["wraps"], false);
    assert_eq!(r[2]["data"]["domain"]["wraps"], true);
    assert_eq!(r[1]["data"]["layout"]["size"]["value"], 1);
}

#[test]
fn constants_aliases_and_unknown_names() {
    let r = session(&[
        declare("bits := 4\n#unrestricted\nByte is in 0 to 255\nBytePointer is @Byte\n"),
        inspect("bits"),
        inspect("BytePointer"),
        inspect("nothing"),
    ]);
    assert_eq!(r[1]["data"]["kind"], "constant");
    assert_eq!(r[1]["data"]["value"]["value"], "4");
    assert_eq!(r[1]["data"]["folded"], true);
    assert_eq!(r[2]["data"]["kind"], "alias");
    assert_eq!(r[2]["data"]["target"], "ptr(Byte)");
    assert_eq!(r[3]["data"]["found"], false);
    assert_eq!(r[3]["data"]["reason"]["kind"], "unknown_name");
}

#[test]
fn a_field_the_layout_queries_do_not_name_is_a_typed_reason() {
    let source = "#unrestricted\nByte is in 0 to 255\nCell has\n    p @Byte\n    n in 0 to 9\n";
    let r = session(&[declare(source), inspect("Cell")]);
    let d = &r[1]["data"];
    assert_eq!(d["found"], true, "{d}");
    assert_eq!(d["layout"]["field_offsets"]["available"], false);
    assert_eq!(d["layout"]["field_offsets"]["reason"]["kind"], "not_defined_yet");
}

#[test]
fn a_function_has_a_signature_claims_a_body_and_its_instances() {
    let source = "#unrestricted\nWord has value in 0 to 255\n\nid(element, x element) element: return x\n\nstart() in 0 to 255:\n    w := id(Word, Word(value: 7))\n    return w.value\n\nclaimed(value in 0 to 16) out in 0 to 16 where out >= value:\n    return 16\n";
    let r = session(&[declare(source), inspect("id"), inspect("claimed"), inspect("start")]);
    let id = &r[1]["data"]["overloads"][0];
    assert_eq!(id["signature"], "id(element) -> element");
    assert_eq!(id["type_parameters"][0]["name"], "element");
    assert_eq!(id["body"]["recorded"], true);
    // The instance the session's code requests, and how it travels.
    let instance = &id["instances"][0];
    assert!(instance["name"].as_str().unwrap().contains("id<main.Word>"), "{instance}");
    assert_eq!(instance["parameters"][0]["registers"], 1);
    assert_eq!(instance["result"]["registers"], 1);
    let claimed = &r[2]["data"]["overloads"][0];
    assert!(claimed["claims"].as_str().unwrap().contains("out >= value"), "{claimed}");
    assert_eq!(claimed["parameters"][0]["name"], "value");
    // A function that is its own value.
    assert_eq!(r[3]["data"]["value"]["known"], true);
    assert_eq!(r[3]["data"]["value"]["value"]["value"], 7);
}

#[test]
fn a_function_without_a_recorded_body_says_why() {
    let source = "answer(n in 0 to 3) in 0 to 9:\n  i in 0 to 9: 0\n  while i < n and i < 2 do\n    i :: i + 1\n  return i\n";
    let r = session(&[declare(source), inspect("answer")]);
    let body = &r[1]["data"]["overloads"][0]["body"];
    assert_eq!(body["recorded"], false);
    assert_eq!(body["reason"]["kind"], "unrecorded_body");
    // The construct the recorder could not place, and where.
    assert!(body["reason"]["declined_construct"].as_str().is_some(), "{body}");
    assert!(body["reason"]["span"]["byte_end"].as_u64().unwrap() > body["reason"]["span"]["byte_start"].as_u64().unwrap());
}

#[test]
fn division_is_recorded_so_it_is_no_longer_a_declined_operator() {
    // It was the example of a declined operator; the recorder now records it (E8).
    let r = session(&[declare("div(a in 0 to 9, b in 1 to 3) in 0 to 9: return a / b\n"), inspect("div")]);
    assert_eq!(r[1]["data"]["overloads"][0]["body"]["recorded"], true);
}

#[test]
fn a_declined_statement_is_named_with_its_place() {
    let source = "answer(n in 0 to 3) in 0 to 9:\n  i in 0 to 9: 0\n  while i < n and i < 2 do\n    i :: i + 1\n  return i\n";
    let r = session(&[declare(source), inspect("answer")]);
    let reason = &r[1]["data"]["overloads"][0]["body"]["reason"];
    assert_eq!(reason["kind"], "unrecorded_body");
    let kind = reason["declined_kind"].as_str().unwrap();
    assert!(["statement", "expression"].contains(&kind), "{reason}");
    let (start, end) = (reason["span"]["byte_start"].as_u64().unwrap() as usize, reason["span"]["byte_end"].as_u64().unwrap() as usize);
    assert!(start < end && end <= source.len() + 8, "{reason}");
    // A function that records has no such reason.
    let ok = session(&[declare("ok(a in 0 to 9) in 0 to 9: return a\n"), inspect("ok")]);
    assert_eq!(ok[1]["data"]["overloads"][0]["body"]["recorded"], true);
}

#[test]
fn a_method_says_whether_a_trait_added_it() {
    let source = "Measure has\n    measure(self) in 0 to 100: return 5\n\n#unrestricted\nSmall is in 0 to 9\nSmall has Measure\n\n#unrestricted\nBig is in 0 to 9\nBig has Measure\n    measure(self) in 0 to 100: return 50\n";
    let r = session(&[declare(source), inspect("Small"), inspect("Big")]);
    assert_eq!(r[1]["data"]["methods"][0]["origin"], json!({"kind":"trait","trait":"Measure"}));
    assert_eq!(r[2]["data"]["methods"][0]["origin"], json!({"kind":"declared"}));
}

#[test]
fn names_written_in_a_recorded_body_are_not_dependencies() {
    // `base` is a parameter name here, not the function: the text alone would say otherwise.
    let r = session(&[
        declare("base() in 0 to 9: return 4\n"),
        declare("other(base in 0 to 9) in 0 to 9: return base\n"),
        json!({"id":"m","op":"$image"}),
    ]);
    let defs = r[2]["data"]["definitions"].as_array().unwrap();
    let other = defs.iter().find(|d| d["key"] == "fn other/1").unwrap();
    assert_eq!(other["uses"], json!([]));
}

#[test]
fn a_value_that_cannot_be_known_gives_the_reason() {
    let r = session(&[declare("pending() in 0 to 9: return #todo\n"), inspect("pending")]);
    let value = &r[1]["data"]["value"];
    assert_eq!(value["known"], false);
    assert_eq!(value["reason"]["kind"], "hole_reached");
}

#[test]
fn inspect_refuses_a_session_that_does_not_check() {
    // Only a rejected declaration could leave flaws, and it is not kept: a fresh session is fine.
    let r = session(&[declare("f(n in 0 to 9) in 0 to 9: return n - 3\n"), inspect("f")]);
    assert_eq!(r[0]["data"]["accepted"], false);
    assert_eq!(r[1]["data"]["found"], false);
}

// ---- the image -------------------------------------------------------------------------

const GRAPH: &str = "base() in 0 to 9: return 4\n";

#[test]
fn the_image_shows_what_uses_what_and_what_a_change_would_affect() {
    let r = session(&[
        declare("base(n in 0 to 9) in 0 to 9: return n\n"),
        declare("middle(n in 0 to 9) in 0 to 9: return base(n)\n"),
        declare("top(n in 0 to 9) in 0 to 9: return middle(n)\n"),
        declare("alone() in 0 to 9: return 1\n"),
        json!({"id":"m","op":"$image"}),
    ]);
    let image = &r[4]["data"];
    let def = |key: &str| image["definitions"].as_array().unwrap().iter().find(|d| d["key"] == key).unwrap().clone();
    assert_eq!(def("fn middle/1")["uses"][0], json!({"key":"fn base/1","via":"checker"}));
    assert_eq!(def("fn base/1")["used_by"], json!(["fn middle/1"]));
    assert_eq!(def("fn base/1")["affects"], json!(["fn middle/1", "fn top/1"]));
    // Negative control: an unrelated definition is affected by nothing and affects nothing.
    assert_eq!(def("fn alone/0")["affects"], json!([]));
    assert_eq!(def("fn alone/0")["uses"], json!([]));
    assert_eq!(image["folded"]["target"]["register_bits"], 64);
    // A call the checker folded into a value leaves no call in the body, but the checker
    // remembers the callee: the edge is exact and says it was folded.
    let folded = &session(&[declare(GRAPH), declare("middle() in 0 to 9: return base()\n"), json!({"id":"m","op":"$image"})])[2];
    let middle = folded["data"]["definitions"].as_array().unwrap().iter().find(|d| d["key"] == "fn middle/0").unwrap().clone();
    assert_eq!(middle["uses"][0], json!({"key":"fn base/0","via":"folded"}));
    let one = &session(&[declare(GRAPH), declare("middle() in 0 to 9: return base()\n"), inspect("base")])[2];
    assert_eq!(one["data"]["image"]["affects"], json!(["fn middle/0"]));
}

#[test]
fn derived_state_is_kept_for_definitions_a_declare_did_not_touch() {
    let r = session(&[
        declare("base() in 0 to 9: return 4\n"),
        declare("top() in 0 to 9: return base()\n"),
        declare("alone() in 0 to 9: return 1\n"),
        inspect("top"),
        inspect("top"),
        declare("alone() in 0 to 9: return 2\n"),
        inspect("top"),
        declare("base() in 0 to 9: return 5\n"),
        inspect("top"),
        inspect("top"),
    ]);
    assert_eq!(r[3]["data"]["cached"], false);
    assert_eq!(r[4]["data"]["cached"], true);
    // Redeclaring an unrelated definition keeps `top`'s state and says so.
    assert_eq!(r[5]["data"]["image"]["changed"], json!(["fn alone/0"]));
    assert!(r[5]["data"]["image"]["kept"].as_array().unwrap().contains(&json!("fn top/0")));
    assert_eq!(r[6]["data"]["cached"], true);
    // Redeclaring what it uses invalidates it and its users, and recomputes the value.
    assert_eq!(r[7]["data"]["image"]["changed"], json!(["fn base/0", "fn top/0"]));
    assert_eq!(r[8]["data"]["cached"], false);
    assert_eq!(r[8]["data"]["value"]["value"]["value"], 5);
    assert_eq!(r[9]["data"]["cached"], true);
}

#[test]
fn the_revision_advances_with_each_accepted_declaration_only() {
    let r = session(&[declare("a() in 0 to 9: return 1\n"), declare("f(n in 0 to 9) in 0 to 9: return n - 3\n"), declare("b() in 0 to 9: return 2\n")]);
    assert_eq!(r[0]["data"]["image"]["revision"], 1);
    assert_eq!(r[1]["data"]["accepted"], false);
    assert_eq!(r[2]["data"]["image"]["revision"], 2);
}

// ---- flaws are ginc's ------------------------------------------------------------------

#[test]
fn a_flaw_is_the_one_ginc_reports() {
    let Some(ginc) = ginc() else {
        assert!(std::env::var_os("INTERPRETER_NEXT_REQUIRE_COMPILED").is_none(), "no ginc binary");
        return;
    };
    let source = "f(n in 0 to 9) in 0 to 9: return n - 3\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, source).unwrap();
    let output = Command::new(ginc).args(["--emit", "check", "--message-format=json"]).arg(&path).output().unwrap();
    let from_ginc: Value = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|m| m["reason"] == "diagnostic")
        .unwrap();
    let ours = &session(&[json!({"id":1,"op":"$check","source":source})])[0]["diagnostics"][0].clone();
    assert_eq!(ours["code"], from_ginc["code"]);
    assert_eq!(ours["message"], from_ginc["message"]);
    // The primary range ginc prints is our `culprit`; its related range is our `span`.
    assert_eq!(ours["culprit"]["byte_start"], from_ginc["range"]["byte_start"]);
    assert_eq!(ours["culprit"]["byte_end"], from_ginc["range"]["byte_end"]);
    assert_eq!(ours["span"]["byte_start"], from_ginc["related"][0]["range"]["byte_start"]);
    let facts: Vec<String> = from_ginc["args"].as_array().unwrap().iter().filter(|a| a[0] == "fact").map(|a| a[1].as_str().unwrap().to_owned()).collect();
    let proved: Vec<String> = ours["facts"]["proved"].as_array().unwrap().iter().map(|f| f["text"].as_str().unwrap().to_owned()).collect();
    assert_eq!(facts, proved);
}
