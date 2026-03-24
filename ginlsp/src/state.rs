use crossbeam_channel::unbounded;
use ginc::{Db, File, FileAst, Symptom, parse::parse::parse, typeck::FlowAnalysis, compilation::compile::flow_analysis};
use salsa::Setter;

pub struct DocumentState {
    pub source: String,
    pub file: File,
}

pub struct JsonDocumentState {
    pub source: String,
}

#[derive(Clone)]
pub struct GinSnapshot {
    pub db: ginc::InputDatabase,
}

impl GinSnapshot {
    pub fn parse(&self, file: File) -> FileAst {
        parse(&self.db, file)
    }

    pub fn diagnostics(&self, file: File) -> Vec<&Symptom> {
        parse::accumulated::<Symptom>(&self.db, file)
    }

    pub fn flow_analysis(&self, file: File) -> FlowAnalysis {
        flow_analysis(&self.db, file)
    }
}

pub struct GinHost {
    pub db: ginc::InputDatabase,
}

impl GinHost {
    pub fn new() -> Self {
        let (tx, _rx) = unbounded();
        Self {
            db: ginc::InputDatabase::new(tx),
        }
    }

    pub fn snapshot(&self) -> GinSnapshot {
        GinSnapshot {
            db: self.db.clone(),
        }
    }

    /// thing
    pub fn upsert_file(&mut self, path: std::path::PathBuf, contents: String) -> Option<File> {
        match self.db.input(path) {
            Ok(file) => {
                file.set_contents(&mut self.db).to(contents);
                Some(file)
            }
            Err(_) => None,
        }
    }
}
