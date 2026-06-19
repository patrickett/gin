use crate::blanket_impl::BlanketImpl;
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

impl Symbol {}

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

    pub fn function_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| matches!(s.kind, SymbolKind::Function(_)))
            .map(|s| s.name)
            .collect()
    }

    pub fn bind_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| matches!(s.kind, SymbolKind::Bind(_)))
            .map(|s| s.name)
            .collect()
    }

    pub fn tag_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| matches!(s.kind, SymbolKind::Tag(_)))
            .map(|s| s.name)
            .collect()
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

/// Output of parsing a gin file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileAst {
    /// Module-level doc comment collected from leading `--| ...` lines.
    pub module_doc: Option<DocComment>,
    pub uses: Vec<Import>,
    pub tags: TagMap,
    pub defs: DefMap,
    pub private_defs: HashSet<Intern<String>>,
    pub private_tags: HashSet<Intern<String>>,
    pub exprs: Vec<(Expr, SpanId)>,
    pub symbol_aliases: Vec<SymbolAlias>,
    pub symbol_alias_spans: Vec<SpanId>,
    /// Span table mapping SpanId → Span (byte ranges).
    pub span_table: SpanTable,
    /// Quantified trait impls: `x.Trait(...)`.
    pub blanket_impls: Vec<BlanketImpl>,
    /// Parse-time warnings (e.g. `name Ty` then `name := expr`).
    pub parse_warnings: Vec<diagnostic::Diagnostic>,
}

impl FileAst {
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
        self.blanket_impls.extend(other.blanket_impls);
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
                let imported = mi
                    .alias
                    .unwrap_or_else(|| Intern::<String>::new(mi.effective_name()));
                if imported != key {
                    continue;
                }
                if let crate::ImportSource::Package(mod_path) = &mi.source
                    && let Some(last_seg) = mod_path.segments.last()
                {
                    let path_span = span_table.get(mod_path.span_id());
                    let path_text = source.get(path_span.start()..path_span.end())?;
                    let dot_pos = path_text.rfind('.')?;
                    let seg_start = path_span.start() + dot_pos + 1;
                    let seg_end = seg_start + last_seg.len();
                    return Some(seg_start..seg_end);
                }
                if let crate::ImportSource::LocalBundle(b) = &mi.source {
                    for member in &b.members {
                        let member_name = member.alias.unwrap_or(member.export);
                        if member_name == key {
                            let mspan = span_table.get(member.span);
                            return Some(mspan.start()..mspan.end());
                        }
                    }
                }
                if let crate::ImportSource::CurrentModule { member } = &mi.source
                    && member.alias.unwrap_or(member.export) == key
                {
                    let mspan = span_table.get(member.span);
                    return Some(mspan.start()..mspan.end());
                }
                let span = span_table.get(mi.source.span_id());
                return Some(span.start()..span.end());
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

    fn collect_refs_type_surface(
        expr: &crate::TypeExpr,
        name: &str,
        span_table: &SpanTable,
        out: &mut Vec<std::ops::Range<usize>>,
    ) {
        if let crate::TypeExpr::Generic { params, .. } = expr {
            for (_, pk) in params {
                match pk {
                    crate::ParameterKind::Default(e) => {
                        Self::collect_refs_expr(&e.value, name, span_table, out)
                    }
                    crate::ParameterKind::Tagged(sp) => {
                        Self::collect_refs_type_surface(&sp.value, name, span_table, out);
                    }
                    crate::ParameterKind::Generic => {}
                }
            }
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
                if let Some(args) = &call.args {
                    for arg in args {
                        Self::collect_refs_expr(arg, name, span_table, out);
                    }
                }
            }
            crate::Expr::TagCall(tc) => {
                for arg in &tc.args {
                    Self::collect_refs_expr(arg, name, span_table, out);
                }
            }
            Binary(bin) => {
                Self::collect_refs_expr(&bin.lhs.value, name, span_table, out);
                Self::collect_refs_expr(&bin.rhs.value, name, span_table, out);
            }
            Bind(bind) => Self::collect_refs_bind_value(&bind.value, name, span_table, out),
            When(when_expr) => {
                if let Some(subject) = &when_expr.subject {
                    Self::collect_refs_expr(subject, name, span_table, out);
                }
                for arm in &when_expr.arms {
                    match arm {
                        crate::WhenArm::Cond {
                            condition, body, ..
                        } => {
                            Self::collect_refs_expr(condition, name, span_table, out);
                            Self::collect_refs_expr(body, name, span_table, out);
                        }
                        crate::WhenArm::Is { pattern, body, .. } => {
                            Self::collect_refs_type_surface(&pattern.value, name, span_table, out);
                            Self::collect_refs_expr(body, name, span_table, out);
                        }
                        crate::WhenArm::Else(body, _) => {
                            Self::collect_refs_expr(body, name, span_table, out);
                        }
                    }
                }
            }
            If(if_expr) => {
                Self::collect_refs_expr(&if_expr.subject.value, name, span_table, out);
                for e in &if_expr.body {
                    Self::collect_refs_expr(e, name, span_table, out);
                }
            }
            Loop(loop_expr) => match loop_expr {
                crate::LoopEnum::ForIn(for_loop) => {
                    Self::collect_refs_expr(&for_loop.iter, name, span_table, out);
                    for e in &for_loop.exprs {
                        Self::collect_refs_expr(e, name, span_table, out);
                    }
                }
                crate::LoopEnum::While(while_loop) => {
                    Self::collect_refs_expr(&while_loop.cond, name, span_table, out);
                    for e in &while_loop.exprs {
                        Self::collect_refs_expr(e, name, span_table, out);
                    }
                }
            },
            FormatString(fs) => {
                for part in &fs.parts {
                    if let crate::FormatPart::Expr(e, _) = part {
                        Self::collect_refs_expr(e, name, span_table, out);
                    }
                }
            }
            Range(range) => {
                Self::collect_refs_expr(&range.start.value, name, span_table, out);
                Self::collect_refs_expr(&range.end.value, name, span_table, out);
            }
            TupleAlloc { init, .. } => Self::collect_refs_expr(init, name, span_table, out),
            TupleGet { base, .. } => Self::collect_refs_expr(base, name, span_table, out),
            TupleSet { base, value, .. } => {
                Self::collect_refs_expr(base, name, span_table, out);
                Self::collect_refs_expr(value, name, span_table, out);
            }
            Cast { expr, .. } => Self::collect_refs_expr(expr, name, span_table, out),
            BufGet { buf, index, .. } => {
                Self::collect_refs_expr(buf, name, span_table, out);
                Self::collect_refs_expr(index, name, span_table, out);
            }
            BufSet {
                buf, index, value, ..
            } => {
                Self::collect_refs_expr(buf, name, span_table, out);
                Self::collect_refs_expr(index, name, span_table, out);
                Self::collect_refs_expr(value, name, span_table, out);
            }
            TakePtr(inner)
            | Ref { inner, .. }
            | ConsumeArg(inner)
            | Eat(inner)
            | Deref(inner)
            | Negate(inner) => {
                Self::collect_refs_expr(inner, name, span_table, out);
            }
            RecordLit(fields) => {
                for (_, e) in fields {
                    Self::collect_refs_expr(e, name, span_table, out);
                }
            }
            RecordSet { base, value, .. } => {
                Self::collect_refs_expr(base, name, span_table, out);
                Self::collect_refs_expr(value, name, span_table, out);
            }
            Destructure { value, .. } => {
                Self::collect_refs_expr(value, name, span_table, out);
            }
            TupleLit(elems) | List(elems) => {
                for e in elems {
                    Self::collect_refs_expr(e, name, span_table, out);
                }
            }
            Lit(_)
            | SelfRef
            | Asm(_)
            | TypeNominal(..)
            | TypeQualified(_)
            | TypeGeneric { .. }
            | TypeInRange(_)
            | TypeRef { .. }
            | RecordGet { .. }
            | AnonymousTag(_) => {}
        }
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
    /// - `AnonymousTag` / `TypeNominal` / `TypeQualified` → the tag/variant name
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
        let st = &self.span_table;
        match expr {
            // Leaf nodes — return self
            Expr::Lit(_) | Expr::SelfRef | Expr::AnonymousTag(..) => Some((expr, span_id)),

            // Nested: check children, then return self as innermost
            Expr::Bind(bind) => {
                let result = self.find_expr_in_bind_value(&bind.value, byte_pos);
                result.or(Some((expr, span_id)))
            }
            Expr::FnCall(call) => {
                if let Some(args) = &call.args {
                    for arg in args {
                        if st.contains(arg.span_id(), byte_pos) {
                            return self.find_expr_at_byte(&arg.value, arg.span_id(), byte_pos);
                        }
                    }
                }
                Some((expr, span_id))
            }
            Expr::Binary(bin) => {
                if st.contains(bin.lhs.span_id(), byte_pos) {
                    return self.find_expr_at_byte(&bin.lhs.value, bin.lhs.span_id(), byte_pos);
                }
                if st.contains(bin.rhs.span_id(), byte_pos) {
                    return self.find_expr_at_byte(&bin.rhs.value, bin.rhs.span_id(), byte_pos);
                }
                Some((expr, span_id))
            }
            Expr::When(when) => {
                if let Some(subject) = &when.subject
                    && st.contains(subject.span_id(), byte_pos)
                {
                    return self.find_expr_at_byte(&subject.value, subject.span_id(), byte_pos);
                }
                for arm in &when.arms {
                    match arm {
                        WhenArm::Cond {
                            condition, body, ..
                        } => {
                            for child in [condition.as_ref(), body.as_ref()] {
                                if st.contains(child.span_id(), byte_pos) {
                                    return self.find_expr_at_byte(
                                        &child.value,
                                        child.span_id(),
                                        byte_pos,
                                    );
                                }
                            }
                        }
                        WhenArm::Is { body, .. } => {
                            if st.contains(body.span_id(), byte_pos) {
                                return self.find_expr_at_byte(
                                    &body.value,
                                    body.span_id(),
                                    byte_pos,
                                );
                            }
                        }
                        WhenArm::Else(body, _) => {
                            if st.contains(body.span_id(), byte_pos) {
                                return self.find_expr_at_byte(
                                    &body.value,
                                    body.span_id(),
                                    byte_pos,
                                );
                            }
                        }
                    }
                }
                Some((expr, span_id))
            }
            Expr::If(ifx) => {
                if st.contains(ifx.subject.span_id(), byte_pos) {
                    return self.find_expr_at_byte(
                        &ifx.subject.value,
                        ifx.subject.span_id(),
                        byte_pos,
                    );
                }
                for e in &ifx.body {
                    if st.contains(e.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&e.value, e.span_id(), byte_pos);
                    }
                }
                if let Some(ret_expr) = &ifx.ret.value
                    && st.contains(ret_expr.span_id(), byte_pos)
                {
                    return self.find_expr_at_byte(&ret_expr.value, ret_expr.span_id(), byte_pos);
                }
                Some((expr, span_id))
            }
            Expr::Loop(loop_val) => match loop_val {
                LoopEnum::While(w) => {
                    if st.contains(w.cond.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&w.cond.value, w.cond.span_id(), byte_pos);
                    }
                    for e in &w.exprs {
                        if st.contains(e.span_id(), byte_pos) {
                            return self.find_expr_at_byte(&e.value, e.span_id(), byte_pos);
                        }
                    }
                    Some((expr, span_id))
                }
                LoopEnum::ForIn(f) => {
                    for child in [&f.pat, &f.iter] {
                        if st.contains(child.span_id(), byte_pos) {
                            return self.find_expr_at_byte(&child.value, child.span_id(), byte_pos);
                        }
                    }
                    for e in &f.exprs {
                        if st.contains(e.span_id(), byte_pos) {
                            return self.find_expr_at_byte(&e.value, e.span_id(), byte_pos);
                        }
                    }
                    Some((expr, span_id))
                }
            },
            Expr::TagCall(tc) => {
                for arg in &tc.args {
                    if st.contains(arg.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&arg.value, arg.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            Expr::FormatString(fs) => {
                for part in &fs.parts {
                    if let FormatPart::Expr(e, _) = part
                        && st.contains(e.span_id(), byte_pos)
                    {
                        return self.find_expr_at_byte(&e.value, e.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            Expr::Range(r) => {
                for child in [&r.start, &r.end] {
                    if st.contains(child.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&child.value, child.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            Expr::Asm(a) => {
                for op in &a.operand_values {
                    if st.contains(op.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&op.value, op.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            Expr::TupleLit(elems) | Expr::List(elems) => {
                for e in elems {
                    if st.contains(e.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&e.value, e.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            Expr::RecordLit(fields) => {
                for (_, e) in fields {
                    if st.contains(e.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&e.value, e.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            Expr::TupleAlloc { init, .. }
            | Expr::TakePtr(init)
            | Expr::Ref { inner: init, .. }
            | Expr::ConsumeArg(init)
            | Expr::Deref(init)
            | Expr::Negate(init)
            | Expr::Eat(init) => {
                if st.contains(init.span_id(), byte_pos) {
                    return self.find_expr_at_byte(&init.value, init.span_id(), byte_pos);
                }
                Some((expr, span_id))
            }
            Expr::TupleGet { base, .. }
            | Expr::RecordGet { base, .. }
            | Expr::Cast { expr: base, .. }
            | Expr::BufGet { buf: base, .. } => {
                if st.contains(base.span_id(), byte_pos) {
                    return self.find_expr_at_byte(&base.value, base.span_id(), byte_pos);
                }
                Some((expr, span_id))
            }
            Expr::TupleSet { base, value, .. }
            | Expr::RecordSet { base, value, .. }
            | Expr::BufSet {
                buf: base,
                index: value,
                ..
            } => {
                for child in [base.as_ref(), value.as_ref()] {
                    if st.contains(child.span_id(), byte_pos) {
                        return self.find_expr_at_byte(&child.value, child.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            Expr::Destructure { value, .. } => {
                if st.contains(value.span_id(), byte_pos) {
                    return self.find_expr_at_byte(&value.value, value.span_id(), byte_pos);
                }
                Some((expr, span_id))
            }
            Expr::TypeNominal(..)
            | Expr::TypeInRange(..)
            | Expr::TypeQualified(_)
            | Expr::TypeGeneric { .. }
            | Expr::TypeRef { .. } => Some((expr, span_id)),
        }
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
