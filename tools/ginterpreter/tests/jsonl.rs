use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use test_fixtures::TempPackage;

fn run(input: &str) -> Output {
    let directory = TempPackage::new("interpreter_jsonl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .current_dir(directory.path())
        .args(["--jsonl", "--isolated"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn requests(requests: &[Value]) -> Output {
    let input = requests
        .iter()
        .map(|request| format!("{request}\n"))
        .collect::<String>();
    run(&input)
}

fn responses(output: &Output) -> Vec<Value> {
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.contains(&0x1b));
    std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect()
}

#[test]
fn catalog_and_query_help_are_discoverable_without_a_package() {
    let output = requests(&[
        json!({"id":"catalog","op":"describe"}),
        json!({"id":2,"op":"queries","filter":"#size"}),
        json!({"id":3,"op":"help","topic":"#size"}),
    ]);
    assert!(output.status.success());
    let replies = responses(&output);
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["id"], "catalog");
    assert_eq!(replies[0]["data"]["protocol_version"], 1);
    assert_eq!(replies[0]["data"]["context"]["isolated"], true);
    assert!(
        replies[0]["data"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| operation["name"] == "inspect")
    );
    assert!(
        replies[0]["data"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability["name"] == "size" || capability["name"] == "#size")
    );
    let queries = replies[1]["data"].as_array().unwrap();
    assert!(!queries.is_empty());
    assert!(
        queries
            .iter()
            .all(|query| query["kind"] == "compiler_query")
    );
    assert!(
        replies[2]["data"]["text"]
            .as_str()
            .unwrap()
            .contains("#size")
    );
}

#[test]
fn shared_session_acceptance_and_rich_results_are_explicit_and_lossless() {
    let output = requests(&[
        json!({"id":1,"op":"submit","source":"answer:\n    return 40 + 2"}),
        json!({"id":2,"op":"submit","source":"answer"}),
        json!({"id":3,"op":"inspect","expression":"1 < 2"}),
        json!({"id":4,"op":"submit","source":"9223372036854775808"}),
        json!({"id":5,"op":"submit","source":"'café 🦀'"}),
    ]);
    assert!(output.status.success(), "{output:?}");
    let replies = responses(&output);
    assert_eq!(replies.len(), 5);
    assert_eq!(replies[0]["kind"], "declaration_accepted");
    assert_eq!(replies[1]["data"]["value"]["display"], "42");
    assert_eq!(replies[2]["kind"], "inspected");
    assert_eq!(
        replies[2]["data"]["value"]["nodes"][0]["kind"],
        "proposition"
    );
    assert_eq!(
        replies[3]["data"]["value"]["display"],
        "9223372036854775808"
    );
    assert_eq!(
        replies[3]["data"]["value"]["nodes"][0]["value"],
        "9223372036854775808"
    );
    assert_eq!(replies[4]["data"]["value"]["display"], "café 🦀");
    for (index, reply) in replies.iter().enumerate() {
        assert_eq!(reply["id"], index + 1);
        assert_eq!(reply["status"], "ok");
    }
}

#[test]
fn failed_queries_preserve_pending_work_for_repair() {
    let output = requests(&[
        json!({"id":1,"op":"submit","source":"missing + 2"}),
        json!({"id":2,"op":"inspect","expression":"also_missing"}),
        json!({"id":3,"op":"queries"}),
        json!({"id":4,"op":"submit","source":"missing := 40"}),
        json!({"id":5,"op":"submit","source":"#retry"}),
    ]);
    assert!(!output.status.success());
    let replies = responses(&output);
    assert_eq!(replies[0]["kind"], "rejected");
    assert_eq!(replies[1]["kind"], "rejected");
    assert_eq!(replies[4]["data"]["value"]["display"], "42");
    assert_eq!(replies[4]["status"], "ok");
}

#[test]
fn diagnostics_keep_utf8_source_coordinates() {
    let source = "-- café 🦀\nanswer:\n    missing";
    let output = requests(&[json!({"id":"unicode","op":"submit","source":source})]);
    assert!(!output.status.success());
    let replies = responses(&output);
    let diagnostic = replies[0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "type-unknown-symbol")
        .unwrap();
    let source = diagnostic["source"].as_str().unwrap();
    let start = diagnostic["span"]["start"].as_u64().unwrap() as usize;
    let end = diagnostic["span"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], "missing");
    assert_eq!(diagnostic["line"], 3);
    assert_eq!(diagnostic["column"], 5);
}

#[test]
fn malformed_requests_are_framed_and_do_not_end_the_session() {
    let output = run(concat!(
        "not json\n",
        "{\"op\":\"describe\"}\n",
        "{\"id\":[],\"op\":\"describe\"}\n",
        "{\"id\":4,\"op\":\"unknown\"}\n",
        "{\"id\":5,\"op\":\"submit\",\"source\":\"42\",\"extra\":true}\n",
        "{\"id\":6,\"op\":\"describe\",\"extra\":true}\n",
        "{\"id\":7,\"op\":\"scope\",\"extra\":true}\n",
        "{\"id\":8,\"op\":\"inspect\",\"expression\":\"42\",\"limits\":{\"nodes_max\":-1}}\n",
        "{\"id\":9,\"op\":\"inspect\",\"expression\":\"42\",\"limits\":{\"unknown\":1}}\n",
        "{\"id\":10,\"op\":\"submit\",\"source\":\"42\"}\n",
    ));
    assert!(!output.status.success());
    let replies = responses(&output);
    assert_eq!(replies.len(), 10);
    for reply in &replies[..9] {
        assert_eq!(reply["status"], "error");
        assert_eq!(reply["kind"], "protocol_error");
    }
    assert_eq!(replies[3]["id"], 4);
    assert_eq!(replies[9]["data"]["value"]["display"], "42");
}

#[test]
fn oversized_requests_are_drained_before_the_next_request() {
    let input = format!(
        "{}\n{{\"id\":2,\"op\":\"submit\",\"source\":\"42\"}}\n",
        " ".repeat(1024 * 1024 + 1)
    );
    let output = run(&input);
    assert!(!output.status.success());
    let replies = responses(&output);
    assert_eq!(replies.len(), 2);
    assert_eq!(
        replies[0]["diagnostics"][0]["code"],
        "jsonl-request-too-large"
    );
    assert_eq!(replies[1]["data"]["value"]["display"], "42");
}

#[test]
fn scope_search_documentation_and_completion_form_a_discovery_workflow() {
    let output = requests(&[
        json!({"id":1,"op":"submit","source":"--- Counts café visits.\nanswer := 42"}),
        json!({"id":2,"op":"scope"}),
        json!({"id":3,"op":"search","text":"CAFÉ"}),
        json!({"id":4,"op":"doc","symbol":"answer"}),
        json!({"id":5,"op":"complete","source":"ans"}),
        json!({"id":6,"op":"search","text":"#alignment"}),
        json!({"id":7,"op":"complete","source":"#target.tr"}),
        json!({"id":8,"op":"submit","source":"answer"}),
    ]);
    assert!(output.status.success(), "{output:?}");
    let replies = responses(&output);
    assert_eq!(replies.len(), 8);
    for (index, reply) in replies.iter().enumerate() {
        assert_eq!(reply["id"], index + 1);
        assert_eq!(reply["status"], "ok");
    }
    assert_eq!(replies[1]["kind"], "scope");
    assert!(
        replies[1]["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "answer")
    );
    assert_eq!(replies[2]["kind"], "search");
    assert_eq!(replies[2]["data"][0]["name"], "answer");
    assert_eq!(replies[3]["kind"], "documentation");
    assert!(
        replies[3]["data"]["text"]
            .as_str()
            .unwrap()
            .contains("Counts café visits.")
    );
    assert_eq!(replies[4]["kind"], "completions");
    assert!(
        replies[4]["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["label"] == "answer")
    );
    assert!(
        replies[5]["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "#alignment" && entry["kind"] == "compiler_query")
    );
    assert_eq!(replies[6]["data"][0]["label"], "triple");
    assert_eq!(replies[6]["data"][0]["kind"], "field");
    assert_eq!(replies[7]["data"]["value"]["display"], "42");
}

#[test]
fn inspect_limits_expose_truncation_and_allow_expanding_a_record() {
    let output = requests(&[
        json!({"id":1,"op":"inspect","expression":"#target",
            "limits":{"depth_max":0,"nodes_max":1,"text_bytes_max":8192}}),
        json!({"id":2,"op":"inspect","expression":"#target",
            "limits":{"depth_max":8,"nodes_max":16,"text_bytes_max":8192}}),
        json!({"id":3,"op":"inspect","expression":"#target.triple"}),
        json!({"id":4,"op":"inspect","expression":"'café 🦀 and more text'",
            "limits":{"text_bytes_max":8}}),
        json!({"id":5,"op":"inspect","expression":"'café 🦀 and more text'",
            "limits":{"text_bytes_max":4096}}),
    ]);
    assert!(output.status.success(), "{output:?}");
    let replies = responses(&output);
    assert_eq!(replies.len(), 5);
    let small = &replies[0]["data"]["value"];
    assert_eq!(small["truncated"], true);
    assert_eq!(small["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(small["nodes"][0]["kind"], "record");
    assert_eq!(small["nodes"][0]["total_children"], 4);
    assert_eq!(small["nodes"][0]["truncated"], true);
    let expanded = &replies[1]["data"]["value"];
    let nodes = expanded["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 5);
    assert_eq!(nodes[0]["truncated"], false);
    let children = nodes[0]["children"].as_array().unwrap();
    assert_eq!(children.len(), 4);
    let triple = children
        .iter()
        .find(|child| child["name"] == "triple")
        .unwrap();
    let triple_node = &nodes[triple["node"].as_u64().unwrap() as usize];
    assert_eq!(triple_node["kind"], "text");
    assert_eq!(
        triple_node["value"],
        replies[2]["data"]["value"]["nodes"][0]["value"]
    );
    assert_eq!(replies[3]["data"]["value"]["truncated"], true);
    assert!(
        replies[3]["data"]["value"]["display"]
            .as_str()
            .unwrap()
            .len()
            <= 8
    );
    assert_eq!(replies[4]["data"]["value"]["nodes"][0]["truncated"], false);
    assert_eq!(
        replies[4]["data"]["value"]["display"],
        "café 🦀 and more text"
    );
}

struct LiveClient {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<Value>,
}

impl LiveClient {
    fn new() -> Self {
        Self::at(Path::new("."))
    }

    fn at(directory: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
            .current_dir(directory)
            .args(["--jsonl", "--isolated"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = serde_json::from_str(&line.unwrap()).unwrap();
                if sender.send(value).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            replies,
        }
    }

    fn send(&mut self, request: Value) {
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn next(&self) -> Value {
        self.replies
            .recv_timeout(Duration::from_secs(10))
            .expect("response while stdin remains open")
    }

    fn request(&mut self, request: Value) -> Value {
        let id = request["id"].clone();
        self.send(request);
        let response = self.next();
        assert_eq!(response["id"], id);
        assert!(response.get("event").is_none());
        response
    }
}

impl Drop for LiveClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn responses_flush_and_background_results_arrive_while_input_is_idle() {
    let mut client = LiveClient::new();
    client.send(json!({"id":1,"op":"describe"}));
    assert_eq!(client.next()["kind"], "description");
    assert!(client.child.try_wait().unwrap().is_none());
    client.send(json!({"id":"background","op":"submit","source":"#run 40 + 2"}));
    let started = client.next();
    assert_eq!(started["data"]["command"], "task_started");
    let completed = client.next();
    assert_eq!(completed["id"], "background");
    assert_eq!(completed["event"], "task");
    assert_eq!(completed["task_id"], started["data"]["task_id"]);
    assert_eq!(completed["data"]["value"]["display"], "42");
    client.send(json!({"id":3,"op":"submit","source":"#quit"}));
    assert_eq!(client.next()["data"]["command"], "quit");
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn jsonl_rejects_conflicting_eval_mode_and_frames_startup_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .args(["--jsonl", "--eval", "42"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let directory = TempPackage::new("jsonl_invalid_context");
    directory.write("mod.flask", "invalid");
    let output = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .arg("--jsonl")
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let replies = responses(&output);
    assert_eq!(
        replies[0]["diagnostics"][0]["code"],
        "interpreter-context-unavailable"
    );
}

#[path = "jsonl/program.rs"]
mod program;
