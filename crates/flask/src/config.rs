use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PACKAGE_CONFIG_NAME: &str = "flask.jsonc";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Feature {}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct DependencyCommon {
    #[serde(default)]
    pub features: Vec<Feature>,
    #[serde(default)]
    pub optional: bool,

    /// TODO:
    /// Automatically insert at the top of all files.
    #[serde(default)]
    pub auto: bool,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct BugInfo {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
#[serde(untagged)]
pub enum DependencyKind {
    Version { version: String },
    Path { path: String },
    Git { url: String },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Dependency {
    #[serde(flatten)]
    pub kind: DependencyKind,

    #[serde(flatten)]
    pub common: DependencyCommon,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Author(pub String);

impl std::fmt::Display for Author {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlaskConfig {
    pub name: String,
    pub description: Option<String>,
    pub version: String,
    #[serde(default)]
    keywords: Option<Vec<String>>,
    authors: Vec<Author>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    license: Option<Vec<String>>,
    #[serde(default)]
    bugs: Option<BugInfo>,
    #[serde(default)]
    funding: Option<Vec<String>>,
    /// `"library"` or full target triple (`arch-vendor-os`).
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    dependencies: HashMap<String, Dependency>,
}

impl FlaskConfig {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            description: None,
            version,
            keywords: None,
            authors: vec![],
            repository: None,
            license: None,
            bugs: None,
            funding: None,
            target: None,
            dependencies: HashMap::new(),
        }
    }
}

impl FlaskConfig {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn authors(&self) -> &[Author] {
        &self.authors
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Qualified module name (e.g. `core.arch`) for a directory inside the package.
    ///
    /// `dir` must be a child of `root_dir`. When `dir == root_dir`, the bare
    /// package name is returned.
    pub fn qualified_name_for(&self, dir: &std::path::Path, root_dir: &std::path::Path) -> String {
        if let Ok(rel) = dir.strip_prefix(root_dir)
            && let Some(rel_str) = rel.to_str()
            && !rel_str.is_empty()
        {
            let subpath = rel_str.replace('/', ".");
            format!("{}.{subpath}", self.name)
        } else {
            self.name.clone()
        }
    }

    pub fn dependency_names(&self) -> Vec<&str> {
        self.dependencies.keys().map(|s| s.as_str()).collect()
    }

    pub fn dependencies(&self) -> &HashMap<String, Dependency> {
        &self.dependencies
    }

    /// Find the package configuration and its root directory by walking up from `dir`.
    /// Returns `Some((config, root_dir))` when a `flask.jsonc` is found.
    pub fn find_package_config(dir: &std::path::Path) -> Option<(FlaskConfig, std::path::PathBuf)> {
        let mut search = dir.to_path_buf();
        loop {
            search.push(PACKAGE_CONFIG_NAME);
            if let Ok(raw) = std::fs::read_to_string(&search)
                && let Ok(config) = json5::from_str::<FlaskConfig>(&raw)
            {
                search.pop(); // remove flask.jsonc
                return Some((config, search));
            }
            search.pop(); // remove flask.jsonc
            if !search.pop() {
                return None;
            }
        }
    }
}
