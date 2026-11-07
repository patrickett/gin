//! The protocol, end to end through the real binary: lifecycle, malformed input and unknown
//! methods. Each test has its negative control.
mod common;

use common::{Client, frame};
use serde_json::{Value, json};

#[test]
fn initialize_advertises_the_implemented_features_and_shutdown_exits_zero() {
    let mut client = Client::start();
    let reply = client.request(
        "initialize",
        json!({"processId": null, "rootUri": null, "capabilities": {}}),
    );
    let caps = &reply["result"]["capabilities"];
    assert_eq!(caps["textDocumentSync"]["change"], 1, "full-text sync");
    assert_eq!(caps["hoverProvider"], true);
    assert_eq!(caps["documentSymbolProvider"], true);
    assert_eq!(caps["definitionProvider"], true);
    assert_eq!(caps["workspaceSymbolProvider"], true);
    assert_eq!(caps["documentFormattingProvider"], true);
    assert_eq!(reply["result"]["serverInfo"]["name"], "ginlsp");
    // Not implemented, so not advertised.
    assert!(
        caps.get("completionProvider").is_none() && caps.get("referencesProvider").is_none(),
        "{caps}"
    );
    client.notify("initialized", json!({}));
    assert_eq!(client.shutdown(), 0);
}

#[test]
fn exit_without_shutdown_is_a_failure_exit() {
    let mut client = Client::initialized(&[]);
    client.notify("exit", Value::Null);
    assert_eq!(client.finish(), 1, "negative control for the zero above");
}

#[test]
fn the_end_of_input_ends_the_server_without_a_hang() {
    let client = Client::initialized(&[]);
    assert_eq!(client.finish(), 1);
}

#[test]
fn requests_before_initialize_are_refused_but_the_server_goes_on() {
    let mut client = Client::start();
    let early = client.request("textDocument/hover", json!({}));
    assert_eq!(early["error"]["code"], -32002, "{early}");
    let ok = client.request(
        "initialize",
        json!({"processId": null, "rootUri": null, "capabilities": {}}),
    );
    assert!(ok["result"]["capabilities"].is_object(), "{ok}");
    let again = client.request(
        "initialize",
        json!({"processId": null, "rootUri": null, "capabilities": {}}),
    );
    assert_eq!(again["error"]["code"], -32600, "{again}");
}

#[test]
fn unknown_request_is_method_not_found_and_unknown_notification_is_ignored() {
    let mut client = Client::initialized(&[]);
    let reply = client.request("textDocument/frobnicate", json!({}));
    assert_eq!(reply["error"]["code"], -32601, "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("frobnicate")
    );
    // Negative control: a known method is not MethodNotFound.
    let known = client.request("workspace/symbol", json!({"query": "x"}));
    assert!(known.get("error").is_none(), "{known}");
    // A notification is never answered, even when unknown.
    client.notify("custom/unknown", json!({}));
    client.notify("$/setTrace", json!({"value": "off"}));
    assert!(client.quiet(300, |m| m.get("error").is_some()));
    let alive = client.request("workspace/symbol", json!({"query": "x"}));
    assert!(alive.get("result").is_some());
}

#[test]
fn malformed_messages_get_errors_and_never_end_the_server() {
    let mut client = Client::initialized(&[]);
    // A body that is not JSON.
    client.send_raw(&frame("{not json"));
    let m = client.wait(|m| m.get("error").is_some());
    assert_eq!(m["error"]["code"], -32700, "{m}");
    assert_eq!(m["id"], Value::Null);
    // JSON, but not an object.
    client.send_raw(&frame("[1,2,3]"));
    assert_eq!(
        client.wait(|m| m.get("error").is_some())["error"]["code"],
        -32600
    );
    // An object that is not a message.
    client.send_raw(&frame("{\"jsonrpc\":\"2.0\",\"id\":9}"));
    let m = client.wait(|m| m.get("error").is_some());
    assert_eq!(m["error"]["code"], -32600);
    assert_eq!(m["id"], 9);
    // An id of the wrong type.
    client.send_raw(&frame(
        "{\"jsonrpc\":\"2.0\",\"id\":[1],\"method\":\"shutdown\"}",
    ));
    assert_eq!(
        client.wait(|m| m.get("error").is_some())["error"]["code"],
        -32600
    );
    // A header block with no Content-Length.
    client.send_raw(b"X-Other: 1\r\n\r\n");
    assert_eq!(
        client.wait(|m| m.get("error").is_some())["error"]["code"],
        -32700
    );
    // A Content-Length that is not a number.
    client.send_raw(b"Content-Length: banana\r\n\r\n");
    assert_eq!(
        client.wait(|m| m.get("error").is_some())["error"]["code"],
        -32700
    );
    // Wrong parameter shapes are InvalidParams for the request that carried them.
    let reply = client.request("textDocument/hover", json!({"textDocument": 5}));
    assert_eq!(reply["error"]["code"], -32602, "{reply}");
    let reply = client.request("workspace/symbol", json!({}));
    assert_eq!(reply["error"]["code"], -32602, "{reply}");
    // Notifications with bad parameters are dropped, and everything after them still works.
    client.notify("textDocument/didOpen", json!({"textDocument": "nope"}));
    client.notify("textDocument/didChange", json!(null));
    // Negative control: after all of that the server answers a good request.
    let good = client.request("workspace/symbol", json!({"query": "x"}));
    assert_eq!(good["result"], json!([]), "{good}");
    // A shutdown that is still accepted.
    assert_eq!(client.shutdown(), 0);
}

#[test]
fn a_truncated_message_at_the_end_of_input_ends_the_server_cleanly() {
    let mut client = Client::initialized(&[]);
    client.send_raw(b"Content-Length: 500\r\n\r\n{\"jsonrpc\"");
    let code = client.finish();
    assert_eq!(code, 1);
}

#[test]
fn requests_after_shutdown_are_invalid() {
    let mut client = Client::initialized(&[]);
    let reply = client.request("shutdown", Value::Null);
    assert_eq!(reply["result"], Value::Null);
    let after = client.request("workspace/symbol", json!({"query": "x"}));
    assert_eq!(after["error"]["code"], -32600, "{after}");
    client.notify("exit", Value::Null);
    assert_eq!(client.finish(), 0);
}

#[test]
fn hover_on_an_unopened_document_is_null_not_an_error() {
    let mut client = Client::initialized(&[]);
    let reply = client.hover("file:///nowhere/never-opened.gin", 0, 0);
    assert_eq!(reply["result"], Value::Null, "{reply}");
}
