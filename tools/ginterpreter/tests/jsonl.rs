//! The JSONL protocol end to end, through the real binary.
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn run_raw(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .arg("--jsonl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn lines(output: &Output) -> Vec<Value> {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

fn session(requests: &[Value]) -> Vec<Value> {
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    lines(&run_raw(&input))
}

fn one(request: Value) -> Value {
    session(&[request]).remove(0)
}

fn codes(response: &Value) -> Vec<String> {
    response["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap().to_owned()).collect()
}

const TRAP: &str = "#unrestricted\nNothing has\n\n#instruction('brk #0')\ntrap(input Type, output Type, inputs input) output\n\nrisky() in 0 to 9:\n    trap(Nothing, Nothing, Nothing())\n    return 2\n\n";

// ---- protocol ------------------------------------------------------------------------

#[test]
fn describe_names_the_protocol_and_every_response_echoes_its_id() {
    let replies = session(&[json!({"id":"a","op":"$describe"}), json!({"id":7,"op":"$scope"})]);
    assert_eq!(replies[0]["id"], "a");
    assert_eq!(replies[0]["v"], 1);
    assert_eq!(replies[0]["status"], "ok");
    assert_eq!(replies[0]["data"]["protocol"], "ginterpreter/1");
    assert_eq!(replies[1]["id"], 7);
    let ops: Vec<&str> = replies[0]["data"]["operations"].as_array().unwrap().iter().map(|o| o["name"].as_str().unwrap()).collect();
    for op in ["$check", "$holes", "$eval", "$declare", "$reset", "$load", "$inspect", "$image", "$save", "$restore"] {
        assert!(ops.contains(&op), "{ops:?}");
    }
    // Every operation and every response kind is spelled with a `$`.
    assert!(ops.iter().all(|op| op.starts_with('$')), "{ops:?}");
    assert_eq!(replies[0]["kind"], "$describe");
    assert_eq!(replies[1]["kind"], "$scope");
}

#[test]
fn malformed_requests_are_structured_errors_and_the_session_goes_on() {
    let input = "not json\n[1]\n{\"op\":\"$describe\"}\n{\"id\":1.5,\"op\":\"$describe\"}\n{\"id\":1}\n{\"id\":2,\"op\":\"nope\"}\n{\"id\":3,\"op\":\"$check\",\"bogus\":1}\n{\"id\":4,\"op\":\"$check\",\"source\":\"a\",\"path\":\"b\"}\n{\"id\":5,\"op\":\"$eval\"}\n{\"id\":6,\"op\":\"$describe\"}\n";
    let replies = lines(&run_raw(input));
    let expected = [
        ("jsonl-invalid-json", Value::Null),
        ("jsonl-invalid-request", Value::Null),
        ("jsonl-invalid-id", Value::Null),
        ("jsonl-invalid-id", Value::Null),
        ("jsonl-invalid-request", json!(1)),
        ("jsonl-unknown-op", json!(2)),
        ("jsonl-invalid-request", json!(3)),
        ("jsonl-invalid-request", json!(4)),
        ("jsonl-invalid-request", json!(5)),
    ];
    assert_eq!(replies.len(), expected.len() + 1);
    for (reply, (code, id)) in replies.iter().zip(expected) {
        assert_eq!(reply["status"], "error", "{reply}");
        assert_eq!(reply["kind"], "protocol_error");
        assert_eq!(codes(reply), [code], "{reply}");
        assert_eq!(reply["id"], id, "{reply}");
    }
    // The request after all of that still works.
    assert_eq!(replies[9]["status"], "ok");
    assert_eq!(replies[9]["id"], 6);
}

#[test]
fn an_oversized_request_is_refused_and_the_next_is_served() {
    let big = format!("{{\"id\":1,\"op\":\"$check\",\"source\":\"{}\"}}\n", "x".repeat(1024 * 1024 + 10));
    let input = format!("{big}{{\"id\":2,\"op\":\"$describe\"}}\n");
    let replies = lines(&run_raw(&input));
    assert_eq!(codes(&replies[0]), ["jsonl-request-too-large"]);
    assert_eq!(replies[1]["id"], 2);
}

#[test]
fn quit_ends_the_session_and_output_is_deterministic() {
    let requests = [
        json!({"id":1,"op":"$declare","source":"five() in 0 to 9: return 5\n"}),
        json!({"id":2,"op":"$check","source":"f(n in 0 to 9) in 0 to 9: return n - 3\n"}),
        json!({"id":3,"op":"$quit"}),
        json!({"id":4,"op":"$describe"}),
    ];
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    let (a, b) = (run_raw(&input), run_raw(&input));
    assert_eq!(a.stdout, b.stdout);
    let replies = lines(&a);
    assert_eq!(replies.len(), 3, "nothing after quit is answered");
}

// ---- check ---------------------------------------------------------------------------

const BAD: &str = "f(n in 0 to 9) in 0 to 9: return n - 3\n";

#[test]
fn a_flaw_has_a_code_an_exact_span_its_construct_and_the_claim() {
    let reply = one(json!({"id":1,"op":"$check","source":BAD}));
    assert_eq!(reply["data"]["complete"], false);
    assert_eq!(reply["data"]["flaw_count"], 1);
    let flaw = &reply["diagnostics"][0];
    assert_eq!(flaw["code"], "source-check-type");
    assert_eq!(flaw["severity"], "flaw");
    assert_eq!(flaw["stage"], "check");
    // The checker locates the enclosing construct: here the whole function, bytes 0 to 38.
    let span = &flaw["span"];
    assert_eq!((span["byte_start"].as_u64(), span["byte_end"].as_u64()), (Some(0), Some(38)));
    assert_eq!(&BAD[0..38], BAD.trim_end());
    assert_eq!((span["start"]["line"].as_u64(), span["start"]["column"].as_u64()), (Some(1), Some(1)));
    assert_eq!((span["end"]["line"].as_u64(), span["end"]["column"].as_u64()), (Some(1), Some(39)));
    assert_eq!(span["file"], "<source>");
    assert_eq!(flaw["construct"]["kind"], "function");
    assert_eq!(flaw["construct"]["name"], "f");
    assert_eq!(flaw["claim"], "source type check failed");
    // The exact expression being checked, inside that construct.
    let culprit = &flaw["culprit"];
    assert_eq!((culprit["byte_start"].as_u64(), culprit["byte_end"].as_u64()), (Some(33), Some(38)));
    assert_eq!(&BAD[33..38], "n - 3");
    assert_eq!((culprit["start"]["column"].as_u64(), culprit["end"]["column"].as_u64()), (Some(34), Some(39)));
    // The facts the checker had proved there, and what the function declared.
    let proved = flaw["facts"]["proved"].as_array().unwrap();
    assert_eq!(proved.len(), 1, "{proved:?}");
    assert_eq!(proved[0], json!({"kind":"interval","name":"n","lower":0,"upper":9,"text":"0 <= n <= 9"}));
    assert_eq!(flaw["facts"]["declared_parameters"][0]["name"], "n");
    assert_eq!(flaw["facts"]["declared_parameters"][0]["type"], "in 0 to 9");
}

#[test]
fn a_correct_program_has_no_flaws() {
    let reply = one(json!({"id":1,"op":"$check","source":"f(n in 0 to 9) in 0 to 9: return n\n"}));
    assert_eq!(reply["data"]["complete"], true);
    assert_eq!(reply["data"]["flaw_count"], 0);
    assert_eq!(reply["diagnostics"], json!([]));
}

#[test]
fn a_syntax_error_is_a_parse_stage_flaw_with_a_span() {
    let reply = one(json!({"id":1,"op":"$check","source":"f(: return\n"}));
    assert_eq!(reply["data"]["complete"], false);
    let flaw = &reply["diagnostics"][0];
    assert_eq!(flaw["stage"], "parse");
    assert_eq!(flaw["severity"], "flaw");
    assert!(flaw["span"]["byte_end"].as_u64().unwrap() >= flaw["span"]["byte_start"].as_u64().unwrap());
}

#[test]
fn independent_flaws_are_all_reported_and_knock_ons_say_so() {
    let source = "f(n in 0 to 9) in 0 to 9: return n - 3\ng(n in 0 to 9) in 0 to 9: return n + 5\nh(n in 0 to 9) in 0 to 9: return n\n";
    let reply = one(json!({"id":1,"op":"$check","source":source}));
    let flaws = reply["diagnostics"].as_array().unwrap();
    let named: Vec<(&str, Vec<&str>)> = flaws
        .iter()
        .map(|f| (f["construct"]["name"].as_str().unwrap(), f["knock_on_of"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect()))
        .collect();
    assert_eq!(named, [("f", vec![]), ("g", vec!["f"])]);
    // Ordered by position, deterministic.
    assert!(flaws[0]["span"]["byte_start"].as_u64() < flaws[1]["span"]["byte_start"].as_u64());
}

#[test]
fn a_package_path_is_checked_with_the_file_of_the_flaw() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mod.flask"), "#settings\nmodule: \"p\"\nversion: 0.1.0\n").unwrap();
    std::fs::write(dir.path().join("main.gin"), BAD).unwrap();
    let reply = one(json!({"id":1,"op":"$check","path":dir.path().to_str().unwrap()}));
    assert_eq!(reply["data"]["complete"], false);
    let flaw = &reply["diagnostics"][0];
    assert_eq!(flaw["code"], "source-check-type");
    assert!(flaw["span"]["file"].as_str().unwrap().ends_with("main.gin"), "{flaw}");
    assert_eq!(flaw["span"]["byte_end"], 38);
    let missing = one(json!({"id":2,"op":"$check","path":"/definitely/not/here.gin"}));
    assert_eq!(missing["status"], "error");
}

// ---- holes ---------------------------------------------------------------------------

const HOLE: &str = "h(n in 0 to 9) in 0 to 9:\n    base in 0 to 9: 2\n    when n < 5 then return 1\n    return #todo\n";

#[test]
fn a_hole_is_listed_with_its_type_scope_and_facts() {
    let reply = one(json!({"id":1,"op":"$holes","source":HOLE}));
    assert_eq!(reply["data"]["complete"], true);
    let holes = reply["data"]["holes"].as_array().unwrap();
    assert_eq!(holes.len(), 1);
    let hole = &holes[0];
    assert_eq!(hole["construct"], "h");
    assert_eq!(hole["expected_type"], "0 to 9");
    assert_eq!(hole["in_scope"]["parameters"][0]["name"], "n");
    assert_eq!(hole["in_scope"]["locals"][0]["name"], "base");
    assert_eq!(hole["in_scope"]["functions"][0], "h(0 to 9) -> 0 to 9");
    // The facts where the hole stands: the guard `n < 5` failed, so `n` is 5 to 9; `base` is known.
    let facts = hole["facts"].as_array().unwrap();
    assert!(facts.contains(&json!({"kind":"interval","name":"n","lower":5,"upper":9,"text":"5 <= n <= 9"})), "{facts:?}");
    assert!(facts.contains(&json!({"kind":"known","name":"base","value":2,"text":"base = 2"})), "{facts:?}");
    assert!(hole["fact_claims"].as_array().unwrap().contains(&json!("5 <= n <= 9")));
    // The span is the `#todo` token itself.
    let start = hole["span"]["byte_start"].as_u64().unwrap() as usize;
    let end = hole["span"]["byte_end"].as_u64().unwrap() as usize;
    assert_eq!(&HOLE[start..end], "#todo");
    assert_eq!(hole["span"]["start"]["line"], 4);
}

#[test]
fn a_hole_is_rechecked_after_each_edit() {
    let replies = session(&[
        json!({"id":1,"op":"$declare","source":HOLE}),
        json!({"id":2,"op":"$holes"}),
        json!({"id":3,"op":"$declare","source":"h(n in 0 to 9) in 0 to 9:\n    when n < 5 then return 1\n    return n\n"}),
        json!({"id":4,"op":"$holes"}),
    ]);
    // A hole is a warning: the declaration is accepted.
    assert_eq!(replies[0]["data"]["accepted"], true);
    assert_eq!(codes(&replies[0]), ["source-check-hole"]);
    assert_eq!(replies[0]["diagnostics"][0]["severity"], "warning");
    assert_eq!(replies[1]["data"]["holes"].as_array().unwrap().len(), 1);
    // Filling it replaces the definition and the hole is gone.
    assert_eq!(replies[2]["data"]["replaced"], json!(["fn h/1"]));
    assert_eq!(replies[3]["data"]["holes"], json!([]));
    assert_eq!(replies[3]["data"]["complete"], true);
}

#[test]
fn holes_in_a_program_with_a_flaw_are_marked_incomplete() {
    let reply = one(json!({"id":1,"op":"$holes","source":format!("{HOLE}{BAD}")}));
    assert_eq!(reply["data"]["complete"], false);
    assert_eq!(reply["data"]["holes"].as_array().unwrap().len(), 1, "the hole in the part that checks is still listed");
}

// ---- eval ----------------------------------------------------------------------------

#[test]
fn an_expression_with_known_inputs_is_evaluated() {
    let replies = session(&[
        json!({"id":1,"op":"$declare","source":"five() in 0 to 9: return 5\n"}),
        json!({"id":2,"op":"$eval","expression":"five() + 1"}),
        json!({"id":3,"op":"$eval","call":"five","args":[]}),
    ]);
    assert_eq!(replies[1]["data"]["known"], true);
    assert_eq!(replies[1]["data"]["display"], "6");
    assert_eq!(replies[1]["data"]["value"], json!({"kind":"integer","value":6}));
    assert_eq!(replies[2]["data"]["value"]["value"], 5);
    assert_eq!(replies[2]["data"]["type"], "0 to 9");
}

#[test]
fn records_and_sums_come_back_structured() {
    let source = "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 9\n\n#unrestricted\nMode is Quiet or Loud\n\nmake(a in 0 to 9) Point: return Point(x: a, y: 4)\n";
    let replies = session(&[
        json!({"id":1,"op":"$declare","source":source}),
        json!({"id":2,"op":"$eval","expression":"make(2)"}),
        json!({"id":3,"op":"$eval","expression":"Mode.Loud"}),
    ]);
    assert_eq!(replies[1]["data"]["display"], "Point(x: 2, y: 4)");
    assert_eq!(replies[1]["data"]["type"], "Point");
    assert_eq!(replies[1]["data"]["value"]["fields"][1], json!({"name":"y","value":{"kind":"integer","value":4}}));
    assert_eq!(replies[2]["data"]["display"], "Loud");
    assert_eq!(replies[2]["data"]["type"], "Mode");
    assert_eq!(replies[2]["data"]["value"]["kind"], "record");
    assert_eq!(replies[2]["data"]["value"]["type"], "Loud");
    assert_eq!(replies[2]["data"]["value"]["sum"], "Mode");
}

#[test]
fn an_unknown_input_is_named_as_the_reason() {
    let source = "f(n in 0 to 9) in 0 to 9: return n\n";
    let reply = one(json!({"id":1,"op":"$eval","source":source,"call":"f","args":[null]}));
    assert_eq!(reply["data"]["known"], false);
    assert_eq!(reply["data"]["reason"]["kind"], "unknown_input");
    let known = one(json!({"id":2,"op":"$eval","source":source,"call":"f","args":[4]}));
    assert_eq!(known["data"]["known"], true);
    assert_eq!(known["data"]["value"]["value"], 4);
    // An input outside its declared domain is not known to be a call at all.
    let outside = one(json!({"id":3,"op":"$eval","source":source,"call":"f","args":[40]}));
    assert_eq!(outside["data"]["reason"]["kind"], "argument_out_of_domain");
}

#[test]
fn a_reached_hole_gives_its_call_chain_and_an_untaken_one_never_fires() {
    let source = "inner(a in 0 to 9) in 0 to 9: return #todo\nouter(a in 0 to 9) in 0 to 9: return when a < 5 then 4 else inner(a)\n";
    let taken = one(json!({"id":1,"op":"$eval","source":source,"call":"outer","args":[8]}));
    assert_eq!(taken["data"]["known"], false);
    assert_eq!(taken["data"]["reason"]["kind"], "hole_reached");
    assert_eq!(taken["data"]["reason"]["function"], "inner");
    assert_eq!(taken["data"]["reason"]["call_chain"], json!(["outer", "inner"]));
    let untaken = one(json!({"id":2,"op":"$eval","source":source,"call":"outer","args":[3]}));
    assert_eq!(untaken["data"]["known"], true);
    assert_eq!(untaken["data"]["value"]["value"], 4);
}

#[test]
fn an_instruction_is_refused_by_name_only_when_reached() {
    let source = format!("{TRAP}choose(n in 0 to 9) in 0 to 9: return when n < 5 then 1 else risky()\n");
    let untaken = one(json!({"id":1,"op":"$eval","source":source,"call":"choose","args":[3]}));
    assert_eq!(untaken["data"]["value"]["value"], 1);
    let taken = one(json!({"id":2,"op":"$eval","source":source,"call":"choose","args":[7]}));
    assert_eq!(taken["data"]["reason"]["kind"], "instruction_refused");
}

#[test]
fn a_body_the_recorder_declined_is_named() {
    let source = "answer(n in 0 to 3) in 0 to 9:\n  i in 0 to 9: 0\n  while i < n and i < 2 do\n    i :: i + 1\n  return i\n";
    let reply = one(json!({"id":1,"op":"$eval","source":source,"call":"answer","args":[2]}));
    assert_eq!(reply["data"]["reason"]["kind"], "unrecorded_body");
    assert_eq!(reply["data"]["reason"]["function"], "answer");
}

#[test]
fn a_flawed_expression_or_program_runs_nothing_and_reports_the_flaw() {
    let unknown_name = one(json!({"id":1,"op":"$eval","expression":"missing(1)"}));
    assert_eq!(unknown_name["data"]["known"], false);
    assert_eq!(unknown_name["data"]["reason"]["kind"], "flawed");
    assert_eq!(codes(&unknown_name), ["source-check-unknown-name"]);
    let program = one(json!({"id":2,"op":"$eval","source":BAD,"call":"f","args":[4]}));
    assert_eq!(program["data"]["reason"]["kind"], "flawed");
    assert_eq!(codes(&program), ["source-check-type"]);
}

#[test]
fn a_package_entry_is_evaluated() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mod.flask"), "#settings\nmodule: \"p\"\nversion: 0.1.0\n").unwrap();
    std::fs::write(dir.path().join("main.gin"), "#entry\nstart() in 0 to 255: return 42\n").unwrap();
    let reply = one(json!({"id":1,"op":"$eval","entry":true,"path":dir.path().to_str().unwrap()}));
    assert_eq!(reply["data"]["value"]["value"], 42);
}

// ---- session -------------------------------------------------------------------------

#[test]
fn a_flawed_declaration_is_rejected_and_leaves_the_session_unchanged() {
    let replies = session(&[
        json!({"id":1,"op":"$declare","source":"five() in 0 to 9: return 5\n"}),
        json!({"id":2,"op":"$declare","source":BAD}),
        json!({"id":3,"op":"$scope"}),
        json!({"id":4,"op":"$eval","expression":"five()"}),
    ]);
    assert_eq!(replies[1]["data"]["accepted"], false);
    assert_eq!(codes(&replies[1]), ["source-check-type"]);
    let names: Vec<&str> = replies[2]["data"]["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["five"]);
    assert_eq!(replies[3]["data"]["value"]["value"], 5);
}

#[test]
fn replace_remove_reset_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib.gin");
    std::fs::write(&file, "two() in 0 to 9: return 2\n").unwrap();
    let replies = session(&[
        json!({"id":1,"op":"$declare","source":"v() in 0 to 9: return 1\n"}),
        json!({"id":2,"op":"$declare","source":"v() in 0 to 9: return 7\n"}),
        json!({"id":3,"op":"$eval","expression":"v()"}),
        json!({"id":4,"op":"$load","path":file.to_str().unwrap()}),
        json!({"id":5,"op":"$eval","expression":"v() + two()"}),
        json!({"id":6,"op":"$remove","name":"two"}),
        json!({"id":7,"op":"$eval","expression":"two()"}),
        json!({"id":8,"op":"$reset"}),
        json!({"id":9,"op":"$scope"}),
        json!({"id":10,"op":"$load","path":"/definitely/not/here.gin"}),
    ]);
    assert_eq!(replies[1]["data"]["replaced"], json!(["fn v/0"]));
    assert_eq!(replies[2]["data"]["value"]["value"], 7);
    assert_eq!(replies[3]["data"]["added"], json!(["fn two/0"]));
    assert_eq!(replies[4]["data"]["value"]["value"], 9);
    assert_eq!(replies[5]["data"]["removed"], json!(["fn two/0"]));
    assert_eq!(codes(&replies[6]), ["source-check-unknown-name"]);
    assert_eq!(replies[8]["data"]["entries"], json!([]));
    assert_eq!(replies[9]["status"], "error");
}

#[test]
fn a_hole_reached_by_a_relation_lists_it() {
    let source = "f(n in 0 to 9, m in 0 to 9) in 0 to 9:\n    when n < m then\n        return #todo\n    return 1\n";
    let reply = one(json!({"id":1,"op":"$holes","source":source}));
    let facts = reply["data"]["holes"][0]["facts"].as_array().unwrap().clone();
    assert!(facts.iter().any(|f| f["kind"] == "relation" && f["text"] == "n < m"), "{facts:?}");
}

#[test]
fn a_hole_warning_carries_its_facts_too_and_an_unlocated_flaw_has_none() {
    let reply = one(json!({"id":1,"op":"$check","source":HOLE}));
    let warning = &reply["diagnostics"][0];
    assert_eq!(warning["code"], "source-check-hole");
    assert!(warning["facts"]["proved"].as_array().unwrap().iter().any(|f| f["text"] == "5 <= n <= 9"));
    // A syntax error arose before the checker held any fact.
    let parse = one(json!({"id":2,"op":"$check","source":"f(: return\n"}));
    assert!(parse["diagnostics"][0]["facts"].is_null());
    assert!(parse["diagnostics"][0]["culprit"].is_null());
}

#[test]
fn the_old_spellings_are_unknown_ops_that_suggest_the_dollar_spelling() {
    for (op, suggestion) in [("check", Some("$check")), ("inspect", Some("$inspect")), (":inspect", Some("$inspect")), ("#inspect", Some("$inspect")), ("restore", Some("$restore")), ("nonsense", None), ("#nonsense", None), ("$nonsense", None)] {
        let reply = one(json!({"id":7,"op":op}));
        assert_eq!(reply["status"], "error", "{op}");
        assert_eq!(codes(&reply), ["jsonl-unknown-op"], "{op}");
        assert_eq!(reply["id"], 7);
        let message = reply["diagnostics"][0]["message"].as_str().unwrap();
        match suggestion {
            Some(s) => assert!(message.contains(&format!("did you mean `{s}`")), "{op}: {message}"),
            None => assert!(!message.contains("did you mean"), "{op}: {message}"),
        }
        assert!(message.contains("$declare"), "{message}");
    }
}

#[test]
fn every_response_kind_is_the_dollar_name_of_its_operation() {
    let replies = session(&[
        json!({"id":1,"op":"$declare","source":"five() in 0 to 9: return 5\n"}),
        json!({"id":2,"op":"$check"}),
        json!({"id":3,"op":"$holes"}),
        json!({"id":4,"op":"$eval","expression":"five()"}),
        json!({"id":5,"op":"$inspect","name":"five"}),
        json!({"id":6,"op":"$image"}),
        json!({"id":7,"op":"$scope"}),
        json!({"id":8,"op":"$remove","name":"nothing"}),
        json!({"id":9,"op":"$reset"}),
        json!({"id":10,"op":"$quit"}),
    ]);
    let kinds: Vec<&str> = replies.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["$declare", "$check", "$holes", "$eval", "$inspect", "$image", "$scope", "$remove", "$reset", "$quit"]);
}

#[test]
fn a_directive_used_as_an_expression_evaluates_as_the_language_directive() {
    let source = "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 300\n";
    let r = session(&[
        json!({"id":1,"op":"$declare","source":source}),
        json!({"id":2,"op":"$eval","expression":"#size(Point)"}),
        json!({"id":3,"op":"$eval","expression":"#alignment(Point) + 1"}),
    ]);
    assert_eq!(r[1]["data"]["known"], true, "{}", r[1]);
    assert_eq!(r[1]["data"]["value"]["value"], 4);
    assert_eq!(r[2]["data"]["value"]["value"], 3);
}
