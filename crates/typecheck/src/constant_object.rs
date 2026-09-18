use ast::{ConstValue, Ty};
use internment::Intern;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConstObjectId(u32);

impl ConstObjectId {
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }
}

impl ConstObjectId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstProjection {
    ResultRowElement(usize),
    FixedArrayElement(usize),
    SequenceElement(usize),
    RecordField(Intern<String>),
    TagArgument(usize),
    RepeatSeed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ConstPath(pub Vec<ConstProjection>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConstRelocation {
    pub source: ConstPath,
    pub target: ConstObjectId,
    pub target_path: ConstPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstObject {
    pub ty: Ty,
    pub value: ConstValue,
    pub relocations: Vec<ConstRelocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstObjectError {
    UnknownTarget(ConstObjectId),
    InvalidSourcePath(ConstPath),
    InvalidTargetPath {
        target: ConstObjectId,
        path: ConstPath,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstObjectStore {
    objects: Vec<ConstObject>,
}

impl ConstObjectStore {
    pub fn intern(&mut self, object: ConstObject) -> Result<ConstObjectId, ConstObjectError> {
        if let Some((id, _)) = self.iter().find(|(_, existing)| **existing == object) {
            return Ok(id);
        }
        self.insert(object)
    }

    pub fn insert(&mut self, object: ConstObject) -> Result<ConstObjectId, ConstObjectError> {
        for relocation in &object.relocations {
            if project(&object.value, &relocation.source).is_none() {
                return Err(ConstObjectError::InvalidSourcePath(
                    relocation.source.clone(),
                ));
            }
            let Some(target) = self.get(relocation.target) else {
                return Err(ConstObjectError::UnknownTarget(relocation.target));
            };
            if project(&target.value, &relocation.target_path).is_none() {
                return Err(ConstObjectError::InvalidTargetPath {
                    target: relocation.target,
                    path: relocation.target_path.clone(),
                });
            }
        }
        let id = ConstObjectId(self.objects.len() as u32);
        self.objects.push(object);
        Ok(id)
    }

    pub fn get(&self, id: ConstObjectId) -> Option<&ConstObject> {
        self.objects.get(id.index())
    }

    pub fn iter(&self) -> impl Iterator<Item = (ConstObjectId, &ConstObject)> {
        self.objects
            .iter()
            .enumerate()
            .map(|(index, object)| (ConstObjectId(index as u32), object))
    }
}

fn project<'a>(value: &'a ConstValue, path: &ConstPath) -> Option<&'a ConstValue> {
    path.0
        .iter()
        .try_fold(value, |value, projection| match (value, projection) {
            (ConstValue::ResultRow(items), ConstProjection::ResultRowElement(index))
            | (ConstValue::FixedArray(items), ConstProjection::FixedArrayElement(index))
            | (ConstValue::Sequence(items), ConstProjection::SequenceElement(index)) => {
                items.get(*index)
            }
            (ConstValue::Record { fields }, ConstProjection::RecordField(name)) => fields
                .iter()
                .find_map(|(field, value)| (field == name).then_some(value)),
            (ConstValue::Tag { args, .. }, ConstProjection::TagArgument(index)) => args.get(*index),
            (ConstValue::SequenceRepeat { seed, .. }, ConstProjection::RepeatSeed) => {
                Some(seed.as_ref())
            }
            _ => None,
        })
}

pub(crate) fn collect_closed_constants(typed: &mut crate::TypedFileAst) {
    for index in 0..typed.exprs.kind.len() {
        if typed.exprs.facts.const_facts.const_object[index].is_some() {
            continue;
        }
        if matches!(
            typed.exprs.facts.const_facts.literal_materialization[index],
            Some(crate::typed::LiteralMaterializationPlan::BorrowedSequence { .. })
        ) {
            let values = if let Some(crate::typed::TypedSequenceLiteral::Elements(items)) =
                typed.exprs.facts.const_facts.sequence_literal[index].as_ref()
            {
                let Some(values) = items
                    .iter()
                    .map(|item| typed.exprs.facts.const_facts.const_value[item.as_usize()].clone())
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                ConstValue::Sequence(values.into())
            } else {
                continue;
            };
            let object = ConstObject {
                ty: Ty::Unit,
                value: values,
                relocations: Vec::new(),
            };
            let Some(id) = typed.constant_objects.intern(object).ok() else {
                continue;
            };
            typed.exprs.facts.const_facts.const_object[index] = Some(id);
            if let Some(plan) =
                typed.exprs.facts.const_facts.literal_materialization[index].as_mut()
            {
                match plan {
                    crate::typed::LiteralMaterializationPlan::BorrowedSequence {
                        backing, ..
                    } => {
                        *backing = Some(id);
                    }
                    _ => {}
                }
                typed.exprs.facts.semantic_facts.sequence_provenance[index] =
                    Some(crate::typed::SequenceProvenance::Static(id));
            }
            continue;
        }
        let Some(value) = typed.exprs.facts.const_facts.const_value[index].as_ref() else {
            continue;
        };
        if !matches!(
            value,
            ConstValue::ResultRow(_)
                | ConstValue::FixedArray(_)
                | ConstValue::Sequence(_)
                | ConstValue::SequenceRepeat { .. }
                | ConstValue::Record { .. }
                | ConstValue::Text(_)
        ) {
            continue;
        }
        let object = ConstObject {
            ty: typed.exprs.ty[index].clone(),
            value: value.clone(),
            relocations: Vec::new(),
        };
        typed.exprs.facts.const_facts.const_object[index] =
            typed.constant_objects.intern(object).ok();
        if let Some(id) = typed.exprs.facts.const_facts.const_object[index]
            && let Some(crate::typed::LiteralMaterializationPlan::BorrowedSequence {
                backing, ..
            }) = typed.exprs.facts.const_facts.literal_materialization[index].as_mut()
        {
            *backing = Some(id);
            typed.exprs.facts.semantic_facts.sequence_provenance[index] =
                Some(crate::typed::SequenceProvenance::Static(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocations_use_semantic_paths_without_target_layout() {
        let mut store = ConstObjectStore::default();
        let backing = store
            .insert(ConstObject {
                ty: Ty::Unit,
                value: ConstValue::FixedArray(
                    vec![ConstValue::Int(11.into()), ConstValue::Int(13.into())].into(),
                ),
                relocations: Vec::new(),
            })
            .unwrap();
        let view = store
            .insert(ConstObject {
                ty: Ty::Unit,
                value: ConstValue::Record {
                    fields: vec![
                        (Intern::from_ref("data"), ConstValue::Int(0.into())),
                        (Intern::from_ref("length"), ConstValue::Int(2.into())),
                    ]
                    .into(),
                },
                relocations: vec![ConstRelocation {
                    source: ConstPath(vec![ConstProjection::RecordField(Intern::from_ref("data"))]),
                    target: backing,
                    target_path: ConstPath(vec![ConstProjection::FixedArrayElement(0)]),
                }],
            })
            .unwrap();

        assert_eq!(store.get(view).unwrap().relocations[0].target, backing);
        assert_eq!(store.iter().count(), 2);
    }
}
