use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::BufReader};

// Settled on flask.json since we now own flasks.io
pub const PACKAGE_CONFIG_NAME: &str = "flask.json";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Feature {}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct DependencyCommon {
    #[serde(default)]
    pub features: Vec<Feature>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct BugInfo {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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

impl Author {
    pub fn new(name: String) -> Self {
        Self(name)
    }
}

impl std::fmt::Display for Author {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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
    targets: Option<Vec<String>>,
    #[serde(default)]
    entry: Option<String>,
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
            targets: None,
            entry: None,
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

    pub fn keywords(&self) -> Option<&[String]> {
        self.keywords.as_deref()
    }

    pub fn authors(&self) -> &[Author] {
        &self.authors
    }

    pub fn repository(&self) -> Option<&str> {
        self.repository.as_deref()
    }

    pub fn license(&self) -> Option<&[String]> {
        self.license.as_deref()
    }

    pub fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn set_version(&mut self, version: String) {
        self.version = version;
    }

    pub fn bugs(&self) -> Option<&BugInfo> {
        self.bugs.as_ref()
    }

    pub fn funding(&self) -> Option<&[String]> {
        self.funding.as_deref()
    }

    pub fn dependency_names(&self) -> Vec<&str> {
        self.dependencies.keys().map(|s| s.as_str()).collect()
    }

    pub fn dependencies(&self) -> &HashMap<String, Dependency> {
        &self.dependencies
    }

    pub fn from_directory(dir: &std::path::Path) -> Option<FlaskConfig> {
        let mut search = dir.to_path_buf();
        loop {
            search.push(PACKAGE_CONFIG_NAME);
            if let Ok(file) = std::fs::File::open(&search) {
                let reader = BufReader::new(file);
                if let Ok(config) = serde_json::from_reader::<_, FlaskConfig>(reader) {
                    return Some(config);
                }
            }
            search.pop(); // remove flask.json
            if !search.pop() {
                return None;
            }
        }
    }

    pub fn from_current_directory() -> Option<FlaskConfig> {
        let mut path = std::env::current_dir().expect("able to get current_dir");
        path.push(PACKAGE_CONFIG_NAME);

        #[cfg(debug_assertions)]
        println!("info: config_path ({path:#?})");

        match std::fs::File::open(&path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                let config: FlaskConfig =
                    serde_json::from_reader(reader).expect("if we have the json expect to read it");
                return Some(config);
            }
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => {
                    path.pop();
                    let original_dir = path.clone();
                    let mut found_path = None;

                    while path.pop() {
                        path.push(PACKAGE_CONFIG_NAME);
                        match std::fs::exists(&path) {
                            Ok(_) => {
                                found_path = Some(path.clone());
                                break;
                            }
                            Err(_) => {
                                path.pop();
                            }
                        }
                    }

                    match found_path {
                        Some(found) => match std::fs::File::open(found) {
                            Ok(file) => {
                                let reader = BufReader::new(file);
                                let config: FlaskConfig = serde_json::from_reader(reader)
                                    .expect("if we have the json expect to read it");
                                return Some(config);
                            }
                            Err(_) => eprintln!(
                                "error: could not find `{PACKAGE_CONFIG_NAME}` in `{}` or any parent directory",
                                original_dir.display()
                            ),
                        },
                        None => {
                            eprintln!(
                                "error: could not find `{PACKAGE_CONFIG_NAME}` in `{}` or any parent directory",
                                original_dir.display()
                            )
                        }
                    }
                }
                err => eprintln!("{err:#?}"),
            },
        };

        None
    }
}
