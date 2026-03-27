use crossbeam_channel::unbounded;
use ginc::{
    Db, File, FileAst, Symptom,
    parse::parse::parse,
    compilation::compile::compile,
};
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
        // Trigger compilation - this runs parse, type check, flow analysis, and codegen
        // All diagnostics are emitted as symptoms and collected here
        let _ = compile(&self.db, file);

        // Collect all accumulated symptoms from the compilation pipeline
        compile::accumulated::<Symptom>(&self.db, file)
    }

    pub fn hover_at(&self, file: File, byte_pos: usize) -> Option<String> {
        ginc::compilation::hover::hover_at(&self.db, file, byte_pos)
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
