#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IntegerExpressionTag {
    Constant = 0,
    Parameter = 1,
    Declaration = 2,
    Size = 3,
    Alignment = 4,
    IndexBits = 5,
    Negate = 6,
    Add = 7,
    MultiplyConstant = 8,
    PowerOfTwo = 9,
    NonnegativeProduct = 10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IntegerPredicateTag {
    Equal = 0,
    NotEqual = 1,
    Less = 2,
    LessOrEqual = 3,
    Greater = 4,
    GreaterOrEqual = 5,
    InclusiveRange = 6,
    And = 7,
    Or = 8,
    Not = 9,
    IsPowerOfTwo = 10,
}

pub const ARTIFACT_SECTIONS: &[&str] = &[
    "header",
    "fingerprints",
    "resolution-index",
    "declaration-directory",
    "declaration-bodies",
    "public-closure",
    "comptime-images",
];
