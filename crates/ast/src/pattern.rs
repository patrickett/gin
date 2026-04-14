use internment::Intern;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pattern {
    Ident(Intern<String>),
    Tuple(Vec<Pattern>),
}
