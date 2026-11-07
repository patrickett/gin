//! Saving and restoring the image, and what happens to a snapshot that is stale or damaged.
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use ginterpreter::image::fnv;
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
fn save(path: &Path) -> Value {
    json!({"id":"s","op":"$save","path":path.to_str().unwrap()})
}
fn restore(path: &Path) -> Value {
    json!({"id":"r","op":"$restore","path":path.to_str().unwrap()})
}

const BASE: &str = "base() in 0 to 9: return 4\n";
const TOP: &str = "top(n in 0 to 9) in 0 to 9: return n\nvalue() in 0 to 9: return base()\n";

/// Write a snapshot of a small session and return its path.
fn snapshot(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("image.json");
    let r = session(&[declare(BASE), declare(TOP), inspect("value"), inspect("base"), save(&path)]);
    assert_eq!(r[4]["data"]["declarations"], 3, "{}", r[4]);
    assert_eq!(r[4]["data"]["cached_entries"], 2);
    path
}

/// Rewrite the body of a snapshot and make its checksum agree, as a valid but different file.
fn edit(path: &Path, change: impl FnOnce(&mut Value)) {
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    change(&mut file["body"]);
    file["checksum"] = json!(format!("{:016x}", fnv(file["body"].to_string().as_bytes())));
    std::fs::write(path, serde_json::to_string_pretty(&file).unwrap()).unwrap();
}

#[test]
fn a_restored_image_keeps_the_derived_state_whose_closure_still_matches() {
    let directory = tempfile::tempdir().unwrap();
    let path = snapshot(directory.path());
    let r = session(&[restore(&path), inspect("value"), inspect("base"), json!({"id":"m","op":"$scope"})]);
    let data = &r[0]["data"];
    assert_eq!(data["restored"], true);
    assert_eq!(data["declarations"], 3);
    assert_eq!(data["notices"], json!([]));
    let mut kept: Vec<&str> = data["kept"].as_array().unwrap().iter().map(|k| k.as_str().unwrap()).collect();
    kept.sort();
    assert_eq!(kept, ["base", "value"]);
    assert_eq!(data["dropped"], json!([]));
    assert_eq!(data["changed"], json!([]));
    // Nothing is recomputed: the values come from the snapshot.
    assert_eq!(r[1]["data"]["cached"], true);
    assert_eq!(r[1]["data"]["value"]["value"]["value"], 4);
    assert_eq!(r[2]["data"]["cached"], true);
    assert_eq!(r[3]["data"]["entries"].as_array().unwrap().len(), 3);
}

#[test]
fn a_stale_snapshot_drops_what_changed_and_recomputes_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = snapshot(directory.path());
    // Someone edited `base` in the file: valid, but the saved closure hashes no longer fit.
    edit(&path, |body| {
        let declarations = body["declarations"].as_array_mut().unwrap();
        let base = declarations.iter_mut().find(|d| d["key"] == "fn base/0").unwrap();
        base["text"] = json!("base() in 0 to 9: return 5");
    });
    let r = session(&[restore(&path), inspect("value"), inspect("base")]);
    let data = &r[0]["data"];
    assert_eq!(data["restored"], true);
    let mut dropped: Vec<&str> = data["dropped"].as_array().unwrap().iter().map(|k| k.as_str().unwrap()).collect();
    dropped.sort();
    assert_eq!(dropped, ["base", "value"]);
    assert_eq!(data["notices"][0]["kind"], "snapshot_stale");
    assert!(data["changed"].as_array().unwrap().contains(&json!("fn base/0")));
    // Recomputed from the declarations, not read from the snapshot.
    assert_eq!(r[1]["data"]["cached"], false);
    assert_eq!(r[1]["data"]["value"]["value"]["value"], 5);
}

#[test]
fn only_the_entries_that_no_longer_match_are_dropped() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.json");
    let r = session(&[declare(BASE), declare("other() in 0 to 9: return 1\n"), inspect("base"), inspect("other"), save(&path)]);
    assert_eq!(r[4]["data"]["cached_entries"], 2);
    edit(&path, |body| {
        let declarations = body["declarations"].as_array_mut().unwrap();
        declarations.iter_mut().find(|d| d["key"] == "fn other/0").unwrap()["text"] = json!("other() in 0 to 9: return 2");
    });
    let r = session(&[restore(&path), inspect("base"), inspect("other")]);
    assert_eq!(r[0]["data"]["kept"], json!(["base"]));
    assert_eq!(r[0]["data"]["dropped"], json!(["other"]));
    assert_eq!(r[1]["data"]["cached"], true);
    assert_eq!(r[2]["data"]["cached"], false);
    assert_eq!(r[2]["data"]["value"]["value"]["value"], 2);
}

#[test]
fn a_damaged_snapshot_changes_nothing_and_says_why() {
    let directory = tempfile::tempdir().unwrap();
    let good = snapshot(directory.path());
    let text = std::fs::read_to_string(&good).unwrap();
    let cases: Vec<(&str, String, &str)> = vec![
        ("truncated", text[..text.len() / 2].to_owned(), "snapshot_corrupt"),
        ("garbage", "\u{0}\u{1}not a snapshot".to_owned(), "snapshot_corrupt"),
        ("empty", String::new(), "snapshot_corrupt"),
        ("wrong format", text.replace("ginterpreter/snapshot/1", "ginterpreter/snapshot/9"), "snapshot_version"),
        // Edited by hand without fixing the checksum.
        ("tampered", text.replace("return 4", "return 7"), "snapshot_corrupt"),
        ("missing declarations", {
            let mut file: Value = serde_json::from_str(&text).unwrap();
            file["body"]["declarations"] = json!("nope");
            file["checksum"] = json!(format!("{:016x}", fnv(file["body"].to_string().as_bytes())));
            file.to_string()
        }, "snapshot_corrupt"),
    ];
    for (what, contents, kind) in cases {
        let bad = directory.path().join("bad.json");
        std::fs::write(&bad, contents).unwrap();
        // A session that already holds something: it must be exactly as it was.
        let r = session(&[declare("mine() in 0 to 9: return 8\n"), restore(&bad), json!({"id":"m","op":"$scope"}), inspect("mine")]);
        assert_eq!(r[1]["data"]["restored"], false, "{what}");
        assert_eq!(r[1]["data"]["notices"][0]["kind"], kind, "{what}: {}", r[1]);
        assert_eq!(r[2]["data"]["entries"].as_array().unwrap().len(), 1, "{what}");
        assert_eq!(r[3]["data"]["value"]["value"]["value"], 8, "{what}");
    }
}

#[test]
fn a_missing_snapshot_and_an_unwritable_path_are_notices() {
    let r = session(&[restore(Path::new("/definitely/not/here.json")), save(Path::new("/definitely/not/here/image.json"))]);
    assert_eq!(r[0]["data"]["restored"], false);
    assert_eq!(r[0]["data"]["notices"][0]["kind"], "snapshot_unreadable");
    assert_eq!(r[1]["data"]["saved"], false);
    assert_eq!(r[1]["data"]["notices"][0]["kind"], "snapshot_unwritable");
}

#[test]
fn declarations_that_no_longer_check_are_not_loaded() {
    let directory = tempfile::tempdir().unwrap();
    let path = snapshot(directory.path());
    edit(&path, |body| {
        body["declarations"].as_array_mut().unwrap()[0]["text"] = json!("base() in 0 to 3: return 4");
    });
    let r = session(&[declare("mine() in 0 to 9: return 8\n"), restore(&path), inspect("mine")]);
    assert_eq!(r[1]["data"]["restored"], false);
    let notice = &r[1]["data"]["notices"][0];
    assert_eq!(notice["kind"], "snapshot_does_not_check");
    assert_eq!(notice["flaws"][0]["severity"], "flaw");
    // Negative control: the session is as it was.
    assert_eq!(r[2]["data"]["value"]["value"]["value"], 8);
}

#[test]
fn a_snapshot_for_another_target_drops_all_derived_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = snapshot(directory.path());
    edit(&path, |body| body["target"]["triple"] = json!("riscv64-unknown-elf"));
    let r = session(&[restore(&path), inspect("value")]);
    assert_eq!(r[0]["data"]["restored"], true);
    assert_eq!(r[0]["data"]["kept"], json!([]));
    assert_eq!(r[0]["data"]["notices"][0]["kind"], "snapshot_target_changed");
    assert_eq!(r[1]["data"]["cached"], false);
}

#[test]
fn the_snapshot_file_is_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let (a, b) = (directory.path().join("a.json"), directory.path().join("b.json"));
    for path in [&a, &b] {
        session(&[declare(BASE), declare(TOP), inspect("value"), save(path)]);
    }
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
}

#[test]
fn the_repl_saves_and_restores() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("repl.json");
    let run = |input: String| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    };
    let out = run(format!("five() in 0 to 9: return 5\n$save {}\n", path.display()));
    assert!(out.contains("saved 1 declaration(s)"), "{out}");
    let out = run(format!("$restore {}\nfive() + 1\n$restore /nope.json\n", path.display()));
    assert!(out.contains("restored 1 declaration(s)"), "{out}");
    assert!(out.contains("6 : 6 to 6"), "{out}");
    assert!(out.contains("snapshot_unreadable"), "{out}");
}

fn verify(path: &Path) -> Value {
    json!({"id":"r","op":"$restore","path":path.to_str().unwrap(),"verify":true})
}

/// A snapshot whose cached value was edited by hand and whose checksum was fixed up.
fn forged(directory: &Path) -> std::path::PathBuf {
    let path = snapshot(directory);
    edit(&path, |body| {
        let entry = body["cache"].as_array_mut().unwrap().iter_mut().find(|e| e["name"] == "value").unwrap();
        entry["value"]["value"]["value"]["value"] = json!(99);
        entry["value"]["value"]["display"] = json!("99");
    });
    path
}

#[test]
fn a_forged_cached_value_is_trusted_without_verify_and_caught_with_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = forged(directory.path());
    // The default is hash-only: the closure hash matches, so the edited value is believed.
    let trusting = session(&[restore(&path), inspect("value")]);
    assert_eq!(trusting[0]["data"]["verified"], false);
    assert_eq!(trusting[0]["data"]["mismatched"], json!([]));
    assert_eq!(trusting[1]["data"]["value"]["value"]["value"], 99);
    // Under verify every kept entry is re-derived and compared.
    let r = session(&[verify(&path), inspect("value"), inspect("base")]);
    let data = &r[0]["data"];
    assert_eq!(data["verified"], true);
    assert_eq!(data["mismatched"], json!(["value"]));
    assert_eq!(data["dropped"], json!(["value"]));
    assert_eq!(data["kept"], json!(["base"]));
    let kinds: Vec<&str> = data["notices"].as_array().unwrap().iter().map(|n| n["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["snapshot_mismatch"]);
    // The recomputed value wins, and the untouched entry is still kept.
    assert_eq!(r[1]["data"]["value"]["value"]["value"], 4);
    assert_eq!(r[2]["data"]["cached"], true);
}

#[test]
fn a_clean_snapshot_verifies_with_nothing_dropped() {
    let directory = tempfile::tempdir().unwrap();
    let path = snapshot(directory.path());
    let r = session(&[verify(&path), inspect("value"), inspect("base")]);
    let data = &r[0]["data"];
    assert_eq!(data["restored"], true);
    assert_eq!(data["verified"], true);
    assert_eq!(data["mismatched"], json!([]));
    assert_eq!(data["dropped"], json!([]));
    assert_eq!(data["notices"], json!([]));
    assert_eq!(data["kept"].as_array().unwrap().len(), 2);
    assert_eq!(r[1]["data"]["cached"], true);
    assert_eq!(r[2]["data"]["cached"], true);
}

#[test]
fn the_repl_verifies_on_request() {
    let directory = tempfile::tempdir().unwrap();
    let path = forged(directory.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(format!("$restore {} --verify\n", path.display()).as_bytes()).unwrap();
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    assert!(out.contains("snapshot_mismatch"), "{out}");
}
