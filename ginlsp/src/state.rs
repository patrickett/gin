use crossbeam_channel::unbounded;
use ginc::{
    compilation::compile::{compile, type_check_entry},
    parse::parse::parse,
    Db, File, FileAst, Symptom,
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
        // Type check first — emits unknown type/binding/variable symptoms.
        // This must run before compile so that type-level diagnostics
        // (e.g. "undeclared type `Bool`") appear before body-level errors
        // (e.g. "Unknown tag 'True'").
        type_check_entry(&self.db, file);

        // Trigger compilation — runs parse, flow analysis, and codegen.
        let _ = compile(&self.db, file);

        // Collect all accumulated symptoms from both type checking and compilation.
        let mut symptoms: Vec<&Symptom> = type_check_entry::accumulated::<Symptom>(&self.db, file);
        symptoms.extend(compile::accumulated::<Symptom>(&self.db, file));

        symptoms
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

    /// Upsert a file into the database.
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
