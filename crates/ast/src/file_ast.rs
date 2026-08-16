use crate::path::ModPath;
use crate::prelude::*;
use crate::span::{SpanId, SpanTable, Spanned};
use indexmap::IndexMap;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path::PathBuf,
};

pub type TagMap = HashMap<Intern<String>, Declare>;
/// Method name → single bind (impl blocks, etc.).
pub type MethodMap = HashMap<Intern<String>, Bind>;
/// Top-level def name → one bind.
pub type DefMap = IndexMap<Intern<String>, Bind>;

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

    pub source_file: PathBuf,

    pub kind: SymbolKind,
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
    pub symbols: HashMap<Intern<String>, Symbol>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
        }
    }

    pub fn insert(&mut self, symbol: Symbol) {
        self.symbols.insert(symbol.name, symbol);
    }

    pub fn get(&self, name: &Intern<String>) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    pub fn contains(&self, name: &Intern<String>) -> bool {
        self.symbols.contains_key(name)
    }

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

    pub fn from_file(file: &FileAst, source_path: PathBuf) -> Self {
        Self::from_file_filtered(file, source_path, false)
    }

    pub fn from_file_public(file: &FileAst, source_path: PathBuf) -> Self {
        Self::from_file_filtered(file, source_path, true)
    }

    fn from_file_filtered(file: &FileAst, source_path: PathBuf, public_only: bool) -> Self {
        let mut table = Self::new();

        for tag_name in file.tags.keys() {
            if public_only && file.private_tags.contains(tag_name) {
                continue;
            }
            table.insert(Symbol {
                name: *tag_name,
                source_file: source_path.to_path_buf(),
                kind: SymbolKind::Tag(*tag_name),
            });
        }

        for (def_name, bind) in &file.defs {
            if public_only && file.private_defs.contains(def_name) {
                continue;
            }
            let source_path = source_path.clone();
            let symbol = if bind.params.is_some() {
                Symbol {
                    name: *def_name,
                    source_file: source_path,
                    kind: SymbolKind::Function(*def_name),
                }
            } else {
                Symbol {
                    name: *def_name,
                    source_file: source_path,
                    kind: SymbolKind::Bind(*def_name),
                }
            };
            table.insert(symbol);
        }

        table
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolAlias {
    pub alias: Intern<String>,
    pub target: Spanned<ModPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSemanticOrigin {
    pub package: crate::ty::PackageInstanceKey,
    pub module: Vec<Intern<String>>,
}

/// Output of parsing a gin file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileAst {
    pub semantic_origin: Option<FileSemanticOrigin>,
    /// Module-level doc comment collected from leading `--| ...` lines.
    pub module_doc: Option<DocComment>,
    pub uses: Vec<Import>,
    pub tags: TagMap,
    pub defs: DefMap,
    pub method_binds: Vec<Bind>,
    pub private_defs: HashSet<Intern<String>>,
    pub private_tags: HashSet<Intern<String>>,
    pub exprs: Vec<(Expr, SpanId)>,
    pub symbol_aliases: Vec<SymbolAlias>,
    pub symbol_alias_spans: Vec<SpanId>,
    /// Span table mapping SpanId → Span (byte ranges).
    pub span_table: SpanTable,
    /// Parse-time warnings (e.g. `name Ty` then `name := expr`).
    pub parse_warnings: Vec<diagnostic::Diagnostic>,
}

impl FileAst {
    pub fn with_semantic_origin(mut self, semantic_origin: FileSemanticOrigin) -> Self {
        self.semantic_origin = Some(semantic_origin);
        self
    }
    /// Minimal AST for compile-time evaluation helpers (no source file).
    pub fn empty_for_tests() -> Self {
        Self::default()
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

    pub fn contains_public_symbol(&self, name: &Intern<String>) -> bool {
        (self.tags.contains_key(name) && !self.private_tags.contains(name))
            || (self.defs.contains_key(name) && !self.private_defs.contains(name))
    }

    pub fn public_symbol_names(&self) -> Vec<Intern<String>> {
        let mut names = self.public_tag_names();
        names.extend(self.public_def_names());
        names.sort();
        names.dedup();
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

    /// Collect trait/type names in scope for compile-time trait dispatch.
    pub fn imported_trait_names(&self) -> HashSet<Intern<String>> {
        let mut set = HashSet::new();
        for import in &self.uses {
            for mi in &import.0 {
                Self::collect_import_symbol(mi, &mut set);
            }
        }
        for alias in &self.symbol_aliases {
            set.insert(alias.alias);
        }
        for name in self.tags.keys() {
            set.insert(*name);
        }
        set
    }

    fn collect_import_symbol(mi: &ModuleImport, set: &mut HashSet<Intern<String>>) {
        match &mi.source {
            ImportSource::CurrentModule { member } => {
                set.insert(member.alias.unwrap_or(member.export));
            }
            ImportSource::LocalBundle(b) => {
                for m in &b.members {
                    set.insert(m.flat_name());
                }
            }
            ImportSource::LocalMember(m) => {
                set.insert(m.member.alias.unwrap_or(m.member.export));
            }
            ImportSource::Package(_) | ImportSource::Local(_, _) => {
                if let Some(alias) = mi.alias {
                    set.insert(alias);
                } else {
                    set.insert(Intern::new(mi.effective_name()));
                }
            }
        }
    }
}

impl Hash for FileAst {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.module_doc.hash(state);
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
        self.exprs.hash(state);
        self.symbol_aliases.hash(state);
        self.symbol_alias_spans.hash(state);
        self.span_table.hash(state);
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
    /// The dependency's top-level exprs and private symbols are not imported.
    pub fn merge_from(&mut self, other: FileAst) {
        for (name, declare) in other.tags {
            self.tags.entry(name).or_insert(declare);
        }
        for (name, bind) in other.defs {
            if self.defs.contains_key(&name) {
                continue;
            }
            self.defs.insert(name, bind);
        }
        self.parse_warnings.extend(other.parse_warnings);
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
            self.defs.insert(name, bind);
        }
        Ok(())
    }
}

impl FileAst {
    /// Find the definition span of a symbol by name.
    pub fn definition_span(&self, name: &str) -> Option<std::ops::Range<usize>> {
        let span_table = &self.span_table;
        let key = Intern::<String>::from_ref(name);
        if let Some(decl) = self.tags.get(&key) {
            let span = span_table.get(decl.name_span);
            return Some(span.start()..span.end());
        }
        self.defs.iter().find_map(|(k, bind)| {
            let s = k.as_str();
            if s == name || (s.contains('.') && s.split('.').next_back() == Some(name)) {
                let span = span_table.get(bind.name_span);
                Some(span.start()..span.end())
            } else {
                None
            }
        })
    }

    /// Definition span of a union variant name (`False` in `Bool is True or False`).
    pub fn variant_definition_span(&self, variant_name: &str) -> Option<std::ops::Range<usize>> {
        let span_table = &self.span_table;
        for decl in self.tags.values() {
            let crate::DeclareValue::Union { variants } = &decl.value else {
                continue;
            };
            for v in variants {
                if let Some(span_id) = v.shape().value.variant_shape_name_span(variant_name) {
                    let span = span_table.get(span_id);
                    return Some(span.start()..span.end());
                }
            }
        }
        None
    }

    /// Byte range of the `use` path text that introduces `name` into scope.
    pub fn find_import_definition_span(
        &self,
        name: &str,
        source: &str,
    ) -> Option<std::ops::Range<usize>> {
        let span_table = &self.span_table;
        let key = Intern::<String>::from_ref(name);
        for imp in &self.uses {
            for mi in &imp.0 {
                match &mi.source {
                    crate::ImportSource::Package(mod_path) => {
                        let imported = mi
                            .alias
                            .unwrap_or_else(|| Intern::<String>::new(mi.effective_name()));
                        if imported == key
                            && let Some(last_seg) = mod_path.segments.last()
                        {
                            let path_span = span_table.get(mod_path.span_id());
                            let path_text = source.get(path_span.start()..path_span.end())?;
                            let dot_pos = path_text.rfind('.')?;
                            let seg_start = path_span.start() + dot_pos + 1;
                            let seg_end = seg_start + last_seg.len();
                            return Some(seg_start..seg_end);
                        }
                    }
                    crate::ImportSource::LocalBundle(b) => {
                        for member in &b.members {
                            let member_name = member.alias.unwrap_or(member.flat_name());
                            if member_name == key {
                                let mspan = span_table.get(member.span);
                                return Some(mspan.start()..mspan.end());
                            }
                        }
                    }
                    crate::ImportSource::CurrentModule { member } => {
                        let imported = member.alias.unwrap_or(member.export);
                        if imported == key {
                            let mspan = span_table.get(member.span);
                            return Some(mspan.start()..mspan.end());
                        }
                    }
                    crate::ImportSource::LocalMember(m) => {
                        let imported = m.member.alias.unwrap_or(m.member.export);
                        if imported == key {
                            let mspan = span_table.get(m.member.span);
                            return Some(mspan.start()..mspan.end());
                        }
                    }
                    _ => {
                        let imported = mi
                            .alias
                            .unwrap_or_else(|| Intern::<String>::new(mi.effective_name()));
                        if imported == key {
                            let span = span_table.get(mi.source.span_id());
                            return Some(span.start()..span.end());
                        }
                    }
                }
            }
        }
        None
    }

    /// Find all use-sites of `name` in the AST.
    pub fn find_references(&self, name: &str) -> Vec<std::ops::Range<usize>> {
        let span_table = &self.span_table;
        let mut out = Vec::new();
        for (expr, _) in &self.exprs {
            Self::collect_refs_expr(expr, name, span_table, &mut out);
        }
        for bind in self.defs.values() {
            Self::collect_refs_bind_value(&bind.value, name, span_table, &mut out);
        }
        out
    }

    fn collect_refs_pattern(
        pattern: &crate::Pattern,
        name: &str,
        span_table: &SpanTable,
        out: &mut Vec<std::ops::Range<usize>>,
    ) {
        match pattern {
            crate::Pattern::Generic { params, .. } => {
                for (_, pk) in params {
                    match pk {
                        crate::ParameterKind::Default(e) => {
                            Self::collect_refs_expr(&e.value, name, span_table, out)
                        }
                        crate::ParameterKind::Tagged(sp) => {
                            Self::collect_refs_expr(&sp.value, name, span_table, out);
                        }
                        crate::ParameterKind::ValueParam { ty }
                        | crate::ParameterKind::Inferred { ty } => {
                            Self::collect_refs_expr(&ty.value, name, span_table, out);
                        }
                        crate::ParameterKind::Generic => {}
                    }
                }
            }
            crate::Pattern::Pointer(inner) | crate::Pattern::Ref { inner, .. } => {
                Self::collect_refs_pattern(&inner.value, name, span_table, out)
            }
            crate::Pattern::ListCons { head, tail } => {
                Self::collect_refs_pattern(&head.value, name, span_table, out);
                Self::collect_refs_pattern(&tail.value, name, span_table, out);
            }
            crate::Pattern::Tuple(elems) => {
                for elem in elems {
                    Self::collect_refs_pattern(&elem.value, name, span_table, out);
                }
            }
            crate::Pattern::Nominal(..)
            | crate::Pattern::Qualified(..)
            | crate::Pattern::Literal(..)
            | crate::Pattern::Unit
            | crate::Pattern::ListEmpty
            | crate::Pattern::InRange { .. } => {}
        }
    }

    fn collect_refs_expr(
        expr: &crate::Expr,
        name: &str,
        span_table: &SpanTable,
        out: &mut Vec<std::ops::Range<usize>>,
    ) {
        use crate::Expr::*;
        match expr {
            FnCall(call) => {
                let call_span = span_table.get(call.path.span_id());
                if call.path.segments.is_empty() {
                    if call.path.root.as_str() == name {
                        let s = call_span.start();
                        out.push(s..s + name.len());
                    }
                } else if call
                    .path
                    .segments
                    .last()
                    .is_some_and(|seg| seg.as_str() == name)
                {
                    let e = call_span.end();
                    out.push(e - name.len()..e);
                }
            }
            When(when_expr) => {
                for arm in &when_expr.arms {
                    if let crate::WhenArm::Is { pattern, .. } = arm {
                        Self::collect_refs_pattern(&pattern.value, name, span_table, out);
                    }
                }
            }
            _ => {}
        }
        let _ = crate::folder::walk_expr_children(expr, &mut |_, child| {
            Self::collect_refs_expr(child, name, span_table, out);
            std::ops::ControlFlow::Continue(())
        });
    }

    fn collect_refs_bind_value(
        value: &crate::BindValue,
        name: &str,
        span_table: &SpanTable,
        out: &mut Vec<std::ops::Range<usize>>,
    ) {
        match value {
            crate::BindValue::Expr(e) => Self::collect_refs_expr(&e.value, name, span_table, out),
            crate::BindValue::Body { exprs, ret } => {
                for e in exprs {
                    Self::collect_refs_expr(&e.value, name, span_table, out);
                }
                if let Some(r) = &ret.value {
                    Self::collect_refs_expr(&r.value, name, span_table, out);
                }
            }
            crate::BindValue::Extern => {}
            crate::BindValue::Unassigned => {}
        }
    }

    pub fn compute_content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    /// Returns the most deeply nested expression containing `byte_pos`.
    /// Searches top-level expressions and all bind bodies.
    pub fn expr_at_byte(&self, byte_pos: usize) -> Option<(&Expr, SpanId)> {
        // Search top-level expressions
        for (expr, span_id) in &self.exprs {
            if self.span_table.contains(*span_id, byte_pos) {
                return self.find_expr_at_byte(expr, *span_id, byte_pos);
            }
        }
        // Search def bodies
        for bind in self.defs.values() {
            let result = self.find_expr_in_bind_value(&bind.value, byte_pos);
            if result.is_some() {
                return result;
            }
        }
        None
    }

    /// Extract the meaningful identifier name at `byte_pos`, using AST structure
    /// instead of scanning source bytes.
    ///
    /// Returns the name for:
    /// - `AnonymousTag` → the tag/variant name
    /// - `SelfRef` → `"self"`
    /// - `TagCall` → the variant constructor name
    /// - `FnCall` → the word under the cursor within the path (handles `obj.method`)
    /// - `Bind` → the bind name (only when `byte_pos` falls within `name_span`)
    ///
    /// Returns `None` for expressions that don't carry an identifier word
    /// (e.g. literals, binaries, ranges). Callers can fall back to
    /// source-text-based `word_at_byte_offset` when this returns `None`.
    pub fn word_at_byte(&self, byte_pos: usize, source: &str) -> Option<String> {
        let (expr, _span_id) = self.expr_at_byte(byte_pos)?;
        match expr {
            Expr::AnonymousTag(name) => Some(name.as_str().to_string()),
            Expr::SelfRef => Some("self".to_string()),
            Expr::TagCall(tc) => Some(tc.name.as_str().to_string()),
            Expr::FnCall(call) => {
                // For method calls like `obj.method`, extract the precise word
                // under the cursor within the path span.
                let span = self.span_table.get(call.path.span_id);
                if span.contains(byte_pos) {
                    let path_text = span.extract(source);
                    let rel_pos = byte_pos - span.start();
                    let bytes = path_text.as_bytes();
                    let mut start = rel_pos;
                    let mut end = rel_pos;
                    fn is_word_char(b: u8) -> bool {
                        (b as char).is_alphanumeric() || b == b'_'
                    }
                    while start > 0 && is_word_char(bytes[start - 1]) {
                        start -= 1;
                    }
                    while end < bytes.len() && is_word_char(bytes[end]) {
                        end += 1;
                    }
                    if start < end {
                        Some(path_text[start..end].to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Expr::Bind(bind) => {
                let name_span = self.span_table.get(bind.name_span);
                if name_span.contains(byte_pos) {
                    Some(bind.name.as_str().to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl FileAst {
    /// Recursively find the innermost `(Expr, SpanId)` containing `byte_pos`.
    fn find_expr_at_byte<'a>(
        &self,
        expr: &'a Expr,
        span_id: SpanId,
        byte_pos: usize,
    ) -> Option<(&'a Expr, SpanId)> {
        let mut found = None;
        let _ = crate::folder::walk_expr_children(expr, &mut |child_span_id, child| {
            if self.span_table.contains(child_span_id, byte_pos) {
                found = self.find_expr_at_byte(child, child_span_id, byte_pos);
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        });
        found.or(Some((expr, span_id)))
    }

    /// Recursively find an expression at `byte_pos` inside a `BindValue`.
    fn find_expr_in_bind_value<'a>(
        &self,
        val: &'a BindValue,
        byte_pos: usize,
    ) -> Option<(&'a Expr, SpanId)> {
        let st = &self.span_table;
        match val {
            BindValue::Expr(expr) => {
                if st.contains(expr.span_id(), byte_pos) {
                    self.find_expr_at_byte(&expr.value, expr.span_id(), byte_pos)
                } else {
                    None
                }
            }
            BindValue::Body { exprs, ret } => {
                for e in exprs {
                    if st.contains(e.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&e.value, e.span_id(), byte_pos);
                    }
                }
                if let Some(ret_expr) = &ret.value
                    && st.contains(ret_expr.span_id(), byte_pos)
                {
                    return self.find_expr_at_byte(&ret_expr.value, ret_expr.span_id(), byte_pos);
                }
                None
            }
            BindValue::Extern | BindValue::Unassigned => None,
        }
    }
}
