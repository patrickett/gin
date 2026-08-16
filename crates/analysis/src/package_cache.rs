//! Generation-based cache for parsed files and package-level analysis.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Sender;
use derive_more::From;
use notify_debouncer_mini::{
    DebounceEventResult, Debouncer, new_debouncer, notify::RecommendedWatcher,
};

use parser::query::{ParseOutput, SourceParseExt};
use typecheck::transform::{PackageTransformArtifacts, PackageTransformOptions};
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
    prepare_diags: Vec<Vec<Diagnostic>>,
    import_diags: HashMap<PathBuf, Vec<Diagnostic>>,
    dependency_dirs: HashMap<String, PathBuf>,
    artifacts: PackageTransformArtifacts,
}
use diagnostic::Diagnostic;
use resolve::{CursorDefinition, ParsedFile};

use crate::cursor::ReferencesInFile;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub enum PackageIdentity {
    Configured { root: PathBuf },
    AdHoc(AdHocPackageId),
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, From)]
pub struct AdHocPackageId(u64);

/// Cached output for a fully analyzed package.
pub struct PackageEntry {
    pub typed_asts: Vec<TypedFileAst>,
    pub public_interface: interface::PublicInterface,
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
            prepare_diags,
            import_diags,
            dependency_dirs,
            artifacts,
        } = self.stage_resolve_and_prepare(package, all_files)?;
        let public_interface = artifacts.public_interface.clone();
        let typed_asts = artifacts.typed_asts;
        let package_index = self.stage_build_index(&file_paths, &typed_asts);
        let (typecheck_outputs, prepare_diags) =
            self.stage_derive_diagnostics(&typed_asts, &prepare_diags);
        Some(PackageEntry {
            typed_asts,
            public_interface,
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
        let parse_symptoms_by_path: std::collections::HashMap<PathBuf, Vec<Diagnostic>> = parsed
            .iter()
            .map(|f| (f.path.clone(), f.output.symptoms.clone()))
            .collect();

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

        let resolve_imports =
            !(matches!(package, PackageIdentity::AdHoc(_)) && parsed.len() <= 1 && deps.is_empty());
        let cache = self
            .module_caches
            .lock()
            .unwrap()
            .remove(package)
            .unwrap_or_default();
        let package_fallback = match package {
            PackageIdentity::Configured { .. } => {
                ast::ty::PackageInstanceKey::workspace("anonymous", "0")
            }
            PackageIdentity::AdHoc(id) => {
                ast::ty::PackageInstanceKey::workspace("adhoc", &format!("session-{}", id.0))
            }
        };
        let front_end = resolve::run_package_front_end(
            parsed,
            &deps,
            cache,
            resolve::PackageFrontEndOptions {
                compile_target,
                transform: PackageTransformOptions::IDE,
                resolve_imports,
                package_fallback,
                transform_paths: None,
            },
        );
        self.module_caches
            .lock()
            .unwrap()
            .insert(package.clone(), front_end.cache);
        let import_graph = front_end.import_graph;
        if let Some(import_graph) = &import_graph {
            self.import_graphs
                .lock()
                .unwrap()
                .insert(package.clone(), import_graph.clone());
        }
        let resolved = front_end.files;
        let raw_file_paths: Vec<_> = resolved.iter().map(|file| file.path.clone()).collect();
        let file_paths = import_graph
            .as_ref()
            .and_then(|graph| {
                (!raw_file_paths.is_empty())
                    .then_some(graph.declaration_file_order(&raw_file_paths))
            })
            .unwrap_or_else(|| {
                let mut sorted = raw_file_paths;
                sorted.sort();
                sorted
            });
        let typecheck::transform::PackageTransformArtifacts {
            typed_asts,
            compile_time_eval_ast,
            trait_registry,
            public_interface,
            interface_publication,
        } = front_end.artifacts;
        let mut typed_asts_by_path = resolved
            .iter()
            .cloned()
            .zip(typed_asts)
            .map(|(file, ast)| (file.path, ast))
            .collect::<HashMap<_, _>>();
        let mut prepare_by_path: HashMap<PathBuf, Vec<Diagnostic>> = resolved
            .iter()
            .map(|file| file.path.clone())
            .zip(front_end.prepare_diagnostics)
            .collect();
        let mut ordered_typed_asts = Vec::with_capacity(file_paths.len());
        for path in &file_paths {
            if let Some(ast) = typed_asts_by_path.remove(path) {
                ordered_typed_asts.push(ast);
            }
        }

        let mut by_path: HashMap<PathBuf, resolve::ParsedFile> =
            resolved.into_iter().map(|f| (f.path.clone(), f)).collect();

        let ordered: Vec<resolve::ParsedFile> = file_paths
            .iter()
            .filter_map(|p| by_path.remove(p))
            .collect();

        let mut prepare_diags: Vec<Vec<Diagnostic>> = file_paths
            .iter()
            .map(|path| prepare_by_path.remove(path).unwrap_or_default())
            .collect();

        let mut import_diags = HashMap::new();
        for (path, resolved_file) in file_paths.iter().zip(ordered.iter()) {
            let mut file_import_diags = resolved_file.output.symptoms.clone();
            let original_syms = parse_symptoms_by_path
                .get(path)
                .map(std::vec::Vec::as_slice)
                .unwrap_or(&[]);
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
        for (path, prep_diags) in file_paths.iter().zip(prepare_diags.iter_mut()) {
            let mut parse_syms = parse_symptoms_by_path
                .get(path)
                .cloned()
                .unwrap_or_default();
            parse_syms.append(prep_diags);
            *prep_diags = parse_syms;
        }

        Some(Stage1Result {
            file_paths,
            prepare_diags,
            import_diags,
            dependency_dirs: deps,
            artifacts: typecheck::transform::PackageTransformArtifacts {
                typed_asts: ordered_typed_asts,
                compile_time_eval_ast,
                trait_registry,
                public_interface,
                interface_publication,
            },
        })
    }

    /// Stage 3: Build cross-file package index from typed ASTs.
    ///
    /// `file_paths` must be sorted (they are, from [`PackageEntry`]).
    fn stage_build_index(
        &self,
        _file_paths: &[PathBuf],
        typed_asts: &[TypedFileAst],
    ) -> PackageSemanticIndex {
        let typed_refs: Vec<&TypedFileAst> = typed_asts.iter().collect();
        PackageSemanticIndex::from_typed_asts(&typed_refs)
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

    pub fn public_interface_for(&self, paths: &[PathBuf]) -> Option<interface::PublicInterface> {
        self.get_or_compute_package(paths)
            .map(|entry| entry.public_interface.clone())
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
                tag_types.insert(
                    tag_id.0,
                    typed.type_registry.resolved_definition_for_type(ty),
                );
            }
        }
        Some(tag_types)
    }

    /// Get hover content at a byte position.
    pub fn hover(&self, path: &Path, byte_pos: u32) -> Option<HoverContent> {
        let snapshot = match self.document_snapshot(path) {
            Some(snapshot) => snapshot,
            None => {
                eprintln!(
                    "[ginlsp] hover: document_snapshot returned None for {:?}",
                    path
                );
                return None;
            }
        };
        let source = &snapshot.source;
        let parse = &snapshot.parse;
        let byte_pos = byte_pos as usize;
        let ast = &parse.ast;
        let word = source.word_at_byte_offset(byte_pos).unwrap_or_default();

        // 1. Parse-only hover (keywords, literals)
        // Skip `self` — the typed hover provides the receiver type (e.g. `self Type`,
        // `ref self Type`, `mut self Type`) which is more useful than a generic keyword doc.
        if word != "self"
            && let Some(h) = crate::hover::HoverSnippet::keyword(source, byte_pos)
        {
            return Some(HoverContent {
                markdown: h.markdown,
                byte_range: h.byte_range,
            });
        }

        // 2. Import hover — fall through to typed_hover when it returns None
        //    (e.g. when the import path references a local folder not in dependencies).
        if let Some(target) = resolve::resolve_import_at(ast, source, byte_pos)
            && let Some(markdown) =
                resolve::hover_for_import_target(path, target, &|p| resolve::ParsedFile::read(p))
        {
            return Some(HoverContent {
                markdown,
                byte_range: source.word_byte_range(byte_pos).map(|(s, e)| s..e),
            });
        }

        // 3. Typed hover (with package context when possible)
        let result = self.typed_hover(path, source, &snapshot.line_index, byte_pos);
        if result.is_none() {
            // Fall back to keyword hover for `self` if the typed hover didn't resolve.
            if word == "self"
                && let Some(h) = crate::hover::HoverSnippet::keyword(source, byte_pos)
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

    fn typed_hover(
        &self,
        path: &Path,
        source: &str,
        line_index: &LineIndex,
        byte_pos: usize,
    ) -> Option<HoverContent> {
        let (line, character) = line_index.byte_to_position(source, byte_pos);

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

    pub fn evict_file(&self, path: &Path) -> bool {
        let mut affected = HashSet::new();

        let removed = {
            let mut files = self.files.lock().unwrap();
            files.remove(path).map(|entry| {
                affected.insert(entry.package.clone());
                entry.package.clone()
            })
        };

        if removed.is_none() {
            return false;
        }

        {
            let import_graphs = self.import_graphs.lock().unwrap();
            for (package, graph) in import_graphs.iter() {
                if !graph.impacted_modules(path).is_empty() {
                    affected.insert(package.clone());
                }
            }
        }

        for package in affected {
            self.invalidate_cached_packages_for(package);
        }

        true
    }

    pub fn cache_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.files.lock().unwrap().len(),
            self.packages.lock().unwrap().len(),
            self.module_caches.lock().unwrap().len(),
            self.import_graphs.lock().unwrap().len(),
        )
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
#[path = "package_cache_tests.rs"]
mod tests;
