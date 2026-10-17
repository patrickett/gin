use super::*;

const PROGRAM: &str = include_str!("../../examples/order_total.gin");

#[test]
fn language_help_supplies_runnable_steps_without_external_syntax_documentation() {
    let mut client = LiveClient::new();
    let description = client.request(json!({"id":1,"op":"describe"}));
    let topic = description["data"]["language_topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|topic| topic["name"] == "functions")
        .expect("function syntax is discoverable");
    let help = client.request(json!({"id":2,"op":"help","topic":topic["name"]}));
    let steps = help["data"]["recipe"]["steps"].as_array().unwrap();
    assert!(!steps.is_empty());
    let mut evaluated = false;
    for (index, step) in steps.iter().enumerate() {
        let response = client.request(json!({
            "id":index + 3, "op":"submit", "source":step["source"]
        }));
        assert_eq!(response["status"], "ok", "{response}");
        evaluated |= response["kind"] == "evaluated";
    }
    assert!(evaluated);
    let search = client.request(json!({"id":"search","op":"search","text":"function"}));
    assert!(
        search["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["kind"] == "language_recipe" && entry["name"] == "functions" })
    );
    client.request(json!({"id":"quit","op":"submit","source":"#quit"}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn bounded_order_program_rejects_invalid_inputs_and_survives_save_and_replay() {
    let directory = TempPackage::new("jsonl_order_program");
    let mut client = LiveClient::at(directory.path());
    let accepted = client.request(json!({"id":1,"op":"submit","source":PROGRAM}));
    assert_eq!(accepted["kind"], "declaration_accepted", "{accepted}");
    for (expression, expected) in [
        ("order_total(3, 125)", "375"),
        ("invoice_total(3, 125)", "425"),
        ("invoice_total(1, 0)", "50"),
        ("invoice_total(10, 10000)", "100000"),
    ] {
        assert_value(&mut client, expression, expected);
    }
    for expression in [
        "invoice_total(0, 125)",
        "invoice_total(11, 125)",
        "invoice_total(3, 10001)",
    ] {
        let rejected = client.request(json!({"id":expression,"op":"submit","source":expression}));
        assert_eq!(rejected["kind"], "rejected", "{rejected}");
        assert!(
            rejected["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| { diagnostic["code"] == "type-integer-literal-out-of-range" })
        );
    }
    assert_value(&mut client, "invoice_total(3, 125)", "425");
    let saved = client.request(json!({"id":"save","op":"submit","source":"#save order.gin"}));
    assert_eq!(saved["status"], "ok", "{saved}");
    let source = std::fs::read_to_string(directory.path().join("order.gin")).unwrap();
    assert_eq!(source, PROGRAM);
    client.request(json!({"id":"quit","op":"submit","source":"#quit"}));
    assert!(!client.child.wait().unwrap().success());
    let mut replay = LiveClient::at(directory.path());
    let accepted = replay.request(json!({"id":1,"op":"submit","source":source}));
    assert_eq!(accepted["kind"], "declaration_accepted", "{accepted}");
    assert_value(&mut replay, "invoice_total(3, 125)", "425");
    replay.request(json!({"id":"quit","op":"submit","source":"#quit"}));
    assert!(replay.child.wait().unwrap().success());
}

fn assert_value(client: &mut LiveClient, expression: &str, expected: &str) {
    let response = client.request(json!({"id":expression,"op":"inspect","expression":expression}));
    assert_eq!(response["status"], "ok", "{response}");
    assert_eq!(
        response["data"]["value"]["display"], expected,
        "{expression}"
    );
}

#[test]
fn declaration_diagnostics_keep_submission_coordinates_after_prior_successes() {
    let source = "-- café 🦀\nanswer:\n    missing";
    let output = requests(&[
        json!({"id":1,"op":"submit","source":"seed := 1"}),
        json!({"id":2,"op":"submit","source":source}),
    ]);
    assert!(!output.status.success());
    let replies = responses(&output);
    let diagnostic = replies[1]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "type-unknown-symbol")
        .unwrap();
    assert_eq!(diagnostic["source"].as_str().unwrap().trim_end(), source);
    assert_eq!(diagnostic["line"], 3);
    assert_eq!(diagnostic["column"], 5);
    let start = diagnostic["span"]["start"].as_u64().unwrap() as usize;
    let end = diagnostic["span"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], "missing");
}
