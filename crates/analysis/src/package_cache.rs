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
use typecheck::{
    CompletionCandidate, PackageSemanticIndex, TypedFileAst, completions::FileAstCompletionExt,
};

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
use resolve::{CursorDefinition, ParsedFile};

use crate::cursor::ReferencesInFile;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
enum PackageIdentity {
    Configured { root: PathBuf },
    AdHoc(AdHocPackageId),
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct AdHocPackageId(u64);

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
    package: PackageIdentity,
}

/// Cache for parsed files and package analysis.
pub struct PackageCache {
    /// File path → (source text, parse output)
    files: Mutex<HashMap<PathBuf, ParseEntry>>,
    /// Package identity → cached package entry.
    packages: Mutex<HashMap<PackageIdentity, Arc<PackageEntry>>>,
    /// Package identity → reusable parsed dependency modules for import resolution.
    module_caches: Mutex<HashMap<PackageIdentity, resolve::ParsedModuleCache>>,
    /// Package identity → import graph from the last import resolution.
    import_graphs: Mutex<HashMap<PackageIdentity, resolve::ImportDependencyGraph>>,
    next_package_id: AtomicU64,
    /// File watcher for disk changes (kept alive for the lifetime of the cache)
    _watcher: Arc<Mutex<Debouncer<RecommendedWatcher>>>,
}

impl PackageCache {
    pub fn new_adhoc_package(&self) -> AdHocPackageId {
        let id = self.next_package_id.fetch_add(1, Ordering::SeqCst);
        if id == 0 {
            AdHocPackageId(self.next_package_id.fetch_add(1, Ordering::SeqCst))
        } else {
            AdHocPackageId(id)
        }
    }

    fn identity_for_path(path: &Path) -> PackageIdentity {
        match resolve::find_package_root(path) {
            Some(root) => PackageIdentity::Configured { root },
            None => PackageIdentity::AdHoc(AdHocPackageId(0)),
        }
    }

    fn package_files_for_identity(&self, package: &PackageIdentity) -> Vec<PathBuf> {
        let files = self.files.lock().unwrap();
        let mut package_files: Vec<PathBuf> = files
            .iter()
            .filter_map(|(path, entry)| (entry.package == *package).then_some(path.clone()))
            .collect();

        package_files.sort();
        package_files
    }

    fn file_identity(&self, path: &Path) -> Option<PackageIdentity> {
        self.files
            .lock()
            .unwrap()
            .get(path)
            .map(|entry| entry.package.clone())
    }

    fn set_parsed_file(&self, path: PathBuf, contents: String, package: PackageIdentity) {
        let parse = Arc::new(contents.parse_source_full());
        let line_index = Arc::new(LineIndex::new(&contents));

        let old_package = {
            let mut files = self.files.lock().unwrap();
            let previous = files.insert(
                path,
                ParseEntry {
                    source: SharedSource::new(contents),
                    parse,
                    line_index,
                    package: package.clone(),
                },
            );
            previous.map(|entry| entry.package)
        };

        if let Some(old_package) = old_package
            && old_package != package
        {
            self.invalidate_cached_packages_for(old_package);
        }
        self.invalidate_cached_packages_for(package);
    }

    pub fn new(tx: Sender<DebounceEventResult>) -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            packages: Mutex::new(HashMap::new()),
            module_caches: Mutex::new(HashMap::new()),
            import_graphs: Mutex::new(HashMap::new()),
            next_package_id: AtomicU64::new(1),
            _watcher: Arc::new(Mutex::new(
                new_debouncer(Duration::from_secs(1), tx)
                    .expect("failed to create file watcher debouncer"),
            )),
        }
    }

    /// Test-friendly constructor: wires an unbounded channel whose receiver is
    /// dropped, so debounced watcher events are discarded.
    pub fn for_test() -> Self {
        let (tx, _rx) = crossbeam_channel::unbounded();
        Self::new(tx)
    }

    /// Register a file by reading it from disk and parsing it.
    ///
    /// If the file does not exist on disk (e.g. LSP adds the file before
    /// receiving its contents), an empty placeholder is used. Call
    /// [`set_contents`](Self::set_contents) to provide the real contents.
    pub fn add_file(&self, path: PathBuf) -> Result<(), String> {
        let source = std::fs::read_to_string(&path).unwrap_or_default();

        let package = match self.file_identity(&path) {
            Some(known) => known,
            None => {
                let mut identity = Self::identity_for_path(&path);
                if matches!(identity, PackageIdentity::AdHoc(_)) {
                    identity = PackageIdentity::AdHoc(self.new_adhoc_package());
                }
                identity
            }
        };

        self.set_parsed_file(path, source, package);
        Ok(())
    }

    pub fn add_file_to_package(
        &self,
        path: PathBuf,
        adhoc_package: AdHocPackageId,
    ) -> Result<(), String> {
        if self.file_identity(&path).is_some() {
            return Err("file already registered".to_string());
        }

        let source = std::fs::read_to_string(&path).unwrap_or_default();
        self.set_parsed_file(path, source, PackageIdentity::AdHoc(adhoc_package));
        Ok(())
    }

    /// Update a file's contents, re-parse it, and evict its package analysis.
    pub fn set_contents(&self, path: &Path, contents: String) {
        let package = match self.file_identity(path) {
            Some(identity) => identity,
            None => {
                let mut identity = Self::identity_for_path(path);
                if matches!(identity, PackageIdentity::AdHoc(_)) {
                    identity = PackageIdentity::AdHoc(self.new_adhoc_package());
                }
                identity
            }
        };

        self.set_parsed_file(path.to_path_buf(), contents, package);
        self.invalidate_package_cache_for(path);
    }

    fn impacted_package_roots(&self, path: &Path) -> HashSet<PackageIdentity> {
        let mut affected = HashSet::new();

        if let Some(identity) = self.file_identity(path) {
            affected.insert(identity);
        }

        {
            let import_graphs = self.import_graphs.lock().unwrap();
            for (package, graph) in import_graphs.iter() {
                if !graph.impacted_modules(path).is_empty() {
                    affected.insert(package.clone());
                }
            }
        }

        affected
    }

    /// Remove resolver/module caches and package entry for a package root.
    fn invalidate_cached_packages_for(&self, package: PackageIdentity) {
        self.packages.lock().unwrap().remove(&package);
        self.import_graphs.lock().unwrap().remove(&package);
        self.module_caches.lock().unwrap().remove(&package);
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

    /// Collect all registered files belonging to the same package as `path`.
    fn package_file_paths(&self, path: &Path) -> Option<Vec<PathBuf>> {
        let package = self.file_identity(path)?;
        let pkg_files = self.package_files_for_identity(&package);
        if pkg_files.is_empty() {
            return None;
        }
        Some(pkg_files)
    }

    /// Get or compute the package entry for a set of files.
    pub fn get_or_compute_package(&self, paths: &[PathBuf]) -> Option<Arc<PackageEntry>> {
        if paths.is_empty() {
            return None;
        }

        let package = self.file_identity(&paths[0])?;

        if let Some(entry) = self.packages.lock().unwrap().get(&package) {
            return Some(Arc::clone(entry));
        }

        // Slow path: compute using all registered files in the package.
        let all_files = self.package_file_paths(&paths[0])?;
        let entry = self.compute_package(&package, &all_files)?;

        let entry = Arc::new(entry);
        self.packages
            .lock()
            .unwrap()
            .insert(package, Arc::clone(&entry));
        Some(entry)
    }

    /// Run the full pipeline: parse → resolve imports → prepare → transform → diagnostics.
    fn compute_package(
        &self,
        package: &PackageIdentity,
        all_files: &[PathBuf],
    ) -> Option<PackageEntry> {
        let Stage1Result {
            file_paths,
            asts,
            prepare_diags,
            import_diags,
            dependency_dirs,
        } = self.stage_resolve_and_prepare(package, all_files)?;
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
        package: &PackageIdentity,
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

        let compile_target = match package {
            PackageIdentity::Configured { root } => FlaskConfig::find_package_config(root)
                .and_then(|(config, _)| CompileTarget::resolve(&config, None).ok())
                .unwrap_or(flask::CompileTarget::Library),
            PackageIdentity::AdHoc(_) => flask::CompileTarget::Library,
        };

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
                .remove(package)
                .unwrap_or_default();
            let resolve::ResolveImportsWithGraph {
                files: resolved,
                cache,
                import_graph,
            } = resolve::resolve_imports_with_graph(parsed, &deps, cache);
            self.module_caches
                .lock()
                .unwrap()
                .insert(package.clone(), cache);
            self.import_graphs
                .lock()
                .unwrap()
                .insert(package.clone(), import_graph);
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
        let mut prepare_diags: Vec<Vec<Diagnostic>> = asts
            .iter_mut()
            .map(|ast| typecheck::prepare_parse_ast(ast, &compile_target))
            .collect();

        let mut import_diags = HashMap::new();
        for (path, (original_syms, resolved_file)) in file_paths
            .iter()
            .zip(parse_symptoms.iter().zip(ordered.iter()))
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

    /// Get semantic outputs in caller-requested path order.
    pub fn package_semantics(&self, paths: &[PathBuf]) -> Vec<FileSemanticOutput> {
        let Some(entry) = self.get_or_compute_package(paths) else {
            return vec![FileSemanticOutput::default(); paths.len()];
        };
        if paths.is_empty() {
            return Vec::new();
        }
        let mut by_path = HashMap::with_capacity(entry.file_paths.len());
        for (fp, out) in entry
            .file_paths
            .iter()
            .zip((0..entry.file_paths.len()).map(|i| entry.semantic_output(i)))
        {
            by_path.insert(fp.clone(), out);
        }
        paths
            .iter()
            .map(|path| by_path.get(path).cloned().unwrap_or_default())
            .collect()
    }

    /// Get completions at a byte position in a source file.
    pub fn completions_at(&self, path: &Path, byte_pos: u32) -> Vec<CompletionCandidate> {
        let Some((source, output)) = self.source_and_parse(path) else {
            return Vec::new();
        };
        let byte_pos = byte_pos as usize;
        let tag_types = self.tag_types_for_file(path);
        output
            .ast
            .completions_for_context(&source, byte_pos, tag_types.as_ref())
    }

    /// Find references to the symbol at `byte_pos` in a source file.
    pub fn references_at(&self, path: &Path, byte_pos: u32) -> Option<ReferencesInFile> {
        let (source, output) = self.source_and_parse(path)?;
        let byte_pos = byte_pos as usize;
        let word = output
            .ast
            .word_at_byte(byte_pos, &source)
            .or_else(|| source.word_at_byte_offset(byte_pos))?;
        let spans = output.ast.find_references(&word);
        if spans.is_empty() {
            return None;
        }
        Some(ReferencesInFile { spans })
    }

    /// Resolve definition location for symbol at a byte position in a file.
    pub fn goto_definition(&self, path: &Path, byte_pos: u32) -> Option<CursorDefinition> {
        let (source, parse) = self.source_and_parse(path)?;
        resolve::cursor_definition(path, &parse.ast, &source, byte_pos as usize, &|p| {
            resolve::ParsedFile::read(p)
        })
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

    /// Drop the cached [`PackageEntry`] for every package that transitively depends on `path`,
    /// plus the package containing `path` itself when it is known.
    ///
    /// Parse cache entries are preserved — only package-level typed ASTs and
    /// diagnostics are evicted because transform/typecheck remains package-wide.
    /// The next request for any file in this package recomputes the entry; safe
    /// to call even if no package entry exists for this path.
    pub fn invalidate_package_cache_for(&self, path: &Path) {
        for package in self.impacted_package_roots(path) {
            self.invalidate_cached_packages_for(package);
        }
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
    ) -> std::collections::HashMap<PathBuf, Vec<Diagnostic>> {
        let mut by_path: std::collections::HashMap<PathBuf, Vec<Diagnostic>> =
            std::collections::HashMap::new();

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
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp_root(suffix: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("gin_package_cache_{suffix}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

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

    fn package_identity(cache: &PackageCache, path: &std::path::Path) -> super::PackageIdentity {
        cache
            .file_identity(path)
            .expect("registered file must have a package identity")
    }

    #[test]
    fn package_cache_semantics_preserve_requested_path_order() {
        let root = temp_root("order");

        let pkg_root = root.join("pkg");
        fs::create_dir_all(&pkg_root).unwrap();
        fs::write(
            pkg_root.join("flask.jsonc"),
            r#"{"name":"pkg","version":"0.0.0","authors":[]}"#,
        )
        .unwrap();

        let first = pkg_root.join("first.gin");
        let second = pkg_root.join("second.gin");
        fs::write(&first, "foo() Int := unknown_one\n").unwrap();
        fs::write(&second, "bar() Int := unknown_two\n").unwrap();

        let cache = PackageCache::for_test();
        cache.add_file(first.clone()).unwrap();
        cache.add_file(second.clone()).unwrap();
        cache.set_contents(&first, fs::read_to_string(&first).unwrap());
        cache.set_contents(&second, fs::read_to_string(&second).unwrap());

        let ordered = cache.package_semantics(&[second.clone(), first.clone()]);
        let first_msgs: Vec<_> = ordered[1]
            .symptoms
            .iter()
            .map(|d| d.message.clone())
            .collect();
        let second_msgs: Vec<_> = ordered[0]
            .symptoms
            .iter()
            .map(|d| d.message.clone())
            .collect();
        assert!(
            first_msgs.iter().any(|m| m.contains("unknown_one")),
            "second request entry should map to first path diagnostics"
        );
        assert!(
            second_msgs.iter().any(|m| m.contains("unknown_two")),
            "first request entry should map to second path diagnostics"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_file_paths_contains_exact_registered_files() {
        let root = temp_root("registry");

        let path = root.join("main.gin");
        let updated = root.join("main_updated.gin");
        fs::write(&path, "value := 1\n").unwrap();

        let cache = PackageCache::for_test();

        cache.add_file(path.clone()).unwrap();
        assert!(cache.contains(&path));
        assert!(cache.file_paths().contains(&path));

        cache.set_contents(&path, "value := 2\n".to_string());
        assert!(cache.contains(&path));
        assert!(cache.file_paths().contains(&path));

        assert!(cache.file_paths().iter().all(|p| cache.contains(p)));

        assert!(!cache.contains(&updated));
        assert!(!cache.file_paths().contains(&updated));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_semantics_omit_unregistered_requested_files() {
        let root = temp_root("unknown");

        let path = root.join("present.gin");
        let missing = root.join("missing.gin");
        fs::write(&path, "value := 1\n").unwrap();

        let cache = PackageCache::for_test();
        cache.add_file(path.clone()).unwrap();
        cache.set_contents(&path, fs::read_to_string(&path).unwrap());

        let present_only = cache.package_semantics(std::slice::from_ref(&path));
        let outputs = cache.package_semantics(&[path.clone(), missing.clone()]);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0], present_only[0]);
        assert!(
            outputs[1].symptoms.is_empty(),
            "unknown requested path should return default output"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_unconfigured_files_require_explicit_grouping() {
        let root = temp_root("unconfigured");

        let first = root.join("first.gin");
        let second = root.join("second.gin");
        fs::write(&first, "value := 1\n").unwrap();
        fs::write(&second, "value := 2\n").unwrap();

        let cache = PackageCache::for_test();

        cache.add_file(first.clone()).unwrap();
        cache.add_file(second.clone()).unwrap();

        let first_entry = cache
            .get_or_compute_package(std::slice::from_ref(&first))
            .unwrap();
        let second_entry = cache
            .get_or_compute_package(std::slice::from_ref(&second))
            .unwrap();

        assert_eq!(first_entry.file_paths, vec![first.clone()]);
        assert_eq!(second_entry.file_paths, vec![second.clone()]);

        let grouped = cache.new_adhoc_package();
        let group_root = root.join("group.gin");
        let other = root.join("group_other.gin");
        fs::write(&group_root, "value := 3\n").unwrap();
        fs::write(&other, "value := 4\n").unwrap();
        cache
            .add_file_to_package(group_root.clone(), grouped)
            .unwrap();
        cache.add_file_to_package(other.clone(), grouped).unwrap();

        let grouped_entry = cache
            .get_or_compute_package(std::slice::from_ref(&group_root))
            .unwrap();
        assert!(grouped_entry.file_paths.contains(&group_root));
        assert!(grouped_entry.file_paths.contains(&other));

        let reverse = cache
            .package_file_paths(&group_root)
            .expect("grouped paths should be discoverable");
        let mut reverse_expected = vec![group_root.clone(), other.clone()];
        reverse_expected.sort();
        assert_eq!(reverse, reverse_expected);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_unconfigured_query_order_does_not_change_membership() {
        let root = temp_root("unconfigured_order");

        let first = root.join("first.gin");
        let second = root.join("second.gin");
        fs::write(&first, "value := 1\n").unwrap();
        fs::write(&second, "value := 2\n").unwrap();

        let cache = PackageCache::for_test();
        let pkg = cache.new_adhoc_package();
        cache.add_file_to_package(first.clone(), pkg).unwrap();
        cache.add_file_to_package(second.clone(), pkg).unwrap();

        let first_entry = cache
            .get_or_compute_package(std::slice::from_ref(&first))
            .unwrap();
        let second_entry = cache
            .get_or_compute_package(std::slice::from_ref(&second))
            .unwrap();

        let mut expected = vec![first.clone(), second.clone()];
        expected.sort();
        assert_eq!(first_entry.file_paths, expected);
        assert_eq!(second_entry.file_paths, expected);

        let entry_from_grouped_paths = cache
            .get_or_compute_package(&[second.clone(), first.clone()])
            .unwrap();
        assert_eq!(entry_from_grouped_paths.file_paths, expected);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_shared_handle_sees_updates_across_clones() {
        let root = temp_root("handle_share");

        let path = root.join("shared.gin");
        fs::write(&path, "value := 1\n").unwrap();

        let cache = std::sync::Arc::new(PackageCache::for_test());
        let other = std::sync::Arc::clone(&cache);

        cache.add_file(path.clone()).unwrap();

        cache.set_contents(&path, "value := 10\n".to_string());
        let doc = other.document_snapshot(&path).unwrap();
        assert_eq!(doc.source.as_str(), "value := 10\n");

        other.set_contents(&path, "value := 20\n".to_string());
        let doc = cache.document_snapshot(&path).unwrap();
        assert_eq!(doc.source.as_str(), "value := 20\n");

        assert_eq!(
            cache.package_semantics(std::slice::from_ref(&path)).len(),
            1
        );
        let docs = other.document_snapshot(&path).unwrap();
        assert_eq!(doc.path, docs.path);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn editing_one_package_preserves_other_package_cache_entry() {
        let root = temp_root("isolation");
        let first_path = package_file(&root, "first");
        let second_path = package_file(&root, "second");

        let cache = PackageCache::for_test();
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
    fn package_cache_unconfigured_siblings_do_not_merge_by_parent() {
        let root = temp_root("unconfigured_isolation");

        let first = root.join("first.gin");
        let second = root.join("second.gin");
        fs::write(&first, "value := 1\n").unwrap();
        fs::write(&second, "value := 2\n").unwrap();

        let cache = PackageCache::for_test();
        let group = cache.new_adhoc_package();

        cache.add_file_to_package(first.clone(), group).unwrap();
        cache
            .add_file_to_package(second.clone(), cache.new_adhoc_package())
            .unwrap();

        let first_entry = cache
            .get_or_compute_package(std::slice::from_ref(&first))
            .unwrap();
        let second_entry = cache
            .get_or_compute_package(std::slice::from_ref(&second))
            .unwrap();

        assert_eq!(first_entry.file_paths, vec![first.clone()]);
        assert_eq!(second_entry.file_paths, vec![second.clone()]);
        assert_ne!(
            first_entry.file_paths, second_entry.file_paths,
            "unrelated ad-hoc packages should remain isolated"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_cache_all_diagnostics_uses_cached_dependency_dirs_and_is_not_duplicated() {
        let root = temp_root("import_diag");

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

        let cache = PackageCache::for_test();
        cache.add_file(app_main.clone()).unwrap();

        let entry = cache
            .get_or_compute_package(std::slice::from_ref(&app_main))
            .unwrap();
        assert_eq!(entry.dependency_dirs, expected_deps);

        let all_diags = cache.all_diagnostics(std::slice::from_ref(&app_main));
        let no_diags = Vec::new();
        let file_diags = all_diags.get(&app_main).unwrap_or(&no_diags);
        let use_not_exported_count = file_diags
            .iter()
            .filter(|diag| diag.code.slug() == "use-not-exported")
            .count();
        assert_eq!(
            use_not_exported_count, 1,
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

        let cache = PackageCache::for_test();
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
                .get(&package_identity(&cache, &app_gin))
                .expect("dependency graph is available after resolve");
            !graph.impacted_modules(&used_dep_path).is_empty()
        };

        let first_entry_symptoms = cache.package_semantics(std::slice::from_ref(&app_gin))[0]
            .symptoms
            .clone();
        let second_entry_symptoms = cache.package_semantics(std::slice::from_ref(&app_gin))[0]
            .symptoms
            .clone();

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

        let cache = PackageCache::for_test();
        cache.add_file(app_a_main.clone()).unwrap();
        cache.add_file(app_b_main.clone()).unwrap();
        cache.add_file(dep_used.clone()).unwrap();
        cache.add_file(dep_unused.clone()).unwrap();

        let app_a_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_a_main))
                .unwrap();
            let app_a_identity = package_identity(&cache, &app_a_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_a_identity])
        };
        let app_b_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_b_main))
                .unwrap();
            let app_b_identity = package_identity(&cache, &app_b_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_b_identity])
        };

        cache.set_contents(&dep_unused, "Unused is 2\n".to_string());

        let app_a_entry_after_unused = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_a_main))
                .unwrap();
            let app_a_identity = package_identity(&cache, &app_a_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_a_identity])
        };
        let app_b_entry_after_unused = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_b_main))
                .unwrap();
            let app_b_identity = package_identity(&cache, &app_b_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_b_identity])
        };

        assert!(Arc::ptr_eq(&app_a_entry_before, &app_a_entry_after_unused));
        assert!(!Arc::ptr_eq(&app_b_entry_before, &app_b_entry_after_unused));

        cache.set_contents(&dep_used, "Used is 3\n".to_string());

        let app_a_entry_after_used = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_a_main))
                .unwrap();
            let app_a_identity = package_identity(&cache, &app_a_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_a_identity])
        };
        let app_b_entry_after_used = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_b_main))
                .unwrap();
            let app_b_identity = package_identity(&cache, &app_b_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_b_identity])
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

        let cache = PackageCache::for_test();
        cache.add_file(app_main.clone()).unwrap();
        cache.add_file(other_main.clone()).unwrap();
        cache.add_file(dep_used.clone()).unwrap();

        let app_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_main))
                .unwrap();
            let app_identity = package_identity(&cache, &app_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_identity])
        };
        let other_entry_before = {
            cache
                .get_or_compute_package(std::slice::from_ref(&other_main))
                .unwrap();
            let other_identity = package_identity(&cache, &other_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&other_identity])
        };

        cache.invalidate_package_cache_for(&dep_used);

        let app_entry_after = {
            cache
                .get_or_compute_package(std::slice::from_ref(&app_main))
                .unwrap();
            let app_identity = package_identity(&cache, &app_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&app_identity])
        };
        let other_entry_after = {
            cache
                .get_or_compute_package(std::slice::from_ref(&other_main))
                .unwrap();
            let other_identity = package_identity(&cache, &other_main);
            let packages = cache.packages.lock().unwrap();
            Arc::clone(&packages[&other_identity])
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

        let cache = PackageCache::for_test();
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

        let cache = PackageCache::for_test();
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
            .get(&package_identity(&cache, &app_main))
            .and_then(|c| c.get_file(&dep_used).map(|f| Arc::new(f.source.clone())));
        assert!(module_cache_before.is_some());

        cache.set_contents(&app_main, "use dep.Used\nvalue := 2\n".to_string());

        let app_identity = package_identity(&cache, &app_main);
        assert!(
            !cache
                .module_caches
                .lock()
                .unwrap()
                .contains_key(&app_identity)
        );
        assert!(
            !cache
                .import_graphs
                .lock()
                .unwrap()
                .contains_key(&app_identity)
        );

        let _ = cache
            .get_or_compute_package(std::slice::from_ref(&app_main))
            .unwrap();

        let parsed_dep_after = cache.parse_output(&dep_used).unwrap();
        let module_cache_after = cache
            .module_caches
            .lock()
            .unwrap()
            .get(&package_identity(&cache, &app_main))
            .and_then(|c| c.get_file(&dep_used).map(|f| Arc::new(f.source.clone())));

        assert!(
            cache
                .module_caches
                .lock()
                .unwrap()
                .contains_key(&package_identity(&cache, &app_main))
        );
        assert!(
            cache
                .import_graphs
                .lock()
                .unwrap()
                .contains_key(&package_identity(&cache, &app_main))
        );
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
