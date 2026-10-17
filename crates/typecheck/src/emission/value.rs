use crate::ResolvedProgram;
use ast::{ConstValue, Ty};
use diagnostic::Diagnostic;
use internment::Intern;
use std::borrow::Cow;
use std::collections::HashSet;

fn error(kind: &str, message: impl Into<String>) -> Diagnostic {
    super::error(format!("{kind}: {}", message.into()))
}

pub(crate) struct Fields<'a>(Vec<(Intern<String>, &'a ConstValue)>);

impl<'a> Fields<'a> {
    pub(crate) fn entries(&self) -> impl Iterator<Item = (Intern<String>, &'a ConstValue)> + '_ {
        self.0.iter().copied()
    }
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

    pub(crate) fn value(&self, name: &str) -> &'a ConstValue {
        self.0
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
}

pub(crate) fn sequence(value: &ConstValue) -> Result<Vec<&ConstValue>, Diagnostic> {
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
