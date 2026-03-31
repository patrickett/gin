use crossbeam_channel::unbounded;
use ginc::{
    analysis::analyze_file, database::input_database::InputDatabase,
    lsp::hover, parse::query::parse, Db, File, FileAst, Symptom,
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
    pub db: InputDatabase,
}

impl GinSnapshot {
    pub fn parse(&self, file: File) -> FileAst {
        parse(&self.db, file)
    }

    pub fn diagnostics(&self, file: File) -> Vec<&Symptom> {
        // Analyze the file with a single-file package context.
        // This runs type checking and flow analysis, accumulating all symptoms.
        let all_files = vec![file];
        let _ = analyze_file(&self.db, file, all_files.clone());

        // Collect accumulated symptoms.
        analyze_file::accumulated::<Symptom>(&self.db, file, all_files)
    }

    pub fn hover_at(&self, file: File, byte_pos: usize) -> Option<String> {
        hover::hover_at(&self.db, file, byte_pos)
    }

    pub fn dot_type_at(&self, file: File, byte_pos: usize) -> Option<ginc::Ty> {
        hover::dot_type_at(&self.db, file, byte_pos)
    }
}

pub struct GinHost {
    pub db: InputDatabase,
}

impl GinHost {
    pub fn new() -> Self {
        let (tx, _rx) = unbounded();
        Self {
            db: InputDatabase::new(tx),
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
