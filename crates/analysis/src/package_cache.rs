//! Generation-based cache for parsed files and package-level analysis.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Sender;
use notify_debouncer_mini::{
    DebounceEventResult, Debouncer, new_debouncer, notify::RecommendedWatcher,
};

use parser::query::{ParseOutput, SourceParseExt};
use typecheck::transform::{
    PackageTransformArtifacts, PackageTransformOptions, transform_package_with_shared_context,
};
use typecheck::{PackageSemanticIndex, TypedFileAst};

use crate::HoverDocExt;
use crate::engine::{DocumentSnapshot, FileSemanticOutput, HoverContent, SharedSource};
use ast::source::{LineIndex, SourceExt};
use flask::{CompileTarget, FlaskConfig};
use resolve::GinPackageExt;

/// Result from the resolve-and-prepare stage.
struct Stage1Result {
    file_paths: Vec<PathBuf>,
    asts: Vec<ast::FileAst>,
    prepare_diags: Vec<Vec<Diagnostic>>,
    import_diags: HashMap<PathBuf, Vec<Diagnostic>>,
    dependency_dirs: HashMap<String, PathBuf>,
}
use diagnostic::Diagnostic;
use resolve::ParsedFile;

/// Cached output for a fully analyzed package.
pub struct PackageEntry {
    pub typed_asts: Vec<TypedFileAst>,
    pub package_index: PackageSemanticIndex,
    /// File paths in the same order as `typed_asts` (sorted by path).
    pub file_paths: Vec<PathBuf>,
    pub typecheck_outputs: Vec<Vec<Diagnostic>>,
    pub import_diags: HashMap<PathBuf, Vec<Diagnostic>>,
    pub dependency_dirs: HashMap<String, PathBuf>,
    /// Prepend/flaw parse diagnostics + non-flaw prepare diagnostics.
    /// Parse-level diagnostics from `ParseOutput.symptoms` are prepended
    /// to typecheck-prepare diagnostics so they flow through to the LSP.
    /// Stored separately to avoid duplicating type-flaw diagnostic data.
    pub prepare_diags: Vec<Vec<Diagnostic>>,
}

impl PackageEntry {
    /// Build the semantic output for a file by merging typecheck and prepare diagnostics.
    pub fn semantic_output(&self, file_idx: usize) -> FileSemanticOutput {
        let mut symptoms = self.typecheck_outputs[file_idx].clone();
        // Include all prepare/parse diagnostics. Parse-level flaws (like
        // `UnexpectedToken`) are not typecheck flaws so they won't be
        // duplicated. Non-flaw typecheck-prepare diagnostics are also included.
        symptoms.extend(self.prepare_diags[file_idx].clone());
        if let Some(imports) = self.import_diags.get(&self.file_paths[file_idx]) {
            symptoms.extend(imports.clone());
        }
        FileSemanticOutput { symptoms }
    }

    /// Build all diagnostic symptoms (type flaws + non-flaw prepare) for a file.
    pub fn diagnostic_symptoms(&self, file_idx: usize) -> Vec<Diagnostic> {
        self.semantic_output(file_idx).symptoms
    }
}

/// Parsed file entry.
struct ParseEntry {
    /// Source text shared via `Arc` so `source_and_parse()` returns an O(1) clone.
    source: SharedSource,
    parse: Arc<ParseOutput>,
    /// Pre-computed line-start offsets for fast LSP position conversion.
    line_index: Arc<LineIndex>,
}

/// Cache for parsed files and package analysis.
///
/// Packages with `flask.jsonc` are invalidated independently. Files without a
/// package configuration fall back to their parent directory and retain global
/// invalidation so independently registered files cannot reuse a stale entry.
pub struct PackageCache {
    /// File path → (source text, parse output)
    files: Mutex<HashMap<PathBuf, ParseEntry>>,
    /// Package root → cached package entry
    packages: Mutex<HashMap<PathBuf, Arc<PackageEntry>>>,
    /// Package root → reusable parsed dependency modules for import resolution
    module_caches: Mutex<HashMap<PathBuf, resolve::ParsedModuleCache>>,
    /// Package root → import graph from the last import resolution.
    import_graphs: Mutex<HashMap<PathBuf, resolve::ImportDependencyGraph>>,
    /// Package root → generation at which an unconfigured package was computed
    package_gens: Mutex<HashMap<PathBuf, u64>>,
    /// File path → package root (reverse lookup)
    file_pkg: Mutex<HashMap<PathBuf, PathBuf>>,
    /// Generation for unconfigured package fallbacks
    next_gen: AtomicU64,
    /// File watcher for disk changes (kept alive for the lifetime of the cache)
    _watcher: Arc<Mutex<Debouncer<RecommendedWatcher>>>,
}

impl PackageCache {
    pub fn new(tx: Sender<DebounceEventResult>) -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            packages: Mutex::new(HashMap::new()),
            module_caches: Mutex::new(HashMap::new()),
            import_graphs: Mutex::new(HashMap::new()),
            package_gens: Mutex::new(HashMap::new()),
            file_pkg: Mutex::new(HashMap::new()),
            next_gen: AtomicU64::new(1),
            _watcher: Arc::new(Mutex::new(
                new_debouncer(Duration::from_secs(1), tx)
                    .expect("failed to create file watcher debouncer"),
            )),
        }
    }

    /// Register a file by reading it from disk and parsing it.
    ///
    /// If the file does not exist on disk (e.g. LSP adds the file before
    /// receiving its contents), an empty placeholder is used. Call
    /// [`set_contents`](Self::set_contents) to provide the real contents.
    pub fn add_file(&self, path: PathBuf) -> Result<(), String> {
        let (source, parse, line_index) = match std::fs::read_to_string(&path) {
            Ok(src) => {
                let parse = Arc::new(src.parse_source_full());
                let line_index = Arc::new(LineIndex::new(&src));
                (SharedSource::new(src), parse, line_index)
            }
            Err(_) => {
                // File doesn't exist on disk yet — use empty placeholder.
                let src = String::new();
                let parse = Arc::new(src.parse_source_full());
                let line_index = Arc::new(LineIndex::new(&src));
                (SharedSource::new(src), parse, line_index)
            }
        };
        self.files.lock().unwrap().insert(
            path.clone(),
            ParseEntry {
                source,
                parse,
                line_index,
            },
        );
        if let Some(pkg_root) = resolve::find_package_root(&path) {
            self.file_pkg
                .lock()
                .unwrap()
                .insert(path.clone(), pkg_root.clone());
            self.invalidate_cached_packages_for(pkg_root.clone());
            self.package_gens.lock().unwrap().remove(&pkg_root);
        }
        Ok(())
    }

    /// Update a file's contents, re-parse it, and evict its package analysis.
    pub fn set_contents(&self, path: &Path, contents: String) {
        let parse = Arc::new(contents.parse_source_full());
        let line_index = Arc::new(LineIndex::new(&contents));
        self.files.lock().unwrap().insert(
            path.to_path_buf(),
            ParseEntry {
                source: SharedSource::new(contents),
                parse,
                line_index,
            },
        );
        self.next_gen.fetch_add(1, Ordering::SeqCst);
        if let Some(pkg_root) = resolve::find_package_root(path) {
            self.file_pkg
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), pkg_root);
        }
        self.invalidate_cached_packages_for_path(path);
    }

    fn impacted_package_roots(&self, path: &Path) -> HashSet<PathBuf> {
        let mut affected = HashSet::new();

        if let Some(pkg_root) = self.file_pkg.lock().unwrap().get(path).cloned() {
            affected.insert(pkg_root);
        }

        {
            let import_graphs = self.import_graphs.lock().unwrap();
            for (pkg_root, graph) in import_graphs.iter() {
                if !graph.impacted_modules(path).is_empty() {
                    affected.insert(pkg_root.clone());
                }
            }
        }

        if affected.is_empty()
            && let Some(pkg_root) = self.resolve_package_root(path)
        {
            affected.insert(pkg_root);
        }

        affected
    }

    /// Remove resolver/module caches and package entry for a package root.
    fn invalidate_cached_packages_for(&self, pkg_root: PathBuf) {
        self.packages.lock().unwrap().remove(&pkg_root);
        self.import_graphs.lock().unwrap().remove(&pkg_root);
        self.module_caches.lock().unwrap().remove(&pkg_root);
    }

    /// Remove cached output for every package that transitively depends on `path`, plus
    /// the package containing `path` itself when it is known.
    ///
    /// This intentionally preserves parse cache entries: only package-level
    /// typed/diagnostic products are invalidated here because transform/typecheck
    /// remains package-wide.
    fn invalidate_cached_packages_for_path(&self, path: &Path) {
        for pkg_root in self.impacted_package_roots(path) {
            self.invalidate_cached_packages_for(pkg_root);
        }
    }

    /// Get source text and parse output for a file.
    pub fn source_and_parse(&self, path: &Path) -> Option<(SharedSource, Arc<ParseOutput>)> {
        let files = self.files.lock().unwrap();
        files
            .get(path)
            .map(|e| (Arc::clone(&e.source), Arc::clone(&e.parse)))
    }

    /// Get parse output for a file.
    pub fn parse_output(&self, path: &Path) -> Option<Arc<ParseOutput>> {
        let files = self.files.lock().unwrap();
        files.get(path).map(|e| e.parse.clone())
    }

    /// Resolve the package root for a file, caching the result.
    ///
    /// Falls back to the file's parent directory when no `flask.jsonc` is found.
    fn resolve_package_root(&self, path: &Path) -> Option<PathBuf> {
        if let Some(root) = self.file_pkg.lock().unwrap().get(path).cloned() {
            return Some(root);
        }
        let root =
            resolve::find_package_root(path).or_else(|| path.parent().map(|p| p.to_path_buf()));
        if let Some(ref r) = root {
            self.file_pkg
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), r.clone());
        }
        root
    }

    /// Collect all registered files belonging to the same package as `path`.
    fn package_file_paths(&self, path: &Path) -> Option<Vec<PathBuf>> {
        let pkg_root = self.resolve_package_root(path)?;
        let files = self.files.lock().unwrap();
        let file_pkg = self.file_pkg.lock().unwrap();
        let mut pkg_files: Vec<PathBuf> = files
            .keys()
            .filter(|file| file_pkg.get(*file) == Some(&pkg_root))
            .cloned()
            .collect();
        if pkg_files.is_empty() {
            return None;
        }
        pkg_files.sort();
        Some(pkg_files)
    }

    /// Get or compute the package entry for a set of files.
    ///
    /// The package is keyed by its root directory (discovered from the first file
    /// in `paths`). All registered files in that package are included in the
    /// computation, not just the ones in `paths`.
    ///
    /// When no `flask.jsonc` exists (single-file / test mode), the parent
    /// directory of the first file is used as the package root.
    pub fn get_or_compute_package(&self, paths: &[PathBuf]) -> Option<Arc<PackageEntry>> {
        if paths.is_empty() {
            return None;
        }

        let pkg_root = resolve::find_package_root(&paths[0])
            .or_else(|| paths[0].parent().map(|p| p.to_path_buf()))
            // Parent always exists for valid file paths; `paths[0]` is valid.
            .expect("every valid file path has a parent directory");

        let configured = pkg_root.join(flask::PACKAGE_CONFIG_NAME).is_file();
        if configured {
            if let Some(entry) = self.packages.lock().unwrap().get(&pkg_root) {
                return Some(Arc::clone(entry));
            }
        } else {
            let current_gen = self.next_gen.load(Ordering::SeqCst);
            if self.package_gens.lock().unwrap().get(&pkg_root) == Some(&current_gen)
                && let Some(entry) = self.packages.lock().unwrap().get(&pkg_root)
            {
                return Some(Arc::clone(entry));
            }
        }

        // Slow path: compute using all registered files in the package.
        let all_files = self.package_file_paths(&paths[0])?;
        let entry = self.compute_package(&pkg_root, &all_files)?;

        let entry = Arc::new(entry);
        self.packages
            .lock()
            .unwrap()
            .insert(pkg_root.clone(), Arc::clone(&entry));
        if !configured {
            self.package_gens
                .lock()
                .unwrap()
                .insert(pkg_root, self.next_gen.load(Ordering::SeqCst));
        }
        Some(entry)
    }

    /// Run the full pipeline: parse → resolve imports → prepare → transform → diagnostics.
    fn compute_package(&self, pkg_root: &Path, all_files: &[PathBuf]) -> Option<PackageEntry> {
        let Stage1Result {
            file_paths,
            asts,
            prepare_diags,
            import_diags,
            dependency_dirs,
        } = self.stage_resolve_and_prepare(pkg_root, all_files)?;
        let artifacts = self.stage_transform(&asts);
        let typed_asts = artifacts.typed_asts;
        let package_index = self.stage_build_index(&file_paths, &typed_asts);
        let (typecheck_outputs, prepare_diags) =
            self.stage_derive_diagnostics(&typed_asts, &prepare_diags);
        Some(PackageEntry {
            typed_asts,
            package_index,
            file_paths,
            typecheck_outputs,
            import_diags,
            dependency_dirs,
            prepare_diags,
        })
    }

    /// Stage 1: Collect parse results, resolve imports, and run prepare passes.
    fn stage_resolve_and_prepare(
        &self,
        pkg_root: &Path,
        all_files: &[PathBuf],
    ) -> Option<Stage1Result> {
        let files = self.files.lock().unwrap();

        let mut sorted: Vec<(PathBuf, &ParseEntry)> = all_files
            .iter()
            .filter_map(|p| files.get(p).map(|e| (p.clone(), e)))
            .collect();
        if sorted.is_empty() {
            return None;
        }
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        let (file_paths, parse_entries): (Vec<_>, Vec<_>) = sorted.into_iter().unzip();

        let parsed: Vec<ParsedFile> = file_paths
            .iter()
            .zip(parse_entries.iter())
            .map(|(path, entry)| ParsedFile {
                path: path.clone(),
                source: (*entry.source).clone(),
                output: (*entry.parse).clone(),
            })
            .collect();
        let parse_symptoms: Vec<Vec<Diagnostic>> =
            parsed.iter().map(|f| f.output.symptoms.clone()).collect();

        let compile_target = FlaskConfig::find_package_config(pkg_root)
            .and_then(|(config, _)| CompileTarget::resolve(&config, None).ok())
            .unwrap_or(flask::CompileTarget::Library);

        let deps = parsed
            .first()
            .map(|f| f.path.flask_path_dependencies())
            .unwrap_or_default();

        let resolved = if parsed.len() <= 1 && deps.is_empty() {
            parsed
        } else {
            let cache = self
                .module_caches
                .lock()
                .unwrap()
                .remove(pkg_root)
                .unwrap_or_default();
            let resolve::ResolveImportsWithGraph {
                files: resolved,
                cache,
                import_graph,
            } = resolve::resolve_imports_with_graph(parsed, &deps, cache);
            self.module_caches
                .lock()
                .unwrap()
                .insert(pkg_root.to_path_buf(), cache);
            self.import_graphs
                .lock()
                .unwrap()
                .insert(pkg_root.to_path_buf(), import_graph);
            resolved
        };

        let mut by_path: HashMap<PathBuf, resolve::ParsedFile> =
            resolved.into_iter().map(|f| (f.path.clone(), f)).collect();

        let mut ordered: Vec<resolve::ParsedFile> = file_paths
            .iter()
            .filter_map(|p| by_path.remove(p))
            .collect();

        let mut asts: Vec<ast::FileAst> = ordered
            .iter_mut()
            .map(|f| std::mem::take(&mut f.output.ast))
            .collect();
        let mut prepare_diags = typecheck::prepare_package_asts(&mut asts, &compile_target);

        let mut import_diags = HashMap::new();
        for (path, (original_syms, resolved_file)) in
            file_paths.iter().zip(parse_symptoms.iter().zip(ordered.iter()))
        {
            let mut file_import_diags = resolved_file.output.symptoms.clone();
            if file_import_diags.len() >= original_syms.len() {
                file_import_diags.drain(0..original_syms.len());
            } else {
                file_import_diags.clear();
            }
            if !file_import_diags.is_empty() {
                import_diags.insert(path.clone(), file_import_diags);
            }
        }

        // Prepend parse symptoms to prepare diags (same file order)
        for (parse_syms, prep_diags) in parse_symptoms.iter().zip(prepare_diags.iter_mut()) {
            let mut parse_syms = parse_syms.clone();
            parse_syms.append(prep_diags);
            *prep_diags = parse_syms;
        }

        Some(Stage1Result {
            file_paths,
            asts,
            prepare_diags,
            import_diags,
            dependency_dirs: deps,
        })
    }

    /// Stage 2: Run the typed AST transform on prepared ASTs.
    fn stage_transform(&self, asts: &[ast::FileAst]) -> PackageTransformArtifacts {
        transform_package_with_shared_context(asts.to_vec(), PackageTransformOptions::IDE)
    }

    /// Stage 3: Build cross-file package index from typed ASTs.
    ///
    /// `file_paths` must be sorted (they are, from [`PackageEntry`]).
    fn stage_build_index(
        &self,
        file_paths: &[PathBuf],
        typed_asts: &[TypedFileAst],
    ) -> PackageSemanticIndex {
        let typed_refs: Vec<&TypedFileAst> = typed_asts.iter().collect();
        let mut package_index = PackageSemanticIndex::from_typed_asts(&typed_refs);

        // Collect module docs per module path.
        // file_paths are sorted alphabetically, so module docs are naturally ordered.
        let mut module_docs: HashMap<String, Vec<String>> = HashMap::new();

        // Resolve the package config once — all files share the same package root.
        let pkg_config = file_paths
            .first()
            .and_then(|p| p.parent())
            .and_then(FlaskConfig::find_package_config);
        let Some((config, root_dir)) = pkg_config.as_ref() else {
            return package_index;
        };

        for (file, typed) in file_paths.iter().zip(typed_asts.iter()) {
            let Some(parent) = file.parent() else {
                continue;
            };
            let module = config.qualified_name_for(parent, root_dir);

            // Index tags and defs.
            for (tag_id, tag) in &typed.tags {
                package_index
                    .tag_module
                    .entry(tag_id.0)
                    .or_insert_with(|| module.clone());
                package_index.tag_decls.insert(tag_id.0, tag.clone());
            }
            for def_id in typed.defs.keys() {
                package_index
                    .def_module
                    .entry(def_id.0)
                    .or_insert_with(|| module.clone());
            }

            // Collect module-level doc comment.
            if let Some(doc) = typed.eval_ast.module_doc.as_ref() {
                module_docs
                    .entry(module.clone())
                    .or_default()
                    .push(doc.value.clone());
            }

            // Also compute the simple (non-prefixed) module path for import-based lookup.
            // e.g. if qualified name is "root.my_pkg", simple path is "my_pkg".
            if let Ok(rel) = parent.strip_prefix(root_dir)
                && let Some(rel_str) = rel.to_str()
                && !rel_str.is_empty()
            {
                let simple = rel_str.replace('/', ".");
                if let Some(doc) = typed.eval_ast.module_doc.as_ref() {
                    module_docs
                        .entry(simple)
                        .or_default()
                        .push(doc.value.clone());
                }
            }
        }

        // Merge module docs in alphabetical file order (already guaranteed by sorted `file_paths`).
        for (module, docs) in &module_docs {
            package_index
                .module_docs
                .insert(module.clone(), docs.join("\n\n"));
        }

        package_index
    }

    /// Stage 4: Derive typecheck diagnostics and collect prepare diagnostics.
    ///
    /// `semantic_outputs` is NOT stored — it is derived on access from
    /// `typecheck_outputs` + `prepare_diags` to avoid duplicating type-flaw
    /// `Diagnostic` vectors.
    fn stage_derive_diagnostics(
        &self,
        typed_asts: &[TypedFileAst],
        prepare_diags: &[Vec<Diagnostic>],
    ) -> (Vec<Vec<Diagnostic>>, Vec<Vec<Diagnostic>>) {
        let mut typecheck_outputs = Vec::with_capacity(typed_asts.len());
        let mut collected_prepare = Vec::with_capacity(typed_asts.len());

        for (typed, prep_diags) in typed_asts.iter().zip(prepare_diags.iter()) {
            let mut file_diags: Vec<Diagnostic> = Vec::new();
            for (_, flaw) in typed.all_flaws() {
                file_diags.push(flaw.clone());
            }
            typecheck_outputs.push(file_diags);
            collected_prepare.push(prep_diags.clone());
        }

        (typecheck_outputs, collected_prepare)
    }

    /// Get semantic outputs. Results are in sorted-path order.
    pub fn package_semantics(&self, paths: &[PathBuf]) -> Option<Vec<FileSemanticOutput>> {
        let entry = self.get_or_compute_package(paths)?;
        Some(
            (0..entry.file_paths.len())
                .map(|i| entry.semantic_output(i))
                .collect(),
        )
    }

    /// Get merged tag types for all files in the package containing `path`.
    pub fn tag_types_for_file(
        &self,
        path: &Path,
    ) -> Option<HashMap<internment::Intern<String>, ast::ty::Ty>> {
        let pkg_files = self.package_file_paths(path)?;
        let entry = self.get_or_compute_package(&pkg_files)?;
        let mut tag_types = HashMap::new();
        for typed in &entry.typed_asts {
            for (tag_id, ty) in &typed.tag_types {
                tag_types.insert(tag_id.0, ty.clone());
            }
        }
        Some(tag_types)
    }

    /// Get hover content at a byte position.
    pub fn hover(&self, path: &Path, byte_pos: u32) -> Option<HoverContent> {
        let (source, parse) = match self.source_and_parse(path) {
            Some(s) => s,
            None => {
                eprintln!(
                    "[ginlsp] hover: source_and_parse returned None for {:?}",
                    path
                );
                return None;
            }
        };
        let byte_pos = byte_pos as usize;
        let ast = &parse.ast;
        let word = source.word_at_byte_offset(byte_pos).unwrap_or_default();

        // 1. Parse-only hover (keywords, literals)
        // Skip `self` — the typed hover provides the receiver type (e.g. `self Type`,
        // `ref self Type`, `mut self Type`) which is more useful than a generic keyword doc.
        if word != "self"
            && let Some(h) = crate::hover::HoverSnippet::keyword(&source, byte_pos)
        {
            return Some(HoverContent {
                markdown: h.markdown,
                byte_range: h.byte_range,
            });
        }

        // 2. Import hover — fall through to typed_hover when it returns None
        //    (e.g. when the import path references a local folder not in dependencies).
        if let Some(target) = resolve::resolve_import_at(ast, &source, byte_pos)
            && let Some(markdown) =
                resolve::hover_for_import_target(path, target, &|p| resolve::ParsedFile::read(p))
        {
            return Some(HoverContent {
                markdown,
                byte_range: source.word_byte_range(byte_pos).map(|(s, e)| s..e),
            });
        }

        // 3. Typed hover (with package context when possible)
        let result = self.typed_hover(path, &source, byte_pos);
        if result.is_none() {
            // Fall back to keyword hover for `self` if the typed hover didn't resolve.
            if word == "self"
                && let Some(h) = crate::hover::HoverSnippet::keyword(&source, byte_pos)
            {
                return Some(HoverContent {
                    markdown: h.markdown,
                    byte_range: h.byte_range,
                });
            }
            eprintln!(
                "[ginlsp] hover returned None for word={word:?} at byte={byte_pos} in {:?}",
                path
            );
        }
        result
    }

    fn typed_hover(&self, path: &Path, source: &str, byte_pos: usize) -> Option<HoverContent> {
        let (line, character) = source.byte_offset_to_position(byte_pos);

        // Try with full package context (better cross-file hover results).
        if let Some(pkg_files) = self.package_file_paths(path)
            && let Some(entry) = self.get_or_compute_package(&pkg_files)
        {
            for (file_path, typed) in entry.file_paths.iter().zip(entry.typed_asts.iter()) {
                if file_path == path {
                    let hover = typed.hover_at_with_package(
                        source,
                        line,
                        character,
                        Some(&entry.package_index),
                    )?;
                    let markdown = ast::HoverDoc::format_markdown(
                        &hover.markdown,
                        hover.module_prefix.as_deref(),
                    );
                    return Some(HoverContent {
                        markdown,
                        byte_range: source.word_byte_range(byte_pos).map(|(s, e)| s..e),
                    });
                }
            }
        }

        // No package context available for typed hover.
        None
    }

    /// Drop the cached [`PackageEntry`] for the package containing `path`.
    ///
    /// The next request for any file in this package will recompute it.
    /// This is safe to call even if no package entry exists for this path.
    ///
    /// Note: the file itself stays in the `files` map — this only evicts
    /// the expensive package-level typed ASTs and diagnostics.
    pub fn invalidate_package_cache_for(&self, path: &Path) {
        self.invalidate_cached_packages_for_path(path);
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.files.lock().unwrap().contains_key(path)
    }

    /// Get source text, parse output, and line index for a file — all from
    /// a single lock scope so they are guaranteed to be consistent.
    pub fn document_snapshot(&self, path: &Path) -> Option<DocumentSnapshot> {
        let files = self.files.lock().unwrap();
        files.get(path).map(|e| DocumentSnapshot {
            path: path.to_path_buf(),
            source: Arc::clone(&e.source),
            parse: Arc::clone(&e.parse),
            line_index: Arc::clone(&e.line_index),
        })
    }

    pub fn file_paths(&self) -> Vec<PathBuf> {
        self.files.lock().unwrap().keys().cloned().collect()
    }

    /// All package diagnostics (type flaws, prepare, import) in one call.
    ///
    /// Semantic diagnostics are derived on access from `typecheck_outputs` +
    /// `prepare_diags` to avoid duplicating type-flaw diagnostic data.
    pub fn all_diagnostics(
        &self,
        paths: &[PathBuf],
        dependency_dirs: &std::collections::HashMap<String, PathBuf>,
    ) -> std::collections::HashMap<PathBuf, Vec<Diagnostic>> {
        let mut by_path: std::collections::HashMap<PathBuf, Vec<Diagnostic>> =
            std::collections::HashMap::new();
        let _ = dependency_dirs;

        let entry = match self.get_or_compute_package(paths) {
            Some(e) => e,
            None => return by_path,
        };

        for (i, path) in entry.file_paths.iter().enumerate() {
            let symptoms = entry.diagnostic_symptoms(i);
            by_path.entry(path.clone()).or_default().extend(symptoms);
        }

        by_path
    }
}

#[cfg(test)]
mod tests {
    use super::PackageCache;
    use crossbeam_channel::unbounded;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn package_file(root: &std::path::Path, package: &str) -> PathBuf {
        let package_dir = root.join(package);
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(
            package_dir.join("flask.jsonc"),
            format!(r#"{{"name":"{package}","version":"0.0.0","authors":[]}}"#),
        )
        .unwrap();
        let source = package_dir.join("main.gin");
        fs::write(&source, "value := 1\n").unwrap();
        source
    }

    #[test]
    fn editing_one_package_preserves_other_package_cache_entry() {
        let root = std::env::temp_dir().join(format!(
            "gin_package_cache_isolation_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let first_path = package_file(&root, "first");
        let second_path = package_file(&root, "second");

        let (tx, _rx) = unbounded();
        let cache = PackageCache::new(tx);
        cache.add_file(first_path.clone()).unwrap();
        cache.add_file(second_path.clone()).unwrap();

        let first_entry = cache
            .get_or_compute_package(std::slice::from_ref(&first_path))
            .unwrap();
        let second_entry = cache
            .get_or_compute_package(std::slice::from_ref(&second_path))
            .unwrap();

        cache.set_contents(&first_path, "value := 2\n".to_string());

        let updated_first_entry = cache
            .get_or_compute_package(std::slice::from_ref(&first_path))
            .unwrap();
        let reused_second_entry = cache
            .get_or_compute_package(std::slice::from_ref(&second_path))
            .unwrap();

        assert!(!Arc::ptr_eq(&first_entry, &updated_first_entry));
        assert!(Arc::ptr_eq(&second_entry, &reused_second_entry));

        let first_sibling = first_path.parent().unwrap().join("sibling.gin");
        fs::write(&first_sibling, "sibling := 1\n").unwrap();
        cache.add_file(first_sibling.clone()).unwrap();
        let expanded_first_entry = cache
            .get_or_compute_package(std::slice::from_ref(&first_path))
            .unwrap();

        assert!(!Arc::ptr_eq(&updated_first_entry, &expanded_first_entry));
        assert!(expanded_first_entry.file_paths.contains(&first_sibling));

        let nested_path = package_file(&root.join("first"), "nested");
        cache.add_file(nested_path.clone()).unwrap();
        let parent_entry = cache
            .get_or_compute_package(std::slice::from_ref(&first_path))
            .unwrap();
        let nested_entry = cache
            .get_or_compute_package(std::slice::from_ref(&nested_path))
            .unwrap();

        assert!(!parent_entry.file_paths.contains(&nested_path));
        assert_eq!(nested_entry.file_paths, vec![nested_path]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_all_diagnostics_uses_cached_dependency_dirs_and_is_not_duplicated() {
        let root = std::env::temp_dir().join(format!(
            "gin_package_cache_import_diag_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let dep_root = root.join("dep");
        let app_root = root.join("app");
        fs::create_dir_all(&dep_root).unwrap();
        fs::create_dir_all(&app_root).unwrap();

        fs::write(
            dep_root.join("flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        )
        .unwrap();
        fs::write(dep_root.join("exports.gin"), "Exported := 1\n").unwrap();

        fs::write(
            app_root.join("flask.jsonc"),
            format!(
                "{{\"name\":\"app\",\"version\":\"0.0.0\",\"authors\":[],\"dependencies\":{{\"dep\":{{\"path\":\"{}\"}}}}}}",
                dep_root.display()
            ),
        )
        .unwrap();
        let app_main = app_root.join("main.gin");
        fs::write(&app_main, "use dep.Missing\nvalue := 1\n").unwrap();

        let expected_deps = resolve::GinPackageExt::flask_path_dependencies(app_main.as_path());
        assert!(
            !expected_deps.is_empty(),
            "app package should have dependency map"
        );

        let (tx, _rx) = unbounded();
        let cache = PackageCache::new(tx);
        cache.add_file(app_main.clone()).unwrap();

        let entry = cache.get_or_compute_package(std::slice::from_ref(&app_main)).unwrap();
        assert_eq!(entry.dependency_dirs, expected_deps);

        let all_diags =
            cache.all_diagnostics(std::slice::from_ref(&app_main), &std::collections::HashMap::new());
        let no_diags = Vec::new();
        let file_diags = all_diags.get(&app_main).unwrap_or(&no_diags);
        let use_not_exported_count = file_diags
            .iter()
            .filter(|diag| diag.code.slug() == "use-not-exported")
            .count();
        assert_eq!(
            use_not_exported_count,
            1,
            "invalid import diagnostic should be emitted exactly once: {file_diags:?}"
        );
    }

    #[test]
    fn package_cache_reuses_dependency_modules_and_is_evicted_on_app_edit() {
        let root = std::env::temp_dir().join(format!(
            "gin_package_cache_dependency_reuse_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let app_root = root.join("app");
        let dep_root = root.join("dep");
        fs::create_dir_all(&app_root).unwrap();
        fs::create_dir_all(&dep_root).unwrap();

        fs::write(
            app_root.join("flask.jsonc"),
            format!(
                "{{\"name\":\"app\",\"version\":\"0.0.0\",\"authors\":[],\"dependencies\":{{\"dep\":{{\"path\":\"{}\"}}}}}}",
                dep_root.display()
            ),
        )
        .unwrap();
        fs::write(
            dep_root.join("flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        )
        .unwrap();
        fs::write(dep_root.join("used.gin"), "Used is 1\n").unwrap();
        fs::write(dep_root.join("unused.gin"), "Unused := 1\n").unwrap();

        let app_gin = app_root.join("app_main.gin");
        fs::write(&app_gin, "use dep.Used\nvalue := 1\n").unwrap();

        let unused_dep_path = dep_root.join("unused.gin");
        let used_dep_path = dep_root.join("used.gin");

        let app_deps = resolve::GinPackageExt::flask_path_dependencies(app_gin.as_path());
        assert!(
            !app_deps.is_empty(),
            "app root flask dependencies are not configured"
        );

        let (tx, _rx) = unbounded();
        let cache = PackageCache::new(tx);
        cache.add_file(app_gin.clone()).unwrap();

        let _first_entry = cache
            .get_or_compute_package(std::slice::from_ref(&app_gin))
            .unwrap();
        let first_used_source = {
            let caches = cache.module_caches.lock().unwrap();
            let cache_entry = caches
                .values()
                .find(|entry| entry.has_file(&used_dep_path))
                .expect("app dependency cache exists after initial analysis");
            cache_entry
                .get_file(&used_dep_path)
                .expect("used dependency module parsed")
                .source
                .clone()
        };

        fs::write(&unused_dep_path, "Unused := if\n").unwrap();
        let _second_entry = cache
            .get_or_compute_package(std::slice::from_ref(&app_gin))
            .unwrap();

        let second_used_source = {
            let caches = cache.module_caches.lock().unwrap();
            let cache_entry = caches
                .values()
                .find(|entry| entry.has_file(&used_dep_path))
                .expect("app dependency cache still exists after unchanged app inputs");
            cache_entry
                .get_file(&used_dep_path)
                .expect("used dependency module still in cache")
                .source
                .clone()
        };

        let first_import_graph_imports_dep = {
            let graphs = cache.import_graphs.lock().unwrap();
            let graph = graphs
                .get(&app_root)
                .expect("dependency graph is available after resolve");
            !graph.impacted_modules(&used_dep_path).is_empty()
        };

        let first_entry_symptoms = cache
            .package_semantics(std::slice::from_ref(&app_gin))
            .unwrap();
        let second_entry_symptoms = cache
            .package_semantics(std::slice::from_ref(&app_gin))
            .unwrap();

        assert_eq!(first_used_source, second_used_source);
        assert!(first_import_graph_imports_dep);
        assert_eq!(first_entry_symptoms, second_entry_symptoms);

        cache.set_contents(&app_gin, "use dep.Used\nvalue := 2\n".to_string());
        assert!(cache.module_caches.lock().unwrap().is_empty());

        let after_app_edit = cache
            .get_or_compute_package(std::slice::from_ref(&app_gin))
            .unwrap();
        assert!(!cache.module_caches.lock().unwrap().is_empty());

        fs::write(&used_dep_path, "Used is 99\n").unwrap();
        cache.set_contents(&used_dep_path, "Used is 99\n".to_string());

        let after_dependency_edit = cache
            .get_or_compute_package(std::slice::from_ref(&app_gin))
            .unwrap();
        assert!(!Arc::ptr_eq(&after_app_edit, &after_dependency_edit));

        let dep_parse_after_edit = {
            let caches = cache.module_caches.lock().unwrap();
            let cache_entry = caches
                .values()
                .find(|entry| entry.has_file(&used_dep_path))
                .expect("app dependency cache exists after dependency edit");
            cache_entry
                .get_file(&used_dep_path)
                .expect("dependency module still reparsed after dependency edit")
                .source
                .clone()
        };

        assert!(dep_parse_after_edit.contains("99"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_invalidation_follows_import_edges() {
        let root = std::env::temp_dir().join(format!(
            "gin_package_cache_import_edges_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dep_root = root.join("dep");
        let app_a_root = root.join("app_a");
        let app_b_root = root.join("app_b");
        fs::create_dir_all(&dep_root).unwrap();
        fs::create_dir_all(&app_a_root).unwrap();
        fs::create_dir_all(&app_b_root).unwrap();

        fs::write(
            dep_root.join("flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        )
        .unwrap();
        fs::write(dep_root.join("used.gin"), "Used is 1\n").unwrap();
        fs::write(dep_root.join("unused.gin"), "Unused is 1\n").unwrap();

        fs::write(
            app_a_root.join("flask.jsonc"),
            format!(
                "{{\"name\":\"app_a\",\"version\":\"0.0.0\",\"authors\":[],\"dependencies\":{{\"dep\":{{\"path\":\"{}\"}}}}}}",
                dep_root.display()
            ),
        )
        .unwrap();
        fs::write(
            app_b_root.join("flask.jsonc"),
            format!(
                "{{\"name\":\"app_b\",\"version\":\"0.0.0\",\"authors\":[],\"dependencies\":{{\"dep\":{{\"path\":\"{}\"}}}}}}",
                dep_root.display()
            ),
        )
        .unwrap();
        fs::write(app_a_root.join("main.gin"), "use dep.Used\nvalue := 1\n").unwrap();
        fs::write(app_b_root.join("main.gin"), "use dep.Unused\nvalue := 1\n").unwrap();

        let dep_used = dep_root.join("used.gin");
        let dep_unused = dep_root.join("unused.gin");
        let app_a_main = app_a_root.join("main.gin");
        let app_b_main = app_b_root.join("main.gin");

        let (tx, _rx) = unbounded();
        let cache = PackageCache::new(tx);
        cache.add_file(app_a_main.clone()).unwrap();
        cache.add_file(app_b_main.clone()).unwrap();
        cache.add_file(dep_used.clone()).unwrap();
        cache.add_file(dep_unused.clone()).unwrap();

        let app_a_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_a_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_a_root])
        };
        let app_b_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_b_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_b_root])
        };

        cache.set_contents(&dep_unused, "Unused is 2\n".to_string());

        let app_a_entry_after_unused = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_a_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_a_root])
        };
        let app_b_entry_after_unused = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_b_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_b_root])
        };

        assert!(Arc::ptr_eq(&app_a_entry_before, &app_a_entry_after_unused));
        assert!(!Arc::ptr_eq(&app_b_entry_before, &app_b_entry_after_unused));

        cache.set_contents(&dep_used, "Used is 3\n".to_string());

        let app_a_entry_after_used = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_a_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_a_root])
        };
        let app_b_entry_after_used = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_b_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_b_root])
        };

        assert!(!Arc::ptr_eq(
            &app_a_entry_after_unused,
            &app_a_entry_after_used
        ));
        assert!(Arc::ptr_eq(
            &app_b_entry_after_unused,
            &app_b_entry_after_used
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_invalidate_package_cache_for_follows_import_edges() {
        let root = std::env::temp_dir().join(format!(
            "gin_package_cache_invalidate_api_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dep_root = root.join("dep");
        let app_root = root.join("app");
        let other_root = root.join("other");
        fs::create_dir_all(&dep_root).unwrap();
        fs::create_dir_all(&app_root).unwrap();
        fs::create_dir_all(&other_root).unwrap();

        fs::write(
            dep_root.join("flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        )
        .unwrap();
        fs::write(dep_root.join("used.gin"), "Used is 1\n").unwrap();
        fs::write(dep_root.join("unused.gin"), "Unused is 1\n").unwrap();

        fs::write(
            app_root.join("flask.jsonc"),
            format!(
                "{{\"name\":\"app\",\"version\":\"0.0.0\",\"authors\":[],\"dependencies\":{{\"dep\":{{\"path\":\"{}\"}}}}}}",
                dep_root.display()
            ),
        )
        .unwrap();
        fs::write(
            other_root.join("flask.jsonc"),
            "{\"name\":\"other\",\"version\":\"0.0.0\",\"authors\":[]}",
        )
        .unwrap();
        fs::write(app_root.join("main.gin"), "use dep.Used\nvalue := 1\n").unwrap();
        fs::write(other_root.join("main.gin"), "value := 1\n").unwrap();

        let dep_used = dep_root.join("used.gin");
        let app_main = app_root.join("main.gin");
        let other_main = other_root.join("main.gin");

        let (tx, _rx) = unbounded();
        let cache = PackageCache::new(tx);
        cache.add_file(app_main.clone()).unwrap();
        cache.add_file(other_main.clone()).unwrap();
        cache.add_file(dep_used.clone()).unwrap();

        let app_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_root])
        };
        let other_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&other_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&other_root])
        };

        cache.invalidate_package_cache_for(&dep_used);

        let app_entry_after = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_root])
        };
        let other_entry_after = {
            cache
                .get_or_compute_package(std::slice::from_ref(&other_main))
                .unwrap();
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&other_root])
        };

        assert!(!Arc::ptr_eq(&app_entry_before, &app_entry_after));
        assert!(Arc::ptr_eq(&other_entry_before, &other_entry_after));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_preserves_parse_cache_across_package_invalidation() {
        let root = std::env::temp_dir().join(format!(
            "gin_package_cache_parse_preserved_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dep_root = root.join("dep");
        let app_root = root.join("app");
        fs::create_dir_all(&dep_root).unwrap();
        fs::create_dir_all(&app_root).unwrap();

        fs::write(
            dep_root.join("flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        )
        .unwrap();
        fs::write(dep_root.join("used.gin"), "Used is 1\n").unwrap();

        fs::write(
            app_root.join("flask.jsonc"),
            format!(
                "{{\"name\":\"app\",\"version\":\"0.0.0\",\"authors\":[],\"dependencies\":{{\"dep\":{{\"path\":\"{}\"}}}}}}",
                dep_root.display()
            ),
        )
        .unwrap();
        fs::write(app_root.join("main.gin"), "use dep.Used\nvalue := 1\n").unwrap();

        let dep_used = dep_root.join("used.gin");
        let app_main = app_root.join("main.gin");
        let unused = app_root.join("unused.gin");
        fs::write(&unused, "unused := 1\n").unwrap();

        let (tx, _rx) = unbounded();
        let cache = PackageCache::new(tx);
        cache.add_file(app_main.clone()).unwrap();
        cache.add_file(unused.clone()).unwrap();
        cache.add_file(dep_used.clone()).unwrap();

        let _ = cache
            .get_or_compute_package(std::slice::from_ref(&app_main))
            .unwrap();

        let dep_parse_before = cache.parse_output(&dep_used).unwrap();
        let app_parse_before = cache.parse_output(&app_main).unwrap();

        cache.set_contents(&app_main, "use dep.Used\nvalue := 2\n".to_string());
        let _ = cache
            .get_or_compute_package(std::slice::from_ref(&app_main))
            .unwrap();

        let dep_parse_after = cache.parse_output(&dep_used).unwrap();
        let app_parse_after = cache.parse_output(&app_main).unwrap();

        assert!(Arc::ptr_eq(&dep_parse_before, &dep_parse_after));
        assert!(!Arc::ptr_eq(&app_parse_before, &app_parse_after));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_invalidation_contract_reuses_parse_but_rebuilds_package_phase() {
        let root = std::env::temp_dir().join(format!(
            "gin_package_cache_invalidation_contract_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dep_root = root.join("dep");
        let app_root = root.join("app");
        fs::create_dir_all(&dep_root).unwrap();
        fs::create_dir_all(&app_root).unwrap();

        fs::write(
            dep_root.join("flask.jsonc"),
            r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
        )
        .unwrap();
        fs::write(dep_root.join("used.gin"), "Used is 1\n").unwrap();

        fs::write(
            app_root.join("flask.jsonc"),
            format!(
                "{{\"name\":\"app\",\"version\":\"0.0.0\",\"authors\":[],\"dependencies\":{{\"dep\":{{\"path\":\"{}\"}}}}}}",
                dep_root.display()
            ),
        )
        .unwrap();
        fs::write(app_root.join("main.gin"), "use dep.Used\nvalue := 1\n").unwrap();

        let dep_used = dep_root.join("used.gin");
        let app_main = app_root.join("main.gin");

        let (tx, _rx) = unbounded();
        let cache = PackageCache::new(tx);
        cache.add_file(app_main.clone()).unwrap();
        cache.add_file(dep_used.clone()).unwrap();

        let _ = cache
            .get_or_compute_package(std::slice::from_ref(&app_main))
            .unwrap();

        let parsed_dep_before = cache.parse_output(&dep_used).unwrap();
        let module_cache_before = cache
            .module_caches
            .lock()
            .unwrap()
            .get(&app_root)
            .and_then(|c| c.get_file(&dep_used).map(|f| Arc::new(f.source.clone())));
        assert!(module_cache_before.is_some());

        cache.set_contents(&app_main, "use dep.Used\nvalue := 2\n".to_string());

        assert!(!cache.module_caches.lock().unwrap().contains_key(&app_root));
        assert!(!cache.import_graphs.lock().unwrap().contains_key(&app_root));

        let _ = cache
            .get_or_compute_package(std::slice::from_ref(&app_main))
            .unwrap();

        let parsed_dep_after = cache.parse_output(&dep_used).unwrap();
        let module_cache_after = cache
            .module_caches
            .lock()
            .unwrap()
            .get(&app_root)
            .and_then(|c| c.get_file(&dep_used).map(|f| Arc::new(f.source.clone())));

        assert!(cache.module_caches.lock().unwrap().contains_key(&app_root));
        assert!(cache.import_graphs.lock().unwrap().contains_key(&app_root));
        assert!(Arc::ptr_eq(&parsed_dep_before, &parsed_dep_after));
        assert!(
            module_cache_after.is_some(),
            "dependency module cache is rebuilt after package invalidation"
        );
        assert!(module_cache_before.is_some());
        assert!(!Arc::ptr_eq(
            &module_cache_before.unwrap(),
            &module_cache_after.unwrap()
        ));

        let _ = fs::remove_dir_all(root);
    }
}
