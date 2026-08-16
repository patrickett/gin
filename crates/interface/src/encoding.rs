use crate::DeclarationRef;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticDecodeError {
    Truncated,
    InvalidTag,
    Noncanonical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedInteger {
    negative: bool,
    magnitude: Vec<u8>,
}

impl SignedInteger {
    pub fn from_i128(value: i128) -> Self {
        let negative = value < 0;
        let magnitude = value.unsigned_abs().to_be_bytes();
        let first = magnitude
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(magnitude.len());
        Self {
            negative,
            magnitude: magnitude[first..].to_vec(),
        }
    }

    pub fn from_sign_and_magnitude(negative: bool, magnitude: Vec<u8>) -> Option<Self> {
        if magnitude.first() == Some(&0) || negative && magnitude.is_empty() {
            return None;
        }
        Some(Self {
            negative,
            magnitude,
        })
    }

    fn encode(&self, output: &mut Vec<u8>) {
        if self.magnitude.is_empty() {
            output.push(0);
            return;
        }
        output.push(if self.negative { 2 } else { 1 });
        write_uleb(self.magnitude.len(), output);
        output.extend_from_slice(&self.magnitude);
    }

    fn to_i128(&self) -> Option<i128> {
        if self.magnitude.len() > 16 {
            return None;
        }
        let mut bytes = [0u8; 16];
        bytes[16 - self.magnitude.len()..].copy_from_slice(&self.magnitude);
        let magnitude = u128::from_be_bytes(bytes);
        if self.negative {
            i128::try_from(magnitude).ok()?.checked_neg()
        } else {
            i128::try_from(magnitude).ok()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntegerExpr {
    Constant(SignedInteger),
    Parameter(String),
    Declaration(DeclarationRef),
    Size(Box<IntegerExpr>),
    Alignment(Box<IntegerExpr>),
    IndexBits(Box<IntegerExpr>),
    Negate(Box<IntegerExpr>),
    Add(Vec<IntegerExpr>),
    MultiplyConstant(SignedInteger, Box<IntegerExpr>),
    PowerOfTwo(Box<IntegerExpr>),
    NonnegativeProduct(Vec<IntegerExpr>),
}

impl IntegerExpr {
    fn encode(&self, output: &mut Vec<u8>) {
        match self {
            Self::Constant(value) => {
                output.push(0);
                value.encode(output);
            }
            Self::Parameter(name) => {
                output.push(1);
                write_string(name, output);
            }
            Self::Declaration(reference) => {
                output.push(2);
                reference.encode_identity(output);
            }
            Self::Size(value) => encode_unary(3, value, output),
            Self::Alignment(value) => encode_unary(4, value, output),
            Self::IndexBits(value) => encode_unary(5, value, output),
            Self::Negate(value) => encode_unary(6, value, output),
            Self::Add(values) => encode_sequence(7, values, output),
            Self::MultiplyConstant(constant, value) => {
                output.push(8);
                constant.encode(output);
                value.encode(output);
            }
            Self::PowerOfTwo(value) => encode_unary(9, value, output),
            Self::NonnegativeProduct(values) => encode_sequence(10, values, output),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntegerPredicate {
    Equal(IntegerExpr, IntegerExpr),
    NotEqual(IntegerExpr, IntegerExpr),
    Less(IntegerExpr, IntegerExpr),
    LessOrEqual(IntegerExpr, IntegerExpr),
    Greater(IntegerExpr, IntegerExpr),
    GreaterOrEqual(IntegerExpr, IntegerExpr),
    InclusiveRange {
        value: IntegerExpr,
        minimum: IntegerExpr,
        maximum: IntegerExpr,
    },
    And(Vec<IntegerPredicate>),
    Or(Vec<IntegerPredicate>),
    Not(Box<IntegerPredicate>),
    IsPowerOfTwo(IntegerExpr),
}

impl IntegerPredicate {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        self.encode(&mut output);
        output
    }

    fn encode(&self, output: &mut Vec<u8>) {
        match self {
            Self::Equal(left, right) => encode_binary_predicate(0, left, right, output),
            Self::NotEqual(left, right) => encode_binary_predicate(1, left, right, output),
            Self::Less(left, right) => encode_binary_predicate(2, left, right, output),
            Self::LessOrEqual(left, right) => encode_binary_predicate(3, left, right, output),
            Self::Greater(left, right) => encode_binary_predicate(4, left, right, output),
            Self::GreaterOrEqual(left, right) => encode_binary_predicate(5, left, right, output),
            Self::InclusiveRange {
                value,
                minimum,
                maximum,
            } => {
                output.push(6);
                value.encode(output);
                minimum.encode(output);
                maximum.encode(output);
            }
            Self::And(values) => encode_predicate_set(7, values, output),
            Self::Or(values) => encode_predicate_set(8, values, output),
            Self::Not(value) => {
                output.push(9);
                value.encode(output);
            }
            Self::IsPowerOfTwo(value) => {
                output.push(10);
                value.encode(output);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedIntegerValidity {
    predicate: IntegerPredicate,
}

impl EncodedIntegerValidity {
    pub fn new(predicate: IntegerPredicate) -> Self {
        Self { predicate }
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.predicate.canonical_bytes()
    }

    pub fn storage_hull_i128(&self) -> Option<(i128, i128)> {
        predicate_hull(&self.predicate)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SemanticDecodeError> {
        let mut input = bytes;
        let predicate = decode_predicate(&mut input)?;
        if !input.is_empty() {
            return Err(SemanticDecodeError::Noncanonical);
        }
        let value = Self::new(predicate);
        (value.canonical_bytes() == bytes)
            .then_some(value)
            .ok_or(SemanticDecodeError::Noncanonical)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum StructuralOperator {
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EncodedResultFamilyOwner {
    Callable(DeclarationRef),
    Structural(StructuralOperator),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedResultAlternative {
    pub label: String,
    pub predicate: IntegerPredicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedResultFamily {
    pub owner: EncodedResultFamilyOwner,
    pub alternatives: Vec<EncodedResultAlternative>,
}

impl EncodedResultFamily {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        match &self.owner {
            EncodedResultFamilyOwner::Callable(reference) => {
                output.push(0);
                reference.encode_identity(&mut output);
            }
            EncodedResultFamilyOwner::Structural(operator) => {
                output.push(1);
                output.push(*operator as u8);
            }
        }
        write_uleb(self.alternatives.len(), &mut output);
        for alternative in &self.alternatives {
            write_string(&alternative.label, &mut output);
            alternative.predicate.encode(&mut output);
        }
        output
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SemanticDecodeError> {
        let mut input = bytes;
        let owner = match take_byte(&mut input)? {
            0 => EncodedResultFamilyOwner::Callable(
                DeclarationRef::decode_identity(&mut input)
                    .ok_or(SemanticDecodeError::Noncanonical)?,
            ),
            1 => EncodedResultFamilyOwner::Structural(match take_byte(&mut input)? {
                0 => StructuralOperator::Equal,
                1 => StructuralOperator::Less,
                2 => StructuralOperator::LessOrEqual,
                3 => StructuralOperator::Greater,
                4 => StructuralOperator::GreaterOrEqual,
                _ => return Err(SemanticDecodeError::InvalidTag),
            }),
            _ => return Err(SemanticDecodeError::InvalidTag),
        };
        let count = read_uleb(&mut input)?;
        let mut alternatives = Vec::with_capacity(count);
        for _ in 0..count {
            alternatives.push(EncodedResultAlternative {
                label: read_string(&mut input)?,
                predicate: decode_predicate(&mut input)?,
            });
        }
        if !input.is_empty() {
            return Err(SemanticDecodeError::Noncanonical);
        }
        let value = Self {
            owner,
            alternatives,
        };
        (value.canonical_bytes() == bytes)
            .then_some(value)
            .ok_or(SemanticDecodeError::Noncanonical)
    }
}

fn encode_unary(tag: u8, value: &IntegerExpr, output: &mut Vec<u8>) {
    output.push(tag);
    value.encode(output);
}

fn encode_sequence(tag: u8, values: &[IntegerExpr], output: &mut Vec<u8>) {
    output.push(tag);
    let mut encoded: Vec<_> = values
        .iter()
        .map(|value| {
            let mut bytes = Vec::new();
            value.encode(&mut bytes);
            bytes
        })
        .collect();
    encoded.sort();
    encoded.dedup();
    write_uleb(encoded.len(), output);
    for value in encoded {
        output.extend_from_slice(&value);
    }
}

fn encode_binary_predicate(tag: u8, left: &IntegerExpr, right: &IntegerExpr, output: &mut Vec<u8>) {
    output.push(tag);
    left.encode(output);
    right.encode(output);
}

fn encode_predicate_set(tag: u8, values: &[IntegerPredicate], output: &mut Vec<u8>) {
    output.push(tag);
    let mut encoded: Vec<_> = values
        .iter()
        .map(IntegerPredicate::canonical_bytes)
        .collect();
    encoded.sort();
    encoded.dedup();
    write_uleb(encoded.len(), output);
    for value in encoded {
        output.extend_from_slice(&value);
    }
}

fn write_string(value: &str, output: &mut Vec<u8>) {
    write_uleb(value.len(), output);
    output.extend_from_slice(value.as_bytes());
}

fn write_uleb(mut value: usize, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn constant_i128(expression: &IntegerExpr) -> Option<i128> {
    match expression {
        IntegerExpr::Constant(value) => value.to_i128(),
        _ => None,
    }
}

fn predicate_hull(predicate: &IntegerPredicate) -> Option<(i128, i128)> {
    match predicate {
        IntegerPredicate::InclusiveRange {
            minimum, maximum, ..
        } => Some((constant_i128(minimum)?, constant_i128(maximum)?)),
        IntegerPredicate::And(predicates) => {
            let mut minimum = i128::MIN;
            let mut maximum = i128::MAX;
            let mut constrained = false;
            for predicate in predicates {
                if let Some((lower, upper)) = predicate_hull(predicate) {
                    minimum = minimum.max(lower);
                    maximum = maximum.min(upper);
                    constrained = true;
                }
            }
            (constrained && minimum <= maximum).then_some((minimum, maximum))
        }
        IntegerPredicate::GreaterOrEqual(_, bound) => Some((constant_i128(bound)?, i128::MAX)),
        IntegerPredicate::Greater(_, bound) => {
            Some((constant_i128(bound)?.checked_add(1)?, i128::MAX))
        }
        IntegerPredicate::LessOrEqual(_, bound) => Some((i128::MIN, constant_i128(bound)?)),
        IntegerPredicate::Less(_, bound) => {
            Some((i128::MIN, constant_i128(bound)?.checked_sub(1)?))
        }
        _ => None,
    }
}

fn decode_signed(input: &mut &[u8]) -> Result<SignedInteger, SemanticDecodeError> {
    match take_byte(input)? {
        0 => Ok(SignedInteger::from_i128(0)),
        sign @ (1 | 2) => {
            let length = read_uleb(input)?;
            if length == 0 || input.len() < length {
                return Err(SemanticDecodeError::Noncanonical);
            }
            let (magnitude, rest) = input.split_at(length);
            *input = rest;
            SignedInteger::from_sign_and_magnitude(sign == 2, magnitude.to_vec())
                .ok_or(SemanticDecodeError::Noncanonical)
        }
        _ => Err(SemanticDecodeError::InvalidTag),
    }
}

fn decode_expr(input: &mut &[u8]) -> Result<IntegerExpr, SemanticDecodeError> {
    Ok(match take_byte(input)? {
        0 => IntegerExpr::Constant(decode_signed(input)?),
        1 => IntegerExpr::Parameter(read_string(input)?),
        2 => IntegerExpr::Declaration(
            DeclarationRef::decode_identity(input).ok_or(SemanticDecodeError::Noncanonical)?,
        ),
        3 => IntegerExpr::Size(Box::new(decode_expr(input)?)),
        4 => IntegerExpr::Alignment(Box::new(decode_expr(input)?)),
        5 => IntegerExpr::IndexBits(Box::new(decode_expr(input)?)),
        6 => IntegerExpr::Negate(Box::new(decode_expr(input)?)),
        7 => IntegerExpr::Add(decode_expr_sequence(input)?),
        8 => IntegerExpr::MultiplyConstant(decode_signed(input)?, Box::new(decode_expr(input)?)),
        9 => IntegerExpr::PowerOfTwo(Box::new(decode_expr(input)?)),
        10 => IntegerExpr::NonnegativeProduct(decode_expr_sequence(input)?),
        _ => return Err(SemanticDecodeError::InvalidTag),
    })
}

fn decode_expr_sequence(input: &mut &[u8]) -> Result<Vec<IntegerExpr>, SemanticDecodeError> {
    let count = read_uleb(input)?;
    (0..count).map(|_| decode_expr(input)).collect()
}

fn decode_predicate(input: &mut &[u8]) -> Result<IntegerPredicate, SemanticDecodeError> {
    let tag = take_byte(input)?;
    if tag <= 5 {
        let left = decode_expr(input)?;
        let right = decode_expr(input)?;
        return Ok(match tag {
            0 => IntegerPredicate::Equal(left, right),
            1 => IntegerPredicate::NotEqual(left, right),
            2 => IntegerPredicate::Less(left, right),
            3 => IntegerPredicate::LessOrEqual(left, right),
            4 => IntegerPredicate::Greater(left, right),
            5 => IntegerPredicate::GreaterOrEqual(left, right),
            _ => unreachable!(),
        });
    }
    Ok(match tag {
        6 => IntegerPredicate::InclusiveRange {
            value: decode_expr(input)?,
            minimum: decode_expr(input)?,
            maximum: decode_expr(input)?,
        },
        7 | 8 => {
            let count = read_uleb(input)?;
            let values: Vec<_> = (0..count)
                .map(|_| decode_predicate(input))
                .collect::<Result<_, _>>()?;
            if tag == 7 {
                IntegerPredicate::And(values)
            } else {
                IntegerPredicate::Or(values)
            }
        }
        9 => IntegerPredicate::Not(Box::new(decode_predicate(input)?)),
        10 => IntegerPredicate::IsPowerOfTwo(decode_expr(input)?),
        _ => return Err(SemanticDecodeError::InvalidTag),
    })
}

fn take_byte(input: &mut &[u8]) -> Result<u8, SemanticDecodeError> {
    let (byte, rest) = input.split_first().ok_or(SemanticDecodeError::Truncated)?;
    *input = rest;
    Ok(*byte)
}

fn read_string(input: &mut &[u8]) -> Result<String, SemanticDecodeError> {
    let length = read_uleb(input)?;
    if input.len() < length {
        return Err(SemanticDecodeError::Truncated);
    }
    let (value, rest) = input.split_at(length);
    *input = rest;
    String::from_utf8(value.to_vec()).map_err(|_| SemanticDecodeError::Noncanonical)
}

fn read_uleb(input: &mut &[u8]) -> Result<usize, SemanticDecodeError> {
    let mut value = 0usize;
    let mut shift = 0;
    loop {
        let byte = take_byte(input)?;
        let payload = usize::from(byte & 0x7f);
        if shift >= usize::BITS || payload.checked_shl(shift).is_none() {
            return Err(SemanticDecodeError::Noncanonical);
        }
        value |= payload << shift;
        if byte & 0x80 == 0 {
            if shift > 0 && payload == 0 {
                return Err(SemanticDecodeError::Noncanonical);
            }
            return Ok(value);
        }
        shift += 7;
    }
}
