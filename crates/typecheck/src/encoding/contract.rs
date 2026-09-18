use std::borrow::Cow;
use std::collections::HashSet;

use crate::resolved_program::ResolvedProgram;
use crate::ty::Ty;
use ast::ConstValue;
use diagnostic::Diagnostic;
use flask::{TargetArch, TargetTriple};
use internment::Intern;

use super::allocation::{Register, RegisterSlot};
use super::error;

pub(super) struct Contract {
    pub slots: Vec<RegisterSlot>,
    pub inputs: Vec<Intern<String>>,
    pub outputs: Vec<Intern<String>>,
    pub clobbers: Vec<Intern<String>>,
    pub reads: Vec<(usize, usize)>,
    pub writes: Vec<(usize, usize)>,
}

impl Contract {
    pub fn from_value(
        value: &ConstValue,
        target: &str,
        program: ResolvedProgram<'_>,
    ) -> Result<Self, Diagnostic> {
        let fields = Fields::new(
            value,
            program,
            &[
                "target", "choices", "inputs", "outputs", "clobbers", "reads", "writes",
            ],
        )?;
        if fields.text("target")? != target {
            return Err(error(
                "target",
                "encoding contract does not match the active target",
            ));
        }
        if !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
        {
            return Err(error("target", "invalid encoding target triple"));
        }
        let mut slots: Vec<RegisterSlot> = Vec::new();
        for value in fields.sequence("choices")? {
            let choice = Fields::new(value, program, &["slot", "register", "code"])?;
            let slot = Intern::from_ref(choice.text("slot")?.as_ref());
            let name: Intern<String> = Intern::from_ref(choice.text("register")?.as_ref());
            let code = choice.integer("code")?;
            if Self::register_code(target, name.as_str()).map(i128::from) != Some(code) {
                return Err(error(
                    "register",
                    format!("unsupported register or incorrect encoding code for `{name}`"),
                ));
            }
            let register = Register {
                name,
                code: code as u8,
            };
            if let Some(existing) = slots.iter_mut().find(|existing| existing.name == slot) {
                existing.candidates.push(register);
            } else {
                slots.push(RegisterSlot {
                    name: slot,
                    candidates: vec![register],
                });
            }
        }
        let inputs = fields.names("inputs")?;
        let outputs = fields.names("outputs")?;
        let clobbers = fields.names("clobbers")?;
        for name in inputs.iter().chain(&outputs) {
            if !slots.iter().any(|slot| slot.name == *name) {
                return Err(error(
                    "operand",
                    format!("contract references unknown register slot `{name}`"),
                ));
            }
        }
        for name in &clobbers {
            if name.as_str() != "cc" && Self::register_code(target, name.as_str()).is_none() {
                return Err(error(
                    "register",
                    format!("unsupported clobbered register `{name}`"),
                ));
            }
        }
        let reads = fields.accesses("reads", inputs.len(), program)?;
        let writes = fields.accesses("writes", inputs.len(), program)?;
        Ok(Self {
            slots,
            inputs,
            outputs,
            clobbers,
            reads,
            writes,
        })
    }

    fn register_code(target: &str, name: &str) -> Option<u8> {
        let target = TargetTriple::parse(target).ok()?;
        if target.arch == TargetArch::Arm64 {
            let code = name.strip_prefix('x')?.parse::<u8>().ok()?;
            return (code < 16 && name == format!("x{code}")).then_some(code);
        }
        if target.arch != TargetArch::X86_64 {
            return None;
        }
        match name {
            "rax" => Some(0),
            "rcx" => Some(1),
            "rdx" => Some(2),
            "rsi" => Some(6),
            "rdi" => Some(7),
            "r8" => Some(8),
            "r9" => Some(9),
            "r10" => Some(10),
            "r11" => Some(11),
            _ => None,
        }
    }
}

pub(crate) struct Fields<'a>(Vec<(Intern<String>, &'a ConstValue)>);

impl<'a> Fields<'a> {
    pub(crate) fn new(
        value: &'a ConstValue,
        program: ResolvedProgram<'_>,
        expected: &[&str],
    ) -> Result<Self, Diagnostic> {
        Self::read(value, program)?.require(expected)
    }

    pub(crate) fn read(
        value: &'a ConstValue,
        program: ResolvedProgram<'_>,
    ) -> Result<Self, Diagnostic> {
        let fields: Vec<_> = match value {
            ConstValue::Record { fields } => {
                fields.iter().map(|(name, value)| (*name, value)).collect()
            }
            ConstValue::Tag {
                qual_path: Some(path),
                args,
                name,
            } => {
                let ty = program
                    .tag_types
                    .get(&program.tag_id_for_value(value).ok_or_else(|| {
                        error(
                            "contract",
                            format!("unknown encoding record declaration `{path}`"),
                        )
                    })?)
                    .ok_or_else(|| {
                        error(
                            "contract",
                            format!("unknown encoding record declaration `{path}`"),
                        )
                    })?;
                let fields = match program.type_registry.resolved_definition_for_type(ty) {
                    Ty::Record { fields, .. } => fields,
                    Ty::Union { variants, .. } => variants
                        .iter()
                        .find(|variant| variant.name == *name)
                        .map(|variant| variant.fields.clone())
                        .ok_or_else(|| error("contract", "unknown encoding variant"))?,
                    _ => return Err(error("contract", "encoding components must be Gin records")),
                };
                if fields.len() != args.len() {
                    return Err(error(
                        "contract",
                        "encoding record has an incorrect field count",
                    ));
                }
                fields
                    .iter()
                    .zip(args.iter())
                    .map(|((name, _), value)| (*name, value))
                    .collect()
            }
            _ => {
                return Err(error(
                    "contract",
                    "encoding contract components must be Gin records",
                ));
            }
        };
        Ok(Self(fields))
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(field, _)| field.as_str() == name)
    }

    pub(crate) fn require(self, expected: &[&str]) -> Result<Self, Diagnostic> {
        let mut seen = HashSet::new();
        for (name, _) in self.0.iter() {
            if !expected.contains(&name.as_str()) || !seen.insert(name.as_str()) {
                return Err(error(
                    "contract",
                    format!("unexpected or duplicate contract field `{name}`"),
                ));
            }
        }
        for name in expected {
            if !seen.contains(name) {
                return Err(error(
                    "contract",
                    format!("missing contract field `{name}`"),
                ));
            }
        }
        Ok(self)
    }

    pub(crate) fn value(&self, name: &str) -> &ConstValue {
        &self
            .0
            .iter()
            .find(|(field, _)| field.as_str() == name)
            .expect("validated contract field")
            .1
    }

    pub(crate) fn text(&self, name: &str) -> Result<Cow<'_, str>, Diagnostic> {
        self.value(name)
            .as_utf8()
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                error(
                    "contract",
                    format!("`{name}` must be nonempty compile-time UTF-8 bytes"),
                )
            })
    }

    pub(crate) fn integer(&self, name: &str) -> Result<i128, Diagnostic> {
        self.value(name)
            .as_const_size_int()
            .ok_or_else(|| error("contract", format!("`{name}` must be an integer")))
    }

    pub(crate) fn sequence(&self, name: &str) -> Result<Vec<&ConstValue>, Diagnostic> {
        sequence(self.value(name))
    }

    fn names(&self, name: &str) -> Result<Vec<Intern<String>>, Diagnostic> {
        let mut names = Vec::new();
        for value in self.sequence(name)? {
            let Some(text) = value.as_utf8() else {
                return Err(error("contract", format!("`{name}` must contain names")));
            };
            if text.is_empty() || names.contains(&Intern::from_ref(text.as_ref())) {
                return Err(error(
                    "contract",
                    format!("`{name}` contains an empty or duplicate name"),
                ));
            }
            names.push(Intern::from_ref(text.as_ref()));
        }
        Ok(names)
    }

    fn accesses(
        &self,
        name: &str,
        inputs: usize,
        program: ResolvedProgram<'_>,
    ) -> Result<Vec<(usize, usize)>, Diagnostic> {
        self.sequence(name)?
            .iter()
            .map(|value| {
                let fields = Fields::new(value, program, &["pointer", "length"])?;
                let pointer = fields.integer("pointer")?;
                let length = fields.integer("length")?;
                if pointer < 0
                    || length < 0
                    || pointer >= inputs as i128
                    || length >= inputs as i128
                    || pointer == length
                {
                    return Err(error(
                        "memory",
                        "memory effect indices must refer to distinct existing inputs",
                    ));
                }
                Ok((pointer as usize, length as usize))
            })
            .collect()
    }
}

pub(super) fn sequence(value: &ConstValue) -> Result<Vec<&ConstValue>, Diagnostic> {
    match value {
        ConstValue::FixedArray(items)
        | ConstValue::Sequence(items)
        | ConstValue::ResultRow(items) => Ok(items.iter().collect()),
        ConstValue::Record { fields } => Ok(fields.iter().map(|(_, value)| value).collect()),
        _ => Err(error(
            "sequence",
            "instruction encoding requires a compile-time sequence",
        )),
    }
}
