//! Compile-time trait dispatch: `Reflectable`, auto trait defaults, and provided trait overrides.

use std::collections::{HashMap, HashSet};

use internment::Intern;

use crate::reflect::{const_value_for_provided_trait_field, reflect_ty_to_const_value};
use crate::ty::Ty;
use crate::typed::{TagId, TypedFileAst, TypedTag};
use ast::ConstValue;
use ast::FileAst;
use ast::declare::ProvidedTrait;

pub const REFLECTABLE_TRAIT: &str = "Reflectable";
pub const REFLECTABLE_SHAPE_FIELD: &str = "shape";
pub const COMPARABLE_TRAIT: &str = "Comparable";
pub const RESERVED_TRAITS: &[&str] = &["Reflectable", "Comparable"];

#[derive(Debug, Clone)]
pub struct CompileTimeTraitRegistry {
    pub imported_traits: HashSet<Intern<String>>,
    /// Shared package-level eval AST; cloning the registry only bumps the Arc refcount
    /// instead of duplicating ~32 files' worth of definitions per `TypedFileAst`.
    pub eval_ast: std::sync::Arc<FileAst>,
}

impl Default for CompileTimeTraitRegistry {
    fn default() -> Self {
        Self {
            imported_traits: HashSet::new(),
            eval_ast: std::sync::Arc::new(FileAst::empty_for_tests()),
        }
    }
}

impl CompileTimeTraitRegistry {
    pub fn from_file_ast(file: &FileAst) -> Self {
        Self::from_parse_ast(file, std::sync::Arc::new(file.clone()))
    }

    pub fn from_parse_ast(file_ast: &FileAst, eval_ast: std::sync::Arc<FileAst>) -> Self {
        Self {
            imported_traits: file_ast.imported_trait_names(),
            eval_ast,
        }
    }

    pub fn trait_in_scope(&self, trait_name: &str) -> bool {
        self.imported_traits
            .iter()
            .any(|t| t.as_str() == trait_name)
    }

    pub fn auto_trait_decl(&self, trait_name: &str) -> Option<&ast::Declare> {
        if !self.trait_in_scope(trait_name) {
            return None;
        }
        self.eval_ast
            .tags
            .get(&Intern::from_ref(trait_name))
            .filter(|decl| decl.attributes.auto)
    }
}

/// Synthetic `Reflectable` provided trait for a resolved type.
pub fn synthesize_reflectable_trait(ty: &Ty) -> ProvidedTrait {
    let cv = reflect_ty_to_const_value(ty);
    ProvidedTrait {
        trait_name: Intern::from_ref(REFLECTABLE_TRAIT),
        trait_name_span: ast::span::SpanId::INVALID,
        fields: vec![(
            Intern::from_ref(REFLECTABLE_SHAPE_FIELD),
            const_value_for_provided_trait_field(cv, ast::span::SpanId::INVALID),
        )],
    }
}

fn reflectable_shape_for_type(
    type_name: Intern<String>,
    typed: &TypedFileAst,
) -> Option<ConstValue> {
    let tag_id = TagId(type_name);
    if let Some(tag) = typed.tags.get(&tag_id) {
        return reflectable_shape_from_tag(tag);
    }
    typed.tag_types.get(&tag_id).map(reflect_ty_to_const_value)
}

/// Look up a trait field value for a nominal type by name.
pub fn trait_field_for_type_name(
    trait_name: &str,
    field_name: &str,
    type_name: Intern<String>,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
    file: Option<&FileAst>,
) -> Option<ConstValue> {
    let tag_id = TagId(type_name);
    if trait_name == REFLECTABLE_TRAIT && field_name == REFLECTABLE_SHAPE_FIELD {
        if !registry.trait_in_scope(trait_name) {
            return None;
        }
        if let Some(tag) = typed.tags.get(&tag_id) {
            return reflectable_shape_from_tag(tag);
        }
        if let Some(ty) = typed.tag_types.get(&tag_id) {
            return Some(reflect_ty_to_const_value(ty));
        }
    }

    if let Some(tag) = typed.tags.get(&tag_id) {
        let eval_ast = file.unwrap_or(registry.eval_ast.as_ref());
        if let Some(cv) = provided_trait_field(tag, trait_name, field_name, eval_ast) {
            return Some(cv);
        }
    }

    let eval_ast = file.unwrap_or(registry.eval_ast.as_ref());
    if let Some(auto_trait) = registry.auto_trait_decl(trait_name)
        && let Some(shape) = reflectable_shape_for_type(type_name, typed)
        && let Some(cv) = evaluate_auto_trait_field(auto_trait, field_name, shape, eval_ast)
    {
        return Some(cv);
    }

    None
}

/// Look up a trait field for any resolved type, including auto trait defaults.
pub fn trait_field_for_ty(
    trait_name: &str,
    field_name: &str,
    ty: &Ty,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
) -> Option<ConstValue> {
    if !registry.trait_in_scope(trait_name) {
        return None;
    }
    if trait_name == REFLECTABLE_TRAIT && field_name == REFLECTABLE_SHAPE_FIELD {
        return Some(reflect_ty_to_const_value(ty));
    }
    if let Some(cv) =
        provided_trait_field_for_matching_ty(trait_name, field_name, ty, typed, registry)
    {
        return Some(cv);
    }
    if let Some(type_name) = ty.type_name()
        && let Some(cv) = trait_field_for_type_name(
            trait_name,
            field_name,
            *type_name,
            typed,
            registry,
            Some(registry.eval_ast.as_ref()),
        )
    {
        return Some(cv);
    }
    let shape = reflect_ty_to_const_value(ty);
    if let Some(auto_trait) = registry.auto_trait_decl(trait_name)
        && let Some(cv) = evaluate_auto_trait_field(
            auto_trait,
            field_name,
            shape.clone(),
            registry.eval_ast.as_ref(),
        )
    {
        return Some(cv);
    }
    None
}

fn provided_trait_field_for_matching_ty(
    trait_name: &str,
    field_name: &str,
    ty: &Ty,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
) -> Option<ConstValue> {
    typed
        .tags
        .iter()
        .find(|(tag_id, _)| {
            typed
                .tag_types
                .get(tag_id)
                .is_some_and(|tag_ty| tag_ty == ty)
        })
        .and_then(|(_, tag)| {
            provided_trait_field(tag, trait_name, field_name, registry.eval_ast.as_ref())
        })
}

fn reflectable_shape_from_tag(tag: &TypedTag) -> Option<ConstValue> {
    for pt in &tag.provided_traits {
        if pt.trait_name.as_str() == REFLECTABLE_TRAIT {
            for (name, expr) in &pt.fields {
                if name.as_str() == REFLECTABLE_SHAPE_FIELD {
                    return expr.const_value.clone();
                }
            }
        }
    }
    Some(reflect_ty_to_const_value(&tag.resolved_ty))
}

fn provided_trait_field(
    tag: &TypedTag,
    trait_name: &str,
    field_name: &str,
    eval_ast: &FileAst,
) -> Option<ConstValue> {
    for pt in &tag.provided_traits {
        if pt.trait_name.as_str() != trait_name {
            continue;
        }
        for (name, expr) in &pt.fields {
            if name.as_str() == field_name {
                return expr.const_value.clone().or_else(|| {
                    crate::analysis::eval_compile_time_expr_public(
                        &expr.value,
                        &HashMap::new(),
                        eval_ast,
                    )
                });
            }
        }
    }
    None
}

fn evaluate_auto_trait_field(
    trait_decl: &ast::Declare,
    field_name: &str,
    shape: ConstValue,
    eval_ast: &FileAst,
) -> Option<ConstValue> {
    let expr = auto_trait_field_expr(trait_decl, field_name)?;
    let mut env = HashMap::new();
    env.insert(Intern::from_ref("Self"), Some(shape));
    crate::analysis::eval_compile_time_expr_with_env(&expr.value, &env, eval_ast)
}

fn auto_trait_field_expr<'a>(
    trait_decl: &'a ast::Declare,
    field_name: &str,
) -> Option<&'a ast::Typed<ast::Expr>> {
    match &trait_decl.value {
        ast::DeclareValue::Has(members) => members.iter().find_map(|member| match member {
            ast::HasMember::Property(property) if property.name.as_str() == field_name => {
                property.body.as_ref()?.as_expr()
            }
            ast::HasMember::Function(function) if function.name.as_str() == field_name => {
                function.body.as_ref()?.as_expr()
            }
            _ => None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepare_file_ast;
    use crate::transform::{TransformCtx, transform};
    use crate::typed::FileId;
    use flask::CompileTarget;
    use parser::cursor::TokenCursor;

    #[test]
    fn auto_trait_default_evaluates_with_self_shape() {
        let source = "\
Type is Primitive(width BigInt, signed Bool) or Ptr(inner Type)
Bool is True or False
BigInt is in 0...18446744073709551615
#auto
Copy has can_copy Bool: is_copy(Self)

is_copy(x Type) Bool := when x is
    Primitive(_, _) then True
    Ptr(_)          then False
";
        let mut ast = TokenCursor::parse_source(source);
        let diags = prepare_file_ast(&mut ast, &CompileTarget::Library);
        assert!(diags.is_empty(), "diags: {diags:?}");

        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let registry = CompileTimeTraitRegistry::from_file_ast(&ast);
        let ty = Ty::Int {
            width: 64,
            signed: true,
            value: None,
            min: None,
            max: None,
        };

        assert!(matches!(
            trait_field_for_ty("Copy", "can_copy", &ty, &typed, &registry),
            Some(ConstValue::Tag { name, .. }) if name.as_str() == "True"
        ));
    }
}
