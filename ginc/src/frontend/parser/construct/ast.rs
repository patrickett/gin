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
    pub name: String,

    /// Which file defined this symbol
    pub source_file: PathBuf,

    /// What kind of symbol this is
    pub kind: SymbolKind,
}

impl Symbol {
    /// Create a new symbol.
    pub fn new(name: String, source_file: PathBuf, kind: SymbolKind) -> Self {
        Self {
            name,
            source_file,
            kind,
        }
    }

    /// Create a tag symbol.
    pub fn tag(name: TagName, source_file: PathBuf) -> Self {
        Self {
            name: name.0.clone(),
            source_file,
            kind: SymbolKind::Tag(name),
        }
    }

    /// Create a function symbol.
    pub fn function(name: DefName, source_file: PathBuf) -> Self {
        Self {
            name: name.as_str().to_string(),
            source_file,
            kind: SymbolKind::Function(name),
        }
    }

    /// Create a bind symbol.
    pub fn bind(name: DefName, source_file: PathBuf) -> Self {
        Self {
            name: name.as_str().to_string(),
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
    pub symbols: HashMap<String, Symbol>,
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
        self.symbols.insert(symbol.name.clone(), symbol);
    }

    /// Look up a symbol by name.
    pub fn get(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    /// Check if a symbol exists.
    pub fn contains(&self, name: &str) -> bool {
        self.symbols.contains_key(name)
    }

    /// Get all function names.
    pub fn function_names(&self) -> Vec<String> {
        self.symbols
            .values()
            .filter(|s| s.is_function())
            .map(|s| s.name.clone())
            .collect()
    }

    /// Get all bind names.
    pub fn bind_names(&self) -> Vec<String> {
        self.symbols
            .values()
            .filter(|s| s.is_bind())
            .map(|s| s.name.clone())
            .collect()
    }

    /// Get all tag names.
    pub fn tag_names(&self) -> Vec<String> {
        self.symbols
            .values()
            .filter(|s| s.is_tag())
            .map(|s| s.name.clone())
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
        let mut table = Self::new();

        // Add all tag definitions
        for tag_name in file.tags.keys() {
            table.insert(Symbol::tag(tag_name.clone(), source_path.to_path_buf()));
        }

        // Add all definitions (functions and binds)
        for (def_name, documented) in &file.defs {
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
    pub uses: Vec<Import>, // import specifiers
    pub tags: TagMap,
    pub defs: DefMap,
    // TODO: pub items: Items
    // where Items is {tags, defs}
}

impl PartialEq for FileAst {
    fn eq(&self, other: &Self) -> bool {
        // Use content hash for comparison
        self.compute_content_hash() == other.compute_content_hash()
    }
}

impl Eq for FileAst {}

impl Hash for FileAst {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Use content hash for hashing
        self.compute_content_hash().hash(state);
    }
}

impl FileAst {
    /// Compute a content-based hash for efficient change detection.
    /// This is deterministic and more efficient than Debug formatting.
    pub fn compute_content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        // Hash imports in order (order matters for imports)
        for import in &self.uses {
            import.hash(&mut hasher);
        }

        // Hash tags (sorted by name for determinism)
        let mut tag_names: Vec<_> = self.tags.keys().collect();
        tag_names.sort();
        for name in tag_names {
            name.hash(&mut hasher);
            // Hash the documented item - use Debug for nested structs
            format!("{:?}", self.tags[name]).hash(&mut hasher);
        }

        // Hash defs (sorted by name for determinism)
        let mut def_names: Vec<_> = self.defs.keys().collect();
        def_names.sort();
        for name in def_names {
            name.hash(&mut hasher);
            // Hash the documented item - use Debug for nested structs
            format!("{:?}", self.defs[name]).hash(&mut hasher);
        }

        hasher.finish()
    }
}
