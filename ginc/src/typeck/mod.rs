pub mod flow;
pub use flow::*;
pub mod flow_analyzer;
pub use flow_analyzer::*;

use std::collections::HashMap;

use crate::ast::{Bind, BindValue, DeclareValue, Expr, FileAst, FormatPart, IfCondition, Literal, Loop, ParameterKind, Tag};
use crate::prelude::BinOp;
use crate::intern::IStr;
use crate::prelude::WhenArm;

/// A concrete compile-time value carried by `Ty::Literal`.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    Int(i64),
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
        name: IStr,
        fields: Vec<(IStr, Box<Ty>)>,
    },
    Union {
        name: IStr,
        /// Each variant: (variant_name, [(field_name, field_type)]) in declaration order.
        #[allow(clippy::type_complexity)]
        variants: Vec<(IStr, Vec<(IStr, Box<Ty>)>)>,
    },
    /// Unresolved / generic type — falls back to `i64` in codegen.
    Opaque(IStr),
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
    pub fn record_fields_sorted(&self) -> Vec<(&IStr, &Ty)> {
        if let Ty::Record { fields, .. } = self {
            let mut indexed: Vec<(usize, &IStr, &Ty)> = fields
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
type UnionFields = Vec<(IStr, Box<Ty>)>;

/// Type alias for union variants: (variant_name, fields)
#[allow(dead_code)]
type UnionVariants = Vec<(IStr, UnionFields)>;

/// Type alias for variant map entries: (union_name, discriminant, fields)
type VariantMapEntry = (IStr, usize, Vec<(IStr, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
type VariantMap = HashMap<IStr, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
type VariantLookupResult<'a> = (IStr, usize, &'a [(IStr, Ty)]);

/// Type environment built from a `FileAst`. Resolves tag names to `Ty` and infers
/// function parameter / return types.
pub struct TyEnv {
    tag_types: HashMap<IStr, Ty>,
    fn_return_types: HashMap<IStr, Ty>,
    /// Reverse map: variant name → [(parent_union_name, discriminant_index, payload_fields)]
    /// A variant may appear in multiple unions if names collide; shape-based disambiguation is TODO.
    variant_map: VariantMap,
}

impl TyEnv {
    pub fn from_file_ast(ast: &FileAst) -> Self {
        let raw: HashMap<IStr, &DeclareValue> =
            ast.tags.iter().map(|(k, v)| (*k, v.value())).collect();

        let mut tag_types = HashMap::new();

        // Inject Str as a builtin record before user declarations so TagCall("Str", ...)
        // can construct it. User declarations of Str will override this.
        let str_ty = str_record_ty();
        tag_types.insert(IStr::new("Str".to_string()), str_ty.clone());
        tag_types.insert(IStr::new("String".to_string()), str_ty);

        for name in ast.tags.keys() {
            let ty = resolve_name(*name, &raw, 0);
            tag_types.insert(*name, ty);
        }

        // Build variant reverse map from all union types.
        let mut variant_map: VariantMap = HashMap::new();
        for (union_name, ty) in &tag_types {
            if let Ty::Union { variants, .. } = ty {
                for (i, (variant_name, fields)) in variants.iter().enumerate() {
                    let field_tys: Vec<(IStr, Ty)> =
                        fields.iter().map(|(n, t)| (*n, *t.clone())).collect();
                    variant_map
                        .entry(*variant_name)
                        .or_default()
                        .push((*union_name, i, field_tys));
                }
            }
        }

        // First pass: seed with explicitly-annotated return types.
        let mut fn_return_types = HashMap::new();
        for (name, bind) in &ast.defs {
            if !bind.attributes().matches_current_platform() {
                continue;
            }
            let ret = infer_bind_ret(bind, &tag_types, &HashMap::new());
            fn_return_types.insert(*name, ret);
        }
        // Second pass: refine with cross-function call resolution now that
        // explicitly-typed functions are in the map.
        for (name, bind) in &ast.defs {
            if !bind.attributes().matches_current_platform() {
                continue;
            }
            let ret = infer_bind_ret(bind, &tag_types, &fn_return_types);
            fn_return_types.insert(*name, ret);
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
    pub fn param_types<'a>(&self, bind: &'a Bind) -> Vec<(&'a IStr, Ty)> {
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
    pub fn fn_return_ty(&self, name: &IStr) -> Option<&Ty> {
        self.fn_return_types.get(name)
    }

    /// Look up a declared type by its tag name.
    pub fn lookup_tag(&self, name: IStr) -> Option<&Ty> {
        self.tag_types.get(&name)
    }

    /// Look up which union a variant belongs to, its discriminant index, and payload fields.
    ///
    /// Returns `(union_name, discriminant, [(field_name, field_type)])` in declaration order.
    /// If multiple unions declare a variant with the same name, the first match is returned.
    /// TODO: shape-based disambiguation per spec (answer #6).
    pub fn lookup_variant(&self, name: IStr) -> Option<VariantLookupResult<'_>> {
        let candidates = self.variant_map.get(&name)?;
        candidates
            .first()
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    /// Return all variant names belonging to `union_name`.
    pub fn all_variants_of(&self, union_name: IStr) -> Vec<IStr> {
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
    pub fn build_union_to_variants(&self) -> HashMap<IStr, Vec<IStr>> {
        let mut map: HashMap<IStr, Vec<IStr>> = HashMap::new();
        for (variant_name, entries) in &self.variant_map {
            for (union_name, _, _) in entries {
                map.entry(*union_name).or_default().push(*variant_name);
            }
        }
        map
    }

    /// Infer the type of an expression given a local variable environment.
    pub fn infer_expr(&self, expr: &Expr, locals: &HashMap<IStr, Ty>) -> Ty {
        infer_expr_ty(expr, locals, &self.tag_types, &self.fn_return_types)
    }
}

// ─── Internals ───────────────────────────────────────────────────────────────

/// Returns the alignment (in bytes) of a type.
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
        Ty::Union { .. } => 8,
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
        Ty::Union { .. } => 16,
        Ty::Tuple(fields) => fields.iter().map(ty_byte_size_static).sum(),
        Ty::Literal(_) => 8,
    }
}

/// Canonical `Str` record type: `{ pointer: Ptr(Byte), len: Int }`.
/// Single definition used by `builtin()`, `TyEnv` injection, and type inference.
pub fn str_record_ty() -> Ty {
    Ty::Record {
        name: IStr::new("Str".to_string()),
        fields: vec![
            (
                IStr::new("pointer".to_string()),
                Box::new(Ty::Ptr {
                    inner: Box::new(Ty::Int(8)),
                }),
            ),
            (IStr::new("len".to_string()), Box::new(Ty::Int(64))),
        ],
    }
}

fn builtin(name: IStr) -> Option<Ty> {
    match name.as_str() {
        "Int" | "I64" => Some(Ty::Int(64)),
        "I128" => Some(Ty::Int(128)),
        "Byte" | "I8" => Some(Ty::Int(8)),
        "I16" => Some(Ty::Int(16)),
        "I32" => Some(Ty::Int(32)),
        "Float" | "F64" => Some(Ty::Float),
        "F32" => Some(Ty::Float),
        "Bool" => Some(Ty::Bool),
        "Str" | "String" => Some(str_record_ty()),
        "Nothing" | "Unit" => Some(Ty::Unit),
        _ => None,
    }
}

fn range_bit_width(min: i64, max: i64) -> u8 {
    let range = (max as i128) - (min as i128);
    if range <= u8::MAX as i128 + 1 {
        8
    } else if range <= u16::MAX as i128 + 1 {
        16
    } else if range <= u32::MAX as i128 + 1 {
        32
    } else {
        64
    }
}

fn resolve_tag_ref(tag: &Tag, raw: &HashMap<IStr, &DeclareValue>, depth: u8) -> Ty {
    match tag {
        Tag::Nominal(name, _) => resolve_name(*name, raw, depth),
        Tag::Generic(name, params, _) => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .values()
                    .find_map(|kind| match kind {
                        ParameterKind::Tagged(t) => Some(resolve_tag_ref(t, raw, depth + 1)),
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
            _ => Ty::Opaque(*name),
        },
        Tag::Qualified(path) => {
            // For qualified types like Bool.True, resolve the union type
            resolve_name(path.root, raw, depth)
        }
    }
}

fn resolve_name(name: IStr, raw: &HashMap<IStr, &DeclareValue>, depth: u8) -> Ty {
    if depth > 16 {
        return Ty::Opaque(name);
    }
    // User declarations override builtins
    match raw.get(&name) {
        Some(DeclareValue::Alias(tag)) => resolve_tag_ref(tag, raw, depth + 1),
        Some(DeclareValue::Range(range)) => Ty::Int(range_bit_width(range.start, range.end)),
        Some(DeclareValue::InRange(range)) => Ty::Int(range_bit_width(range.start, range.end)),
        Some(DeclareValue::Record(params)) => {
            let fields = params
                .iter()
                .map(|(field_name, kind)| {
                    let field_ty = match kind {
                        ParameterKind::Tagged(tag) => resolve_tag_ref(tag, raw, depth + 1),
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
                    let variant_name = IStr::new(tag.name().to_string());
                    let fields = match tag {
                        Tag::Generic(_, params, _) if !params.is_empty() => params
                            .iter()
                            .filter_map(|(field_name, kind)| match kind {
                                ParameterKind::Tagged(inner) => Some((
                                    *field_name,
                                    Box::new(resolve_tag_ref(inner, raw, depth + 1)),
                                )),
                                ParameterKind::Generic => Some((
                                    *field_name,
                                    Box::new(Ty::Opaque(*field_name)),
                                )),
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
        Some(DeclareValue::Set()) => Ty::Opaque(name),
        None => builtin(name).unwrap_or(Ty::Opaque(name)),
    }
}

fn resolve_tag_from_map(tag: &Tag, tag_types: &HashMap<IStr, Ty>) -> Ty {
    match tag {
        Tag::Nominal(name, _) => tag_types.get(name).cloned()
            .or_else(|| builtin(*name))
            .unwrap_or(Ty::Int(64)),
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
            _ => Ty::Opaque(*name),
        },
        Tag::Qualified(path) => {
            // For qualified types like Bool.True, we need to resolve them
            // to the type of the variant. The type of a variant is the union type itself.
            // E.g., Bool.True has type Bool (the union type)
            let union_name = path.root;
            tag_types.get(&union_name).cloned()
                .unwrap_or(Ty::Opaque(union_name))
        }
    }
}

fn infer_bind_ret(
    bind: &Bind,
    tag_types: &HashMap<IStr, Ty>,
    fn_return_types: &HashMap<IStr, Ty>,
) -> Ty {
    // Explicit annotation wins.
    if let Some(tag) = &bind.return_tag {
        return resolve_tag_from_map(tag, tag_types);
    }

    let mut locals: HashMap<IStr, Ty> = match bind.params().as_ref() {
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
        locals.insert(IStr::new("self".to_string()), recv_ty);
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

fn infer_expr_ty(
    expr: &Expr,
    locals: &HashMap<IStr, Ty>,
    tag_types: &HashMap<IStr, Ty>,
    fn_return_types: &HashMap<IStr, Ty>,
) -> Ty {
    match expr {
        Expr::Lit(lit) => match lit {
            Literal::Int(n) => Ty::Literal(LiteralValue::Int(*n)),
            Literal::Number(n) => Ty::Literal(LiteralValue::Int(*n as i64)),
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
                    folded.map(|v| Ty::Literal(LiteralValue::Int(v))).unwrap_or(lhs_ty)
                }
                (Ty::Literal(LiteralValue::Float(a)), Ty::Literal(LiteralValue::Float(b))) => {
                    let folded = match bin.op {
                        BinOp::Add => Some(a + b),
                        BinOp::Subtract => Some(a - b),
                        BinOp::Multiply => Some(a * b),
                        BinOp::Divide => Some(a / b),
                        _ => None,
                    };
                    folded.map(|v| Ty::Literal(LiteralValue::Float(v))).unwrap_or(lhs_ty)
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
        Expr::AnonymousTag(name, _) => builtin(*name)
            .or_else(|| tag_types.get(name).cloned())
            .unwrap_or(Ty::Opaque(*name)),
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
        Expr::Range(_) => Ty::Opaque(IStr::new("Range".to_string())),
        Expr::SelfRef => locals
            .get(&IStr::new("self".to_string()))
            .cloned()
            .unwrap_or_else(|| Ty::Opaque(IStr::new("Self".to_string()))),
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
        Expr::Cast { ty, .. } => builtin(*ty).unwrap_or(Ty::Opaque(*ty)),
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
    pub fn check_unknowns(&self, ast: &FileAst, db: &dyn crate::database::input_database::Db) {
        for bind in ast.defs.values() {
            let mut locals = std::collections::HashSet::new();
            if let Some(params) = bind.params() {
                for (name, _) in params.iter() {
                    locals.insert(*name);
                }
            }
            self.check_bind(bind, db, &locals);
        }
    }

    fn check_bind(
        &self,
        bind: &Bind,
        db: &dyn crate::database::input_database::Db,
        locals: &std::collections::HashSet<IStr>,
    ) {
        if let Some(tag) = &bind.return_tag {
            self.check_tag(tag, db);
        }
        match bind.value() {
            BindValue::Expr(expr) => self.check_expr(expr, db, locals),
            BindValue::Body { exprs, ret } => {
                use crate::diagnostic::type_ as type_symptom;
                use salsa::Accumulator;

                let mut body_locals = locals.clone();
                for (i, expr) in exprs.iter().enumerate() {
                    if let Expr::Bind(inner) = expr {
                        self.check_bind(inner, db, &body_locals);
                        let name = inner.name();
                        let used = exprs[i + 1..].iter().any(|e| expr_references_name(e, name))
                            || ret.0.as_ref().is_some_and(|e| expr_references_name(e, name));
                        if !used {
                            type_symptom::unused_binding(inner.name_span, name.to_string())
                                .accumulate(db);
                        }
                        body_locals.insert(name);
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

    fn check_expr(
        &self,
        expr: &Expr,
        db: &dyn crate::database::input_database::Db,
        locals: &std::collections::HashSet<IStr>,
    ) {
        use crate::diagnostic::type_ as type_symptom;
        use salsa::Accumulator;

        match expr {
            Expr::FnCall(call) => {
                let name = call.path.root;
                if let Some(args) = &call.args {
                    if self.fn_return_ty(&name).is_none() && !is_builtin_func(name.as_str()) {
                        type_symptom::unknown(call.path.span).accumulate(db);
                    }
                    for arg in args {
                        self.check_expr(arg, db, locals);
                    }
                } else if call.path.segments.is_empty()
                    && !locals.contains(&name)
                    && self.fn_return_ty(&name).is_none()
                {
                    type_symptom::unknown(call.path.span).accumulate(db);
                }
            }
            Expr::Bind(bind) => self.check_bind(bind, db, locals),
            Expr::Binary(bin) => {
                self.check_expr(&bin.lhs, db, locals);
                self.check_expr(&bin.rhs, db, locals);
            }
            Expr::When(w) => {
                if let Some(subject) = &w.subject {
                    self.check_expr(subject, db, locals);
                }
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { condition, body } => {
                            self.check_expr(condition, db, locals);
                            self.check_expr(body, db, locals);
                        }
                        WhenArm::Is { body, .. } | WhenArm::Else(body) => {
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
                        if_locals.extend(
                            params
                                .iter()
                                .filter_map(|(k, _)| (k.as_str() != "_").then_some(*k)),
                        );
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
            Expr::BufSet { buf, index, value, .. } => {
                self.check_expr(buf, db, locals);
                self.check_expr(index, db, locals);
                self.check_expr(value, db, locals);
            }
            Expr::Cast { expr, .. } => self.check_expr(expr, db, locals),
            Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
                self.check_expr(e, db, locals);
            }
            Expr::Lit(_)
            | Expr::SelfRef
            | Expr::Range(_)
            | Expr::FormatString(_)
            | Expr::AnonymousTag(..)
            | Expr::TagCall(_) => {}
        }
    }

    fn check_tag(&self, tag: &Tag, db: &dyn crate::database::input_database::Db) {
        use crate::diagnostic::type_ as type_symptom;
        use salsa::Accumulator;

        match tag {
            Tag::Nominal(name, span) => {
                if self.lookup_tag(*name).is_none() && builtin(*name).is_none() {
                    type_symptom::unknown(*span).accumulate(db);
                }
            }
            Tag::Generic(name, params, span) => {
                if self.lookup_tag(*name).is_none() && builtin(*name).is_none() {
                    type_symptom::unknown(*span).accumulate(db);
                }
                for kind in params.values() {
                    if let ParameterKind::Tagged(inner) = kind {
                        self.check_tag(inner, db);
                    }
                }
            }
            Tag::Qualified(path) => {
                if self.lookup_tag(path.root).is_none() && builtin(path.root).is_none() {
                    type_symptom::unknown(path.span).accumulate(db);
                }
            }
        }
    }
}

fn expr_references_name(expr: &Expr, name: IStr) -> bool {
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
                    || ret.0.as_ref().is_some_and(|e| expr_references_name(e, name))
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
        Expr::BufSet { buf, index, value, .. } => {
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
        Expr::Lit(_) | Expr::SelfRef | Expr::AnonymousTag(..) | Expr::TagCall(_) => false,
    }
}

fn is_builtin_func(name: &str) -> bool {
    matches!(name, "syscall" | "float_bits" | "print" | "println")
}
