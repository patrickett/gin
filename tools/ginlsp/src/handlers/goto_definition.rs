use crate::diagnostics::span_to_range;
use crate::Backend;
use ast::{HasSpanId, ImportSource};
use database::file_parse_output;
use typeck::{
    find_definition_span, find_import_definition_span, get_word_at_position, position_to_byte_offset,
};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

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
                    return Some(zero_location(
                        Url::from_file_path(&flask_jsonc_path).ok()?,
                    ));
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

    resolve_nested_package_manifest_location(&dep_dir, &segments[..=seg_idx])
}

fn part_index_in_dotted_path(span_text: &str, byte_in_span: usize) -> Option<usize> {
    let mut part = 0usize;
    for (i, ch) in span_text.char_indices() {
        if i >= byte_in_span {
            break;
        }
        if ch == '.' {
            part += 1;
        }
    }
    Some(part)
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

        let (source, file) = match self.documents.get(&uri.to_string()) {
            Some(state) => (state.source.clone(), state.file),
            None => return Ok(None),
        };

        // `file_parse_output` runs the parser via Salsa; offload so a stuck
        // parse cannot pin the async runtime.
        let response = self
            .run_blocking_request("goto_definition", move |this| {
                let snapshot = this.snapshot();
                let ast = file_parse_output(&snapshot.db, file).ast.clone();

                if let Some(byte_pos) =
                    position_to_byte_offset(&source, position.line, position.character)
                {
                    if let Some(link) = this.resolve_use_import_at(&uri, &ast, &source, byte_pos) {
                        return Some(GotoDefinitionResponse::Link(vec![link]));
                    }
                }

                if let Some(word) =
                    get_word_at_position(&source, position.line, position.character)
                {
                    let range = find_definition_span(&ast, &word)
                        .map(|span| span_to_range(span.start, span.end, &source))
                        .unwrap_or_default();
                    if range != Range::default() {
                        return Some(GotoDefinitionResponse::Scalar(Location { uri, range }));
                    }
                    if let Some(span) = find_import_definition_span(&ast, &word) {
                        let range = span_to_range(span.start, span.end, &source);
                        return Some(GotoDefinitionResponse::Scalar(Location { uri, range }));
                    }
                }

                None
            })
            .await;

        Ok(response.flatten())
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
                        (b.span_id(), ImportSource::LocalBundle(b.clone()))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::{BundleExportImport, LocalBundleImport};
    use ast::ModPath;
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
        let mp = ModPath::new(
            Intern::<String>::from_ref("dep"),
            vec![],
            SpanId::new(0),
        );

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
        write_file(&dir.join("main.gin"), "use utils.(io)\n\nmain:\n    return 0\n");
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
            }],
            span: SpanId::new(0),
        };

        let loc = resolve_use_import_source_fs(&base_uri, &ImportSource::LocalBundle(lb)).unwrap();
        assert_eq!(loc.uri.to_file_path().unwrap(), dir.join("utils/flask.jsonc"));

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
            &[seg_a.clone(), seg_b.clone()],
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
        write_file(&dir.join("main.gin"), "use core.io\n\nmain:\n    return 0\n");

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
}
