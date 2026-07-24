//! Symbol definition sites and import navigation targets (goto-def, hover def spans).

use std::ops::Range;
use std::path::{Path, PathBuf};

use ast::source::{CharExt, SourceExt};
use ast::{FileAst, HasSpanId, ImportSource, LocalBundleImport};
use flask::{DependencyKind, FlaskPathExt};
use internment::Intern;

use crate::ParsedFile;
use crate::file_helpers::GinPackageExt;
use crate::folder_module::{
    normalize_local_import_path, resolve_logical_module_dir, split_dep_path_segments,
};
use crate::import_query::{find_package_root, resolve_dep_dir, resolve_import_at};

/// A public symbol's definition file and source byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefLocation {
    pub file: PathBuf,
    pub byte_range: Range<usize>,
}

/// Import path navigation — either a manifest (`flask.jsonc`) or a symbol definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportNavLocation {
    /// `flask.jsonc` or folder module root (navigate to start of file).
    Manifest(PathBuf),
    /// Same-file `use` of current-module symbols.
    FileRoot(PathBuf),
    Definition(DefLocation),
}

/// Result of resolving the cursor for goto-definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorDefinition {
    SameFile(Range<usize>),
    OtherFile(DefLocation),
    Import {
        target: ImportNavLocation,
        origin: Option<Range<usize>>,
    },
}

/// Find a public `symbol` under `scope_dir` (package or folder module).
pub fn public_symbol_def_location(
    scope_dir: &Path,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let def_file = scope_dir.find_public_def(symbol)?;
    let parsed = file_reader(&def_file)?;
    let byte_range = parsed.output.ast.definition_span(symbol)?;
    Some(DefLocation {
        file: def_file,
        byte_range,
    })
}

/// Dependency member: `use dep.sym`.
pub fn dep_symbol_def_location(
    file_path: &Path,
    dep_name: &str,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    public_symbol_def_location(&dep_dir, symbol, file_reader)
}

/// Folder module containing a bundle export such as `folder.Symbol` or `Sym`.
pub fn public_symbol_in_module_dir(
    module_dir: &Path,
    export: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let segments: Vec<&str> = export.split('.').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    if segments.len() == 1 {
        return public_symbol_def_location(module_dir, segments[0], file_reader);
    }
    let (module_segs, trailing_member) = split_dep_path_segments(module_dir, &segments);
    let sym = trailing_member?;
    let folder = if module_segs.is_empty() {
        module_dir.to_path_buf()
    } else {
        let refs: Vec<&str> = module_segs.iter().map(|s| s.as_str()).collect();
        resolve_logical_module_dir(module_dir, &refs)?
    };
    public_symbol_def_location(&folder, &sym, file_reader)
}

/// Dependency bundle member: `use dep.(folder.Symbol)` or `use core.(Sym)`.
pub fn dep_bundle_member_def_location(
    file_path: &Path,
    b: &LocalBundleImport,
    export: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let dep_dir = dep_dir_from_file(file_path, b.root.as_str())?;
    let module_dir = if b.path_segments.is_empty() {
        dep_dir
    } else {
        let segs: Vec<&str> = b.path_segments.iter().map(|s| s.as_str()).collect();
        resolve_logical_module_dir(&dep_dir, &segs)?
    };
    public_symbol_in_module_dir(&module_dir, export, file_reader)
}

/// Local bundle member: `use './path'.(Sym)` or `use './path'.(folder.Symbol)`.
pub fn local_bundle_def_location(
    file_path: &Path,
    local_path: &Path,
    export: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let base_dir = file_path.parent()?;
    let resolved = base_dir.join(local_path);
    let resolved = std::fs::canonicalize(&resolved).unwrap_or(resolved);
    if resolved.is_dir() {
        return public_symbol_in_module_dir(&resolved, export, file_reader);
    }
    let sym = export.rsplit('.').next().unwrap_or(export);
    let parent = resolved.parent()?;
    let def_file = parent.find_public_def(sym)?;
    let def_canonical = std::fs::canonicalize(&def_file).unwrap_or(def_file.clone());
    if std::fs::canonicalize(&resolved).unwrap_or(resolved) != def_canonical {
        return None;
    }
    let parsed = file_reader(&def_file)?;
    let byte_range = parsed.output.ast.definition_span(sym)?;
    Some(DefLocation {
        file: def_file,
        byte_range,
    })
}

/// Same-folder sibling: `use Sym` in a `use` line (definition in another file in the folder).
pub fn current_module_sibling_def_location(
    file_path: &Path,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let file_dir = file_path.parent()?;
    let def_file = crate::public_symbols::find_public_def(file_dir, symbol, false)?;
    let parsed = file_reader(&def_file)?;
    let byte_range = parsed.output.ast.definition_span(symbol)?;
    Some(DefLocation {
        file: def_file,
        byte_range,
    })
}

/// Package-root def in the current package (`use Sym` body reference).
pub fn current_module_package_def_location(
    file_path: &Path,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let pkg_root = find_package_root(file_path)?;
    public_symbol_def_location(&pkg_root, symbol, file_reader)
}

/// Map [`crate::ImportTarget`] to a definition or manifest location.
pub fn def_location_for_import_target(
    file_path: &Path,
    target: crate::ImportTarget,
    byte_pos: usize,
    ast: &FileAst,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<ImportNavLocation> {
    use crate::ImportTarget;

    match target {
        ImportTarget::DepRoot { dep_name } => {
            let dep_dir = resolve_dep_dir(file_path, &dep_name)?;
            let manifest = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
            if !manifest.is_file() {
                return None;
            }
            for import in &ast.uses {
                for mi in &import.0 {
                    if let ImportSource::Package(mp) = &mi.source {
                        let span = ast.span_table.get(mp.span_id());
                        if byte_pos >= span.start() && byte_pos <= span.end() {
                            return Some(ImportNavLocation::Manifest(manifest));
                        }
                    }
                }
            }
            None
        }
        ImportTarget::DepSymbol { dep_name, symbol }
        | ImportTarget::BodySymbol { dep_name, symbol } => {
            dep_symbol_def_location(file_path, &dep_name, &symbol, file_reader)
                .map(ImportNavLocation::Definition)
        }
        ImportTarget::DepBundleMember {
            dep_name,
            path_segments,
            symbol,
        } => {
            let b = LocalBundleImport {
                root: Intern::new(dep_name),
                path_segments: path_segments
                    .iter()
                    .map(|s| Intern::new(s.clone()))
                    .collect(),
                members: Vec::new(),
                span: ast::span::SpanId::INVALID,
                local_path: None,
            };
            dep_bundle_member_def_location(file_path, &b, &symbol, file_reader)
                .map(ImportNavLocation::Definition)
        }
        ImportTarget::LocalBundleSymbol { local_path, symbol } => {
            local_bundle_def_location(file_path, &local_path, &symbol, file_reader)
                .map(ImportNavLocation::Definition)
        }
        ImportTarget::CurrentModuleSymbol { symbol } => {
            current_module_package_def_location(file_path, &symbol, file_reader)
                .map(ImportNavLocation::Definition)
        }
    }
}

/// Cursor on an imported name in the file body (`use core.true` → `true`).
pub fn body_import_def_location(
    file_path: &Path,
    ast: &FileAst,
    source: &str,
    word: &str,
    byte_pos: usize,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let _ = word;
    let target = resolve_import_at(ast, source, byte_pos)?;
    match target {
        crate::ImportTarget::BodySymbol { dep_name, symbol } => {
            dep_symbol_def_location(file_path, &dep_name, &symbol, file_reader)
        }
        crate::ImportTarget::LocalBundleSymbol { local_path, symbol } => {
            local_bundle_def_location(file_path, &local_path, &symbol, file_reader)
        }
        crate::ImportTarget::CurrentModuleSymbol { symbol } => {
            current_module_package_def_location(file_path, &symbol, file_reader)
        }
        crate::ImportTarget::DepBundleMember {
            dep_name,
            path_segments,
            symbol,
        } => {
            let b = LocalBundleImport {
                root: Intern::new(dep_name),
                path_segments: path_segments
                    .iter()
                    .map(|s| Intern::new(s.clone()))
                    .collect(),
                members: Vec::new(),
                span: ast::span::SpanId::INVALID,
                local_path: None,
            };
            dep_bundle_member_def_location(file_path, &b, &symbol, file_reader)
        }
        _ => None,
    }
}

/// Whether `byte_pos` is on an identifier inside a `use` path (not whitespace/string).
pub fn is_import_identifier_at(source: &str, byte_pos: usize) -> bool {
    source
        .get(byte_pos..)
        .and_then(|tail| tail.chars().next())
        .map(|c| c.is_identifier())
        .unwrap_or(false)
}

fn nested_package_manifest(start_dir: &Path, segments: &[&str]) -> Option<PathBuf> {
    match start_dir.resolve_nested_package_path(segments).ok()? {
        flask::NestedPackageTarget::FolderModule(dir) => {
            let cfg = dir.join(flask::PACKAGE_CONFIG_NAME);
            if cfg.exists() { Some(cfg) } else { None }
        }
    }
}

fn dep_dir_from_file(file_path: &Path, dep_name: &str) -> Option<PathBuf> {
    let base_dir = file_path.parent()?;
    let handle = flask::FlaskConfigHandle::load(base_dir).ok()?;
    let cfg = handle.read();
    let config_dir = handle.source_dir();
    let dep = cfg.config.dependencies().get(dep_name)?;
    match &dep.kind {
        DependencyKind::Path { path } => Some(config_dir.join(path)),
        _ => None,
    }
}

/// `use dep.folder.(sym, …)` — `part_index` 0 = dep root, 1+ = `path_segments`.
pub fn dep_bundle_import_part_location(
    file_path: &Path,
    b: &LocalBundleImport,
    part_index: usize,
    _file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<ImportNavLocation> {
    let dep_dir = dep_dir_from_file(file_path, b.root.as_str())?;

    if part_index == 0 {
        let manifest = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
        return manifest
            .exists()
            .then_some(ImportNavLocation::Manifest(manifest));
    }

    let seg_idx = part_index.saturating_sub(1);
    if seg_idx >= b.path_segments.len() {
        return None;
    }

    let seg_strs: Vec<&str> = b.path_segments[..=seg_idx]
        .iter()
        .map(|s| s.as_str())
        .collect();
    if let Some(manifest) = nested_package_manifest(&dep_dir, &seg_strs) {
        return Some(ImportNavLocation::Manifest(manifest));
    }

    resolve_logical_module_dir(&dep_dir, &seg_strs).map(ImportNavLocation::Manifest)
}

/// Dotted package import `use dep.seg.sym` — `part_index` 0 = dep root, 1+ = segments.
pub fn package_import_part_location(
    file_path: &Path,
    root: &str,
    segments: &[Intern<String>],
    part_index: usize,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<ImportNavLocation> {
    let dep_dir = dep_dir_from_file(file_path, root)?;

    if part_index == 0 {
        let manifest = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
        return manifest
            .exists()
            .then_some(ImportNavLocation::Manifest(manifest));
    }

    let seg_count = segments.len();
    let seg_idx = part_index.saturating_sub(1);
    if seg_idx >= seg_count {
        return None;
    }

    let seg_strs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    if let Some(manifest) = nested_package_manifest(&dep_dir, &seg_strs[..=seg_idx]) {
        return Some(ImportNavLocation::Manifest(manifest));
    }

    if seg_idx == seg_count - 1 {
        let symbol = segments[seg_idx].as_str();
        return public_symbol_def_location(&dep_dir, symbol, file_reader)
            .map(ImportNavLocation::Definition);
    }

    None
}

/// Resolve a `use` import source to a manifest or definition (folder / dep / local path).
pub fn import_source_nav_location(
    file_path: &Path,
    source: &ImportSource,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<ImportNavLocation> {
    match source {
        ImportSource::CurrentModule { .. } => {
            Some(ImportNavLocation::FileRoot(file_path.to_path_buf()))
        }
        ImportSource::Local(path, _) => {
            let base_dir = file_path.parent()?;
            let pkg_root = find_package_root(base_dir)?;
            let full = normalize_local_import_path(base_dir, &pkg_root, path).ok()?;
            full.is_dir().then_some(ImportNavLocation::Manifest(full))
        }
        ImportSource::LocalMember(m) => {
            let local_path = m.local_path.as_ref()?;
            local_bundle_def_location(file_path, local_path, m.member.export.as_str(), file_reader)
                .map(ImportNavLocation::Definition)
        }
        ImportSource::LocalBundle(b) => {
            if let Some(local_path) = &b.local_path {
                let base_dir = file_path.parent()?;
                let pkg_root = find_package_root(base_dir)?;
                let full = normalize_local_import_path(base_dir, &pkg_root, local_path).ok()?;
                full.is_dir().then_some(ImportNavLocation::Manifest(full))
            } else if b.path_segments.is_empty() {
                let dep_dir = dep_dir_from_file(file_path, b.root.as_str())?;
                let manifest = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
                manifest
                    .exists()
                    .then_some(ImportNavLocation::Manifest(manifest))
            } else {
                dep_bundle_import_part_location(file_path, b, b.path_segments.len(), file_reader)
            }
        }
        ImportSource::Package(mod_path) => {
            if mod_path.segments.is_empty() {
                let dep_dir = dep_dir_from_file(file_path, mod_path.root.as_str())?;
                let manifest = dep_dir.join(flask::PACKAGE_CONFIG_NAME);
                return manifest
                    .exists()
                    .then_some(ImportNavLocation::Manifest(manifest));
            }
            package_import_part_location(
                file_path,
                mod_path.root.as_str(),
                &mod_path.segments,
                mod_path.segments.len(),
                file_reader,
            )
        }
    }
}

/// Union variant literal in a type pattern (`False` in `Ref(_, False)`).
pub fn union_variant_def_location(
    file_path: &Path,
    variant_name: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    if let Some(parsed) = file_reader(file_path)
        && let Some(byte_range) = parsed.output.ast.variant_definition_span(variant_name)
    {
        return Some(DefLocation {
            file: file_path.to_path_buf(),
            byte_range,
        });
    }

    let mut search_dirs = Vec::new();
    if let Some(pkg) = find_package_root(file_path) {
        search_dirs.push(pkg);
    } else if let Some(module_root) = file_path.parent().and_then(Path::parent) {
        search_dirs.push(module_root.to_path_buf());
    }
    if let Some(parent) = file_path.parent() {
        let parent = parent.to_path_buf();
        if !search_dirs.iter().any(|d| d == &parent) {
            search_dirs.push(parent);
        }
    }

    for dir in search_dirs {
        for gin_path in dir.collect_gin_files() {
            if gin_path == file_path {
                continue;
            }
            let parsed = file_reader(&gin_path)?;
            if let Some(byte_range) = parsed.output.ast.variant_definition_span(variant_name) {
                return Some(DefLocation {
                    file: gin_path,
                    byte_range,
                });
            }
        }
    }
    None
}

/// Unified goto-definition: same-file, import path, body import, or in-file def.
pub fn cursor_definition(
    file_path: &Path,
    ast: &FileAst,
    source: &str,
    byte_pos: usize,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<CursorDefinition> {
    if let Some(def) = definition_in_use_span(file_path, ast, source, byte_pos, file_reader) {
        return Some(def);
    }

    let word = ast
        .word_at_byte(byte_pos, source)
        .or_else(|| source.word_at_byte_offset(byte_pos))?;

    if let Some(span) = parameter_definition_span(ast, source, &word, byte_pos) {
        return Some(CursorDefinition::SameFile(span));
    }

    // `self` keyword → receiver type.
    if word == "self"
        && let Some(span) = self_keyword_def_span(ast, byte_pos)
    {
        return Some(CursorDefinition::SameFile(span));
    }

    // Type parameter reference (e.g. `x` in `start x` inside `Range(x) has ...`.
    if let Some(span) = type_param_def_span(ast, source, &word, byte_pos) {
        return Some(CursorDefinition::SameFile(span));
    }

    // `self.field` in provisions → has-member field definition.
    if let Some(span) = record_get_field_def_span(ast, byte_pos, &word) {
        return Some(CursorDefinition::SameFile(span));
    }

    // Has-body method name (e.g. `new` in `new(start x, ...)`) → method declaration.
    if let Some(span) = has_method_def_span(ast, byte_pos, &word) {
        return Some(CursorDefinition::SameFile(span));
    }

    if ast
        .union_variant_reference_at_byte(source, byte_pos)
        .is_some()
        && let Some(loc) = union_variant_def_location(file_path, &word, file_reader)
    {
        if loc.file == file_path {
            return Some(CursorDefinition::SameFile(loc.byte_range));
        }
        return Some(CursorDefinition::OtherFile(loc));
    }

    if let Some(loc) =
        body_import_def_location(file_path, ast, source, &word, byte_pos, file_reader)
    {
        return Some(CursorDefinition::OtherFile(loc));
    }

    if let Some(span) = ast.definition_span(&word) {
        return Some(CursorDefinition::SameFile(span));
    }

    if let Some(span) = ast.find_import_definition_span(&word, source) {
        return Some(CursorDefinition::SameFile(span));
    }

    if word.starts_with(char::is_uppercase)
        && let Some(loc) = union_variant_def_location(file_path, &word, file_reader)
    {
        if loc.file == file_path {
            return Some(CursorDefinition::SameFile(loc.byte_range));
        }
        return Some(CursorDefinition::OtherFile(loc));
    }

    None
}

fn parameter_definition_span(
    ast: &FileAst,
    source: &str,
    word: &str,
    byte_pos: usize,
) -> Option<Range<usize>> {
    let span_table = &ast.span_table;
    let key = internment::Intern::<String>::from_ref(word);
    // Search standard defs (top-level binds).
    for bind in ast.defs.values() {
        let Some(params) = bind.params.as_ref() else {
            continue;
        };
        if !params.contains_key(&key) || !bind_body_contains(span_table, bind, byte_pos) {
            continue;
        }
        if let Some(span) = inferred_param_name_span(span_table, source, bind, word) {
            return Some(span);
        }
    }
    // Search has-body function params.
    for decl in ast.tags.values() {
        if !span_table.get(decl.span).contains(byte_pos) {
            continue;
        }
        let members = match &decl.value {
            ast::DeclareValue::Has(members) => members,
            _ => continue,
        };
        for m in members {
            if let ast::HasMember::Function(f) = m
                && f.params.contains_key(&key)
                && byte_in_has_function_body(f, byte_pos, span_table)
                && let Some(span) = has_param_name_span_from_fn(span_table, source, f, word)
            {
                return Some(span);
            }
        }
    }
    None
}

/// Check if a byte position falls within a HasFunction's body expression.
fn byte_in_has_function_body(
    f: &ast::HasFunction,
    byte_pos: usize,
    span_table: &ast::span::SpanTable,
) -> bool {
    f.body.as_ref().is_some_and(|body| {
        let value = match body {
            ast::HasMemberBody::Overrideable(value) | ast::HasMemberBody::Final(value) => value,
        };
        match value {
            ast::BindValue::Expr(expr) => span_table.get(expr.span_id).contains(byte_pos),
            ast::BindValue::Body { exprs, ret } => {
                let start = exprs
                    .first()
                    .map(|expr| span_table.get(expr.span_id).start())
                    .unwrap_or_else(|| span_table.get(ret.span_id).start());
                let end = span_table.get(ret.span_id).end();
                (start..end).contains(&byte_pos)
            }
            ast::BindValue::Extern | ast::BindValue::Unassigned => false,
        }
    })
}

/// Find a param name span in a HasFunction by scanning source between name and return type.
fn has_param_name_span_from_fn(
    span_table: &ast::span::SpanTable,
    source: &str,
    f: &ast::HasFunction,
    param_name: &str,
) -> Option<Range<usize>> {
    let start = span_table.get(f.name_span).end();
    let end = f
        .return_ty
        .as_ref()
        .map(|rt| span_table.get(rt.span_id).start())
        .unwrap_or(start);
    find_param_name_between(source, start, end, param_name).map(|pos| pos..pos + param_name.len())
}

/// Find the tag definition that `self` refers to.
/// Searches provided trait field expressions and has-body members
/// to find the containing tag.
fn self_keyword_def_span(ast: &FileAst, byte_pos: usize) -> Option<Range<usize>> {
    let span_table = &ast.span_table;
    for decl in ast.tags.values() {
        // Check provided trait fields (e.g. `Range.Bounded has min: self.start`.
        for pt in &decl.provided_traits {
            for (_, expr) in &pt.fields {
                if span_table.get(expr.span_id).contains(byte_pos) {
                    let span = span_table.get(decl.name_span);
                    return Some(span.start()..span.end());
                }
            }
        }
    }
    None
}

/// Find a `HasMember::Function` declaration matching the word at the byte position.
/// Used for goto-def on method names in calls.
fn has_method_def_span(ast: &FileAst, byte_pos: usize, word: &str) -> Option<Range<usize>> {
    let span_table = &ast.span_table;
    let key = internment::Intern::<String>::from_ref(word);
    for decl in ast.tags.values() {
        if !span_table.get(decl.span).contains(byte_pos) {
            continue;
        }
        let members = match &decl.value {
            ast::DeclareValue::Has(members) => members,
            _ => continue,
        };
        for m in members {
            if let ast::HasMember::Function(f) = m
                && f.name == key
            {
                let span = span_table.get(f.name_span);
                return Some(span.start()..span.end());
            }
        }
    }
    None
}

/// Find the has-member field definition that a `self.field` expression refers to.
/// Searches `ProvidedTrait` field expressions and `HasMember::Property` definitions.
fn record_get_field_def_span(ast: &FileAst, byte_pos: usize, word: &str) -> Option<Range<usize>> {
    let span_table = &ast.span_table;
    let field_key = internment::Intern::<String>::from_ref(word);

    // Search provided trait field expressions for `RecordGet { base: SelfRef, field }`.
    for decl in ast.tags.values() {
        for pt in &decl.provided_traits {
            for (_, expr) in &pt.fields {
                if !span_table.get(expr.span_id).contains(byte_pos) {
                    continue;
                }
                // Found an expression at this byte. Check if it's a RecordGet for our field.
                if let ast::Expr::RecordGet { field, .. } = &expr.value
                    && *field == field_key
                {
                    // Find the field definition in this tag's has members.
                    return find_has_member_def_span(ast, &decl.name, &field_key, span_table);
                }
            }
        }
    }
    None
}

/// Search a tag's `DeclareValue::Has` members for a property/function with the given name,
/// and return its name span.
fn find_has_member_def_span(
    ast: &FileAst,
    tag_name: &internment::Intern<String>,
    member_name: &internment::Intern<String>,
    span_table: &ast::span::SpanTable,
) -> Option<Range<usize>> {
    let decl = ast.tags.get(tag_name)?;
    let members = match &decl.value {
        ast::DeclareValue::Has(members) => members,
        _ => return None,
    };
    for m in members {
        let (name, name_span) = match m {
            ast::HasMember::Property(p) => (p.name, p.name_span),
            ast::HasMember::Function(f) => (f.name, f.name_span),
        };
        if name == *member_name {
            let span = span_table.get(name_span);
            return Some(span.start()..span.end());
        }
    }
    None
}

/// Find the type parameter definition that a word refers to.
/// For example, `x` in property `start x` inside `Range(x) has ...`
/// should resolve to the `x` in `Range(x)`.
fn type_param_def_span(
    ast: &FileAst,
    source: &str,
    word: &str,
    byte_pos: usize,
) -> Option<Range<usize>> {
    let span_table = &ast.span_table;
    let key = internment::Intern::<String>::from_ref(word);
    for decl in ast.tags.values() {
        if !span_table.get(decl.span).contains(byte_pos) {
            continue;
        }
        let Some(params) = &decl.params else {
            continue;
        };
        if !params.contains_key(&key) {
            continue;
        }
        // Scan between name end and next `) or newline for the param name.
        let name_span = span_table.get(decl.name_span);
        let close_paren = source[name_span.end()..].find(')')? + name_span.end();
        let param_str = source.get(name_span.end()..close_paren)?;
        // Find `word` as a token in the param string.
        let mut search_start = 0;
        while let Some(rel) = param_str[search_start..].find(word) {
            let abs = name_span.end() + search_start + rel;
            let before = abs
                .checked_sub(1)
                .and_then(|i| source.as_bytes().get(i))
                .copied();
            let after = source.as_bytes().get(abs + word.len()).copied();
            let word_start = |b: u8| b.is_ascii_whitespace() || b == b'(' || b == b',';
            let word_end = |b: u8| b.is_ascii_whitespace() || b == b')' || b == b',';
            if word_start(before.unwrap_or(b'(')) && after.is_none_or(word_end) {
                return Some(abs..abs + word.len());
            }
            search_start += rel + 1;
        }
    }
    None
}

fn bind_body_contains(
    span_table: &ast::span::SpanTable,
    bind: &ast::Bind,
    byte_pos: usize,
) -> bool {
    match &bind.value {
        ast::BindValue::Expr(expr) => span_table.get(expr.span_id).contains(byte_pos),
        ast::BindValue::Body { exprs, ret } => {
            exprs
                .iter()
                .any(|expr| span_table.get(expr.span_id).contains(byte_pos))
                || span_table.get(ret.span_id).contains(byte_pos)
        }
        ast::BindValue::Extern | ast::BindValue::Unassigned => false,
    }
}

fn inferred_param_name_span(
    span_table: &ast::span::SpanTable,
    source: &str,
    bind: &ast::Bind,
    param_name: &str,
) -> Option<Range<usize>> {
    let start = span_table.get(bind.name_span).end();
    let end = match &bind.value {
        ast::BindValue::Expr(expr) => span_table.get(expr.span_id).start(),
        ast::BindValue::Body { exprs, ret } => exprs
            .first()
            .map(|expr| span_table.get(expr.span_id).start())
            .unwrap_or_else(|| span_table.get(ret.span_id).start()),
        ast::BindValue::Extern | ast::BindValue::Unassigned => start,
    };
    if start >= end {
        return None;
    }
    for (name, _) in bind.params.as_ref()? {
        let name = name.as_str();
        let offset = find_param_name_between(source, start, end, name)?;
        if name == param_name {
            return Some(offset..offset + name.len());
        }
    }
    None
}

fn find_param_name_between(source: &str, start: usize, end: usize, name: &str) -> Option<usize> {
    let haystack = source.get(start..end)?;
    let mut search_from = 0;
    while let Some(relative) = haystack.get(search_from..)?.find(name) {
        let offset = start + search_from + relative;
        let before = source.as_bytes().get(offset.wrapping_sub(1)).copied();
        let after = source.as_bytes().get(offset + name.len()).copied();
        let ident_before = before.is_some_and(is_ident_byte);
        let ident_after = after.is_some_and(is_ident_byte);
        if !ident_before && !ident_after {
            return Some(offset);
        }
        search_from += relative + name.len();
    }
    None
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Cursor inside a `use` statement (import path or member).
fn definition_in_use_span(
    file_path: &Path,
    ast: &FileAst,
    source: &str,
    byte_pos: usize,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<CursorDefinition> {
    use ast::LocalBundleImport;

    let span_table = &ast.span_table;

    for import in &ast.uses {
        for module_import in &import.0 {
            let (import_span, source_ref) = match &module_import.source {
                ImportSource::Local(path, span_id) => {
                    (*span_id, ImportSource::Local(path.clone(), *span_id))
                }
                ImportSource::LocalBundle(b) => {
                    let mut found: Option<(diagnostic::SpanId, ImportSource)> = None;
                    for member in &b.members {
                        let mspan = span_table.get(member.span);
                        if byte_pos >= mspan.start() && byte_pos <= mspan.end() {
                            let lb = LocalBundleImport {
                                root: b.root,
                                path_segments: b.path_segments.clone(),
                                members: vec![member.clone()],
                                span: member.span,
                                local_path: b.local_path.clone(),
                            };
                            found = Some((member.span, ImportSource::LocalBundle(lb)));
                            break;
                        }
                    }
                    found.unwrap_or_else(|| (b.span_id(), ImportSource::LocalBundle(b.clone())))
                }
                ImportSource::Package(mod_path) => {
                    (mod_path.span_id(), ImportSource::Package(mod_path.clone()))
                }
                ImportSource::CurrentModule { member } => (
                    member.span,
                    ImportSource::CurrentModule {
                        member: member.clone(),
                    },
                ),
                ImportSource::LocalMember(m) => (m.span, ImportSource::LocalMember(m.clone())),
            };

            let span = span_table.get(import_span);
            if byte_pos < span.start() || byte_pos > span.end() {
                continue;
            }

            if !is_import_identifier_at(source, byte_pos) {
                continue;
            }

            let origin = Some(span.start()..span.end());

            let target = match &source_ref {
                ImportSource::Package(mp) => {
                    let span_text = source.get(span.start()..span.end()).unwrap_or("");
                    let byte_in_span = byte_pos.saturating_sub(span.start());
                    let part = crate::part_index_in_dotted_path(span_text, byte_in_span);
                    package_import_part_location(
                        file_path,
                        mp.root.as_str(),
                        &mp.segments,
                        part,
                        file_reader,
                    )?
                }
                ImportSource::LocalBundle(b) if b.members.len() == 1 => {
                    let symbol = b.members.first()?.export.as_str();
                    if let Some(local_path) = &b.local_path {
                        ImportNavLocation::Definition(local_bundle_def_location(
                            file_path,
                            local_path,
                            symbol,
                            file_reader,
                        )?)
                    } else {
                        ImportNavLocation::Definition(dep_bundle_member_def_location(
                            file_path,
                            b,
                            symbol,
                            file_reader,
                        )?)
                    }
                }
                ImportSource::LocalBundle(b)
                    if b.local_path.is_none() && !b.path_segments.is_empty() =>
                {
                    let span_text = source.get(span.start()..span.end()).unwrap_or("");
                    let byte_in_span = byte_pos.saturating_sub(span.start());
                    let part = crate::part_index_in_dotted_path(span_text, byte_in_span);
                    dep_bundle_import_part_location(file_path, b, part, file_reader)?
                }
                ImportSource::LocalMember(m) => {
                    let local_path = m.local_path.as_ref()?;
                    ImportNavLocation::Definition(local_bundle_def_location(
                        file_path,
                        local_path,
                        m.member.export.as_str(),
                        file_reader,
                    )?)
                }
                ImportSource::CurrentModule { member } => {
                    ImportNavLocation::Definition(current_module_sibling_def_location(
                        file_path,
                        member.export.as_str(),
                        file_reader,
                    )?)
                }
                _ => import_source_nav_location(file_path, &source_ref, file_reader)?,
            };

            return Some(CursorDefinition::Import { target, origin });
        }
    }

    None
}
