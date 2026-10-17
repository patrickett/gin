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
}

fn write_string(value: &str, output: &mut Vec<u8>) {
    let length = u32::try_from(value.len()).expect("interface identity field exceeds u32");
    crate::write_uleb(length, output);
    output.extend_from_slice(value.as_bytes());
}
