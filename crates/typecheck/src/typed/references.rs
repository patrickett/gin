use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArgumentReference {
    pub parameter: usize,
    pub fields: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReferenceSource {
    Argument(ArgumentReference),
    ArgumentElements(ArgumentReference),
    Target(ReferenceTargetGroup),
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceValue {
    pub sources: HashSet<ReferenceSource>,
    pub loans: HashSet<ReferenceLoan>,
    pub fields: Vec<ReferenceValue>,
    pub array_element: Option<Box<ReferenceValue>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceLoan {
    pub expression: ExprId,
    pub fields: Vec<usize>,
}

impl ReferenceLoan {
    pub fn new(expression: ExprId) -> Self {
        Self {
            expression,
            fields: Vec::new(),
        }
    }
}

impl ReferenceValue {
    /// Shared empty value for expressions that carry no reference information,
    /// so a `None` slot can be read as an empty `ReferenceValue` without a copy.
    pub fn empty() -> &'static ReferenceValue {
        static EMPTY: std::sync::LazyLock<ReferenceValue> =
            std::sync::LazyLock::new(ReferenceValue::default);
        &EMPTY
    }

    pub fn constrain_to(&mut self, shape: &Self) {
        if shape.sources.is_empty() {
            self.sources.clear();
            self.loans.clear();
        }
        if let Some(element) = &shape.array_element {
            for field in &mut self.fields {
                field.constrain_to(element);
            }
            if let Some(value) = &mut self.array_element {
                value.constrain_to(element);
            }
        } else {
            self.array_element = None;
            self.fields.truncate(shape.fields.len());
            for (field, shape) in self.fields.iter_mut().zip(&shape.fields) {
                field.constrain_to(shape);
            }
        }
    }

    pub fn join(&mut self, other: &Self) {
        self.sources.extend(other.sources.iter().cloned());
        self.loans.extend(other.loans.iter().cloned());
        if let Some(element) = &other.array_element {
            self.array_element
                .get_or_insert_with(Default::default)
                .join(element);
        }
        self.fields
            .resize_with(self.fields.len().max(other.fields.len()), Self::default);
        for (field, other) in self.fields.iter_mut().zip(&other.fields) {
            field.join(other);
        }
    }

    pub(crate) fn array_contents(&self) -> Self {
        let mut value = Self::default();
        for element in self.fields.iter().chain(self.array_element.as_deref()) {
            element.visit(&mut |element| {
                value.sources.extend(element.sources.iter().cloned());
                value.loans.extend(element.loans.iter().cloned());
            });
        }
        value
    }

    pub fn field(&self, path: &[usize]) -> Option<&Self> {
        path.iter().try_fold(self, |value, index| {
            value.fields.get(*index).or(value.array_element.as_deref())
        })
    }

    pub(crate) fn targets_in_definition(
        &self,
        definition: &TypedBind,
        registry: &crate::TypeRegistry,
    ) -> Option<ReferenceTargetSet> {
        let mut targets = ReferenceTargetSet::default();
        for source in &self.sources {
            let target = match source {
                ReferenceSource::Target(target) => target.clone(),
                ReferenceSource::Argument(argument) => {
                    let group = definition
                        .param_groups
                        .get(argument.parameter)
                        .copied()
                        .flatten()?;
                    let (_, parameter_ty) = definition.params.get(argument.parameter)?;
                    let mut ty =
                        crate::ty::reference_pointee_for_type(parameter_ty, Some(registry))
                            .unwrap_or_else(|| parameter_ty.clone());
                    let mut target = ReferenceTargetGroup::Param(group);
                    for index in &argument.fields {
                        let definition = registry.resolved_definition_for_type(&ty);
                        let array = matches!(definition, Ty::Array { .. });
                        let field_ty = match definition {
                            Ty::Record { fields, .. } => fields.get(*index)?.1.as_ref().clone(),
                            Ty::Sequence(fields) => fields.get(*index)?.ty.as_ref().clone(),
                            Ty::Array { elem, .. } => elem.as_ref().clone(),
                            _ => return None,
                        };
                        target = if array {
                            ReferenceTargetGroup::ItemRegion {
                                base: Box::new(target),
                                index: TargetIndex::Symbolic(ast::NormalExpr::Value(
                                    ast::ConstValue::Int((*index as u128).into()),
                                )),
                            }
                        } else {
                            ReferenceTargetGroup::Field {
                                base: Box::new(target),
                                index: *index,
                            }
                        };
                        if let Some(pointee) =
                            crate::ty::reference_pointee_for_type(&field_ty, Some(registry))
                        {
                            target = ReferenceTargetGroup::Deref(Box::new(target));
                            ty = pointee;
                        } else {
                            ty = field_ty;
                        }
                    }
                    target
                }
                ReferenceSource::ArgumentElements(_) | ReferenceSource::Unknown => return None,
            };
            targets.extend(&ReferenceTargetSet::singleton(target));
        }
        Some(targets)
    }

    pub fn targets(&self) -> ReferenceTargetSet {
        let mut targets = ReferenceTargetSet::default();
        for source in &self.sources {
            if let ReferenceSource::Target(target) = source {
                targets.extend(&ReferenceTargetSet::singleton(target.clone()));
            }
        }
        targets
    }

    pub fn without_loans(mut self) -> Self {
        self.loans.clear();
        self.fields = self.fields.into_iter().map(Self::without_loans).collect();
        self.array_element = self
            .array_element
            .map(|element| Box::new(element.without_loans()));
        self
    }

    pub fn visit(&self, f: &mut impl FnMut(&Self)) {
        f(self);
        if let Some(element) = &self.array_element {
            element.visit(f);
        }
        for field in &self.fields {
            field.visit(f);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceWriteTarget {
    pub base: ArgumentReference,
    pub field: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReferenceWriteOrigin {
    Slot(ArgumentReference),
    Elements(ArgumentReference),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceSummary {
    pub result: ReferenceValue,
    pub reads: HashSet<ArgumentReference>,
    pub writes: HashMap<ReferenceWriteTarget, ReferenceValue>,
    pub unknown_writes: bool,
    pub untracked_writes: bool,
    pub unknown_slot_writes: HashMap<ReferenceWriteOrigin, ReferenceValue>,
}

impl ReferenceSummary {
    pub(crate) fn surface_key(&self) -> String {
        fn value_key(value: &ReferenceValue) -> String {
            let mut sources: Vec<_> = value
                .sources
                .iter()
                .map(|source| format!("{source:?}"))
                .collect();
            sources.sort();
            format!(
                "{:?}:{:?}:{:?}",
                sources,
                value
                    .array_element
                    .as_ref()
                    .map(|element| value_key(element)),
                value.fields.iter().map(value_key).collect::<Vec<_>>()
            )
        }
        let mut reads: Vec<_> = self.reads.iter().map(|read| format!("{read:?}")).collect();
        reads.sort();
        let mut writes: Vec<_> = self
            .writes
            .iter()
            .map(|(target, value)| format!("{target:?}:{}", value_key(value)))
            .collect();
        writes.sort();
        let mut unknown_slots: Vec<_> = self
            .unknown_slot_writes
            .iter()
            .map(|(base, value)| format!("{base:?}:{}", value_key(value)))
            .collect();
        unknown_slots.sort();
        format!(
            "{}:{reads:?}:{writes:?}:{unknown_slots:?}:{}:{}",
            value_key(&self.result),
            self.unknown_writes,
            self.untracked_writes
        )
    }
}
