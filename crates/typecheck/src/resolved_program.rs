use std::collections::HashMap;

use ast::ConstValue;
use internment::Intern;

use crate::ty::Ty;
use crate::typed::{
    DefId, ExprId, FileId, TagId, TypedBind, TypedExprRef, TypedExprVec, TypedFileAst, TypedPlace,
    TypedPlaceVersion, TypedTag, VariantLookupResult, VariantMap,
};
use crate::{TypeRegistry, layout::TargetLayout};

#[derive(Clone, Copy)]
pub struct ResolvedProgram<'a> {
    pub file_id: FileId,
    pub type_registry: &'a TypeRegistry,
    pub target_layout: Option<TargetLayout>,
    pub tags: &'a HashMap<TagId, TypedTag>,
    pub defs: &'a HashMap<DefId, TypedBind>,
    pub exprs: &'a TypedExprVec,
    pub(crate) sequence_index_checks: &'a HashMap<ExprId, crate::typed::SequenceIndexCheck>,
    pub places: &'a [TypedPlace],
    pub place_versions: &'a [TypedPlaceVersion],
    pub tag_types: &'a HashMap<TagId, Ty>,
    pub fn_return_types: &'a HashMap<DefId, Ty>,
    pub(crate) local_callable_calls: &'a HashMap<ExprId, ExprId>,
    pub(crate) local_callable_parameters:
        &'a HashMap<ExprId, Vec<crate::typed::LocalCallableParameter>>,
    pub variant_map: &'a VariantMap,
}

impl ResolvedProgram<'_> {
    pub fn local_call(&self, expression: ExprId) -> Option<ExprId> {
        self.local_callable_calls.get(&expression).copied()
    }

    pub fn local_parameters(
        &self,
        expression: ExprId,
    ) -> impl Iterator<Item = (Intern<String>, &Ty)> {
        self.local_callable_parameters
            .get(&expression)
            .into_iter()
            .flatten()
            .map(|parameter| (parameter.name, &parameter.ty))
    }

    pub fn is_local_callable(&self, expression: ExprId) -> bool {
        self.local_callable_parameters.contains_key(&expression)
    }

    pub(crate) fn tag_id_for_value(&self, value: &ConstValue) -> Option<TagId> {
        let ConstValue::Tag {
            name, qual_path, ..
        } = value
        else {
            return None;
        };
        let declared = |name: &str| {
            let id = TagId(Intern::from_ref(name));
            (self.tag_types.contains_key(&id) || self.tags.contains_key(&id)).then_some(id)
        };
        if name.contains('.')
            && let Some(id) = declared(name)
        {
            return Some(id);
        }
        if let Some(path) = qual_path
            && let Some(id) = declared(&format!("{path}.{name}")).or_else(|| declared(path))
        {
            return Some(id);
        }
        declared(name)
    }

    pub fn expr(&self, id: ExprId) -> Option<TypedExprRef<'_>> {
        self.exprs.get(id.as_usize())
    }

    pub fn sequence_index_is_proven(&self, access: ExprId, buffer: ExprId, index: ExprId) -> bool {
        matches!(
            self.exprs.kind_of(access),
            Some(crate::typed::TypedExprKind::BufGet { buf, index: position })
                if *buf == buffer && *position == index
        ) && self
            .sequence_index_checks
            .get(&access)
            .is_some_and(|proof| proof.proves(buffer, index))
    }

    pub fn lookup_variant(
        &self,
        name: &internment::Intern<String>,
    ) -> Option<VariantLookupResult<'_>> {
        self.variant_map
            .get(name)
            .and_then(|candidates| candidates.first())
            .map(|(union, index, fields)| (*union, *index, fields.as_slice()))
    }
}

impl TypedFileAst {
    pub fn resolved_program(&self) -> ResolvedProgram<'_> {
        ResolvedProgram {
            file_id: self.file_id,
            type_registry: &self.type_registry,
            target_layout: self.target_layout,
            tags: &self.tags,
            defs: &self.defs,
            exprs: &self.exprs,
            sequence_index_checks: &self.sequence_index_checks,
            places: &self.places,
            place_versions: &self.place_versions,
            tag_types: &self.tag_types,
            fn_return_types: &self.fn_return_types,
            local_callable_calls: &self.local_callable_calls,
            local_callable_parameters: &self.local_callable_parameters,
            variant_map: &self.variant_map,
        }
    }
}
