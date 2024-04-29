use std::{ops::Add, str::FromStr};

#[derive(Debug, Clone, PartialEq)]
pub enum GinNumber {
    // Unsigned8(u8),
    // Unsigned16(u16),
    // Unsigned(usize),
    // Unsigned32(u32),
    // Unsigned64(u64),
    // Unsigned128(u128),

    // Signed8(i8),
    // Signed16(i16),
    // Signed(isize),
    // Signed32(i32),
    // Signed64(i64),
    Signed128(i128),

    // Float32(f32),
    Float64(f64),
}

impl ToString for GinNumber {
    fn to_string(&self) -> String {
        match self {
            // GinNumber::Unsigned8(n) => n.to_string(),
            // GinNumber::Unsigned16(n) => n.to_string(),
            // GinNumber::Unsigned(n) => n.to_string(),
            // GinNumber::Unsigned32(n) => n.to_string(),
            // GinNumber::Unsigned64(n) => n.to_string(),
            // GinNumber::Unsigned128(n) => n.to_string(),
            // GinNumber::Signed8(n) => n.to_string(),
            // GinNumber::Signed16(n) => n.to_string(),
            // GinNumber::Signed(n) => n.to_string(),
            // GinNumber::Signed32(n) => n.to_string(),
            // GinNumber::Signed64(n) => n.to_string(),
            GinNumber::Signed128(n) => n.to_string(),
            // GinNumber::Float32(n) => n.to_string(),
            GinNumber::Float64(n) => n.to_string(),
        }
    }
}

impl FromStr for GinNumber {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // if let Ok(u) = s.parse::<u8>() {
        //     Ok(GinNumber::Unsigned8(u))
        // } else if let Ok(u) = s.parse::<u16>() {
        //     Ok(GinNumber::Unsigned16(u))
        // } else if let Ok(u) = s.parse::<usize>() {
        //     Ok(GinNumber::Unsigned(u))
        // } else if let Ok(u) = s.parse::<u32>() {
        //     Ok(GinNumber::Unsigned32(u))
        // } else if let Ok(u) = s.parse::<u64>() {
        //     Ok(GinNumber::Unsigned64(u))
        // }

        // if let Ok(u) = s.parse::<u128>() {
        //     Ok(GinNumber::Unsigned128(u))
        // }
        // else if let Ok(i) = s.parse::<i8>() {
        //     Ok(GinNumber::Signed8(i))
        // } else if let Ok(i) = s.parse::<i16>() {
        //     Ok(GinNumber::Signed16(i))
        // } else if let Ok(i) = s.parse::<isize>() {
        //     Ok(GinNumber::Signed(i))
        // } else if let Ok(i) = s.parse::<i32>() {
        //     Ok(GinNumber::Signed32(i))
        // }
        // else if let Ok(i) = s.parse::<i64>() {
        //     Ok(GinNumber::Signed64(i))
        // }
        if let Ok(i) = s.parse::<i128>() {
            Ok(GinNumber::Signed128(i))
        }
        // else if let Ok(float_val) = s.parse::<f32>() {
        //     Ok(GinNumber::Float32(float_val))
        // }
        else if let Ok(float_val) = s.parse::<f64>() {
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
