use std::collections::HashMap;

use ast::ast::{
    Bind, BindValue, DeclareValue, Expr, FileAst, FormatPart, IfCondition, Literal, Loop,
    ParameterKind, Tag,
};
use internment::Intern;
use ast::ast::BinOp;
use ast::ast::WhenArm;

/// A concrete compile-time value carried by `Ty::Literal`.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    Int(i128),
    Float(f64),
}

impl std::fmt::Display for LiteralValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiteralValue::Int(n) => write!(f, "{n}"),
            LiteralValue::Float(n) => write!(f, "{n}"),
        }
    }
}

/// Resolved type — the canonical representation after resolving `Tag` names against declarations.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int(u8),
    Float,
    Bool,
    Unit,
    Record {
        name: Intern::<::std::string::String>,
        fields: Vec<(Intern::<::std::string::String>, Box<Ty>)>,
    },
    Union {
        name: Intern::<::std::string::String>,
        /// Each variant: (variant_name, [(field_name, field_type)]) in declaration order.
        #[allow(clippy::type_complexity)]
        variants: Vec<(Intern::<::std::string::String>, Vec<(Intern::<::std::string::String>, Box<Ty>)>)>,
    },
    /// Unresolved / generic type — falls back to `i64` in codegen.
    Opaque(Intern::<::std::string::String>),
    /// Fixed-size stack-allocated array (`(T.new; N)`). The value is a `!llvm.ptr`.
    Array {
        elem: Box<Ty>,
        size: usize,
    },
    /// Raw pointer — erases `T` from layout, kept only for type checking. Maps to `!llvm.ptr`.
    Ptr {
        inner: Box<Ty>,
    },
    /// Reference (borrow-checked pointer in future). Same layout as `Ptr` for now.
    Ref {
        inner: Box<Ty>,
    },
    /// Positional tuple VALUE — used for tuple literals `(e1, e2, …)`. Maps to an LLVM struct.
    Tuple(Vec<Ty>),
    /// A known compile-time literal value (e.g. `3`, `1.5`).
    /// In codegen this maps to the same MLIR type as the equivalent `Int`/`Float`.
    Literal(LiteralValue),
}

impl Ty {
    /// Return record fields in layout order.
    ///
    /// Fields are sorted by descending alignment (then descending size, then declaration order
    /// for ties) so the compiler packs them without padding. The programmer writes fields in
    /// any logical order; the physical layout is determined here.
    /// Empty for non-record types.
    pub fn record_fields_sorted(&self) -> Vec<(&Intern::<::std::string::String>, &Ty)> {
        if let Ty::Record { fields, .. } = self {
            let mut indexed: Vec<(usize, &Intern::<::std::string::String>, &Ty)> = fields
                .iter()
                .enumerate()
                .map(|(i, (k, v))| (i, k, v.as_ref()))
                .collect();
            indexed.sort_by(|(i, _, a), (j, _, b)| {
                let align_a = ty_alignment(a);
                let align_b = ty_alignment(b);
                align_b
                    .cmp(&align_a)
                    .then_with(|| ty_byte_size_static(b).cmp(&ty_byte_size_static(a)))
                    .then_with(|| i.cmp(j))
            });
            indexed.into_iter().map(|(_, k, v)| (k, v)).collect()
        } else {
            vec![]
        }
    }
}

/// Type alias for union variant fields: (field_name, field_type)
#[allow(dead_code)]
type UnionFields = Vec<(Intern::<::std::string::String>, Box<Ty>)>;

/// Type alias for union variants: (variant_name, fields)
#[allow(dead_code)]
type UnionVariants = Vec<(Intern::<::std::string::String>, UnionFields)>;

/// Type alias for variant map entries: (union_name, discriminant, fields)
type VariantMapEntry = (Intern::<::std::string::String>, usize, Vec<(Intern::<::std::string::String>, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
type VariantMap = HashMap<Intern::<::std::string::String>, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
type VariantLookupResult<'a> = (Intern::<::std::string::String>, usize, &'a [(Intern::<::std::string::String>, Ty)]);

/// Type environment built from a `FileAst`. Resolves tag names to `Ty` and infers
/// function parameter / return types.
pub struct TyEnv {
    pub tag_types: HashMap<Intern::<::std::string::String>, Ty>,
    pub fn_return_types: HashMap<Intern::<::std::string::String>, Ty>,
    /// Reverse map: variant name → [(parent_union_name, discriminant_index, payload_fields)]
    /// A variant may appear in multiple unions if names collide; shape-based disambiguation is TODO.
    pub variant_map: VariantMap,
}

impl TyEnv {
    pub fn from_file_ast(ast: &FileAst) -> Self {
        Self::from_multiple_file_asts(std::slice::from_ref(ast))
    }

    pub fn from_multiple_file_asts(files: &[FileAst]) -> Self {
        let mut tag_types = HashMap::new();

        // Resolve all tag types from all files
        for ast in files {
            for name in ast.tags.keys() {
                let ty = resolve_name_from_files(*name, files, 0);
                tag_types.insert(*name, ty);
            }
        }

        // Build variant reverse map from all union types.
        let mut variant_map: VariantMap = HashMap::new();
        for (union_name, ty) in &tag_types {
            if let Ty::Union { variants, .. } = ty {
                for (i, (variant_name, fields)) in variants.iter().enumerate() {
                    let field_tys: Vec<(Intern::<::std::string::String>, Ty)> =
                        fields.iter().map(|(n, t)| (*n, *t.clone())).collect();
                    variant_map
                        .entry(*variant_name)
                        .or_default()
                        .push((*union_name, i, field_tys));
                }
            }
        }

        // First pass: seed with explicitly-annotated return types from all files.
        let mut fn_return_types = HashMap::new();
        for ast in files {
            for (name, bind) in &ast.defs {
                if !bind.attributes().matches_current_platform() {
                    continue;
                }
                let ret = infer_bind_ret(bind, &tag_types, &HashMap::new());
                fn_return_types.insert(*name, ret);
            }
        }
        // Second pass: refine with cross-function call resolution now that
        // explicitly-typed functions are in the map.
        for ast in files {
            for (name, bind) in &ast.defs {
                if !bind.attributes().matches_current_platform() {
                    continue;
                }
                let ret = infer_bind_ret(bind, &tag_types, &fn_return_types);
                fn_return_types.insert(*name, ret);
            }
        }

        TyEnv {
            tag_types,
            fn_return_types,
            variant_map,
        }
    }

    /// Resolve an AST `Tag` to a `Ty`.
    pub fn resolve_tag(&self, tag: &Tag) -> Ty {
        resolve_tag_from_map(tag, &self.tag_types)
    }

    /// Return the typed parameter list for a function binding.
    /// Preserves insertion order of the `Parameters` map.
    pub fn param_types<'a>(&self, bind: &'a Bind) -> Vec<(&'a Intern::<::std::string::String>, Ty)> {
        match bind.params().as_ref() {
            None => vec![],
            Some(params) => params
                .iter()
                .map(|(name, kind)| {
                    let ty = match kind {
                        ParameterKind::Tagged(tag) => self.resolve_tag(tag),
                        ParameterKind::Generic => Ty::Int(64),
                        ParameterKind::Default(expr) => self.infer_expr(expr, &HashMap::new()),
                    };
                    (name, ty)
                })
                .collect(),
        }
    }

    /// Infer the return type of a binding.
    pub fn return_ty(&self, bind: &Bind) -> Ty {
        infer_bind_ret(bind, &self.tag_types, &self.fn_return_types)
    }

    /// Look up the pre-computed return type of a top-level function by name.
    pub fn fn_return_ty(&self, name: &Intern::<::std::string::String>) -> Option<&Ty> {
        self.fn_return_types.get(name)
    }

    /// Look up a declared type by its tag name.
    pub fn lookup_tag(&self, name: Intern::<::std::string::String>) -> Option<&Ty> {
        self.tag_types.get(&name)
    }

    /// Look up which union a variant belongs to, its discriminant index, and payload fields.
    ///
    /// Returns `(union_name, discriminant, [(field_name, field_type)])` in declaration order.
    /// If multiple unions declare a variant with the same name, the first match is returned.
    /// TODO: shape-based disambiguation per spec (answer #6).
    pub fn lookup_variant(&self, name: Intern::<::std::string::String>) -> Option<VariantLookupResult<'_>> {
        let candidates = self.variant_map.get(&name)?;
        candidates
            .first()
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    /// Return all variant names belonging to `union_name`.
    pub fn all_variants_of(&self, union_name: Intern::<::std::string::String>) -> Vec<Intern::<::std::string::String>> {
        self.variant_map
            .iter()
            .filter_map(|(variant_name, entries)| {
                if entries.iter().any(|(u, _, _)| *u == union_name) {
                    Some(*variant_name)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Build the union→variants reverse map for use in flow analysis display.
    pub fn build_union_to_variants(&self) -> HashMap<Intern::<::std::string::String>, Vec<Intern::<::std::string::String>>> {
        let mut map: HashMap<Intern::<::std::string::String>, Vec<Intern::<::std::string::String>>> = HashMap::new();
        for (variant_name, entries) in &self.variant_map {
            for (union_name, _, _) in entries {
                map.entry(*union_name).or_default().push(*variant_name);
            }
        }
        map
    }

    /// Infer the type of an expression given a local variable environment.
    pub fn infer_expr(&self, expr: &Expr, locals: &HashMap<Intern::<::std::string::String>, Ty>) -> Ty {
        infer_expr_ty(expr, locals, &self.tag_types, &self.fn_return_types)
    }

    /// Resolve the union type reachable via a dot expression from `name`.
    ///
    /// Returns `Some(Ty::Union { .. })` when `name` is either:
    /// - A union type directly (e.g., `Bool`, `Maybe`)
    /// - A variable binding whose type annotation names a union type
    ///
    /// Returns `None` for non-union types and unresolvable names.
    pub fn resolve_dot_type(&self, ast: &FileAst, name: Intern::<::std::string::String>) -> Option<Ty> {
        if let Some(ty) = self.lookup_tag(name)
            && matches!(ty, Ty::Union { .. })
        {
            return Some(ty.clone());
        }
        let type_name = binding_type_annotation(ast, name)?;
        self.lookup_tag(type_name).cloned()
    }
}

// ─── Internals ───────────────────────────────────────────────────────────────

/// Find the type annotation name for a binding named `name` in the AST.
/// Checks top-level defs first, then local bindings inside function bodies.
fn binding_type_annotation(ast: &FileAst, name: Intern::<::std::string::String>) -> Option<Intern::<::std::string::String>> {
    if let Some(bind) = ast.defs().values().find(|b| b.name() == name) {
        return bind.type_annotation.as_ref().map(|(tn, _)| *tn);
    }
    ast.defs().values().find_map(|bind| {
        let BindValue::Body { exprs, .. } = bind.value() else {
            return None;
        };
        exprs.iter().find_map(|expr| {
            let Expr::Bind(b) = &**expr else { return None };
            if b.name() == name {
                b.type_annotation.as_ref().map(|(tn, _)| *tn)
            } else {
                None
            }
        })
    })
}

/// Returns the alignment (in bytes) of a type.
/// Calculate the size needed for a union discriminant based on number of variants.
/// For 2 variants (like Bool), only 1 byte is needed.
/// For 3-256 variants, 1 byte is needed.
/// For 257-65536 variants, 2 bytes are needed.
/// Otherwise, 8 bytes.
fn ty_union_discriminant_size(num_variants: usize) -> usize {
    if num_variants <= 256 {
        1
    } else if num_variants <= 65536 {
        2
    } else {
        8
    }
}

/// Type alias for union variant fields: (variant_name, [(field_name, field_type)])
type UnionVariant<'a> = (Intern::<::std::string::String>, Vec<(Intern::<::std::string::String>, Box<Ty>)>);

/// Calculate the maximum field size across all union variants.
fn ty_union_max_field_size(variants: &[UnionVariant<'_>]) -> usize {
    variants
        .iter()
        .flat_map(|(_, fields)| fields.iter().map(|(_, ft)| ty_byte_size_static(ft)))
        .max()
        .unwrap_or(0)
}

pub fn ty_alignment(ty: &Ty) -> usize {
    match ty {
        Ty::Int(8) | Ty::Bool => 1,
        Ty::Int(16) => 2,
        Ty::Int(32) => 4,
        Ty::Int(128) => 16,
        Ty::Int(_) | Ty::Float | Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => 8,
        Ty::Unit | Ty::Opaque(_) => 8,
        Ty::Record { fields, .. } => fields
            .iter()
            .map(|(_, ft)| ty_alignment(ft))
            .max()
            .unwrap_or(1),
        Ty::Union { variants, .. } => {
            // Check if all variants have no fields
            let all_empty = variants.iter().all(|(_, fields)| fields.is_empty());
            if all_empty {
                // Only discriminant needed, align to 1 byte
                1
            } else {
                // Align to the maximum alignment of all field types
                variants
                    .iter()
                    .flat_map(|(_, fields)| fields.iter().map(|(_, ft)| ty_alignment(ft)))
                    .max()
                    .unwrap_or(8)
            }
        }
        Ty::Tuple(fields) => fields.iter().map(ty_alignment).max().unwrap_or(1),
        Ty::Literal(_) => 8,
    }
}

/// Returns the in-memory size (bytes) of a type without recursing into the typeck context.
pub fn ty_byte_size_static(ty: &Ty) -> usize {
    match ty {
        Ty::Int(8) | Ty::Bool => 1,
        Ty::Int(16) => 2,
        Ty::Int(32) => 4,
        Ty::Int(128) => 16,
        Ty::Int(_) | Ty::Float => 8,
        Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => 8,
        Ty::Unit | Ty::Opaque(_) => 8,
        Ty::Record { fields, .. } => fields.iter().map(|(_, ft)| ty_byte_size_static(ft)).sum(),
        Ty::Union { variants, .. } => {
            // For unions with 2 variants and no fields (like Bool), use 1 byte
            let all_empty = variants.iter().all(|(_, fields)| fields.is_empty());
            if all_empty && variants.len() <= 256 {
                // Only discriminant needed, 1 byte is sufficient for 2-256 variants
                1
            } else if all_empty {
                // Many variants but no fields
                ty_union_discriminant_size(variants.len())
            } else {
                // Calculate: discriminant + max field size
                let discriminant_size = ty_union_discriminant_size(variants.len());
                let max_field_size = ty_union_max_field_size(variants);
                discriminant_size + max_field_size
            }
        }
        Ty::Tuple(fields) => fields.iter().map(ty_byte_size_static).sum(),
        Ty::Literal(_) => 8,
    }
}

/// Canonical `Str` record type: `{ pointer: Ptr(Byte), len: Int }`.
/// Single definition used by `builtin()`, `TyEnv` injection, and type inference.
pub fn str_record_ty() -> Ty {
    Ty::Record {
        name: Intern::<::std::string::String>::new("Str".to_string()),
        fields: vec![
            (
                Intern::<::std::string::String>::new("pointer".to_string()),
                Box::new(Ty::Ptr {
                    inner: Box::new(Ty::Int(8)),
                }),
            ),
            (Intern::<::std::string::String>::new("len".to_string()), Box::new(Ty::Int(64))),
        ],
    }
}

fn range_bit_width(min: i128, max: i128) -> u8 {
    let range = max - min;
    if range <= u8::MAX as i128 + 1 {
        8
    } else if range <= u16::MAX as i128 + 1 {
        16
    } else if range <= u32::MAX as i128 + 1 {
        32
    } else if range <= u64::MAX as i128 + 1 {
        64
    } else {
        128
    }
}

fn resolve_tag_ref(tag: &Tag, raw: &HashMap<Intern::<::std::string::String>, &DeclareValue>, recursion_depth: usize) -> Ty {
    match tag {
        Tag::Nominal(name, _) => resolve_name(*name, raw, recursion_depth),
        Tag::Generic(name, params, _) => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .values()
                    .find_map(|kind| match kind {
                        ParameterKind::Tagged(t) => {
                            Some(resolve_tag_ref(t, raw, recursion_depth + 1))
                        }
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(*name));
                if name.as_str() == "Ptr" {
                    Ty::Ptr {
                        inner: Box::new(inner),
                    }
                } else {
                    Ty::Ref {
                        inner: Box::new(inner),
                    }
                }
            }
            _ => resolve_name(*name, raw, recursion_depth + 1),
        },
        Tag::Qualified(path) => {
            // For qualified types like Bool.True, resolve the union type
            resolve_name(path.root, raw, recursion_depth)
        }
    }
}

fn resolve_name_from_files(name: Intern::<::std::string::String>, files: &[FileAst], recursion_depth: usize) -> Ty {
    // Build a temporary lookup map for this file set
    let mut raw: HashMap<Intern::<::std::string::String>, &DeclareValue> = HashMap::new();
    for ast in files {
        for (k, v) in ast.tags.iter() {
            raw.insert(*k, v.value());
        }
    }

    resolve_name(name, &raw, recursion_depth)
}

fn resolve_name(name: Intern::<::std::string::String>, raw: &HashMap<Intern::<::std::string::String>, &DeclareValue>, recursion_depth: usize) -> Ty {
    if recursion_depth > 16 {
        return Ty::Opaque(name);
    }
    // User declarations override builtins
    match raw.get(&name) {
        Some(DeclareValue::Alias(tag)) => resolve_tag_ref(tag, raw, recursion_depth + 1),
        Some(DeclareValue::Range(range)) => Ty::Int(range_bit_width(range.start, range.end)),
        Some(DeclareValue::InRange(range)) => Ty::Int(range_bit_width(range.start, range.end)),
        Some(DeclareValue::Record(params)) => {
            let fields = params
                .iter()
                .map(|(field_name, kind)| {
                    let field_ty = match kind {
                        ParameterKind::Tagged(tag) => {
                            resolve_tag_ref(tag, raw, recursion_depth + 1)
                        }
                        ParameterKind::Generic => Ty::Opaque(*field_name),
                        ParameterKind::Default(_) => Ty::Int(64),
                    };
                    (*field_name, Box::new(field_ty))
                })
                .collect();
            Ty::Record { name, fields }
        }
        Some(DeclareValue::Union { variants }) => {
            let resolved = variants
                .iter()
                .map(|v| {
                    let tag = v.tag();
                    let variant_name = Intern::<::std::string::String>::new(tag.name().to_string());
                    let fields = match tag {
                        Tag::Generic(_, params, _) if !params.is_empty() => params
                            .iter()
                            .filter_map(|(field_name, kind)| match kind {
                                ParameterKind::Tagged(inner) => Some((
                                    *field_name,
                                    Box::new(resolve_tag_ref(inner, raw, recursion_depth + 1)),
                                )),
                                ParameterKind::Generic => {
                                    Some((*field_name, Box::new(Ty::Opaque(*field_name))))
                                }
                                _ => None,
                            })
                            .collect(),
                        _ => vec![],
                    };
                    (variant_name, fields)
                })
                .collect();
            Ty::Union {
                name,
                variants: resolved,
            }
        }
        _ => Ty::Opaque(name),
    }
}

fn resolve_tag_from_map(tag: &Tag, tag_types: &HashMap<Intern::<::std::string::String>, Ty>) -> Ty {
    match tag {
        Tag::Nominal(name, _) => tag_types.get(name).cloned().unwrap_or(Ty::Int(64)),
        Tag::Generic(name, params, _) => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .values()
                    .find_map(|kind| match kind {
                        ParameterKind::Tagged(t) => Some(resolve_tag_from_map(t, tag_types)),
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(*name));
                if name.as_str() == "Ptr" {
                    Ty::Ptr {
                        inner: Box::new(inner),
                    }
                } else {
                    Ty::Ref {
                        inner: Box::new(inner),
                    }
                }
            }
            _ => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        },
        Tag::Qualified(path) => {
            // For qualified types like Bool.True, we need to resolve them
            // to the type of the variant. The type of a variant is the union type itself.
            // E.g., Bool.True has type Bool (the union type)
            let union_name = path.root;
            tag_types
                .get(&union_name)
                .cloned()
                .unwrap_or(Ty::Opaque(union_name))
        }
    }
}

fn infer_bind_ret(
    bind: &Bind,
    tag_types: &HashMap<Intern::<::std::string::String>, Ty>,
    fn_return_types: &HashMap<Intern::<::std::string::String>, Ty>,
) -> Ty {
    // Explicit annotation wins.
    if let Some(tag) = &bind.return_tag {
        return resolve_tag_from_map(tag, tag_types);
    }

    let mut locals: HashMap<Intern::<::std::string::String>, Ty> = match bind.params().as_ref() {
        None => HashMap::new(),
        Some(params) => params
            .iter()
            .map(|(name, kind)| {
                let ty = match kind {
                    ParameterKind::Tagged(tag) => resolve_tag_from_map(tag, tag_types),
                    ParameterKind::Generic => Ty::Int(64),
                    ParameterKind::Default(expr) => {
                        infer_expr_ty(expr, &HashMap::new(), tag_types, fn_return_types)
                    }
                };
                (*name, ty)
            })
            .collect(),
    };
    if let Some(recv_tag) = bind.receiver_type() {
        let recv_ty = resolve_tag_from_map(recv_tag, tag_types);
        locals.insert(Intern::<::std::string::String>::new("self".to_string()), recv_ty);
    }

    match bind.value() {
        BindValue::Expr(expr) => infer_expr_ty(expr, &locals, tag_types, fn_return_types),
        BindValue::Body { ret, .. } => match &ret.0 {
            Some(expr) => infer_expr_ty(expr, &locals, tag_types, fn_return_types),
            None => Ty::Unit,
        },
        BindValue::Extern => Ty::Unit,
    }
}

pub fn infer_expr_ty(
    expr: &Expr,
    locals: &HashMap<Intern::<::std::string::String>, Ty>,
    tag_types: &HashMap<Intern::<::std::string::String>, Ty>,
    fn_return_types: &HashMap<Intern::<::std::string::String>, Ty>,
) -> Ty {
    match expr {
        Expr::Lit(lit) => match lit {
            Literal::Int(n) => Ty::Literal(LiteralValue::Int(*n)),
            Literal::Number(n) => Ty::Literal(LiteralValue::Int(*n as i128)),
            Literal::Float(f) => Ty::Literal(LiteralValue::Float(*f)),
            Literal::String(_) => str_record_ty(),
        },
        Expr::Binary(bin) => {
            if bin.op.is_comparison() {
                return Ty::Bool;
            }
            let lhs_ty = infer_expr_ty(&bin.lhs, locals, tag_types, fn_return_types);
            let rhs_ty = infer_expr_ty(&bin.rhs, locals, tag_types, fn_return_types);
            match (&lhs_ty, &rhs_ty) {
                (Ty::Literal(LiteralValue::Int(a)), Ty::Literal(LiteralValue::Int(b))) => {
                    let folded = match bin.op {
                        BinOp::Add => Some(a + b),
                        BinOp::Subtract => Some(a - b),
                        BinOp::Multiply => Some(a * b),
                        BinOp::Divide if *b != 0 => Some(a / b),
                        BinOp::Modulo if *b != 0 => Some(a % b),
                        _ => None,
                    };
                    folded
                        .map(|v| Ty::Literal(LiteralValue::Int(v)))
                        .unwrap_or(lhs_ty)
                }
                (Ty::Literal(LiteralValue::Float(a)), Ty::Literal(LiteralValue::Float(b))) => {
                    let folded = match bin.op {
                        BinOp::Add => Some(a + b),
                        BinOp::Subtract => Some(a - b),
                        BinOp::Multiply => Some(a * b),
                        BinOp::Divide => Some(a / b),
                        _ => None,
                    };
                    folded
                        .map(|v| Ty::Literal(LiteralValue::Float(v)))
                        .unwrap_or(lhs_ty)
                }
                _ => {
                    if matches!(lhs_ty, Ty::Float | Ty::Literal(LiteralValue::Float(_))) {
                        lhs_ty
                    } else if matches!(rhs_ty, Ty::Float | Ty::Literal(LiteralValue::Float(_))) {
                        rhs_ty
                    } else {
                        lhs_ty
                    }
                }
            }
        }
        Expr::FnCall(call) => {
            let name = call.path.root;
            if let Some(local_ty) = locals.get(&name) {
                return local_ty.clone();
            }
            fn_return_types.get(&name).cloned().unwrap_or(Ty::Int(64))
        }
        Expr::Bind(bind) => infer_bind_ret(bind, tag_types, fn_return_types),
        Expr::TagCall(tc) => {
            // Try union variant first.
            if let Some(ty) = tag_types.values().find_map(|ty| {
                if let Ty::Union { variants, .. } = ty
                    && variants.iter().any(|(vname, _)| *vname == tc.name)
                {
                    return Some(ty.clone());
                }
                None
            }) {
                return ty;
            }
            // Fall back to a record type with that name.
            tag_types
                .get(&tc.name)
                .cloned()
                .unwrap_or(Ty::Opaque(tc.name))
        }
        Expr::AnonymousTag(name, _) => Ty::Opaque(*name),
        Expr::FormatString(_) => str_record_ty(),
        Expr::Loop(_) => Ty::Unit,
        Expr::When(when_expr) => {
            // Prefer the else arm; fall back to the first arm's body.
            let body = when_expr
                .arms
                .iter()
                .find_map(|a| {
                    if let WhenArm::Else(b) = a {
                        Some(b.as_ref())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    when_expr.arms.first().map(|a| match a {
                        WhenArm::Cond { body, .. }
                        | WhenArm::Is { body, .. }
                        | WhenArm::Else(body) => body.as_ref(),
                    })
                });
            match body {
                Some(b) => infer_expr_ty(b, locals, tag_types, fn_return_types),
                None => Ty::Unit,
            }
        }
        Expr::If(_) => Ty::Unit,
        Expr::Range(_) => Ty::Opaque(Intern::<::std::string::String>::new("Range".to_string())),
        Expr::SelfRef(_) => locals
            .get(&Intern::<::std::string::String>::new("self".to_string()))
            .cloned()
            .unwrap_or_else(|| Ty::Opaque(Intern::<::std::string::String>::new("Self".to_string()))),
        Expr::TupleAlloc { init, size } => {
            let elem = infer_expr_ty(init, locals, tag_types, fn_return_types);
            Ty::Array {
                elem: Box::new(elem),
                size: *size,
            }
        }
        Expr::TupleGet { base, index } => {
            match infer_expr_ty(base, locals, tag_types, fn_return_types) {
                Ty::Array { elem, .. } => *elem,
                Ty::Tuple(fields) => fields.into_iter().nth(*index).unwrap_or(Ty::Int(64)),
                _ => Ty::Int(8),
            }
        }
        Expr::TupleSet { .. } => Ty::Unit,
        Expr::Cast { ty, .. } => Ty::Opaque(*ty),
        Expr::BufGet { buf, .. } => match infer_expr_ty(buf, locals, tag_types, fn_return_types) {
            Ty::Array { elem, .. } => *elem,
            _ => Ty::Int(8),
        },
        Expr::TupleLit(elems) => {
            let field_tys = elems
                .iter()
                .map(|e| infer_expr_ty(e, locals, tag_types, fn_return_types))
                .collect();
            Ty::Tuple(field_tys)
        }
        Expr::BufSet { .. } => Ty::Unit,
        Expr::TakePtr(inner) => {
            let inner_ty = infer_expr_ty(inner, locals, tag_types, fn_return_types);
            Ty::Ptr {
                inner: Box::new(inner_ty),
            }
        }
        Expr::TakeRef(inner) => {
            let inner_ty = infer_expr_ty(inner, locals, tag_types, fn_return_types);
            Ty::Ref {
                inner: Box::new(inner_ty),
            }
        }
        Expr::Deref(inner) => match infer_expr_ty(inner, locals, tag_types, fn_return_types) {
            Ty::Ptr { inner } | Ty::Ref { inner } => *inner,
            _ => Ty::Int(64),
        },
        Expr::Negate(inner) => {
            let ty = infer_expr_ty(inner, locals, tag_types, fn_return_types);
            match ty {
                Ty::Literal(LiteralValue::Int(n)) => Ty::Literal(LiteralValue::Int(-n)),
                Ty::Literal(LiteralValue::Float(f)) => Ty::Literal(LiteralValue::Float(-f)),
                other => other,
            }
        }
    }
}

// ─── Unknown reference checking ──────────────────────────────────────────────

impl TyEnv {
    pub fn check_unknowns<D>(&self, ast: &FileAst, db: &D)
    where
        D: salsa::Database + ?Sized,
    {
        for bind in ast.defs.values() {
            let mut locals = HashMap::new();
            if let Some(params) = bind.params() {
                for (name, kind) in params.iter() {
                    let ty = match kind {
                        ParameterKind::Tagged(tag) => self.resolve_tag(tag),
                        ParameterKind::Generic => Ty::Int(64),
                        ParameterKind::Default(expr) => self.infer_expr(expr, &HashMap::new()),
                    };
                    locals.insert(*name, ty);
                }
            }
            self.check_bind(bind, db, &locals);
        }
    }

    fn check_bind<D>(
        &self,
        bind: &Bind,
        db: &D,
        locals: &HashMap<Intern::<::std::string::String>, Ty>,
    )
    where
        D: salsa::Database + ?Sized,
    {
        if let Some(tag) = &bind.return_tag {
            self.check_tag(tag, db);
        }
        match bind.value() {
            BindValue::Expr(expr) => self.check_expr(expr, db, locals),
            BindValue::Body { exprs, ret } => {
                use diagnostic::type_ as type_symptom;
                use salsa::Accumulator;

                let mut body_locals = locals.clone();
                for (i, expr) in exprs.iter().enumerate() {
                    if let Expr::Bind(inner) = &**expr {
                        self.check_bind(inner, db, &body_locals);
                        let name = inner.name();
                        let used = exprs[i + 1..].iter().any(|e| expr_references_name(e, name))
                            || ret
                                .0
                                .as_ref()
                                .is_some_and(|e| expr_references_name(e, name));
                        if !used {
                            type_symptom::unused_binding(inner.name_span, name.to_string())
                                .accumulate(db);
                        }
                        body_locals.insert(name, self.return_ty(inner));
                    } else {
                        self.check_expr(expr, db, &body_locals);
                    }
                }
                if let Some(ret_expr) = &ret.0 {
                    self.check_expr(ret_expr, db, &body_locals);
                }
            }
            BindValue::Extern => {}
        }
    }

    fn check_expr<D>(
        &self,
        expr: &Expr,
        db: &D,
        locals: &HashMap<Intern::<::std::string::String>, Ty>,
    )
    where
        D: salsa::Database + ?Sized,
    {
        use diagnostic::type_ as type_symptom;
        use salsa::Accumulator;

        match expr {
            Expr::FnCall(call) => {
                let name = call.path.root;
                if let Some(args) = &call.args {
                    if self.fn_return_ty(&name).is_none() && !is_builtin_func(name.as_str()) {
                        type_symptom::unknown_binding(call.path.span, name.to_string())
                            .accumulate(db);
                    }
                    for arg in args {
                        self.check_expr(arg, db, locals);
                    }
                } else if call.path.segments.is_empty()
                    && !locals.contains_key(&name)
                    && self.fn_return_ty(&name).is_none()
                {
                    type_symptom::unknown_binding(call.path.span, name.to_string()).accumulate(db);
                }
            }
            Expr::Bind(bind) => self.check_bind(bind, db, locals),
            Expr::Binary(bin) => {
                self.check_expr(&bin.lhs, db, locals);
                self.check_expr(&bin.rhs, db, locals);
            }
            Expr::When(w) => {
                // Infer subject type once for pattern-variant checking.
                let subject_ty = w.subject.as_ref().map(|s| self.infer_expr(s, locals));
                if let Some(subject) = &w.subject {
                    self.check_expr(subject, db, locals);
                }
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { condition, body } => {
                            self.check_expr(condition, db, locals);
                            self.check_expr(body, db, locals);
                        }
                        WhenArm::Is { pattern, body } => {
                            let variant_name = Intern::<::std::string::String>::new(pattern.name().to_string());
                            match &subject_ty {
                                Some(Ty::Union {
                                    name: union_name,
                                    variants,
                                }) => {
                                    if !variants.iter().any(|(vname, _)| vname == &variant_name) {
                                        type_symptom::not_a_variant(
                                            pattern.span(),
                                            pattern.name().to_string(),
                                            union_name.to_string(),
                                        )
                                        .accumulate(db);
                                    }
                                }
                                _ => {
                                    // Subject type unknown or not a union — fall back to generic check.
                                    if self.lookup_variant(variant_name).is_none() {
                                        type_symptom::unknown_tag(
                                            pattern.span(),
                                            pattern.name().to_string(),
                                        )
                                        .accumulate(db);
                                    }
                                }
                            }
                            self.check_expr(body, db, locals);
                        }
                        WhenArm::Else(body) => {
                            self.check_expr(body, db, locals);
                        }
                    }
                }
            }
            Expr::If(if_expr) => match &if_expr.condition {
                IfCondition::Bool(cond) => {
                    self.check_expr(cond, db, locals);
                    for e in &if_expr.body {
                        self.check_expr(e, db, locals);
                    }
                }
                IfCondition::Pattern { subject, tag } => {
                    self.check_expr(subject, db, locals);
                    let mut if_locals = locals.clone();
                    if let Tag::Generic(_, params, _) = tag {
                        if_locals.extend(params.iter().filter_map(|(k, _)| {
                            if k.as_str() != "_" {
                                Some((*k, Ty::Opaque(*k)))
                            } else {
                                None
                            }
                        }));
                    }
                    for e in &if_expr.body {
                        self.check_expr(e, db, &if_locals);
                    }
                }
            },
            Expr::Loop(loop_expr) => match loop_expr {
                Loop::While(w) => {
                    self.check_expr(&w.cond, db, locals);
                    for e in &w.exprs {
                        self.check_expr(e, db, locals);
                    }
                }
                Loop::ForIn(f) => {
                    self.check_expr(&f.iter, db, locals);
                    for e in &f.exprs {
                        self.check_expr(e, db, locals);
                    }
                }
            },
            Expr::TupleLit(elems) => {
                for e in elems {
                    self.check_expr(e, db, locals);
                }
            }
            Expr::TupleAlloc { init, .. } => self.check_expr(init, db, locals),
            Expr::TupleGet { base, .. } => self.check_expr(base, db, locals),
            Expr::TupleSet { base, value, .. } => {
                self.check_expr(base, db, locals);
                self.check_expr(value, db, locals);
            }
            Expr::BufGet { buf, index, .. } => {
                self.check_expr(buf, db, locals);
                self.check_expr(index, db, locals);
            }
            Expr::BufSet {
                buf, index, value, ..
            } => {
                self.check_expr(buf, db, locals);
                self.check_expr(index, db, locals);
                self.check_expr(value, db, locals);
            }
            Expr::Cast { expr, .. } => self.check_expr(expr, db, locals),
            Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
                self.check_expr(e, db, locals);
            }
            Expr::AnonymousTag(name, span) => {
                if self.lookup_variant(*name).is_none() {
                    type_symptom::unknown_tag(*span, name.to_string()).accumulate(db);
                }
            }
            Expr::TagCall(tc) => {
                if let Some(path) = &tc.qual_path {
                    // Qualified form like Maybe.Some(3) — check the root type
                    if self.lookup_tag(path.root).is_none() {
                        type_symptom::unknown_tag(path.span, path.root.to_string()).accumulate(db);
                    }
                } else if self.lookup_variant(tc.name).is_none() {
                    // Simple form like Some(3) — check the variant
                    type_symptom::unknown_tag(tc.span, tc.name.to_string()).accumulate(db);
                }
                for arg in &tc.args {
                    self.check_expr(arg, db, locals);
                }
            }
            Expr::Lit(_) | Expr::SelfRef(_) | Expr::Range(_) | Expr::FormatString(_) => {}
        }
    }

    fn check_tag<D>(&self, tag: &Tag, db: &D)
    where
        D: salsa::Database + ?Sized,
    {
        use diagnostic::type_ as type_symptom;
        use salsa::Accumulator;

        match tag {
            Tag::Nominal(name, span) => {
                if self.lookup_tag(*name).is_none() {
                    type_symptom::unknown_tag(*span, name.to_string()).accumulate(db);
                }
            }
            Tag::Generic(name, params, span) => {
                if self.lookup_tag(*name).is_none() {
                    type_symptom::unknown_tag(*span, name.to_string()).accumulate(db);
                }
                for kind in params.values() {
                    if let ParameterKind::Tagged(inner) = kind {
                        self.check_tag(inner, db);
                    }
                }
            }
            Tag::Qualified(path) => {
                if self.lookup_tag(path.root).is_none() {
                    type_symptom::unknown_tag(path.span, path.root.to_string()).accumulate(db);
                }
            }
        }
    }
}

fn expr_references_name(expr: &Expr, name: Intern::<::std::string::String>) -> bool {
    match expr {
        Expr::FnCall(call) => {
            (call.path.root == name && call.path.segments.is_empty())
                || call
                    .args
                    .as_ref()
                    .is_some_and(|args| args.iter().any(|a| expr_references_name(a, name)))
        }
        Expr::Bind(bind) => match bind.value() {
            BindValue::Expr(e) => expr_references_name(e, name),
            BindValue::Body { exprs, ret } => {
                exprs.iter().any(|e| expr_references_name(e, name))
                    || ret
                        .0
                        .as_ref()
                        .is_some_and(|e| expr_references_name(e, name))
            }
            BindValue::Extern => false,
        },
        Expr::Binary(bin) => {
            expr_references_name(&bin.lhs, name) || expr_references_name(&bin.rhs, name)
        }
        Expr::When(w) => {
            w.subject
                .as_ref()
                .is_some_and(|s| expr_references_name(s, name))
                || w.arms.iter().any(|arm| match arm {
                    WhenArm::Cond { condition, body } => {
                        expr_references_name(condition, name) || expr_references_name(body, name)
                    }
                    WhenArm::Is { body, .. } | WhenArm::Else(body) => {
                        expr_references_name(body, name)
                    }
                })
        }
        Expr::If(if_expr) => {
            let cond_ref = match &if_expr.condition {
                IfCondition::Bool(c) => expr_references_name(c, name),
                IfCondition::Pattern { subject, .. } => expr_references_name(subject, name),
            };
            cond_ref || if_expr.body.iter().any(|e| expr_references_name(e, name))
        }
        Expr::Loop(loop_expr) => match loop_expr {
            Loop::While(w) => {
                expr_references_name(&w.cond, name)
                    || w.exprs.iter().any(|e| expr_references_name(e, name))
            }
            Loop::ForIn(f) => {
                expr_references_name(&f.iter, name)
                    || f.exprs.iter().any(|e| expr_references_name(e, name))
            }
        },
        Expr::TupleLit(elems) => elems.iter().any(|e| expr_references_name(e, name)),
        Expr::TupleAlloc { init, .. } => expr_references_name(init, name),
        Expr::TupleGet { base, .. } => expr_references_name(base, name),
        Expr::TupleSet { base, value, .. } => {
            expr_references_name(base, name) || expr_references_name(value, name)
        }
        Expr::BufGet { buf, index, .. } => {
            expr_references_name(buf, name) || expr_references_name(index, name)
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            expr_references_name(buf, name)
                || expr_references_name(index, name)
                || expr_references_name(value, name)
        }
        Expr::Cast { expr, .. } => expr_references_name(expr, name),
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
            expr_references_name(e, name)
        }
        Expr::FormatString(fs) => fs.parts.iter().any(|p| {
            if let FormatPart::Expr(e) = p {
                expr_references_name(e, name)
            } else {
                false
            }
        }),
        Expr::Range(range) => {
            expr_references_name(&range.start, name) || expr_references_name(&range.end, name)
        }
        Expr::Lit(_) | Expr::SelfRef(_) | Expr::AnonymousTag(..) | Expr::TagCall(_) => false,
    }
}

fn is_builtin_func(name: &str) -> bool {
    matches!(name, "syscall" | "float_bits" | "print" | "println")
}
