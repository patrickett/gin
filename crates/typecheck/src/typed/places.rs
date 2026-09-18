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
