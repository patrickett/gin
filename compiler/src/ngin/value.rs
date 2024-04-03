use std::ops::{Add, Div, Mul, Sub};

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
                _ => panic!("The right-hand side of an arithmetic operation must be of type `number`, `string` or `float` type"),
            },
            GinValue::Number(n1) => match other {
                GinValue::String(s1) => GinValue::String(n1.to_string() + &s1),
                GinValue::Number(n2) => GinValue::Number(n1 + n2),
                GinValue::Nothing => GinValue::Number(n1),
                _ => panic!("The right-hand side of an arithmetic operation must be of type `number`, `string` or `float` type"),
            },
            GinValue::Nothing => other,
            _ => panic!("The left-hand side of an arithmetic operation must be of type `number`, `string` or `float` type"),
        }
    }
}

impl Mul for GinValue {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
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
            GinValue::Nothing => GinValue::Nothing,
            val => panic!("{val:#?} cannot be added together"),
        }
    }
}

impl Div for GinValue {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        // nothing + something<t> = something<t>
        // nothing + nothing = nothing
        match self {
            GinValue::String(s1) => match other {
                GinValue::String(s2) => GinValue::String(s1 + &s2),
                GinValue::Number(n1) => GinValue::String(s1 + &n1.to_string()),
                GinValue::Nothing => GinValue::Nothing,
                val => panic!("{val:#?} cannot be divided"),
            },
            GinValue::Number(n1) => match other {
                GinValue::String(s1) => GinValue::String(n1.to_string() + &s1),
                GinValue::Number(n2) => GinValue::Number(n1 + n2),
                GinValue::Nothing => GinValue::Nothing,
                val => panic!("{val:#?} cannot be divided"),
            },
            GinValue::Nothing => GinValue::Nothing,
            val => panic!("{val:#?} cannot be divided"),
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