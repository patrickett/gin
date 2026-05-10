use crate::diagnostics::span_to_range;
use crate::Backend;
use ast::{HasSpanId, ImportSource, LocalBundleImport};

use diagnostic::SpanId;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typeck::{
    find_definition_span, find_import_definition_span, is_identifier_char, position_to_byte_offset,
};

fn zero_location(uri: Url) -> Location {
    Location {
        uri,
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        },
    }
}

/// Resolve a `use` import to a file location (folder module → `flask.jsonc`, file → `.gin`).
///
/// This function is intentionally side-effect free (filesystem reads only) so it can be unit tested.
fn resolve_use_import_source_fs(base_uri: &Url, source: &ImportSource) -> Option<Location> {
    match source {
        ImportSource::Local(path, _) => {
            let base_path = base_uri.to_file_path().ok()?;
            let base_dir = base_path.parent()?;

            let resolved = base_dir.join(path);
            if resolved.is_dir() {
                let flask_jsonc_path = resolved.join(flask::PACKAGE_CONFIG_NAME);
                if flask_jsonc_path.is_file() {
                    return Some(zero_location(Url::from_file_path(&flask_jsonc_path).ok()?));
                }
            }

            let mut file_res = resolved.clone();
            if file_res.extension().is_none() {
                let with_ext = file_res.with_extension("gin");
                if with_ext.is_file() {
                    file_res = with_ext;
                }
            }

            if !file_res.is_file() {
                return None;
            }
            Some(zero_location(Url::from_file_path(&file_res).ok()?))
        }
        ImportSource::LocalBundle(b) => {
            let base_path = base_uri.to_file_path().ok()?;
            let base_dir = base_path.parent()?;

            let handle = flask::FlaskConfigHandle::load(base_dir).ok()?;
            let cfg = handle.read();
            let config_dir = handle.source_dir();

            let dep = cfg.config.dependencies().get(b.root.as_str())?;
            let dep_dir = match &dep.kind {
                flask::DependencyKind::Path { path } => config_dir.join(path),
                _ => return None,
            };

            let flask_path = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
            if !flask_path.is_file() {
                return None;
            }
            Some(zero_location(Url::from_file_path(&flask_path).ok()?))
        }
        ImportSource::Package(mod_path) => {
            let base_path = base_uri.to_file_path().ok()?;
            let base_dir = base_path.parent()?;

            let handle = flask::FlaskConfigHandle::load(base_dir).ok()?;
            let cfg = handle.read();
            let config_dir = handle.source_dir();

            let dep = cfg.config.dependencies().get(mod_path.root.as_str())?;
            let dep_dir = match &dep.kind {
                flask::DependencyKind::Path { path } => config_dir.join(path),
                _ => return None,
            };

            if mod_path.segments.is_empty() {
                let dep_cfg = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
                if !dep_cfg.exists() {
                    return None;
                }
                return Some(zero_location(Url::from_file_path(&dep_cfg).ok()?));
            }

            resolve_nested_package_manifest_location(&dep_dir, &mod_path.segments)
        }
    }
}

fn resolve_package_part_location_fs(
    base_uri: &Url,
    root: &internment::Intern<String>,
    segments: &[internment::Intern<String>],
    selected_part: usize,
) -> Option<Location> {
    let base_path = base_uri.to_file_path().ok()?;
    let base_dir = base_path.parent()?;

    let handle = flask::FlaskConfigHandle::load(base_dir).ok()?;
    let cfg = handle.read();
    let config_dir = handle.source_dir();

    let dep = cfg.config.dependencies().get(root.as_str())?;
    let dep_dir = match &dep.kind {
        flask::DependencyKind::Path { path } => config_dir.join(path),
        _ => return None,
    };

    if selected_part == 0 {
        let dep_cfg = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
        if dep_cfg.exists() {
            return Some(zero_location(Url::from_file_path(&dep_cfg).ok()?));
        }
        return None;
    }

    let seg_count = segments.len();
    let seg_idx = selected_part.saturating_sub(1);
    if seg_idx >= seg_count {
        return None;
    }

    if let Some(loc) = resolve_nested_package_manifest_location(&dep_dir, &segments[..=seg_idx]) {
        return Some(loc);
    }

    // When the last segment is not a nested sub-package, check if it's a
    // public definition (symbol or tag) in the dependency package and
    // navigate to the exact definition location (line, column).
    if seg_idx == seg_count - 1 {
        let symbol = segments[seg_idx].as_str();
        if let Some((loc_uri, loc_range)) = resolve_symbol_location(&dep_dir, symbol) {
            return Some(Location {
                uri: loc_uri,
                range: loc_range,
            });
        }
    }

    None
}

fn part_index_in_dotted_path(span_text: &str, byte_in_span: usize) -> Option<usize> {
    Some(resolve::part_index_in_dotted_path(span_text, byte_in_span))
}

fn is_import_identifier_at(source: &str, byte_pos: usize) -> bool {
    source
        .get(byte_pos..)
        .and_then(|tail| tail.chars().next())
        .map(is_identifier_char)
        .unwrap_or(false)
}

/// `flask.jsonc` for the folder module at `start_dir/seg1/.../segN/`.
fn resolve_nested_package_manifest_location(
    start_dir: &std::path::Path,
    segments: &[internment::Intern<String>],
) -> Option<Location> {
    let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    match flask::resolve_nested_package_path(start_dir, &segs).ok()? {
        flask::NestedPackageTarget::FolderModule(dir) => {
            let cfg = dir.join(flask::PACKAGE_CONFIG_NAME);
            if !cfg.exists() {
                return None;
            }
            Some(zero_location(Url::from_file_path(&cfg).ok()?))
        }
    }
}

impl Backend {
    pub(crate) async fn handle_goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let position = params.text_document_position_params.position;

        let (source, file_path) = match self.documents.get(&uri.to_string()) {
            Some(state) => (state.source.clone(), state.file_path.clone()),
            None => return Ok(None),
        };

        // parse runs the parser; offload so a stuck parser does not pin the async runtime.

        let locations = self
            .run_blocking_request("goto_definition", move |this| {
                let snapshot = this.snapshot();
                let ast = snapshot.engine.parse_output(&file_path)?.ast.clone();

                if let Some(byte_pos) =
                    position_to_byte_offset(&source, position.line, position.character)
                {
                    // Phase 1: cursor is directly inside a `use` statement.
                    if let Some(link) = this.resolve_use_import_at(&uri, &ast, &source, byte_pos) {
                        return Some(GotoDefinitionResponse::Link(vec![link]));
                    }

                    // Phase 2: cursor is on an imported symbol used unqualified
                    // in the body. `use core.true` is syntactic sugar for
                    // `use core.true as true`, making the bare name available
                    // throughout the file.
                    if let Some(word) = ast
                        .word_at_byte(byte_pos, &source)
                        .or_else(|| typeck::word_at_byte_offset(&source, byte_pos))
                    {
                        if let Some(link) = this.resolve_body_import_at(&uri, &ast, &source, &word)
                        {
                            return Some(GotoDefinitionResponse::Link(vec![link]));
                        }
                    }

                    // Phase 3: cursor is on a definition or import reference.
                    if let Some(word) = ast
                        .word_at_byte(byte_pos, &source)
                        .or_else(|| typeck::word_at_byte_offset(&source, byte_pos))
                    {
                        let range = find_definition_span(&ast, &word)
                            .map(|span| span_to_range(span.start, span.end, &source))
                            .unwrap_or_default();
                        if range != Range::default() {
                            return Some(GotoDefinitionResponse::Scalar(Location { uri, range }));
                        }
                        if let Some(span) = find_import_definition_span(&ast, &word, &source) {
                            let range = span_to_range(span.start, span.end, &source);
                            return Some(GotoDefinitionResponse::Scalar(Location { uri, range }));
                        }
                    }
                }

                None
            })
            .await;

        Ok(locations.flatten())
    }
}

impl Backend {
    fn resolve_use_import_at(
        &self,
        base_uri: &Url,
        ast: &ast::FileAst,
        source: &str,
        byte_pos: usize,
    ) -> Option<LocationLink> {
        let span_table = ast.span_table();

        for import in ast.uses() {
            for module_import in &import.0 {
                let (import_span, source_ref) = match &module_import.source {
                    ImportSource::Local(path, span_id) => {
                        let _span = span_table.get(*span_id);
                        (*span_id, ImportSource::Local(path.clone(), *span_id))
                    }
                    ImportSource::LocalBundle(b) => {
                        // Check per-member spans.  If the cursor is on a specific
                        // bundle member we can resolve it individually.
                        let mut found: Option<(SpanId, ImportSource)> = None;
                        for member in &b.members {
                            let mspan = span_table.get(member.span);
                            if byte_pos >= mspan.start && byte_pos <= mspan.end {
                                // Build a synthetic Local source that points to the
                                // dependency's module root so the resolution opens
                                // the dependency's flask.jsonc (or the file with the
                                // definition for symbol imports).
                                let lb = LocalBundleImport {
                                    root: b.root,
                                    members: vec![member.clone()],
                                    span: member.span,
                                };
                                found = Some((member.span, ImportSource::LocalBundle(lb)));
                                break;
                            }
                        }
                        found.unwrap_or_else(|| (b.span_id(), ImportSource::LocalBundle(b.clone())))
                    }
                    ImportSource::Package(mod_path) => {
                        let _span = span_table.get(mod_path.span_id());
                        (mod_path.span_id(), ImportSource::Package(mod_path.clone()))
                    }
                };

                let span = span_table.get(import_span);
                if byte_pos < span.start || byte_pos > span.end {
                    continue;
                }

                if !is_import_identifier_at(source, byte_pos) {
                    continue;
                }

                let origin_range = span_to_range(span.start, span.end, source);

                let target_location = match &source_ref {
                    ImportSource::Package(mp) => {
                        let span_text = source.get(span.start..span.end).unwrap_or("");
                        let byte_in_span = byte_pos.saturating_sub(span.start);
                        let part = part_index_in_dotted_path(span_text, byte_in_span).unwrap_or(0);
                        resolve_package_part_location_fs(base_uri, &mp.root, &mp.segments, part)
                    }
                    _ => resolve_use_import_source_fs(base_uri, &source_ref),
                };

                if let Some(target_location) = target_location {
                    return Some(LocationLink {
                        origin_selection_range: Some(origin_range),
                        target_uri: target_location.uri,
                        target_range: target_location.range,
                        target_selection_range: target_location.range,
                    });
                }
            }
        }

        None
    }

    /// When a bare word in the body matches an import's effective name (e.g.
    /// `true` from `use core.true`), resolve it to the definition file.
    /// This is Phase 2, used when the cursor is *outside* the `use` span.
    fn resolve_body_import_at(
        &self,
        base_uri: &Url,
        ast: &ast::FileAst,
        source: &str,
        word: &str,
    ) -> Option<LocationLink> {
        // Use the shared import resolution for Phase 2 (body word matching).
        // We need a byte position; scan for the word.
        let byte_pos = source.find(word)?;
        match resolve::resolve_import_at(ast, source, byte_pos) {
            Some(resolve::ImportTarget::BodySymbol { dep_name, symbol }) => {
                let dep_dir = resolve_dep_dir_from_uri(base_uri, &dep_name)?;
                let (target_uri, target_range) = resolve_symbol_location(&dep_dir, &symbol)?;
                Some(LocationLink {
                    origin_selection_range: None,
                    target_uri,
                    target_range,
                    target_selection_range: target_range,
                })
            }
            _ => None,
        }
    }
}

/// Resolve the dependency directory for `dep_name` from the `flask.jsonc`
/// found relative to the file at `uri`.
fn resolve_dep_dir_from_uri(uri: &Url, dep_name: &str) -> Option<std::path::PathBuf> {
    let base_path = uri.to_file_path().ok()?;
    resolve::resolve_dep_dir(&base_path, dep_name)
}

/// Given a dependency directory and a symbol name, parse the definition file
/// and return the exact `(Url, Range)` of the symbol's definition, so goto-def
/// navigates to the correct line and column instead of the top of the file.
fn resolve_symbol_location(dep_dir: &std::path::Path, symbol: &str) -> Option<(Url, Range)> {
    let def_file = resolve::find_public_def_in_package(dep_dir, symbol)?;
    let def_source = std::fs::read_to_string(&def_file).ok()?;
    let def_output = parser::parse_source_full(&def_source);
    let def_span = typeck::find_definition_span(&def_output.ast, symbol)?;
    let (start_line, start_char) = typeck::byte_offset_to_position(def_span.start, &def_source);
    let (end_line, end_char) = typeck::byte_offset_to_position(def_span.end, &def_source);
    let uri = Url::from_file_path(&def_file).ok()?;
    let range = Range {
        start: Position {
            line: start_line,
            character: start_char,
        },
        end: Position {
            line: end_line,
            character: end_char,
        },
    };
    Some((uri, range))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::ModPath;
    use ast::{BundleExportImport, LocalBundleImport};
    use diagnostic::SpanId;
    use internment::Intern;
    use std::fs;
    use std::path::PathBuf;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        dir.push(format!("ginlsp_goto_def_{name}_{pid}_{nanos}"));
        dir
    }

    fn write_file(path: &std::path::Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn goto_def_dep_chained_nested_opens_nested_flask_jsonc() {
        let dir = unique_temp_dir("export_nested");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
        );

        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(
            &dir.join("dep/filemod/flask.jsonc"),
            r#"{"name":"filemod","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("dep/filemod/x.gin"), "x: 1\n");
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let mp = ModPath::new(
            Intern::<String>::from_ref("dep"),
            vec![Intern::<String>::from_ref("filemod")],
            SpanId::new(0),
        );

        let loc = resolve_use_import_source_fs(&base_uri, &ImportSource::Package(mp)).unwrap();
        assert_eq!(
            loc.uri.to_file_path().unwrap(),
            dir.join("dep/filemod/flask.jsonc")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_nested_folder_opens_folder_flask_jsonc() {
        let dir = unique_temp_dir("export_folder");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
        );

        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(
            &dir.join("dep/foldermod/flask.jsonc"),
            r#"{"name":"foldermod","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let mp = ModPath::new(
            Intern::<String>::from_ref("dep"),
            vec![Intern::<String>::from_ref("foldermod")],
            SpanId::new(0),
        );

        let loc = resolve_use_import_source_fs(&base_uri, &ImportSource::Package(mp)).unwrap();
        assert_eq!(
            loc.uri.to_file_path().unwrap(),
            dir.join("dep/foldermod/flask.jsonc")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_root_opens_dep_flask_jsonc() {
        let dir = unique_temp_dir("dep_root");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
        );

        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let mp = ModPath::new(Intern::<String>::from_ref("dep"), vec![], SpanId::new(0));

        let loc = resolve_use_import_source_fs(&base_uri, &ImportSource::Package(mp)).unwrap();
        assert_eq!(loc.uri.to_file_path().unwrap(), dir.join("dep/flask.jsonc"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_bundle_opens_dependency_flask_jsonc() {
        let dir = unique_temp_dir("dep_bundle");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "utils": { "path": "utils" }
  }
}
"#,
        );
        write_file(
            &dir.join("main.gin"),
            "use utils.(io)\n\nmain:\n    return 0\n",
        );
        write_file(
            &dir.join("utils/flask.jsonc"),
            r#"{"name":"utils","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("utils/io/flask.jsonc"),
            r#"{"name":"io","version":"0.0.0","authors":[]}"#,
        );

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let lb = LocalBundleImport {
            root: Intern::<String>::from_ref("utils"),
            members: vec![BundleExportImport {
                export: Intern::<String>::from_ref("io"),
                alias: None,
                span: SpanId::new(0),
            }],
            span: SpanId::new(0),
        };

        let loc = resolve_use_import_source_fs(&base_uri, &ImportSource::LocalBundle(lb)).unwrap();
        assert_eq!(
            loc.uri.to_file_path().unwrap(),
            dir.join("utils/flask.jsonc")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_a_b_opens_deepest_folder_flask_jsonc() {
        let dir = unique_temp_dir("dep_a_b_folder");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
        );

        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("dep/a/flask.jsonc"),
            r#"{"name":"dep_a","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("dep/a/b/flask.jsonc"),
            r#"{"name":"dep_ab","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let mp = ModPath::new(
            Intern::<String>::from_ref("dep"),
            vec![
                Intern::<String>::from_ref("a"),
                Intern::<String>::from_ref("b"),
            ],
            SpanId::new(0),
        );

        let loc = resolve_use_import_source_fs(&base_uri, &ImportSource::Package(mp)).unwrap();
        assert_eq!(
            loc.uri.to_file_path().unwrap(),
            dir.join("dep/a/b/flask.jsonc")
        );

        let seg_a = Intern::<String>::from_ref("a");
        let seg_b = Intern::<String>::from_ref("b");
        let loc_mid = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("dep"),
            &[seg_a, seg_b],
            1,
        )
        .unwrap();
        assert_eq!(
            loc_mid.uri.to_file_path().unwrap(),
            dir.join("dep/a/flask.jsonc")
        );
        let loc_root = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("dep"),
            &[seg_a, seg_b],
            0,
        )
        .unwrap();
        assert_eq!(
            loc_root.uri.to_file_path().unwrap(),
            dir.join("dep/flask.jsonc")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_core_io_nested_package_opens_io_flask_jsonc() {
        let dir = unique_temp_dir("core_io_nested");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "core": { "path": "core" } }
}
"#,
        );
        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("core/io/flask.jsonc"),
            r#"{"name":"io","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("core/io/x.gin"), "x: 1\n");
        write_file(
            &dir.join("main.gin"),
            "use core.io\n\nmain:\n    return 0\n",
        );

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();

        let loc_root = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("core"),
            &[Intern::<String>::from_ref("io")],
            0,
        )
        .unwrap();
        assert_eq!(
            loc_root.uri.to_file_path().unwrap(),
            dir.join("core/flask.jsonc")
        );

        let loc_io = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("core"),
            &[Intern::<String>::from_ref("io")],
            1,
        )
        .unwrap();
        assert_eq!(
            loc_io.uri.to_file_path().unwrap(),
            dir.join("core/io/flask.jsonc")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_identifier_guard_blocks_non_words() {
        let source = "use core.(io, fs as store)";

        let core = source.find("core").unwrap();
        let io = source.find("io").unwrap();
        let fs = source.find("fs").unwrap();
        let store = source.find("store").unwrap();
        let dot = source.find('.').unwrap();
        let comma = source.find(',').unwrap();
        let paren = source.find('(').unwrap();
        let space = source.find(' ').unwrap();

        assert!(is_import_identifier_at(source, core));
        assert!(is_import_identifier_at(source, io));
        assert!(is_import_identifier_at(source, fs));
        assert!(is_import_identifier_at(source, store));

        assert!(!is_import_identifier_at(source, dot));
        assert!(!is_import_identifier_at(source, comma));
        assert!(!is_import_identifier_at(source, paren));
        assert!(!is_import_identifier_at(source, space));
    }

    #[test]
    fn goto_def_dep_symbol_opens_definition_gin_file() {
        let dir = unique_temp_dir("dep_symbol");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "core": { "path": "core" }
  }
}
"#,
        );
        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        // core/bool.gin exports `true := Bool.True` (a public bind)
        write_file(&dir.join("core/bool.gin"), "true := Bool.True\n");
        write_file(
            &dir.join("main.gin"),
            "use core.true\n\nmain:\n    true\nreturn\n",
        );

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();

        // part=0 → root "core" → should open core/flask.jsonc
        let loc_root = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("core"),
            &[Intern::<String>::from_ref("true")],
            0,
        )
        .unwrap();
        assert_eq!(
            loc_root.uri.to_file_path().unwrap(),
            dir.join("core/flask.jsonc")
        );

        // part=1 → segment "true" → should open core/bool.gin (the definition file)
        // at the exact line/column of `true`'s definition.
        let loc_true = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("core"),
            &[Intern::<String>::from_ref("true")],
            1,
        )
        .unwrap();
        assert_eq!(
            loc_true.uri.to_file_path().unwrap(),
            dir.join("core/bool.gin")
        );
        // Verify the range points to the definition of `true` at
        // line 0, col 0 (the only line in the file `true := Bool.True\n`)
        assert_eq!(loc_true.range.start.line, 0, "should be line 0");
        assert_eq!(loc_true.range.start.character, 0, "should start at col 0");
        assert_eq!(loc_true.range.end.line, 0, "should end on line 0");
        assert_eq!(
            loc_true.range.end.character, 4,
            "should end at col 4 (length of 'true')"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_nonexistent_symbol_returns_none() {
        let dir = unique_temp_dir("dep_nonexistent_symbol");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "core": { "path": "core" }
  }
}
"#,
        );
        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("core/bool.gin"), "true := Bool.True\n");
        // Note: `false` is not exported
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();

        // A symbol that doesn't exist in the dependency should return None
        let loc = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("core"),
            &[Intern::<String>::from_ref("nonexistent")],
            1,
        );
        assert!(loc.is_none(), "expected None for nonexistent symbol");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_symbol_among_nested_packages_prefers_sub_package() {
        // When a segment matches both a sub-package (folder with flask.jsonc)
        // and a public definition, the sub-package should be preferred.
        let dir = unique_temp_dir("dep_symbol_vs_folder");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
        );
        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        );
        // Both a sub-package `dep/io/` and a symbol `io` in dep/io.gin
        write_file(
            &dir.join("dep/io/flask.jsonc"),
            r#"{"name":"io","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("dep/io.gin"), "io: 42\n");
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let base_uri = Url::from_file_path(dir.join("main.gin")).unwrap();

        // Should prefer the sub-package (folder with flask.jsonc) over the symbol
        let loc = resolve_package_part_location_fs(
            &base_uri,
            &Intern::<String>::from_ref("dep"),
            &[Intern::<String>::from_ref("io")],
            1,
        )
        .unwrap();
        assert_eq!(
            loc.uri.to_file_path().unwrap(),
            dir.join("dep/io/flask.jsonc"),
            "should prefer sub-package over symbol definition"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// `resolve_symbol_location` must return the exact range of the symbol's
    /// definition in the target file, not just line 0 / col 0.
    #[test]
    fn resolve_symbol_location_returns_exact_definition_span() {
        let dir = unique_temp_dir("precise_location");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("core")).unwrap();

        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        // Use the exact content from the user's bool.gin
        write_file(
            &dir.join("core/bool.gin"),
            r#"Bool is True or False

false := Bool.False
true  := Bool.True
"#,
        );

        // Resolve `true` — it's on line 3 (0-indexed), col 0, length 4
        let (uri, range) = resolve_symbol_location(&dir.join("core"), "true")
            .expect("should resolve 'true' location");
        assert_eq!(
            uri.to_file_path().unwrap(),
            dir.join("core/bool.gin"),
            "should point to bool.gin"
        );
        // bool.gin content (0-indexed):
        //   line 0: Bool is True or False
        //   line 1: (empty)
        //   line 2: false := Bool.False
        //   line 3: true  := Bool.True
        assert_eq!(range.start.line, 3, "true definition starts on line 3");
        assert_eq!(range.start.character, 0, "true definition starts at col 0");
        assert_eq!(range.end.line, 3, "true definition ends on line 3");
        assert_eq!(
            range.end.character, 4,
            "true definition ends at col 4 (length of 'true')"
        );

        // Resolve `false` — line 2, col 0, length 5
        let (uri, range) = resolve_symbol_location(&dir.join("core"), "false")
            .expect("should resolve 'false' location");
        assert_eq!(
            uri.to_file_path().unwrap(),
            dir.join("core/bool.gin"),
            "should point to bool.gin"
        );
        assert_eq!(range.start.line, 2, "false definition starts on line 2");
        assert_eq!(range.start.character, 0, "false definition starts at col 0");
        assert_eq!(range.end.line, 2, "false definition ends on line 2");
        assert_eq!(
            range.end.character, 5,
            "false definition ends at col 5 (length of 'false')"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// `resolve_body_import_at` on `true` (used unqualified in `main`'s body)
    /// must resolve to the definition file `core/bool.gin`.
    #[test]
    fn goto_def_body_import_symbol_resolves_to_definition_file() {
        let dir = unique_temp_dir("body_import");
        let _ = fs::remove_dir_all(&dir);

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "core": { "path": "core" }
  }
}
"#,
        );
        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("core/bool.gin"),
            r#"Bool is True or False

false := Bool.False
true  := Bool.True
"#,
        );
        let main_path = dir.join("main.gin");
        write_file(&main_path, "use core.true\n\nmain:\n    true\nreturn\n");

        let base_uri = Url::from_file_path(&main_path).unwrap();
        let source = std::fs::read_to_string(&main_path).unwrap();
        let po = parser::parse_source_full(&source);
        let file_ast = &po.ast;

        // We cannot instantiate `Backend` in a unit test (it holds LSP state),
        // so we test the core logic via the free function `resolve_dep_dir_from_uri`
        // and then manually simulate what `resolve_body_import_at` does.

        // Verify the dependency directory resolves correctly
        let dep_dir = resolve_dep_dir_from_uri(&base_uri, "core").expect("dep dir should resolve");
        assert!(dep_dir.join("bool.gin").exists());

        // Verify `true` is a public definition in core
        let def_file = resolve::find_public_def_in_package(&dep_dir, "true")
            .expect("'true' should be a public def");
        assert_eq!(def_file, dir.join("core/bool.gin"));

        // Verify the effective_name of the import matches "true"
        let mut found_import = false;
        for imp in file_ast.uses() {
            for mi in &imp.0 {
                let imported_name: String = match mi.alias {
                    Some(ref alias) => format!("{}", alias),
                    None => mi.effective_name(),
                };
                if imported_name == "true" {
                    found_import = true;
                    // Must be a single-segment Package import
                    if let ast::ImportSource::Package(mp) = &mi.source {
                        assert_eq!(
                            mp.segments.len(),
                            1,
                            "'true' import should have exactly one segment"
                        );
                    } else {
                        panic!("expected Package import");
                    }
                }
            }
        }
        assert!(found_import, "should have found import for 'true'");

        let _ = fs::remove_dir_all(&dir);
    }
}
