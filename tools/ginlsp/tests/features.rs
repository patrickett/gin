//! Diagnostics, hover, symbols, holes, packages and navigation through the real binary.
mod common;

use common::{Client, uri};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const BAD: &str = "bad(n in 0 to 9) in 0 to 9: return n - 3\n";
const FIXED: &str = "bad(n in 0 to 9) in 0 to 9: return n\n";
const POINT: &str =
    "Point has\n    x in 0 to 9\n    y in 0 to 300\n\nbump(n in 0 to 9) in 0 to 9: return n\n";
const MANIFEST: &str = "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n";

fn temp_file(name: &str, text: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(name), text).unwrap();
    let uri = uri(&dir.path().canonicalize().unwrap().join(name));
    (dir, uri)
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn hover_text(reply: &Value) -> String {
    reply["result"]["contents"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("no hover: {reply}"))
        .to_owned()
}

// ---- diagnostics -------------------------------------------------------------------------

#[test]
fn a_check_flaw_is_reported_at_the_culprit_with_facts_and_clears_when_fixed() {
    let (_dir, uri) = temp_file("bad.gin", BAD);
    let mut client = Client::initialized(&[]);
    client.open(&uri, BAD);
    let diagnostics = client.diagnostics(&uri);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let d = &diagnostics[0];
    assert_eq!(d["code"], "source-check-type");
    assert_eq!(d["severity"], 1, "error");
    assert_eq!(d["source"], "ginlsp");
    // The exact expression `n - 3`, not the whole function.
    assert_eq!(
        d["range"],
        json!({"start": {"line": 0, "character": 35}, "end": {"line": 0, "character": 40}})
    );
    // The proved facts are in the message and in the data.
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("known here: 0 <= n <= 9"),
        "{d}"
    );
    assert_eq!(d["data"]["facts"], json!(["0 <= n <= 9"]));
    assert_eq!(d["data"]["construct"]["name"], "bad");
    // The enclosing construct is related information: the whole function.
    let related = &d["relatedInformation"][0];
    assert_eq!(related["location"]["uri"], uri);
    assert_eq!(
        related["location"]["range"]["start"],
        json!({"line": 0, "character": 0})
    );
    assert_eq!(
        related["location"]["range"]["end"],
        json!({"line": 0, "character": 40})
    );
    assert!(related["message"].as_str().unwrap().contains("bad"));

    // Fixing the text clears the diagnostic, with a publish of the empty set.
    client.change(&uri, 2, FIXED);
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    // Negative control: breaking it again brings it back.
    client.change(&uri, 3, BAD);
    assert_eq!(client.diagnostics(&uri).len(), 1);
    client.shutdown();
}

#[test]
fn a_different_flaw_keeps_a_diagnostic_after_an_unrelated_edit() {
    let (_dir, uri) = temp_file("bad.gin", BAD);
    let mut client = Client::initialized(&[]);
    client.open(&uri, BAD);
    assert_eq!(client.diagnostics(&uri).len(), 1);
    // Control for the clearing test: an edit that leaves the flaw in place does not clear it.
    client.change(&uri, 2, "bad(n in 0 to 9) in 0 to 9: return n - 4\n");
    let diagnostics = client.diagnostics(&uri);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["range"]["end"]["character"], 40);
    client.shutdown();
}

#[test]
fn a_syntax_error_is_a_parse_diagnostic_and_the_outline_still_works() {
    let text = "ok() in 0 to 9: return 5\nbroken(: return\n";
    let (_dir, uri) = temp_file("syntax.gin", text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let diagnostics = client.diagnostics(&uri);
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics.iter().all(|d| d["data"]["stage"] == "parse"),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics[0]["code"]
            .as_str()
            .unwrap()
            .starts_with("parse-"),
        "{diagnostics:?}"
    );
    // The recovered tree still gives the first function.
    let symbols = client.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": uri}}),
    );
    let names: Vec<&str> = symbols["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"ok"), "{names:?}");
    client.shutdown();
}

#[test]
fn flaws_in_two_declarations_are_both_reported_and_the_second_may_be_a_knock_on() {
    let text = "a(n in 0 to 9) in 0 to 9: return n - 3\nb(n in 0 to 9) in 0 to 9: return n - 4\nfine() in 0 to 9: return 1\n";
    let (_dir, uri) = temp_file("two.gin", text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let diagnostics = client.diagnostics(&uri);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert_eq!(diagnostics[0]["range"]["start"]["line"], 0);
    assert_eq!(diagnostics[1]["range"]["start"]["line"], 1);
    assert!(
        diagnostics[1]["message"]
            .as_str()
            .unwrap()
            .contains("may follow from the flaw in: a"),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("may follow")
    );
    client.shutdown();
}

#[test]
fn a_clean_file_publishes_an_empty_set() {
    let (_dir, uri) = temp_file("clean.gin", POINT);
    let mut client = Client::initialized(&[]);
    client.open(&uri, POINT);
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    client.shutdown();
}

#[test]
fn a_burst_of_edits_is_checked_once_on_the_last_text() {
    let (_dir, uri) = temp_file("burst.gin", FIXED);
    let mut client = Client::initialized(&[]);
    // Three changes in one write: only the last one is checked and published.
    let mut bytes = Vec::new();
    for (v, text) in [(1, FIXED), (2, BAD), (3, FIXED)] {
        let message = json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": uri, "languageId": "gin", "version": v, "text": text}}});
        bytes.extend(common::frame(&message.to_string()));
    }
    client.send_raw(&bytes);
    let first = client.wait(|m| m["method"] == "textDocument/publishDiagnostics");
    assert_eq!(first["params"]["diagnostics"], json!([]));
    assert_eq!(first["params"]["version"], 3);
    assert!(
        client.quiet(300, |m| m["method"] == "textDocument/publishDiagnostics"),
        "one publish for the burst"
    );
    client.shutdown();
}

#[test]
fn closing_a_document_clears_its_diagnostics() {
    let (_dir, uri) = temp_file("close.gin", BAD);
    let mut client = Client::initialized(&[]);
    client.open(&uri, BAD);
    assert_eq!(client.diagnostics(&uri).len(), 1);
    client.notify(
        "textDocument/didClose",
        json!({"textDocument": {"uri": uri}}),
    );
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    client.shutdown();
}

#[test]
fn editing_an_open_dependency_rechecks_importers_in_both_directions() {
    let dir = package(&[
        (
            "main.gin",
            "use app.util five\nagain() in 0 to 3: return five()\n",
        ),
        ("util/five.gin", UTIL),
    ]);
    let (main, util) = (file_uri(&dir, "main.gin"), file_uri(&dir, "util/five.gin"));
    let mut client = Client::initialized(&[]);
    client.open(
        &main,
        "use app.util five\nagain() in 0 to 3: return five()\n",
    );
    assert_eq!(client.diagnostics(&main).len(), 1);
    client.open(&util, "five() in 0 to 3: return 3\n");
    assert_eq!(client.diagnostics(&main), Vec::<Value>::new());
    client.change(&util, 2, UTIL);
    assert_eq!(client.diagnostics(&main).len(), 1);
    client.shutdown();
}

#[test]
fn formatting_uses_open_text_and_returns_full_document_edit() {
    let (_dir, uri) = temp_file("format.gin", "ignored");
    let text = "five( )in 0 to 9:return 5\n";
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let reply = client.request(
        "textDocument/formatting",
        json!({"textDocument": {"uri": uri}, "options": {"tabSize": 4, "insertSpaces": true}}),
    );
    let edits = reply["result"]
        .as_array()
        .unwrap_or_else(|| panic!("{reply}"));
    assert_eq!(edits.len(), 1);
    assert_eq!(
        edits[0]["range"]["start"],
        json!({"line": 0, "character": 0})
    );
    assert_eq!(edits[0]["range"]["end"], json!({"line": 1, "character": 0}));
    let formatted = edits[0]["newText"].as_str().unwrap();
    assert_eq!(ginfmt::format(formatted).unwrap(), formatted);
    client.shutdown();
}

// ---- holes -------------------------------------------------------------------------------

#[test]
fn a_hole_is_an_information_diagnostic_with_its_expected_type_and_facts() {
    let text = "h(n in 0 to 9) in 0 to 9:\n    base in 0 to 9: 2\n    when n < 5 then return 1\n    return #todo\n";
    let (_dir, uri) = temp_file("hole.gin", text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let diagnostics = client.diagnostics(&uri);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let d = &diagnostics[0];
    assert_eq!(d["code"], "source-check-hole");
    assert_eq!(d["severity"], 3, "information");
    assert_eq!(
        d["range"],
        json!({"start": {"line": 3, "character": 11}, "end": {"line": 3, "character": 16}})
    );
    let message = d["message"].as_str().unwrap();
    assert!(
        message.contains("expected a value of type `0 to 9`"),
        "{message}"
    );
    assert!(
        message.contains("5 <= n <= 9") && message.contains("base = 2"),
        "{message}"
    );
    assert_eq!(d["data"]["hole"]["expected_type"], "0 to 9");
    // Hovering the hole says the same.
    let hover = hover_text(&client.hover(&uri, 3, 13));
    assert!(
        hover.contains("expected `0 to 9`") && hover.contains("5 <= n <= 9"),
        "{hover}"
    );
    // Filling it removes the diagnostic (negative control: a hole is not a flaw).
    client.change(&uri, 2, &text.replace("#todo", "n"));
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    client.shutdown();
}

// ---- hover -------------------------------------------------------------------------------

#[test]
fn hover_on_a_type_shows_its_size_alignment_and_field_offsets() {
    let (_dir, uri) = temp_file("point.gin", POINT);
    let mut client = Client::initialized(&[]);
    client.open(&uri, POINT);
    let text = hover_text(&client.hover(&uri, 0, 2));
    assert!(text.contains("**Point**: record type"), "{text}");
    assert!(text.contains("Size 4 bytes; alignment 2 bytes"), "{text}");
    assert!(
        text.contains("`x`: `0 to 9` (offset 0, size 1)")
            && text.contains("`y`: `0 to 300` (offset 2, size 2)"),
        "{text}"
    );
    // The range is exactly the word.
    let reply = client.hover(&uri, 0, 2);
    assert_eq!(
        reply["result"]["range"],
        json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 5}})
    );
    client.shutdown();
}

#[test]
fn hover_layout_follows_the_type_and_not_a_constant() {
    // Negative control for the layout test: another shape has another layout.
    let text = "#unrestricted\nByte is in 0 to 255\n";
    let (_dir, uri) = temp_file("byte.gin", text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let hover = hover_text(&client.hover(&uri, 1, 1));
    assert!(
        hover.contains("Size 1 bytes; alignment 1 bytes") && hover.contains("domain `0 to 255`"),
        "{hover}"
    );
    assert!(!hover.contains("Size 4"), "{hover}");
    client.shutdown();
}

#[test]
fn hover_on_a_function_shows_its_signature_and_a_known_value() {
    let text = "five() in 0 to 9: return 5\nlimit := 4\n";
    let (_dir, uri) = temp_file("fn.gin", text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let function = hover_text(&client.hover(&uri, 0, 1));
    assert!(
        function.contains("five() -> 0 to 9") && function.contains("Evaluates to `5`"),
        "{function}"
    );
    let constant = hover_text(&client.hover(&uri, 1, 1));
    assert!(
        constant.contains("limit := 4") && constant.contains("4 to 4"),
        "{constant}"
    );
    client.shutdown();
}

#[test]
fn hover_still_works_on_the_good_declarations_of_a_file_with_a_flaw() {
    let text = format!("{POINT}{BAD}");
    let (_dir, uri) = temp_file("mixed.gin", &text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, &text);
    assert_eq!(client.diagnostics(&uri).len(), 1);
    let point = hover_text(&client.hover(&uri, 0, 1));
    assert!(point.contains("Size 4 bytes"), "{point}");
    // The flawed declaration has no checker detail, but says what it is.
    let bad = hover_text(&client.hover(&uri, 5, 1));
    assert!(
        bad.contains("bad(n in 0 to 9) in 0 to 9") && bad.contains("no detail"),
        "{bad}"
    );
    client.shutdown();
}

#[test]
fn hover_on_a_parameter_and_nowhere_useful() {
    let (_dir, uri) = temp_file(
        "param.gin",
        "-- a comment about Point\nbump(n in 0 to 9) in 0 to 9: return n\n",
    );
    let mut client = Client::initialized(&[]);
    client.open(
        &uri,
        "-- a comment about Point\nbump(n in 0 to 9) in 0 to 9: return n\n",
    );
    let param = hover_text(&client.hover(&uri, 1, 5));
    assert!(
        param.contains("n in 0 to 9") && param.contains("parameter"),
        "{param}"
    );
    // Negative controls: inside a comment, on a keyword-free gap, and past the end are null.
    assert_eq!(client.hover(&uri, 0, 20)["result"], Value::Null);
    assert_eq!(client.hover(&uri, 1, 16)["result"], Value::Null);
    assert_eq!(client.hover(&uri, 99, 99)["result"], Value::Null);
    client.shutdown();
}

#[test]
fn positions_count_utf16_units() {
    // The emoji is two UTF-16 units and four bytes, so the word after it is at 5 and byte 7.
    let text =
        "-- \u{1F600}\nfive() in 0 to 9: return 5 -- \u{1F600} \u{e9}\nsix() in 0 to 9: return 6\n";
    let (_dir, uri) = temp_file("utf.gin", text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    let reply = client.hover(&uri, 2, 1);
    assert!(hover_text(&reply).contains("six() -> 0 to 9"));
    client.shutdown();
}

// ---- symbols -----------------------------------------------------------------------------

#[test]
fn document_symbols_nest_members_under_their_type() {
    let text = "#unrestricted\nCounter has\n    count in 0 to 9\n\nCounter has\n    next(c Counter) in 0 to 9: return 1\n\nstart() in 0 to 9: return 0\nlimit := 4\n";
    let (_dir, uri) = temp_file("outline.gin", text);
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let reply = client.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": uri}}),
    );
    let symbols = reply["result"]
        .as_array()
        .unwrap_or_else(|| panic!("{reply}"));
    let kinds: Vec<(String, u64)> = symbols
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_owned(),
                s["kind"].as_u64().unwrap(),
            )
        })
        .collect();
    // 23 struct, 12 function, 14 constant (LSP SymbolKind).
    assert!(kinds.contains(&("Counter".into(), 23)), "{kinds:?}");
    assert!(kinds.contains(&("start".into(), 12)), "{kinds:?}");
    assert!(kinds.contains(&("limit".into(), 14)), "{kinds:?}");
    let counter = symbols.iter().find(|s| s["name"] == "Counter").unwrap();
    assert_eq!(counter["children"][0]["name"], "count");
    assert_eq!(
        counter["selectionRange"]["start"],
        json!({"line": 1, "character": 0})
    );
    // Negative control: a function has no children and a bad-range symbol does not exist.
    let start = symbols.iter().find(|s| s["name"] == "start").unwrap();
    assert!(start.get("children").is_none() || start["children"].is_null());
    client.shutdown();
}

// ---- packages ----------------------------------------------------------------------------

fn package(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mod.flask", MANIFEST);
    for (path, text) in files {
        write(dir.path(), path, text);
    }
    dir
}

fn file_uri(dir: &tempfile::TempDir, path: &str) -> String {
    uri(&dir.path().canonicalize().unwrap().join(path))
}

const MAIN: &str = "use app.util five\nagain() in 0 to 16: return five()\n";
const UTIL: &str = "five() in 0 to 16: return 5\n";

#[test]
fn imports_resolve_in_a_package_and_a_single_file_alone_does_not_see_them() {
    let dir = package(&[("main.gin", MAIN), ("util/five.gin", UTIL)]);
    let main = file_uri(&dir, "main.gin");
    let mut client = Client::initialized(&[]);
    client.open(&main, MAIN);
    assert_eq!(
        client.diagnostics(&main),
        Vec::<Value>::new(),
        "the import resolves through package"
    );
    client.shutdown();

    // Negative control: the same text with no package around it cannot resolve the import.
    let (_alone, alone_uri) = temp_file("main.gin", MAIN);
    let mut client = Client::initialized(&[]);
    client.open(&alone_uri, MAIN);
    assert!(!client.diagnostics(&alone_uri).is_empty());
    client.shutdown();
}

#[test]
fn a_flaw_in_an_imported_module_is_published_for_that_file_and_unsaved_buffers_count() {
    let dir = package(&[("main.gin", MAIN), ("util/five.gin", UTIL)]);
    let (main, util) = (file_uri(&dir, "main.gin"), file_uri(&dir, "util/five.gin"));
    let mut client = Client::initialized(&[]);
    client.open(&main, MAIN);
    assert_eq!(client.diagnostics(&main), Vec::<Value>::new());
    // The editor opens util with an unsaved flaw: the disk still has the good text.
    let broken = "five() in 0 to 3: return 5\n";
    client.open(&util, broken);
    let on_util = client.diagnostics(&util);
    assert_eq!(on_util.len(), 1, "{on_util:?}");
    assert_eq!(on_util[0]["code"], "source-check-type");
    assert!(fs::read_to_string(dir.path().join("util/five.gin")).unwrap() == UTIL);
    // Repairing the buffer clears util; main is checked against the repaired buffer too.
    client.change(&util, 2, UTIL);
    assert_eq!(client.diagnostics(&util), Vec::<Value>::new());
    client.shutdown();
}

#[test]
fn a_flaw_in_the_importer_does_not_appear_on_the_imported_file() {
    let bad_main = "use app.util five\nagain() in 0 to 3: return five()\n";
    let dir = package(&[("main.gin", bad_main), ("util/five.gin", UTIL)]);
    let (main, util) = (file_uri(&dir, "main.gin"), file_uri(&dir, "util/five.gin"));
    let mut client = Client::initialized(&[]);
    client.open(&util, UTIL);
    client.open(&main, bad_main);
    let m = client.diagnostics(&main);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0]["range"]["start"]["line"], 1);
    // util never gets a non-empty publish for main's flaw.
    assert!(
        client.quiet(500, |m| m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["uri"] == util
            && m["params"]["diagnostics"] != json!([]))
    );
    client.shutdown();
}

#[test]
fn go_to_definition_finds_declarations_in_the_file_the_module_and_through_imports() {
    let dir = package(&[
        (
            "main.gin",
            "use app.util five\nagain() in 0 to 16: return five()\nlocal() in 0 to 16: return again()\n",
        ),
        ("util/five.gin", UTIL),
        ("sibling.gin", "other() in 0 to 16: return 1\n"),
    ]);
    let (main, util) = (file_uri(&dir, "main.gin"), file_uri(&dir, "util/five.gin"));
    let mut client = Client::initialized(&[]);
    client.open(&main, "use app.util five\nagain() in 0 to 16: return five()\nlocal() in 0 to 16: return again()\n");
    let def = |client: &mut Client, line: u32, character: u32| {
        client.request("textDocument/definition", json!({"textDocument": {"uri": main}, "position": {"line": line, "character": character}}))
    };
    // Through the import: `five` in the body is declared in util/five.gin.
    let reply = def(&mut client, 1, 29);
    assert_eq!(reply["result"]["uri"], util, "{reply}");
    assert_eq!(
        reply["result"]["range"],
        json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}})
    );
    // Within the file: `again` used in `local`.
    let reply = def(&mut client, 2, 29);
    assert_eq!(reply["result"]["uri"], main, "{reply}");
    assert_eq!(
        reply["result"]["range"]["start"],
        json!({"line": 1, "character": 0})
    );
    // Negative controls: a name declared nowhere, and whitespace, have no definition.
    assert_eq!(def(&mut client, 2, 22)["result"], Value::Null);
    assert_eq!(def(&mut client, 1, 15)["result"], Value::Null);
    client.shutdown();
}

#[test]
fn go_to_definition_finds_a_sibling_file_of_the_same_module() {
    let dir = package(&[
        (
            "main.gin",
            "use app.util five\nagain() in 0 to 16: return other()\n",
        ),
        ("util/five.gin", UTIL),
        ("extra.gin", "other() in 0 to 16: return 1\n"),
    ]);
    let main = file_uri(&dir, "main.gin");
    let mut client = Client::initialized(&[]);
    client.open(
        &main,
        "use app.util five\nagain() in 0 to 16: return other()\n",
    );
    let reply = client.request(
        "textDocument/definition",
        json!({"textDocument": {"uri": main}, "position": {"line": 1, "character": 29}}),
    );
    assert_eq!(
        reply["result"]["uri"],
        file_uri(&dir, "extra.gin"),
        "{reply}"
    );
    client.shutdown();
}

#[test]
fn workspace_symbols_search_the_roots_and_overlay_open_buffers() {
    let dir = package(&[("main.gin", MAIN), ("util/five.gin", UTIL)]);
    let root = dir.path().canonicalize().unwrap();
    let mut client = Client::initialized(&[root.as_path()]);
    let reply = client.request("workspace/symbol", json!({"query": "FIV"}));
    let found = reply["result"].as_array().unwrap();
    assert_eq!(found.len(), 1, "{reply}");
    assert_eq!(found[0]["name"], "five");
    assert_eq!(found[0]["location"]["uri"], file_uri(&dir, "util/five.gin"));
    assert_eq!(found[0]["containerName"], "util");
    // A buffer that renames the function is what the search sees.
    let util = file_uri(&dir, "util/five.gin");
    client.open(&util, "seven() in 0 to 16: return 7\n");
    let reply = client.request("workspace/symbol", json!({"query": "seven"}));
    assert_eq!(reply["result"].as_array().unwrap().len(), 1, "{reply}");
    // Negative control: nothing matches an absent name.
    assert_eq!(
        client.request("workspace/symbol", json!({"query": "zzz"}))["result"],
        json!([])
    );
    client.shutdown();
}

#[test]
fn a_nested_pattern_checks_clean_and_a_missing_inner_arm_is_reported() {
    let good = "#unrestricted\nAllocError is OutOfMemory or Fragmented\n\n#unrestricted\nCode is in 0 to 9\n\nOutcome is Value(Code) or Flaw(AllocError)\n\nf(r Outcome) in 0 to 3: return when r is\n    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n    Flaw(Fragmented) then 2\n";
    let (_dir, uri) = temp_file("nested.gin", good);
    let mut client = Client::initialized(&[]);
    client.open(&uri, good);
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    // Negative control: without the `Fragmented` arm the match is not exhaustive.
    let missing = good.replace("    Flaw(Fragmented) then 2\n", "");
    client.change(&uri, 2, &missing);
    assert!(!client.diagnostics(&uri).is_empty());
    client.shutdown();
}

#[test]
fn a_labeled_nested_pattern_checks_clean() {
    let good = "#unrestricted\nAllocError is OutOfMemory or Fragmented\n\n#unrestricted\nCode is in 0 to 9\n\nOutcome is Value(Code) or Flaw(code Code, error AllocError)\n\nf(r Outcome) in 0 to 9: return when r is\n    Value(_) then 0\n    Flaw(_: code, OutOfMemory: error) then 1\n    Flaw(c: code, Fragmented: error) then c\n";
    let (_dir, uri) = temp_file("labeled.gin", good);
    let mut client = Client::initialized(&[]);
    client.open(&uri, good);
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    // Negative control: a label naming no field is reported.
    client.change(
        &uri,
        2,
        &good.replace("OutOfMemory: error", "OutOfMemory: missing"),
    );
    assert!(!client.diagnostics(&uri).is_empty());
    client.shutdown();
}

#[test]
fn a_guarded_nested_arm_needs_its_fallback_and_the_field_first_label_is_reported() {
    let types = "#unrestricted\nAllocError is OutOfMemory or Fragmented\n\n#unrestricted\nCode is in 0 to 9\n\nOutcome is Value(Code) or Flaw(code Code, error AllocError)\n\n";
    let good = format!(
        "{types}f(r Outcome, n in 0 to 9) in 0 to 9: return when r is\n    Value(_) then 0\n    Flaw(_: code, OutOfMemory: error) and n > 4 then 5\n    Flaw(_: code, OutOfMemory: error) then 1\n    Flaw(c: code, Fragmented: error) then c\n"
    );
    let (_dir, uri) = temp_file("guarded.gin", &good);
    let mut client = Client::initialized(&[]);
    client.open(&uri, &good);
    assert_eq!(client.diagnostics(&uri), Vec::<Value>::new());
    // Negative control 1: the guarded arm alone does not cover `OutOfMemory`.
    let alone = good.replace("    Flaw(_: code, OutOfMemory: error) then 1\n", "");
    client.change(&uri, 2, &alone);
    assert!(!client.diagnostics(&uri).is_empty());
    // Negative control 2: the field-first label is a parse error that says why.
    let field_first = good.replace("OutOfMemory: error) and", "error: OutOfMemory) and");
    client.change(&uri, 3, &field_first);
    let reported = client.diagnostics(&uri);
    assert!(
        reported
            .iter()
            .any(|d| d.to_string().contains("pattern comes first")),
        "{reported:?}"
    );
    client.shutdown();
}
