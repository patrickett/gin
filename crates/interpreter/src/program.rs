//! Loading: source text or a package directory to checked modules.

use std::path::Path;

use typecheck::{CheckedSourceModule, Limits, SourceCheckFailure, check_module};

use crate::{Tag, Value};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("syntax errors: {0:?}")]
    Syntax(Vec<String>),
    #[error("check failed: {0:?}")]
    Check(SourceCheckFailure),
    #[error("{0}")]
    Package(String),
}

/// A checked program: its modules in dependency order, the root module last.
#[derive(Debug)]
pub struct Program {
    pub(crate) modules: Vec<CheckedSourceModule>,
}

impl AsRef<[CheckedSourceModule]> for Program {
    fn as_ref(&self) -> &[CheckedSourceModule] {
        &self.modules
    }
}

impl Program {
    /// One source file, checked as the module `main`.
    pub fn from_source(text: &str) -> Result<Self, LoadError> {
        let file = parser::parse_strict(text).map_err(|diagnostics| {
            LoadError::Syntax(diagnostics.iter().map(|d| d.message.clone()).collect())
        })?;
        let module = check_module("main", &file, &[], Limits::default())
            .map_err(|error| LoadError::Check(error.into()))?;
        Ok(Self { modules: vec![module] })
    }

    /// Modules already checked, dependencies first and the root last.
    pub fn from_modules(modules: Vec<CheckedSourceModule>) -> Self {
        assert!(!modules.is_empty(), "a program has a root module");
        Self { modules }
    }

    /// A package directory (a `mod.flask` and its `.gin` files), with its path dependencies.
    pub fn from_package(root: &Path) -> Result<Self, LoadError> {
        let package =
            package::Package::open(root).map_err(|e| LoadError::Package(e.to_string()))?;
        let checked = package
            .check(package::GraphLimits::default(), Limits::default())
            .map_err(|e| LoadError::Package(e.to_string()))?;
        Ok(Self { modules: checked.capsules })
    }

    /// The root module.
    pub fn root(&self) -> &CheckedSourceModule {
        self.modules.last().expect("a program has a root module")
    }

    /// `Type.method` or `function` names the root module defines itself, in table order.
    pub fn function_names(&self) -> Vec<&str> {
        let root = self.root();
        root.checked()
            .functions
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                root.function_home(*index)
                    .is_some_and(|(home, own)| home == root.name() && own as usize == *index)
            })
            .map(|(_, function)| function.name.as_str())
            .collect()
    }

    /// `value` as source would write it, with field names.
    pub fn render(&self, value: &Value) -> String {
        match value {
            Value::Record { tag, fields } if !fields.is_empty() => {
                let names = self.field_names(tag);
                let shown: Vec<String> = fields
                    .iter()
                    .enumerate()
                    .map(|(position, field)| match names.get(position) {
                        Some(name) if names.len() == fields.len() && !name.is_empty() => {
                            format!("{name}: {}", self.render(field))
                        }
                        _ => self.render(field),
                    })
                    .collect();
                format!("{}({})", tag.short(), shown.join(", "))
            }
            other => other.to_string(),
        }
    }

    /// The field names of the type `tag` names, in order (empty when unknown).
    pub fn field_names(&self, tag: &Tag) -> Vec<String> {
        self.modules
            .iter()
            .filter(|module| module.name() == &*tag.home)
            .flat_map(|module| module.checked().aggregates.iter())
            .find(|aggregate| aggregate.name == *tag.name)
            .map(|aggregate| aggregate.fields.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default()
    }
}
