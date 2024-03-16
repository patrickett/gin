use std::ops::Add;

#[derive(Debug, Clone)]
pub enum GinValue {
    Bool(bool),
    String(String),
    Number(usize),
    Nothing,
}

impl Add for GinValue {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        // nothing + something<t> = something<t>
        // nothing + nothing = nothing
        match self {
            GinValue::String(s1) => match other {
                GinValue::String(s2) => GinValue::String(s1 + &s2),
                GinValue::Number(n1) => GinValue::String(s1 + &n1.to_string()),
                GinValue::Nothing => GinValue::String(s1),
                val => panic!("{val:#?} cannot be added together"),
            },
            GinValue::Number(n1) => match other {
                GinValue::String(s1) => GinValue::String(n1.to_string() + &s1),
                GinValue::Number(n2) => GinValue::Number(n1 + n2),
                GinValue::Nothing => GinValue::Number(n1),
                val => panic!("{val:#?} cannot be added together"),
            },
            GinValue::Nothing => other,
            val => panic!("{val:#?} cannot be added together"),
        }
    }
}

impl std::fmt::Display for GinValue {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match self {
            GinValue::Nothing => Ok(()),
            GinValue::String(s) => write!(fmt, "{}", s),
            GinValue::Number(n) => write!(fmt, "{}", n),
            GinValue::Bool(b) => write!(fmt, "{}", b),
        }
    }
}
