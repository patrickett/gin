use crate::frontend::prelude::*;
use chumsky::container::Container;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
};

pub type TagMap = HashMap<TagName, Documented<Params<TagValue>>>;
pub type DefMap = HashMap<DefName, Documented<Params<DefValue>>>;

/// Symbol kind - distinguishes between different types of symbols.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A tag/type definition (e.g., `Person ::= ...`)
    Tag(TagName),
    /// A function definition (e.g., `foo : { ... }`)
    Function(DefName),
    /// A value binding (e.g., `x : 42`)
    Bind(DefName),
}

/// Compile-time symbol with source information.
///
/// This tracks symbol metadata at compile time, separate from runtime
/// MLIR values which are tracked during codegen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    /// The symbol name (e.g., "http.web.handle" or "foo")
    pub name: IStr,

    /// Which file defined this symbol
    pub source_file: PathBuf,

    /// What kind of symbol this is
    pub kind: SymbolKind,
}

impl Symbol {
    /// Create a new symbol.
    pub fn new(name: IStr, source_file: PathBuf, kind: SymbolKind) -> Self {
        Self {
            name,
            source_file,
            kind,
        }
    }

    /// Create a tag symbol.
    pub fn tag(name: TagName, source_file: PathBuf) -> Self {
        Self {
            name: name.0,
            source_file,
            kind: SymbolKind::Tag(name),
        }
    }

    /// Create a function symbol.
    pub fn function(name: DefName, source_file: PathBuf) -> Self {
        Self {
            name: name.0,
            source_file,
            kind: SymbolKind::Function(name),
        }
    }

    /// Create a bind symbol.
    pub fn bind(name: DefName, source_file: PathBuf) -> Self {
        Self {
            name: name.0,
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
    pub symbols: HashMap<IStr, Symbol>,
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
    pub fn get(&self, name: &IStr) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    /// Check if a symbol exists.
    pub fn contains(&self, name: &IStr) -> bool {
        self.symbols.contains_key(name)
    }

    /// Get all function names.
    pub fn function_names(&self) -> Vec<IStr> {
        self.symbols
            .values()
            .filter(|s| s.is_function())
            .map(|s| s.name)
            .collect()
    }

    /// Get all bind names.
    pub fn bind_names(&self) -> Vec<IStr> {
        self.symbols
            .values()
            .filter(|s| s.is_bind())
            .map(|s| s.name)
            .collect()
    }

    /// Get all tag names.
    pub fn tag_names(&self) -> Vec<IStr> {
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

        // Add all tag definitions
        for tag_name in file.tags.keys() {
            if public_only && file.private_tags.contains(tag_name) {
                continue;
            }
            table.insert(Symbol::tag(tag_name.clone(), source_path.to_path_buf()));
        }

        // Add all definitions (functions and binds)
        for (def_name, documented) in &file.defs {
            if public_only && file.private_defs.contains(def_name) {
                continue;
            }
            let source_path = source_path.clone();
            let symbol = if documented.item.0.is_some() {
                // Has parameters - it's a function
                Symbol::function(def_name.clone(), source_path)
            } else {
                // No parameters - it's a bind/value
                Symbol::bind(def_name.clone(), source_path)
            };
            table.insert(symbol);
        }

        table
    }
}

impl Container<Item> for (TagMap, DefMap) {
    fn push(&mut self, item: Item) {
        match item.value {
            ItemValue::TagValue(tag_name, params) => {
                let doc = Documented {
                    item: params,
                    doc: item.doc_comment,
                };

                self.0.insert(tag_name, doc);
            }
            ItemValue::DefValue(def_name, params) => {
                let doc = Documented {
                    item: params,
                    doc: item.doc_comment,
                };
                self.1.insert(def_name, doc);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
/// Output of parsing a gin file
pub struct FileAst {
    pub uses: Vec<Import>,
    // TODO: items (TagMap, DefMap)
    pub tags: TagMap,
    pub defs: DefMap,
    // TODO: private_items (TagMap, DefMap)
    pub private_defs: HashSet<DefName>,
    pub private_tags: HashSet<TagName>,
}

impl PartialEq for FileAst {
    fn eq(&self, other: &Self) -> bool {
        self.uses == other.uses
            && self.tags == other.tags
            && self.defs == other.defs
            && self.private_defs == other.private_defs
            && self.private_tags == other.private_tags
    }
}

impl Eq for FileAst {}

impl Hash for FileAst {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.uses.hash(state);
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
