#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageInstanceId {
    pub package: String,
    pub version: String,
    pub source: String,
    pub instance: String,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DeclarationRef {
    Subject {
        module_path: String,
        declaration_path: String,
    },
    External {
        package: PackageInstanceId,
        module_path: String,
        declaration_path: String,
    },
}

impl DeclarationRef {
    pub(crate) fn encode_identity(&self, output: &mut Vec<u8>) {
        match self {
            Self::Subject {
                module_path,
                declaration_path,
            } => {
                output.push(0);
                write_string(module_path, output);
                write_string(declaration_path, output);
            }
            Self::External {
                package,
                module_path,
                declaration_path,
            } => {
                output.push(1);
                write_string(&package.package, output);
                write_string(&package.version, output);
                write_string(&package.source, output);
                write_string(&package.instance, output);
                write_string(module_path, output);
                write_string(declaration_path, output);
            }
        }
    }

    pub(crate) fn decode_identity(input: &mut &[u8]) -> Option<Self> {
        match take_byte(input)? {
            0 => Some(Self::Subject {
                module_path: read_string(input)?,
                declaration_path: read_string(input)?,
            }),
            1 => Some(Self::External {
                package: PackageInstanceId {
                    package: read_string(input)?,
                    version: read_string(input)?,
                    source: read_string(input)?,
                    instance: read_string(input)?,
                },
                module_path: read_string(input)?,
                declaration_path: read_string(input)?,
            }),
            _ => None,
        }
    }
}

fn write_string(value: &str, output: &mut Vec<u8>) {
    let length = u32::try_from(value.len()).expect("interface identity field exceeds u32");
    write_uleb(length, output);
    output.extend_from_slice(value.as_bytes());
}

fn write_uleb(mut value: u32, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn take_byte(input: &mut &[u8]) -> Option<u8> {
    let (byte, rest) = input.split_first()?;
    *input = rest;
    Some(*byte)
}

fn read_string(input: &mut &[u8]) -> Option<String> {
    let length = read_uleb(input)? as usize;
    let (value, rest) = input.split_at_checked(length)?;
    *input = rest;
    String::from_utf8(value.to_vec()).ok()
}

fn read_uleb(input: &mut &[u8]) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0;
    loop {
        let byte = take_byte(input)?;
        let payload = u32::from(byte & 0x7f);
        if shift >= 32 || payload.checked_shl(shift)? >> shift != payload {
            return None;
        }
        value |= payload << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
    }
}
