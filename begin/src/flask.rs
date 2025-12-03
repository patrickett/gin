// TODO: rename package name
// `bottle.json`
// `begin.json`
// `module.json`
// `tonic.json`
// `rack.json`
// `sauce.json`
// `vial.json`
// `flask.json`
// `still.json` might be the most cute since stills produce gin
const PACKAGE_CONFIG_NAME: &str = "flask.json";

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::BufReader};

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
    made_in: String,
    targets: Option<Vec<String>>,
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
                    let path_display = path.display();
                    eprintln!(
                        "error: could not find `{PACKAGE_CONFIG_NAME}` in `{path_display}`" // "error: could not find `{PACKAGE_CONFIG_NAME}` in `{path}` or any parent directory"
                    )
                }
                err => eprintln!("{err:#?}"),
            },
        };

        None
    }
}

// error
// alert

// warn (diagnostic recomendation)
// info (debug)
// flaw (error)

// problem

// issue (error)
// debug (debug info)
// fault (error)
// alert (warning)
