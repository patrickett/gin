use crate::prelude::*;
use crate::span::{SpanId, SpanTable};
use indexmap::IndexMap;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path::PathBuf,
};

pub type TagMap = HashMap<Intern<String>, Declare>;
/// Method name → single bind (impl blocks, etc.).
pub type MethodMap = HashMap<Intern<String>, Bind>;
/// Top-level def name → one bind after platform filtering (see [`collapse_defs_for_platform`]).
pub type DefMap = IndexMap<Intern<String>, Bind>;

/// Collapse parser scratch (`Vec` per name from raw top-level collection) to a single bind per name
/// for the current host `#[os]` / `#[arch]`. Names with no matching overload are dropped.
pub fn collapse_defs_for_platform(multi: IndexMap<Intern<String>, Vec<Bind>>) -> DefMap {
    let mut defs = DefMap::new();
    for (name, binds) in multi {
        if let Some(bind) = pick_bind_for_platform(binds) {
            defs.insert(name, bind);
        }
    }
    defs
}

fn pick_bind_for_platform(binds: Vec<Bind>) -> Option<Bind> {
    if binds.is_empty() {
        return None;
    }
    let mut matching: Vec<Bind> = binds
        .into_iter()
        .filter(|b| b.attributes().matches_current_platform())
        .collect();
    if matching.is_empty() {
        None
    } else {
        // If several overloads match (misconfiguration), keep the first.
        Some(matching.remove(0))
    }
}

/// Symbol kind - distinguishes between different types of symbols.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A tag/type definition (e.g., `Person ::= ...`)
    Tag(Intern<String>),
    /// A function definition (e.g., `foo : { ... }`)
    Function(Intern<String>),
    /// A value binding (e.g., `x : 42`)
    Bind(Intern<String>),
}

/// Compile-time symbol with source information.
///
/// This tracks symbol metadata at compile time, separate from runtime
/// MLIR values which are tracked during codegen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    /// The symbol name (e.g., "http.web.handle" or "foo")
    pub name: Intern<String>,

    /// Which file defined this symbol
    pub source_file: PathBuf,

    /// What kind of symbol this is
    pub kind: SymbolKind,
}

impl Symbol {
    /// Create a new symbol.
    pub fn new(name: Intern<String>, source_file: PathBuf, kind: SymbolKind) -> Self {
        Self {
            name,
            source_file,
            kind,
        }
    }

    /// Create a tag symbol.
    pub fn tag(name: Intern<String>, source_file: PathBuf) -> Self {
        Self {
            name,
            source_file,
            kind: SymbolKind::Tag(name),
        }
    }

    /// Create a function symbol.
    pub fn function(name: Intern<String>, source_file: PathBuf) -> Self {
        Self {
            name,
            source_file,
            kind: SymbolKind::Function(name),
        }
    }

    /// Create a bind symbol.
    pub fn bind(name: Intern<String>, source_file: PathBuf) -> Self {
        Self {
            name,
            source_file,
            kind: SymbolKind::Bind(name),
        }
    }

    /// Check if this is a function.
    pub fn is_function(&self) -> bool {
        matches!(self.kind, SymbolKind::Function(_))
    }

    /// Check if this is a bind (value).
    pub fn is_bind(&self) -> bool {
        matches!(self.kind, SymbolKind::Bind(_))
    }

    /// Check if this is a tag.
    pub fn is_tag(&self) -> bool {
        matches!(self.kind, SymbolKind::Tag(_))
    }
}

/// Compile-time symbol table.
///
/// This tracks all visible symbols at compile time, enabling:
/// - Cross-file symbol resolution
/// - Duplicate detection
/// - Import validation
///
/// Runtime MLIR values are tracked separately during codegen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolTable {
    /// Map of symbol name to symbol information
    pub symbols: HashMap<Intern<String>, Symbol>,
}

impl SymbolTable {
    /// Create a new empty symbol table.
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
        }
    }

    /// Insert a symbol into the table.
    pub fn insert(&mut self, symbol: Symbol) {
        self.symbols.insert(symbol.name, symbol);
    }

    /// Look up a symbol by name.
    pub fn get(&self, name: &Intern<String>) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    /// Check if a symbol exists.
    pub fn contains(&self, name: &Intern<String>) -> bool {
        self.symbols.contains_key(name)
    }

    /// Get all function names.
    pub fn function_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| s.is_function())
            .map(|s| s.name)
            .collect()
    }

    /// Get all bind names.
    pub fn bind_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| s.is_bind())
            .map(|s| s.name)
            .collect()
    }

    /// Get all tag names.
    pub fn tag_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| s.is_tag())
            .map(|s| s.name)
            .collect()
    }

    /// Merge another symbol table into this one.
    /// Returns conflicting symbols (names that exist in both).
    pub fn merge(&mut self, other: SymbolTable) -> Vec<Symbol> {
        let mut conflicts = Vec::new();

        for (name, symbol) in other.symbols {
            if let std::collections::hash_map::Entry::Vacant(e) = self.symbols.entry(name) {
                e.insert(symbol);
            } else {
                conflicts.push(symbol);
            }
        }

        conflicts
    }

    /// Create a symbol table from a single file's AST.
    pub fn from_file(file: &FileAst, source_path: PathBuf) -> Self {
        Self::from_file_filtered(file, source_path, false)
    }

    pub fn from_file_public(file: &FileAst, source_path: PathBuf) -> Self {
        Self::from_file_filtered(file, source_path, true)
    }

    fn from_file_filtered(file: &FileAst, source_path: PathBuf, public_only: bool) -> Self {
        let mut table = Self::new();

        for tag_name in file.tags().keys() {
            if public_only && file.private_tags().contains(tag_name) {
                continue;
            }
            table.insert(Symbol::tag(*tag_name, source_path.to_path_buf()));
        }

        for (def_name, bind) in file.defs() {
            if public_only && file.private_defs().contains(def_name) {
                continue;
            }
            let source_path = source_path.clone();
            let symbol = if bind.params().is_some() {
                Symbol::function(*def_name, source_path)
            } else {
                Symbol::bind(*def_name, source_path)
            };
            table.insert(symbol);
        }

        table
    }
}

/// Output of parsing a gin file.
#[derive(Debug, Clone, Default)]
pub struct FileAst {
    /// Module-level doc comment collected from leading `--| ...` lines.
    pub module_doc: Option<DocComment>,
    pub uses: Vec<Import>,
    pub tags: TagMap,
    pub defs: DefMap,
    pub private_defs: HashSet<Intern<String>>,
    pub private_tags: HashSet<Intern<String>>,
    pub exprs: Vec<(Expr, SpanId)>,
    /// Span table mapping SpanId → Span (byte ranges).
    /// Populated during parsing; excluded from Hash/Eq.
    pub span_table: SpanTable,
}

impl FileAst {
    pub fn module_doc(&self) -> Option<&DocComment> {
        self.module_doc.as_ref()
    }

    pub fn uses(&self) -> &[Import] {
        &self.uses
    }

    pub fn tags(&self) -> &TagMap {
        &self.tags
    }

    pub fn defs(&self) -> &DefMap {
        &self.defs
    }

    /// Access the span table for resolving SpanId → byte ranges.
    pub fn span_table(&self) -> &SpanTable {
        &self.span_table
    }

    pub fn private_defs(&self) -> &HashSet<Intern<String>> {
        &self.private_defs
    }

    pub fn private_tags(&self) -> &HashSet<Intern<String>> {
        &self.private_tags
    }

    pub fn top_level_exprs(&self) -> &[(Expr, SpanId)] {
        &self.exprs
    }

    /// Return sorted public tag names.
    pub fn public_tag_names(&self) -> Vec<Intern<String>> {
        let mut names: Vec<_> = self
            .tags
            .keys()
            .filter(|n| !self.private_tags.contains(n))
            .copied()
            .collect();
        names.sort();
        names
    }

    /// Return sorted public def names.
    pub fn public_def_names(&self) -> Vec<Intern<String>> {
        let mut names: Vec<_> = self
            .defs
            .keys()
            .filter(|n| !self.private_defs.contains(n))
            .copied()
            .collect();
        names.sort();
        names
    }

    /// Remove `private` tags and defs so importers only see the public API (after [`crate::qualify_module_defs`]).
    pub fn strip_private_for_importer(mut self) -> Self {
        self.tags.retain(|k, _| !self.private_tags.contains(k));
        self.defs.retain(|k, _| !self.private_defs.contains(k));
        self.private_tags.clear();
        self.private_defs.clear();
        self
    }
}

/// Duplicate top-level name when merging compilation units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeConflict {
    Tag { name: Intern<String> },
    Def { name: Intern<String> },
}

impl FileAst {
    /// Merge defs and tags from `other` into `self`.
    ///
    /// Existing entries in `self` take precedence (entry file can shadow dependency symbols).
    /// Dependency defs that don't match the current build platform are skipped, allowing the same
    /// name (e.g. `SYS_WRITE`) to be defined in separate platform-specific files.
    /// The dependency's top-level exprs and private symbols are not imported.
    pub fn merge_from(&mut self, other: FileAst) {
        for (name, declare) in other.tags {
            self.tags.entry(name).or_insert(declare);
        }
        for (name, bind) in other.defs {
            if self.defs.contains_key(&name) {
                continue;
            }
            if bind.attributes().matches_current_platform() {
                self.defs.insert(name, bind);
            }
        }
    }

    /// Like [`merge_from`], but returns an error if `other` introduces a tag or def that already exists.
    pub fn merge_from_checked(&mut self, other: FileAst) -> Result<(), MergeConflict> {
        for name in other.tags.keys() {
            if self.tags.contains_key(name) {
                return Err(MergeConflict::Tag { name: *name });
            }
        }
        for name in other.defs.keys() {
            if self.defs.contains_key(name) {
                return Err(MergeConflict::Def { name: *name });
            }
        }
        for (name, declare) in other.tags {
            self.tags.insert(name, declare);
        }
        for (name, bind) in other.defs {
            if bind.attributes().matches_current_platform() {
                self.defs.insert(name, bind);
            }
        }
        Ok(())
    }
}

impl PartialEq for FileAst {
    fn eq(&self, other: &Self) -> bool {
        self.module_doc == other.module_doc
            && self.uses == other.uses
            && self.tags == other.tags
            && self.defs == other.defs
            && self.private_defs == other.private_defs
            && self.private_tags == other.private_tags
            && self.exprs == other.exprs
            && self.span_table == other.span_table
    }
}

impl Eq for FileAst {}

impl Hash for FileAst {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.module_doc.hash(state);
        self.uses.hash(state);
        // Sort keys for deterministic hashing
        let mut tag_keys: Vec<_> = self.tags.keys().collect();
        tag_keys.sort();
        for k in tag_keys {
            k.hash(state);
            self.tags[k].hash(state);
            self.private_tags.contains(k).hash(state);
        }
        let mut def_keys: Vec<_> = self.defs.keys().collect();
        def_keys.sort();
        for k in def_keys {
            k.hash(state);
            self.defs[k].hash(state);
            self.private_defs.contains(k).hash(state);
        }
        self.exprs.hash(state);
        self.span_table.hash(state);
    }
}

impl FileAst {
    /// Compute a content-based hash for change detection within a compilation session.
    pub fn compute_content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}
