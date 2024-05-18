use std::{ops::Add, str::FromStr};

#[derive(Debug, Clone, PartialEq)]
pub enum GinNumber {
    Signed128(i128),
    Float64(f64),
}

impl ToString for GinNumber {
    fn to_string(&self) -> String {
        match self {
            GinNumber::Signed128(n) => n.to_string(),
            GinNumber::Float64(n) => n.to_string(),
        }
    }
}

impl FromStr for GinNumber {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(i) = s.parse::<i128>() {
            Ok(GinNumber::Signed128(i))
        } else if let Ok(float_val) = s.parse::<f64>() {
            Ok(GinNumber::Float64(float_val))
        } else {
            panic!("Unable to convert {} to GinNumber", s);
        }
    }
}

impl Add for GinNumber {
    type Output = GinNumber;

    fn add(self, rhs: Self) -> Self::Output {
        // println!("{:#?} + {:#?}", &self, &rhs);
        match self {
            GinNumber::Signed128(left_128) => match rhs {
                GinNumber::Signed128(right_128) => GinNumber::Signed128(left_128 + right_128),
                GinNumber::Float64(right_float) => {
                    GinNumber::Float64(left_128 as f64 + right_float)
                }
            },
            GinNumber::Float64(left_float) => match rhs {
                GinNumber::Signed128(right_128) => {
                    GinNumber::Float64(left_float + right_128 as f64)
                }
                GinNumber::Float64(right_float) => GinNumber::Float64(left_float + right_float),
            },
        }
    }
}
