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
use typecheck::transform::PackageTransformOptions;
use typecheck::{
    CompletionCandidate, PackageSemanticIndex, TypedFileAst, completions::FileAstCompletionExt,
};

use crate::HoverDocExt;
use crate::engine::{DocumentSnapshot, FileSemanticOutput, HoverContent, SharedSource};
use ast::source::{LineIndex, SourceExt};
use flask::{CompileTarget, FlaskConfig};
use resolve::GinPackageExt;

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
    registry_resolution: Option<flask::PackageResolution>,
    pub typed_asts: Vec<TypedFileAst>,
    pub public_interface: interface::PublicInterface,
    pub package_index: PackageSemanticIndex,
    /// File paths in declaration order, matching `typed_asts`.
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

        let config = FlaskConfig::find_package_config(&paths[0]).map(|(config, _)| config);
        let resolution = config.as_ref().and_then(|config| config.resolution.clone());
        let registry_valid = config.as_ref().is_none_or(|config| {
            if !config
                .dependencies()
                .values()
                .any(|dep| matches!(dep.kind, flask::DependencyKind::Version { .. }))
            {
                return true;
            }
            let verified = (|| -> std::io::Result<()> {
                let client = flask::RegistryClient::load(flask::GinHome::discover()?)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(std::io::Error::other)?
                    .as_secs();
                client.verify_cached(config, now)?;
                Ok(())
            })();
            verified.is_ok()
        });
        if registry_valid {
            if let Some(entry) = self.packages.lock().unwrap().get(&package) {
                if entry.registry_resolution == resolution {
                    return Some(Arc::clone(entry));
                }
            }
        }
        if !registry_valid || resolution.is_some() {
            self.module_caches.lock().unwrap().remove(&package);
        }

        // Slow path: compute using all registered files in the package.
        let all_files = self.package_file_paths(&paths[0])?;
        let entry = self.compute_package(&package, &all_files)?;

        let mut entry = entry;
        entry.registry_resolution = resolution;
        let entry = Arc::new(entry);
        self.packages
            .lock()
            .unwrap()
            .insert(package, Arc::clone(&entry));
        Some(entry)
    }

    fn compute_package(
        &self,
        package: &PackageIdentity,
        all_files: &[PathBuf],
    ) -> Option<PackageEntry> {
        let files = self.files.lock().unwrap();

        let mut parsed: Vec<ParsedFile> = all_files
            .iter()
            .filter_map(|path| {
                files.get(path).map(|entry| ParsedFile {
                    path: path.clone(),
                    source: (*entry.source).clone(),
                    output: (*entry.parse).clone(),
                })
            })
            .collect();
        if parsed.is_empty() {
            return None;
        }
        parsed.sort_by(|a, b| a.path.cmp(&b.path));

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
                gin_home: None,
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
        let mut file_paths = Vec::with_capacity(front_end.files.len());
        let mut typed_asts = Vec::with_capacity(front_end.files.len());
        let mut prepare_diags = Vec::with_capacity(front_end.files.len());
        let mut typecheck_outputs = Vec::with_capacity(front_end.files.len());
        let mut import_diags = HashMap::new();
        for file in front_end.files {
            let typed = file
                .typed
                .expect("IDE package transformation checks every file");
            typecheck_outputs.push(
                typed
                    .all_flaws()
                    .into_iter()
                    .map(|(_, flaw)| flaw.clone())
                    .collect(),
            );
            let mut diagnostics = file.parsed.output.symptoms;
            diagnostics.extend(file.prepare_diagnostics);
            prepare_diags.push(diagnostics);
            if !file.import_diagnostics.is_empty() {
                import_diags.insert(file.parsed.path.clone(), file.import_diagnostics);
            }
            file_paths.push(file.parsed.path);
            typed_asts.push(typed);
        }
        let package_index =
            PackageSemanticIndex::from_typed_asts(&typed_asts.iter().collect::<Vec<_>>());
        Some(PackageEntry {
            registry_resolution: None,
            typed_asts,
            public_interface: front_end.public_interface,
            package_index,
            file_paths,
            typecheck_outputs,
            import_diags,
            dependency_dirs: deps,
            prepare_diags,
        })
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
        // `self ref Type`, `self mut Type`) which is more useful than a generic keyword doc.
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
