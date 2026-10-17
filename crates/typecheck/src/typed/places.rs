use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct PlaceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct PlaceVersionId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct TypedPlace {
    pub binder: BinderId,
    pub name: Intern<String>,
    pub parent: Option<PlaceId>,
    pub projection: Option<PlaceProjection>,
    pub mutable: bool,
    pub explicit_contract: bool,
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlaceProjection {
    Field(usize),
    Item(TargetIndex),
    Deref,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaceVersionOrigin {
    Parameter,
    Declared,
    Initializer(ExprId),
    Temporary(ExprId),
    Mutation(ExprId),
    Projection {
        base: PlaceVersionId,
    },
    Rebind {
        value: ExprId,
        predecessor: Option<PlaceVersionId>,
    },
    Join(Vec<PlaceVersionId>),
    LoopPhi {
        incoming: PlaceVersionId,
        backedges: Vec<PlaceVersionId>,
    },
}

impl PlaceVersionOrigin {
    pub(crate) fn predecessors(&self) -> impl Iterator<Item = PlaceVersionId> + '_ {
        let (first, remaining): (Option<PlaceVersionId>, &[PlaceVersionId]) = match self {
            Self::Parameter
            | Self::Declared
            | Self::Initializer(_)
            | Self::Temporary(_)
            | Self::Mutation(_) => (None, &[]),
            Self::Projection { base } => (Some(*base), &[]),
            Self::Rebind { predecessor, .. } => (*predecessor, &[]),
            Self::Join(predecessors) => (None, predecessors),
            Self::LoopPhi {
                incoming,
                backedges,
            } => (Some(*incoming), backedges),
        };
        first.into_iter().chain(remaining.iter().copied())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedPlaceVersion {
    pub place: PlaceId,
    pub origin: PlaceVersionOrigin,
    pub ty: Ty,
    pub integer_knowledge: Option<ast::integer::IntegerKnowledge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaceVersionComponent {
    pub versions: Vec<PlaceVersionId>,
    pub cyclic: bool,
}

impl TypedFileAst {
    pub(crate) fn temporary_owner(&self, expression: ExprId) -> Option<PlaceId> {
        let version = self.exprs.place_version_of(expression).flatten()?;
        let version = &self.place_versions[version.0 as usize];
        matches!(version.origin, PlaceVersionOrigin::Temporary(source) if source == expression)
            .then_some(version.place)
    }

    pub(crate) fn add_place(&mut self, place: TypedPlace) -> PlaceId {
        assert_eq!(
            place.parent.is_some(),
            place.projection.is_some(),
            "place parent and projection must be present together"
        );
        if let Some(parent) = place.parent {
            assert!(
                (parent.0 as usize) < self.places.len(),
                "projected place refers to an unallocated parent: {parent:?}"
            );
        }
        let id = PlaceId(u32::try_from(self.places.len()).expect("place arena exceeds u32 IDs"));
        self.places.push(place);
        id
    }

    pub(crate) fn add_place_version(
        &mut self,
        place: PlaceId,
        origin: PlaceVersionOrigin,
        ty: Ty,
        integer_knowledge: Option<ast::integer::IntegerKnowledge>,
    ) -> PlaceVersionId {
        assert!(
            (place.0 as usize) < self.places.len(),
            "place version refers to an unallocated place: {place:?}"
        );
        let predecessor_place = match &origin {
            PlaceVersionOrigin::Projection { .. } => self.places[place.0 as usize]
                .parent
                .expect("projection version requires a projected place"),
            _ => place,
        };
        for predecessor in origin.predecessors() {
            let version = self
                .place_versions
                .get(predecessor.0 as usize)
                .expect("place version refers to an unallocated predecessor");
            assert_eq!(
                version.place, predecessor_place,
                "place version predecessor belongs to a different place"
            );
        }
        let id = PlaceVersionId(
            u32::try_from(self.place_versions.len()).expect("place version arena exceeds u32 IDs"),
        );
        self.place_versions.push(TypedPlaceVersion {
            place,
            origin,
            ty,
            integer_knowledge,
        });
        id
    }

    pub(crate) fn borrowed_parameter_referent(&self, place: PlaceId) -> Option<&Ty> {
        let info = &self.places[place.0 as usize];
        if info.parent.is_some() {
            return None;
        }
        let ast::BinderOwner::Parameter { definition, slot } = info.binder.owner else {
            return None;
        };
        if self
            .defs
            .get(&definition)?
            .param_conventions
            .get(slot as usize)
            != Some(&ParamConvention::Mutate)
        {
            return None;
        }
        match &info.ty {
            Ty::Ref { inner, .. } => Some(inner),
            _ => None,
        }
    }

    pub(crate) fn owns_projection(&self, place: PlaceId) -> bool {
        let info = &self.places[place.0 as usize];
        let Some(parent) = info.parent else {
            return false;
        };
        let ty = &self.places[parent.0 as usize].ty;
        if self.type_registry.reference_former_for_type(ty).is_some() {
            return false;
        }
        match info.projection {
            Some(PlaceProjection::Field(_)) => true,
            Some(PlaceProjection::Item(_)) => matches!(
                self.type_registry.resolved_definition_for_type(ty),
                Ty::Array { .. }
            ),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn fixture() -> TypedFileAst {
        TypedFileAst::new(FileId(0), SpanTable::new())
    }

    fn root() -> TypedPlace {
        let name = Intern::from_ref("place");
        TypedPlace {
            binder: BinderId::new(0, ast::BinderOwner::Expression(0)),
            name,
            parent: None,
            projection: None,
            mutable: true,
            explicit_contract: true,
            ty: Ty::Unit,
        }
    }

    #[test]
    fn place_allocation_preserves_parent_and_version_ids() {
        let mut typed = fixture();
        let parent = typed.add_place(root());
        assert_eq!(parent, PlaceId(0));
        let child = typed.add_place(TypedPlace {
            parent: Some(parent),
            projection: Some(PlaceProjection::Field(0)),
            ..root()
        });
        assert_eq!(child, PlaceId(1));
        let version = typed.add_place_version(parent, PlaceVersionOrigin::Declared, Ty::Unit, None);
        let projection = typed.add_place_version(
            child,
            PlaceVersionOrigin::Projection { base: version },
            Ty::Unit,
            None,
        );
        assert_eq!(version, PlaceVersionId(0));
        assert_eq!(projection, PlaceVersionId(1));
        assert_eq!(typed.place_versions[projection.0 as usize].place, child);
    }

    #[test]
    fn incomplete_projection_metadata_is_rejected_before_insertion() {
        let mut typed = fixture();
        let parent = typed.add_place(root());
        for place in [
            TypedPlace {
                parent: Some(parent),
                ..root()
            },
            TypedPlace {
                projection: Some(PlaceProjection::Deref),
                ..root()
            },
        ] {
            let before = typed.places.clone();
            assert!(catch_unwind(AssertUnwindSafe(|| typed.add_place(place))).is_err());
            assert_eq!(typed.places, before);
        }
    }

    #[test]
    fn unallocated_parent_is_rejected_before_insertion() {
        let mut typed = fixture();
        let place = TypedPlace {
            parent: Some(PlaceId(0)),
            projection: Some(PlaceProjection::Deref),
            ..root()
        };
        assert!(catch_unwind(AssertUnwindSafe(|| typed.add_place(place))).is_err());
        assert!(typed.places.is_empty());
    }

    #[test]
    fn unallocated_version_place_is_rejected_before_insertion() {
        let mut typed = fixture();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                typed.add_place_version(PlaceId(0), PlaceVersionOrigin::Declared, Ty::Unit, None)
            }))
            .is_err()
        );
        assert!(typed.place_versions.is_empty());
    }
    fn origins(predecessor: PlaceVersionId) -> Vec<PlaceVersionOrigin> {
        vec![
            PlaceVersionOrigin::Projection { base: predecessor },
            PlaceVersionOrigin::Rebind {
                value: ExprId(0),
                predecessor: Some(predecessor),
            },
            PlaceVersionOrigin::Join(vec![predecessor]),
            PlaceVersionOrigin::LoopPhi {
                incoming: predecessor,
                backedges: vec![],
            },
            PlaceVersionOrigin::LoopPhi {
                incoming: PlaceVersionId(0),
                backedges: vec![predecessor],
            },
        ]
    }

    #[test]
    fn unallocated_predecessors_are_rejected_before_insertion() {
        let mut typed = fixture();
        let parent = typed.add_place(root());
        let child = typed.add_place(TypedPlace {
            parent: Some(parent),
            projection: Some(PlaceProjection::Deref),
            ..root()
        });
        typed.add_place_version(child, PlaceVersionOrigin::Declared, Ty::Unit, None);
        for origin in origins(PlaceVersionId(1)) {
            let before = typed.place_versions.clone();
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    typed.add_place_version(child, origin, Ty::Unit, None)
                }))
                .is_err()
            );
            assert_eq!(typed.place_versions, before);
        }
    }

    #[test]
    fn predecessors_from_other_places_are_rejected_before_insertion() {
        let mut typed = fixture();
        let place = typed.add_place(root());
        let other = typed.add_place(root());
        typed.add_place_version(place, PlaceVersionOrigin::Declared, Ty::Unit, None);
        let foreign = typed.add_place_version(other, PlaceVersionOrigin::Declared, Ty::Unit, None);
        for origin in origins(foreign) {
            let before = typed.place_versions.clone();
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    typed.add_place_version(place, origin, Ty::Unit, None)
                }))
                .is_err()
            );
            assert_eq!(typed.place_versions, before);
        }
    }

    #[test]
    fn projection_base_must_belong_to_the_parent() {
        let mut typed = fixture();
        let parent = typed.add_place(root());
        let child = typed.add_place(TypedPlace {
            parent: Some(parent),
            projection: Some(PlaceProjection::Deref),
            ..root()
        });
        let child_version =
            typed.add_place_version(child, PlaceVersionOrigin::Declared, Ty::Unit, None);
        let before = typed.place_versions.clone();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                typed.add_place_version(
                    child,
                    PlaceVersionOrigin::Projection {
                        base: child_version,
                    },
                    Ty::Unit,
                    None,
                )
            }))
            .is_err()
        );
        assert_eq!(typed.place_versions, before);
    }

    #[test]
    fn empty_joins_and_same_place_predecessors_are_valid() {
        let mut typed = fixture();
        let place = typed.add_place(root());
        let first =
            typed.add_place_version(place, PlaceVersionOrigin::Join(vec![]), Ty::Unit, None);
        for origin in origins(first).into_iter().skip(1) {
            typed.add_place_version(place, origin, Ty::Unit, None);
        }
        assert_eq!(typed.place_versions.len(), 5);
    }
}
