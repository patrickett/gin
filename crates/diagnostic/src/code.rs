use strum::AsRefStr;

use crate::{LexSymptom, ParseSymptom, TypeSymptom, IoSymptom, ImportSymptom, CodegenSymptom};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum DiagnosticCode {
    Import(ImportSymptom),
    Lex(LexSymptom),
    Parse(ParseSymptom),
    Type(TypeSymptom),
    Io(IoSymptom),
    Codegen(CodegenSymptom),
}
