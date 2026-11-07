//! Saving the image to a file and restoring it. A snapshot holds the declarations and the
//! derived state (inspected layouts and values) with the closure hash each was computed under.
//! Restoring re-checks the declarations and keeps a piece of derived state only if the closure
//! hash it was saved with is the one the definition has now; a snapshot that cannot be read,
//! fails its checksum, or no longer checks is reported with a typed notice and changes nothing.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::check::{SESSION_FILE, check_text};
use crate::image::{Image, fnv};
use crate::session::Session;

pub const FORMAT: &str = "ginterpreter/snapshot/1";

fn hex(n: u64) -> String {
    format!("{n:016x}")
}

fn notice(kind: &str, detail: impl Into<String>) -> Value {
    json!({"kind": kind, "detail": detail.into()})
}

impl Session {
    /// Write the image to `path` (through a temporary file, so a failure never leaves half of
    /// one). Returns what was written.
    pub fn save(&mut self, path: &Path) -> Result<Value, Value> {
        let (image, checked) = {
            self.ensure_current_pub();
            let checked = self.current.take().expect("current");
            (Image::build(&checked), checked)
        };
        let target = typecheck::Limits::default().target;
        let cache: Vec<Value> = {
            let mut entries: Vec<(&String, &(u64, Value))> = self.cache.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            entries.into_iter().map(|(name, (closure, value))| json!({"name": name, "closure": hex(*closure), "value": value})).collect()
        };
        let body = json!({
            "revision": self.revision,
            "target": {"triple": target.triple, "register_bits": target.register_bits},
            "declarations": self.decls.iter().map(|(key, text)| json!({"key": key, "text": text})).collect::<Vec<_>>(),
            "definitions": image.defs.iter().enumerate().map(|(i, d)| json!({"key": d.key, "closure": hex(image.closure[i])})).collect::<Vec<_>>(),
            "cache": cache,
        });
        self.current = Some(checked);
        let checksum = hex(fnv(body.to_string().as_bytes()));
        let file = json!({"format": FORMAT, "checksum": checksum, "body": body});
        let text = serde_json::to_string_pretty(&file).map_err(|e| notice("snapshot_unwritable", e.to_string()))?;
        let temporary = path.with_extension("tmp-snapshot");
        std::fs::write(&temporary, &text).map_err(|e| notice("snapshot_unwritable", format!("{}: {e}", temporary.display())))?;
        std::fs::rename(&temporary, path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            notice("snapshot_unwritable", format!("{}: {e}", path.display()))
        })?;
        Ok(json!({"path": path.display().to_string(), "declarations": self.decls.len(), "cached_entries": self.cache.len(), "checksum": checksum, "revision": self.revision}))
    }

    /// Restore from `path`. Never panics and never leaves the session half changed: it either
    /// loads declarations that check, or keeps the session as it was and says why in `notices`.
    ///
    /// With `verify`, every kept entry is re-derived and compared with the snapshot's value: a
    /// mismatch (a hand-edited value whose checksum was fixed up) is reported as
    /// `snapshot_mismatch` and the recomputed value is the one kept.
    pub fn restore(&mut self, path: &Path, verify: bool) -> Value {
        let refused = |notices: Vec<Value>| json!({"restored": false, "notices": notices});
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => return refused(vec![notice("snapshot_unreadable", format!("{}: {e}", path.display()))]),
        };
        let file: Value = match serde_json::from_str(&text) {
            Ok(file) => file,
            Err(e) => return refused(vec![notice("snapshot_corrupt", format!("not JSON: {e}"))]),
        };
        if file["format"] != FORMAT {
            return refused(vec![notice("snapshot_version", format!("format {} is not {FORMAT}", file["format"]))]);
        }
        let body = &file["body"];
        if file["checksum"].as_str() != Some(hex(fnv(body.to_string().as_bytes())).as_str()) {
            return refused(vec![notice("snapshot_corrupt", "the checksum does not match the contents")]);
        }
        let Some(declarations) = body["declarations"].as_array().and_then(|list| {
            list.iter().map(|d| Some((d["key"].as_str()?.to_owned(), d["text"].as_str()?.to_owned()))).collect::<Option<Vec<_>>>()
        }) else {
            return refused(vec![notice("snapshot_corrupt", "the declarations are missing or malformed")]);
        };
        // Re-check what was saved: it is only trusted as far as it checks now.
        let joined: String = declarations.iter().map(|(_, t)| format!("{}\n\n", t.trim_end())).collect();
        let checked = check_text(SESSION_FILE, &joined);
        if !checked.flaws.is_empty() {
            let mut n = notice("snapshot_does_not_check", format!("{} flaw(s) when re-checked; the session is unchanged", checked.flaws.len()));
            n["flaws"] = json!(checked.flaws.iter().map(|f| f.json()).collect::<Vec<_>>());
            return refused(vec![n]);
        }
        let image = Image::build(&checked);
        let mut notices = Vec::new();
        let target = typecheck::Limits::default().target;
        let same_target = body["target"]["triple"] == target.triple && body["target"]["register_bits"] == target.register_bits;
        if !same_target {
            notices.push(notice("snapshot_target_changed", "saved for another target; derived state is dropped and recomputed"));
        }
        let mut cache = std::collections::HashMap::new();
        let (mut kept, mut dropped, mut mismatched) = (Vec::new(), Vec::new(), Vec::new());
        for entry in body["cache"].as_array().into_iter().flatten() {
            let (Some(name), Some(saved)) = (entry["name"].as_str(), entry["closure"].as_str()) else {
                notices.push(notice("snapshot_entry_ignored", "a cache entry is malformed"));
                continue;
            };
            let head = name.split('(').next().unwrap_or(name).trim();
            let now = hex(image.closure_of_name(head));
            if same_target && now == saved && !image.find(head).is_empty() {
                let mut value = entry["value"].clone();
                if verify {
                    let fresh = crate::session::derive(&checked, name);
                    if fresh != value {
                        mismatched.push(name.to_owned());
                        value = fresh;
                    }
                }
                cache.insert(name.to_owned(), (image.closure_of_name(head), value));
                if mismatched.last().map(String::as_str) == Some(name) {
                    dropped.push(name.to_owned());
                } else {
                    kept.push(name.to_owned());
                }
            } else {
                dropped.push(name.to_owned());
            }
        }
        let stale: Vec<&String> = dropped.iter().filter(|name| !mismatched.contains(name)).collect();
        if !stale.is_empty() {
            notices.push(notice("snapshot_stale", format!("{} entr{} no longer match their definitions and will be recomputed", stale.len(), if stale.len() == 1 { "y" } else { "ies" })));
        }
        for name in &mismatched {
            notices.push(notice("snapshot_mismatch", format!("the saved value of `{name}` differs from what its definition derives now; the recomputed value is used")));
        }
        // Definitions whose closure hash differs from the saved one, for the report.
        let saved: BTreeMap<&str, &str> = body["definitions"].as_array().into_iter().flatten().filter_map(|d| Some((d["key"].as_str()?, d["closure"].as_str()?))).collect();
        let changed: Vec<String> = image
            .defs
            .iter()
            .enumerate()
            .filter(|(i, d)| saved.get(d.key.as_str()).is_none_or(|s| *s != hex(image.closure[*i])))
            .map(|(_, d)| d.key.clone())
            .collect();
        self.decls = declarations;
        self.cache = cache;
        self.revision = body["revision"].as_u64().unwrap_or(0) + 1;
        self.last_flaws.clear();
        let count = self.decls.len();
        self.current = Some(checked);
        json!({"restored": true, "declarations": count, "kept": kept, "dropped": dropped, "changed": changed,
               "mismatched": mismatched, "verified": verify, "notices": notices, "revision": self.revision})
    }
}
