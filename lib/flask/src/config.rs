use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::BufReader};

// Settled on flask.json since we now own flasks.io
pub const PACKAGE_CONFIG_NAME: &str = "flask.json";

#[derive(Serialize, Deserialize, Debug)]
pub struct Feature {}

#[derive(Debug, Serialize, Deserialize)]
pub struct DependencyCommon {
    #[serde(default)]
    pub features: Vec<Feature>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencyKind {
    Version { version: String },
    Path { path: String },
    Git { url: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dependency {
    #[serde(flatten)]
    pub kind: DependencyKind,

    #[serde(flatten)]
    pub common: DependencyCommon,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Author(String); // TODO: author struct

#[derive(Serialize, Deserialize, Debug)]
pub struct FlaskConfig {
    name: String,
    description: Option<String>,
    version: String, // TODO: version struct
    authors: Vec<Author>,
    targets: Option<Vec<String>>,
    // TODO: replace HashMap with BTreeMap?
    dependencies: HashMap<String, Dependency>,
}

impl FlaskConfig {
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
                            Err(_) => todo!(),
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
