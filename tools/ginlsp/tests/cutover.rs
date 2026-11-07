mod common;

use common::Client;
use serde_json::{Value, json};

#[test]
fn path_dependency_buffers_recheck_importers_and_close_restores_disk_state() {
    let workspace = tempfile::tempdir().unwrap();
    let app = workspace.path().join("app");
    let library = workspace.path().join("library");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        app.join("mod.flask"),
        "#settings\nmodule: 'app'\nversion: 1.0.0\n#dependencies\nlibrary: '../library'\n",
    )
    .unwrap();
    std::fs::write(
        library.join("mod.flask"),
        "#settings\nmodule: 'library'\nversion: 1.0.0\n",
    )
    .unwrap();
    let importing = "use library five\nagain() in 0 to 3: return five()\n";
    let on_disk = "five() in 0 to 9: return 5\n";
    std::fs::write(app.join("main.gin"), importing).unwrap();
    std::fs::write(library.join("main.gin"), on_disk).unwrap();
    let main = common::uri(&app.join("main.gin"));
    let dependency = common::uri(&library.join("main.gin"));
    let mut client = Client::initialized(&[]);
    client.open(&main, importing);
    assert_eq!(client.diagnostics(&main).len(), 1);
    client.open(&dependency, "five() in 0 to 3: return 3\n");
    assert!(client.diagnostics(&main).is_empty());
    client.change(&dependency, 2, on_disk);
    assert_eq!(client.diagnostics(&main).len(), 1);
    client.change(&dependency, 3, "five() in 0 to 3: return 3\n");
    assert!(client.diagnostics(&main).is_empty());
    client.notify(
        "textDocument/didClose",
        json!({"textDocument": {"uri": dependency}}),
    );
    assert_eq!(client.diagnostics(&main).len(), 1);
    client.shutdown();
}

#[test]
fn formatting_preserves_utf16_ranges_and_refuses_malformed_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("format.gin");
    std::fs::write(&path, "on disk").unwrap();
    let uri = common::uri(&path);
    let text = "value:1 -- 🦀";
    let mut client = Client::initialized(&[]);
    client.open(&uri, text);
    let params = json!({
        "textDocument": {"uri": uri},
        "options": {"tabSize": 4, "insertSpaces": true}
    });
    let reply = client.request("textDocument/formatting", params.clone());
    let edit = &reply["result"][0];
    assert_eq!(
        edit["range"]["end"],
        json!({"line": 0, "character": text.encode_utf16().count()})
    );
    let formatted = edit["newText"].as_str().expect("a formatting edit");
    client.change(&uri, 2, formatted);
    let unchanged = client.request("textDocument/formatting", params.clone());
    assert_eq!(unchanged["result"], json!([]));
    client.change(&uri, 3, "value: (");
    let failed = client.request("textDocument/formatting", params);
    assert_eq!(failed["error"]["code"], ginlsp::server::REQUEST_FAILED);
    assert!(failed.get("result").is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "on disk");
    client.shutdown();
}

#[test]
fn non_gin_documents_are_not_checked_as_gin_source() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::initialized(&[]);
    for (name, language) in [("mod.flask", "gin-flask"), ("settings.json", "json")] {
        let uri = common::uri(&directory.path().join(name));
        client.notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": language, "version": 1, "text": "{\"key\": 1}"
            }}),
        );
        let symbols = client.request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": uri}}),
        );
        assert_eq!(symbols["result"], Value::Null);
        assert!(client.quiet(100, |message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
        }));
    }
    client.shutdown();
}
