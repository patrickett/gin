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
use typecheck::transform::{PackageCheckSession, PackageCheckStats, PackageTransformOptions};
use typecheck::{
    CompletionCandidate, PackageSemanticIndex, TypedFileAst, completions::FileAstCompletionExt,
};

use crate::SemanticQueries;
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

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct AdHocPackageId(u64);

/// Cached output for a fully analyzed package.
pub struct PackageEntry {
    registry_resolution: Option<flask::PackageResolution>,
    configuration: Option<serde_json::Value>,
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

impl ParseEntry {
    fn parsed_file(&self, path: PathBuf) -> ParsedFile {
        ParsedFile {
            path,
            source: (*self.source).clone(),
            output: (*self.parse).clone(),
        }
    }
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
    revision: AtomicU64,
    checking: Option<CheckingSessions>,
    resolved_results: Option<Mutex<HashMap<PackageIdentity, ResolvedResult>>>,
    /// File watcher for disk changes (kept alive for the lifetime of the cache)
    _watcher: Arc<Mutex<Debouncer<RecommendedWatcher>>>,
}

struct CheckingSessions {
    template: PackageCheckSession,
    packages: Mutex<HashMap<PackageIdentity, Arc<Mutex<PackageCheckSession>>>>,
}

struct ResolvedResult {
    inputs: Arc<resolve::PackageFrontEndInputs>,
    entry: Arc<PackageEntry>,
    reused: bool,
}

struct PackageComputation {
    entry: Arc<PackageEntry>,
    inputs: Option<Arc<resolve::PackageFrontEndInputs>>,
    reused: bool,
    modules: resolve::ParsedModuleCache,
    graph: Option<resolve::ImportDependencyGraph>,
}

impl PackageCache {
    pub fn new_adhoc_package(&self) -> AdHocPackageId {
        AdHocPackageId(self.next_package_id.fetch_add(1, Ordering::SeqCst))
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

    fn package_entry_for(&self, path: &Path) -> Option<Arc<PackageEntry>> {
        let files = self.package_file_paths(path)?;
        self.get_or_compute_package(&files)
    }

    fn compile_target_for(path: &Path) -> CompileTarget {
        FlaskConfig::find_package_config(path)
            .and_then(|(config, _)| CompileTarget::resolve(&config, None).ok())
            .unwrap_or(CompileTarget::Library)
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
            self.revision.fetch_add(1, Ordering::SeqCst);
            let mut packages = self.packages.lock().unwrap();
            packages.remove(&package);
            if let Some(previous) = &previous {
                packages.remove(&previous.package);
            }
            previous.map(|entry| entry.package)
        };

        if let Some(old_package) = old_package
            && old_package != package
        {
            self.invalidate_cached_packages_for(&old_package);
        }
        self.invalidate_cached_packages_for(&package);
    }

    pub fn new(tx: Sender<DebounceEventResult>) -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            packages: Mutex::new(HashMap::new()),
            module_caches: Mutex::new(HashMap::new()),
            import_graphs: Mutex::new(HashMap::new()),
            next_package_id: AtomicU64::new(1),
            revision: AtomicU64::new(0),
            checking: None,
            resolved_results: None,
            _watcher: Arc::new(Mutex::new(
                new_debouncer(Duration::from_secs(1), tx)
                    .expect("failed to create file watcher debouncer"),
            )),
        }
    }

    pub fn with_check_session(
        tx: Sender<DebounceEventResult>,
        session: PackageCheckSession,
    ) -> Self {
        let mut cache = Self::new(tx);
        cache.checking = Some(CheckingSessions {
            template: session.fork_empty(),
            packages: Mutex::new(HashMap::new()),
        });
        cache
    }

    pub fn with_resolved_result_reuse(mut self) -> Self {
        self.resolved_results = Some(Mutex::new(HashMap::new()));
        self
    }

    pub fn resolved_result_reused(&self, path: &Path) -> Option<bool> {
        let package = self.file_identity(path)?;
        let results = self.resolved_results.as_ref()?.lock().unwrap();
        Some(results.get(&package)?.reused)
    }

    pub fn check_stats(&self, path: &Path) -> Option<PackageCheckStats> {
        let package = self.file_identity(path)?;
        let checking = self.checking.as_ref()?;
        let session = checking.packages.lock().unwrap().get(&package)?.clone();
        let stats = session.lock().unwrap().stats().clone();
        Some(stats)
    }

    /// Test-friendly constructor: wires an unbounded channel whose receiver is
    /// dropped, so debounced watcher events are discarded.
    pub fn for_test() -> Self {
        let (tx, _rx) = crossbeam_channel::unbounded();
        Self::new(tx)
    }

    pub fn for_test_with_file(path: PathBuf) -> Self {
        let cache = Self::for_test();
        cache.add_file(path).unwrap();
        cache
    }

    /// Register a file by reading it from disk and parsing it.
    ///
    /// If the file does not exist on disk (e.g. LSP adds the file before
    /// receiving its contents), an empty placeholder is used. Call
    /// [`set_contents`](Self::set_contents) to provide the real contents.
    pub fn add_file(&self, path: PathBuf) -> Result<(), String> {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        let package = self.package_identity_for(&path);
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
        let package = self.package_identity_for(path);
        self.set_parsed_file(path.to_path_buf(), contents, package);
        self.invalidate_package_cache_for(path);
    }

    fn package_identity_for(&self, path: &Path) -> PackageIdentity {
        match self.file_identity(path) {
            Some(identity) => identity,
            None => {
                let mut identity = Self::identity_for_path(path);
                if matches!(identity, PackageIdentity::AdHoc(_)) {
                    identity = PackageIdentity::AdHoc(self.new_adhoc_package());
                }
                identity
            }
        }
    }

    fn import_graph_path(path: &Path) -> PathBuf {
        path.canonicalize()
            .ok()
            .or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?)))
            .unwrap_or_else(|| path.to_path_buf())
    }

    fn impacted_package_roots(&self, path: &Path) -> HashSet<PackageIdentity> {
        let graph_path = Self::import_graph_path(path);
        let mut affected = HashSet::new();

        if let Some(identity) = self.file_identity(path) {
            affected.insert(identity);
        } else if let identity @ PackageIdentity::Configured { .. } = Self::identity_for_path(path)
        {
            affected.insert(identity);
        }
        {
            let import_graphs = self.import_graphs.lock().unwrap();
            for (package, graph) in import_graphs.iter() {
                if !graph.impacted_modules(&graph_path).is_empty() {
                    affected.insert(package.clone());
                }
            }
        }

        affected
    }

    /// Remove resolver/module caches and package entry for a package root.
    fn invalidate_cached_packages_for(&self, package: &PackageIdentity) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.packages.lock().unwrap().remove(package);
        self.import_graphs.lock().unwrap().remove(package);
        self.module_caches.lock().unwrap().remove(package);
        if self.package_files_for_identity(package).is_empty() {
            if let Some(checking) = &self.checking {
                checking.packages.lock().unwrap().remove(package);
            }
            if let Some(results) = &self.resolved_results {
                results.lock().unwrap().remove(package);
            }
        }
    }

    /// Get source text and parse output for a file.
    pub fn source_and_parse(&self, path: &Path) -> Option<(SharedSource, Arc<ParseOutput>)> {
        let files = self.files.lock().unwrap();
        files
            .get(path)
            .map(|e| (Arc::clone(&e.source), Arc::clone(&e.parse)))
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

        loop {
            let revision = self.revision.load(Ordering::SeqCst);
            let package = self.file_identity(&paths[0])?;
            let discovered = FlaskConfig::discover(&paths[0]);
            let discovered = discovered.and_then(|found| {
                found
                    .map(|(config, root)| {
                        config
                            .with_workspace_requirements(&root)
                            .map_err(|e| e.to_string())
                    })
                    .transpose()
            });
            let manifest_valid = discovered.is_ok();
            let config = discovered.ok().flatten();
            let configuration = config.as_ref().map(|config| {
                serde_json::to_value(config).expect("package configuration is serializable")
            });
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
            let configuration_changed = {
                let packages = self.packages.lock().unwrap();
                if let Some(entry) = packages.get(&package) {
                    if manifest_valid
                        && registry_valid
                        && entry.registry_resolution == resolution
                        && entry.configuration == configuration
                    {
                        return Some(Arc::clone(entry));
                    }
                    entry.configuration != configuration
                } else {
                    false
                }
            };
            if configuration_changed || !manifest_valid || !registry_valid || resolution.is_some() {
                self.module_caches.lock().unwrap().remove(&package);
            }
            let compile_target = match &package {
                PackageIdentity::Configured { root } => Self::compile_target_for(root),
                PackageIdentity::AdHoc(_) => CompileTarget::Library,
            };
            let all_files = self.package_file_paths(&paths[0])?;
            let mut computation = self.compute_package(
                revision,
                &package,
                &all_files,
                compile_target,
                &configuration,
                &resolution,
            )?;
            if !computation.reused {
                let entry = Arc::get_mut(&mut computation.entry)
                    .expect("fresh package result is uniquely owned");
                entry.registry_resolution = resolution;
                entry.configuration = configuration;
            }
            if let Some(entry) = self.publish_package(package, revision, computation) {
                return Some(entry);
            }
        }
    }

    fn publish_package(
        &self,
        package: PackageIdentity,
        revision: u64,
        computation: PackageComputation,
    ) -> Option<Arc<PackageEntry>> {
        let mut packages = self.packages.lock().unwrap();
        if self.revision.load(Ordering::SeqCst) != revision {
            return None;
        }
        self.module_caches
            .lock()
            .unwrap()
            .insert(package.clone(), computation.modules);
        let mut graphs = self.import_graphs.lock().unwrap();
        if let Some(graph) = computation.graph {
            graphs.insert(package.clone(), graph);
        } else {
            graphs.remove(&package);
        }
        let entry = computation.entry;
        if let Some(results) = &self.resolved_results {
            let mut results = results.lock().unwrap();
            if let Some(inputs) = computation.inputs {
                results.insert(
                    package.clone(),
                    ResolvedResult {
                        inputs,
                        entry: Arc::clone(&entry),
                        reused: computation.reused,
                    },
                );
            }
        }
        packages.insert(package, Arc::clone(&entry));
        Some(entry)
    }

    fn compute_package(
        &self,
        revision: u64,
        package: &PackageIdentity,
        all_files: &[PathBuf],
        compile_target: CompileTarget,
        configuration: &Option<serde_json::Value>,
        resolution: &Option<flask::PackageResolution>,
    ) -> Option<PackageComputation> {
        let files = self.files.lock().unwrap();

        let mut parsed: Vec<ParsedFile> = all_files
            .iter()
            .filter_map(|path| files.get(path).map(|entry| entry.parsed_file(path.clone())))
            .collect();
        if parsed.is_empty() {
            return None;
        }
        parsed.sort_by(|a, b| a.path.cmp(&b.path));

        drop(files);

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
        let options = resolve::PackageFrontEndOptions {
            gin_home: None,
            compile_target,
            transform: PackageTransformOptions::IDE,
            resolve_imports,
            package_fallback,
            transform_paths: None,
        };
        let prepared = resolve::prepare_package_front_end(parsed, &deps, cache, options);
        if let Some(results) = &self.resolved_results {
            let mut results = results.lock().unwrap();
            if let Some(previous) = results.get(package) {
                if previous.entry.configuration == *configuration
                    && previous.entry.registry_resolution == *resolution
                    && previous.entry.dependency_dirs == deps
                    && previous.inputs.matches(&prepared)
                {
                    let entry = Arc::clone(&previous.entry);
                    let inputs = Some(Arc::clone(&previous.inputs));
                    let (modules, graph) = prepared.into_dependency_state();
                    return Some(PackageComputation {
                        entry,
                        inputs,
                        reused: true,
                        modules,
                        graph,
                    });
                }
                if self.revision.load(Ordering::SeqCst) == revision {
                    results.remove(package);
                }
            }
        }
        let inputs = self
            .resolved_results
            .as_ref()
            .map(|_| Arc::new(prepared.input_snapshot()));
        let front_end = match &self.checking {
            Some(checking) => {
                let session = checking
                    .packages
                    .lock()
                    .unwrap()
                    .entry(package.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(checking.template.fork_empty())))
                    .clone();
                let mut session = session.lock().unwrap();
                resolve::check_package_front_end(prepared, Some(&mut session))
            }
            None => resolve::check_package_front_end(prepared, None),
        };
        let solved = resolve::AssembledPackage::assemble(front_end);
        let package_index = solved.semantic_index();
        let mut file_paths = Vec::with_capacity(solved.files.len());
        let mut typed_asts = Vec::with_capacity(solved.files.len());
        let mut prepare_diags = Vec::with_capacity(solved.files.len());
        let mut typecheck_outputs = Vec::with_capacity(solved.files.len());
        let mut import_diags = HashMap::new();
        for file in solved.files {
            let typed = file.typed;
            let mut flaws = typed.flaws_with_spans();
            flaws.extend_from_slice(typed.all_warnings());
            flaws.sort_by_cached_key(|flaw| {
                (flaw.span.start(), flaw.span.end(), format!("{flaw:?}"))
            });
            typecheck_outputs.push(flaws);
            let mut diagnostics = file.parsed.output.symptoms;
            diagnostics.extend(file.prepare_diagnostics);
            prepare_diags.push(diagnostics);
            if !file.import_diagnostics.is_empty() {
                import_diags.insert(file.parsed.path.clone(), file.import_diagnostics);
            }
            file_paths.push(file.parsed.path);
            typed_asts.push(typed);
        }
        let entry = PackageEntry {
            registry_resolution: None,
            configuration: None,
            typed_asts,
            public_interface: solved.public_interface,
            package_index,
            file_paths,
            typecheck_outputs,
            import_diags,
            dependency_dirs: deps,
            prepare_diags,
        };
        Some(PackageComputation {
            entry: Arc::new(entry),
            inputs,
            reused: false,
            modules: solved.cache,
            graph: solved.import_graph,
        })
    }

    /// Get semantic outputs in caller-requested path order.
    pub fn package_semantics(&self, paths: &[PathBuf]) -> Vec<FileSemanticOutput> {
        let by_path = self.all_diagnostics(paths);
        paths
            .iter()
            .map(|path| FileSemanticOutput {
                symptoms: by_path
                    .get(path)
                    .or_else(|| path.canonicalize().ok().and_then(|path| by_path.get(&path)))
                    .cloned()
                    .unwrap_or_default(),
            })
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
        if let Some((probe, receiver, partial)) =
            typecheck::completions::member_completion_input(&source, byte_pos)
            && let Some(entry) = self.package_entry_for(path)
            && let Some(index) = entry.file_paths.iter().position(|file| file == path)
        {
            let original = &entry.typed_asts[index];
            let mut ctx = typecheck::transform::TransformCtx::from_typed_asts(
                &entry.typed_asts.iter().collect::<Vec<_>>(),
            );
            if let Some(origin) = &original.semantic_origin {
                ctx.package_instance = origin.package.clone();
            }
            ctx.ide_package = true;
            ctx.compile_time_eval_ast = original.eval_ast.clone();
            ctx.compile_target = Self::compile_target_for(path);
            let input = ParsedFile {
                path: path.to_path_buf(),
                output: probe.parse_source_full(),
                source: probe,
            };
            let mut inputs: Vec<_> = self
                .files
                .lock()
                .unwrap()
                .iter()
                .filter(|(file, _)| entry.file_paths.contains(file) && file.as_path() != path)
                .map(|(file, cached)| cached.parsed_file(file.clone()))
                .collect();
            inputs.push(input);
            let resolved = resolve::resolve_imports(inputs, &entry.dependency_dirs);
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let Some(mut parsed) = resolved.into_iter().find(|file| file.path == canonical) else {
                return Vec::new();
            };
            parsed.output.ast.semantic_origin = original.semantic_origin.as_deref().cloned();
            typecheck::prepare_parse_ast(&mut parsed.output.ast, &ctx.compile_target);
            let members = typecheck::transform::member_completions(
                &parsed.output.ast,
                original.file_id,
                &ctx,
                receiver,
            );
            return members
                .into_iter()
                .filter(|member| member.label.starts_with(&partial))
                .collect();
        }
        let query_path = Self::import_graph_path(path);
        if let Some(entry) = self.package_entry_for(path)
            && let Some(index) = entry
                .file_paths
                .iter()
                .position(|file| file == path || *file == query_path)
        {
            return SemanticQueries::new(
                &source,
                &output.ast,
                &entry.typed_asts[index],
                &entry.package_index,
            )
            .completions(byte_pos);
        }
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
        let entry = self.package_entry_for(path)?;
        let mut tag_types = HashMap::new();
        for typed in &entry.typed_asts {
            for (tag_id, ty) in typed.tag_types.iter() {
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

        if let Some(hover) = SemanticQueries::source_hover(path, source, ast, byte_pos, &|p| {
            self.document_snapshot(p)
                .map(|snapshot| resolve::ParsedFile {
                    path: p.to_path_buf(),
                    source: (*snapshot.source).clone(),
                    output: (*snapshot.parse).clone(),
                })
                .or_else(|| resolve::ParsedFile::read(p))
        }) {
            return Some(hover);
        }

        // 3. Typed hover (with package context when possible)
        let result = self.typed_hover(path, source, &snapshot.parse.ast, byte_pos);
        if result.is_none() {
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
        ast: &ast::FileAst,
        byte_pos: usize,
    ) -> Option<HoverContent> {
        // Try with full package context (better cross-file hover results).
        if let Some(entry) = self.package_entry_for(path) {
            let canonical_path = path.canonicalize().ok();
            for (file_path, typed) in entry.file_paths.iter().zip(entry.typed_asts.iter()) {
                if file_path == path || canonical_path.as_ref() == Some(file_path) {
                    return SemanticQueries::new(source, ast, typed, &entry.package_index)
                        .hover(byte_pos);
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
        self.invalidate_impacted_packages(path);
    }

    pub fn evict_package_cache_for(&self, path: &Path) {
        for package in self.invalidate_impacted_packages(path) {
            if let Some(checking) = &self.checking {
                checking.packages.lock().unwrap().remove(&package);
            }
            if let Some(results) = &self.resolved_results {
                results.lock().unwrap().remove(&package);
            }
        }
    }

    fn invalidate_impacted_packages(&self, path: &Path) -> HashSet<PackageIdentity> {
        self.revision.fetch_add(1, Ordering::SeqCst);
        let impacted = self.impacted_package_roots(path);
        for package in &impacted {
            self.invalidate_cached_packages_for(package);
        }
        impacted
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.files.lock().unwrap().contains_key(path)
    }

    pub fn evict_file(&self, path: &Path) -> bool {
        let affected = self.impacted_package_roots(path);

        let removed = {
            let mut files = self.files.lock().unwrap();
            files.remove(path).map(|entry| {
                self.revision.fetch_add(1, Ordering::SeqCst);
                self.packages.lock().unwrap().remove(&entry.package);
                entry.package.clone()
            })
        };

        if removed.is_none() {
            return false;
        }

        for package in affected {
            self.invalidate_cached_packages_for(&package);
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
            let symptoms = entry.semantic_output(i).symptoms;
            by_path.entry(path.clone()).or_default().extend(symptoms);
        }

        by_path
    }
}

#[cfg(test)]
#[path = "package_cache_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "package_cache/session_tests.rs"]
mod session_tests;

#[cfg(test)]
#[path = "package_cache/resolved_result_tests.rs"]
mod resolved_result_tests;
