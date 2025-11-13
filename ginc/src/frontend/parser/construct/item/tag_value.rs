use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum TagValue {
    // Tag2 ::= Tag1
    // PossibleTags ::= Tag1 | Tag2
    Alias(Tag),
    // Person ::= (name String, age Number)
    Record(Vec<Parameter>),
    // Record(std::iter::Map<&'src str, Box<TagValue<'src>>>),
    // PersonSet ::= { p : Person }
    Set(/* TODO */),
}
