use crate::diagnostics::span_to_range;
use crate::Backend;
use ast::{HasSpanId, ImportSource};
use database::file_parse_output;
use typeck::{find_definition_span, get_word_at_position, position_to_byte_offset};
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

/// Resolve a `use` import to a file location, matching `ginc`'s export-based rules.
///
/// This function is intentionally side-effect free (filesystem reads only) so it can be unit tested.
fn resolve_use_import_source_fs(base_uri: &Url, source: &ImportSource) -> Option<Location> {
    match source {
        ImportSource::Local(path, _) => {
            let base_path = base_uri.to_file_path().ok()?;
            let base_dir = base_path.parent()?;

            let mut resolved = base_dir.join(path);

            if resolved.extension().is_none() {
                let with_ext = resolved.with_extension("gin");
                if with_ext.exists() {
                    resolved = with_ext;
                }
            }

            if !resolved.exists() {
                return None;
            }
            Some(zero_location(Url::from_file_path(&resolved).ok()?))
        }
        ImportSource::LocalBundle(b) => {
            let base_path = base_uri.to_file_path().ok()?;
            let base_dir = base_path.parent()?;
            let folder = base_dir.join(b.root.as_str());

            let flask_jsonc_path = folder.join(flask::PACKAGE_CONFIG_NAME);
            if !flask_jsonc_path.exists() {
                return None;
            }
            Some(zero_location(
                Url::from_file_path(&flask_jsonc_path).ok()?,
            ))
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

            resolve_chained_export_location(&dep_dir, &mod_path.segments)
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

    resolve_chained_export_location(&dep_dir, &segments[..=seg_idx])
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

fn resolve_chained_export_location(
    start_dir: &std::path::Path,
    segments: &[internment::Intern<String>],
) -> Option<Location> {
    let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    match flask::resolve_chained_exports(start_dir, &segs).ok()? {
        flask::ExportTarget::File(p) => {
            if !p.exists() {
                return None;
            }
            Some(zero_location(Url::from_file_path(&p).ok()?))
        }
        flask::ExportTarget::FolderModule(dir) => {
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

        if let Some(state) = self.documents.get(&uri.to_string()) {
            let snapshot = self.snapshot();
            let ast = file_parse_output(&snapshot.db, state.file).ast.clone();

            let byte_pos = position_to_byte_offset(&state.source, position.line, position.character);

            if let Some(byte_pos) = byte_pos {
                if let Some(link) = self.resolve_use_import_at(&uri, &ast, &state.source, byte_pos)
                {
                    return Ok(Some(GotoDefinitionResponse::Link(vec![link])));
                }
            }

            if let Some(word) =
                get_word_at_position(&state.source, position.line, position.character)
            {
                let range = find_definition_span(&ast, &word)
                    .map(|span| span_to_range(span.start, span.end, &state.source))
                    .unwrap_or_default();
                if range != Range::default() {
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri,
                        range,
                    })));
                }
            }
        }

        Ok(None)
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
    fn goto_def_dep_export_to_file_opens_file() {
        let dir = unique_temp_dir("export_file");
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
  "authors": [],
  "exports": {
    "filemod": { "path": "filemod.gin" }
  }
}
"#,
        );
        write_file(&dir.join("dep/filemod.gin"), "x: 1\n");
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
            dir.join("dep/filemod.gin")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_export_to_folder_opens_folder_flask_jsonc() {
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
  "authors": [],
  "exports": {
    "foldermod": { "path": "foldermod" }
  }
}
"#,
        );
        write_file(
            &dir.join("dep/foldermod/flask.jsonc"),
            r#"
{
  "name": "foldermod",
  "version": "0.0.0",
  "authors": [],
  "exports": {}
}
"#,
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
  "authors": [],
  "exports": {}
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
    fn goto_def_local_bundle_opens_folder_flask_jsonc() {
        let dir = unique_temp_dir("local_bundle");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(&dir.join("main.gin"), "use utils.(io)\n\nmain:\n    return 0\n");
        write_file(
            &dir.join("utils/flask.jsonc"),
            r#"
{
  "name": "utils",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "io": { "path": "io.gin" }
  }
}
"#,
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
        assert_eq!(
            loc.uri.to_file_path().unwrap(),
            dir.join("utils/flask.jsonc")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_a_b_resolves_to_file() {
        let dir = unique_temp_dir("dep_a_b_file");
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
  "authors": [],
  "exports": {
    "a": { "path": "a" }
  }
}
"#,
        );
        write_file(
            &dir.join("dep/a/flask.jsonc"),
            r#"
{
  "name": "dep_a",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "b": { "path": "b.gin" }
  }
}
"#,
        );
        write_file(&dir.join("dep/a/b.gin"), "x: 1\n");
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
        assert_eq!(loc.uri.to_file_path().unwrap(), dir.join("dep/a/b.gin"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_dep_a_b_opens_folder_module_flask_jsonc() {
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
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "a": { "path": "a" }
  }
}
"#,
        );
        write_file(
            &dir.join("dep/a/flask.jsonc"),
            r#"
{
  "name": "dep_a",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "b": { "path": "b" }
  }
}
"#,
        );
        write_file(
            &dir.join("dep/a/b/flask.jsonc"),
            r#"
{
  "name": "dep_ab",
  "version": "0.0.0",
  "authors": [],
  "exports": {}
}
"#,
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

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goto_def_core_io_root_opens_core_config_but_io_opens_export() {
        let dir = unique_temp_dir("core_io_root_vs_seg");
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
            r#"
{
  "name": "core",
  "version": "0.0.0",
  "authors": [],
  "exports": { "io": { "path": "io.gin" } }
}
"#,
        );
        write_file(&dir.join("core/io.gin"), "x: 1\n");
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
        assert_eq!(loc_io.uri.to_file_path().unwrap(), dir.join("core/io.gin"));

        let _ = fs::remove_dir_all(&dir);
    }
}
