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

impl Symbol {
    pub fn new(name: Intern<String>, source_file: PathBuf, kind: SymbolKind) -> Self {
        Self {
            name,
            source_file,
            kind,
        }
    }

    pub fn tag(name: Intern<String>, source_file: PathBuf) -> Self {
        Self {
            name,
            source_file,
            kind: SymbolKind::Tag(name),
        }
    }

    pub fn function(name: Intern<String>, source_file: PathBuf) -> Self {
        Self {
            name,
            source_file,
            kind: SymbolKind::Function(name),
        }
    }

    pub fn bind(name: Intern<String>, source_file: PathBuf) -> Self {
        Self {
            name,
            source_file,
            kind: SymbolKind::Bind(name),
        }
    }

    pub fn is_function(&self) -> bool {
        matches!(self.kind, SymbolKind::Function(_))
    }

    pub fn is_bind(&self) -> bool {
        matches!(self.kind, SymbolKind::Bind(_))
    }

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
            .filter(|s| s.is_function())
            .map(|s| s.name)
            .collect()
    }

    pub fn bind_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| s.is_bind())
            .map(|s| s.name)
            .collect()
    }

    pub fn tag_names(&self) -> Vec<Intern<String>> {
        self.symbols
            .values()
            .filter(|s| s.is_tag())
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

impl FileAst {
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
                return find_expr_at_byte(&self.span_table, expr, *span_id, byte_pos);
            }
        }
        // Search def bodies
        for bind in self.defs.values() {
            let result = find_expr_in_bind_value(&self.span_table, bind.value(), byte_pos);
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
                    let rel_pos = byte_pos - span.start;
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
                    Some(bind.name().as_str().to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn definition_span(&self, name: &str) -> Option<std::ops::Range<usize>> {
        crate::hover::definition_span(self, name)
    }
}

/// Recursively find the innermost `(Expr, SpanId)` containing `byte_pos`.
fn find_expr_at_byte<'a>(
    st: &SpanTable,
    expr: &'a Expr,
    span_id: SpanId,
    byte_pos: usize,
) -> Option<(&'a Expr, SpanId)> {
    match expr {
        // Leaf nodes — return self
        Expr::Lit(_) | Expr::SelfRef | Expr::AnonymousTag(..) => Some((expr, span_id)),

        // Nested: check children, then return self as innermost
        Expr::Bind(bind) => {
            let result = find_expr_in_bind_value(st, bind.value(), byte_pos);
            result.or(Some((expr, span_id)))
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    if st.contains(arg.span_id(), byte_pos) {
                        return find_expr_at_byte(st, &arg.value, arg.span_id(), byte_pos);
                    }
                }
            }
            Some((expr, span_id))
        }
        Expr::Binary(bin) => {
            if st.contains(bin.lhs.span_id(), byte_pos) {
                return find_expr_at_byte(st, &bin.lhs.value, bin.lhs.span_id(), byte_pos);
            }
            if st.contains(bin.rhs.span_id(), byte_pos) {
                return find_expr_at_byte(st, &bin.rhs.value, bin.rhs.span_id(), byte_pos);
            }
            Some((expr, span_id))
        }
        Expr::When(when) => {
            if let Some(subject) = &when.subject
                && st.contains(subject.span_id(), byte_pos)
            {
                return find_expr_at_byte(st, &subject.value, subject.span_id(), byte_pos);
            }
            for arm in &when.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        for child in [condition.as_ref(), body.as_ref()] {
                            if st.contains(child.span_id(), byte_pos) {
                                return find_expr_at_byte(
                                    st,
                                    &child.value,
                                    child.span_id(),
                                    byte_pos,
                                );
                            }
                        }
                    }
                    WhenArm::Is { body, .. } => {
                        if st.contains(body.span_id(), byte_pos) {
                            return find_expr_at_byte(st, &body.value, body.span_id(), byte_pos);
                        }
                    }
                    WhenArm::Else(body, _) => {
                        if st.contains(body.span_id(), byte_pos) {
                            return find_expr_at_byte(st, &body.value, body.span_id(), byte_pos);
                        }
                    }
                }
            }
            Some((expr, span_id))
        }
        Expr::If(ifx) => {
            match &ifx.condition {
                IfCondition::Bool(e) => {
                    if st.contains(e.span_id(), byte_pos) {
                        return find_expr_at_byte(st, &e.value, e.span_id(), byte_pos);
                    }
                }
                IfCondition::Pattern { subject, .. } => {
                    if st.contains(subject.span_id(), byte_pos) {
                        return find_expr_at_byte(st, &subject.value, subject.span_id(), byte_pos);
                    }
                }
            }
            for e in &ifx.body {
                if st.contains(e.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &e.value, e.span_id(), byte_pos);
                }
            }
            if let Some(ret_expr) = &ifx.ret.value
                && st.contains(ret_expr.span_id(), byte_pos)
            {
                return find_expr_at_byte(st, &ret_expr.value, ret_expr.span_id(), byte_pos);
            }
            Some((expr, span_id))
        }
        Expr::Loop(loop_val) => match loop_val {
            LoopEnum::While(w) => {
                if st.contains(w.cond.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &w.cond.value, w.cond.span_id(), byte_pos);
                }
                for e in &w.exprs {
                    if st.contains(e.span_id(), byte_pos) {
                        return find_expr_at_byte(st, &e.value, e.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
            LoopEnum::ForIn(f) => {
                for child in [&f.pat, &f.iter] {
                    if st.contains(child.span_id(), byte_pos) {
                        return find_expr_at_byte(st, &child.value, child.span_id(), byte_pos);
                    }
                }
                for e in &f.exprs {
                    if st.contains(e.span_id(), byte_pos) {
                        return find_expr_at_byte(st, &e.value, e.span_id(), byte_pos);
                    }
                }
                Some((expr, span_id))
            }
        },
        Expr::TagCall(tc) => {
            for arg in &tc.args {
                if st.contains(arg.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &arg.value, arg.span_id(), byte_pos);
                }
            }
            Some((expr, span_id))
        }
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let FormatPart::Expr(e, _) = part
                    && st.contains(e.span_id(), byte_pos)
                {
                    return find_expr_at_byte(st, &e.value, e.span_id(), byte_pos);
                }
            }
            Some((expr, span_id))
        }
        Expr::Range(r) => {
            for child in [&r.start, &r.end] {
                if st.contains(child.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &child.value, child.span_id(), byte_pos);
                }
            }
            Some((expr, span_id))
        }
        Expr::Asm(a) => {
            for op in &a.operands {
                if st.contains(op.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &op.value, op.span_id(), byte_pos);
                }
            }
            Some((expr, span_id))
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                if st.contains(e.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &e.value, e.span_id(), byte_pos);
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
                return find_expr_at_byte(st, &init.value, init.span_id(), byte_pos);
            }
            Some((expr, span_id))
        }
        Expr::TupleGet { base, .. }
        | Expr::Cast { expr: base, .. }
        | Expr::BufGet { buf: base, .. } => {
            if st.contains(base.span_id(), byte_pos) {
                return find_expr_at_byte(st, &base.value, base.span_id(), byte_pos);
            }
            Some((expr, span_id))
        }
        Expr::TupleSet { base, value, .. }
        | Expr::BufSet {
            buf: base,
            index: value,
            ..
        } => {
            for child in [base.as_ref(), value.as_ref()] {
                if st.contains(child.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &child.value, child.span_id(), byte_pos);
                }
            }
            Some((expr, span_id))
        }
        Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {
            Some((expr, span_id))
        }
    }
}

/// Recursively find an expression at `byte_pos` inside a `BindValue`.
fn find_expr_in_bind_value<'a>(
    st: &SpanTable,
    val: &'a BindValue,
    byte_pos: usize,
) -> Option<(&'a Expr, SpanId)> {
    match val {
        BindValue::Expr(expr) => {
            if st.contains(expr.span_id(), byte_pos) {
                find_expr_at_byte(st, &expr.value, expr.span_id(), byte_pos)
            } else {
                None
            }
        }
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                if st.contains(e.span_id(), byte_pos) {
                    return find_expr_at_byte(st, &e.value, e.span_id(), byte_pos);
                }
            }
            if let Some(ret_expr) = &ret.value
                && st.contains(ret_expr.span_id(), byte_pos)
            {
                return find_expr_at_byte(st, &ret_expr.value, ret_expr.span_id(), byte_pos);
            }
            None
        }
        BindValue::Extern => None,
    }
}
