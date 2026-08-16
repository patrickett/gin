use diagnostic::Diagnostic;

#[derive(Debug, Clone)]
pub struct InterpreterDiagnostic {
    source: String,
    display_path: String,
    diagnostic: Diagnostic,
}

impl InterpreterDiagnostic {
    pub fn new(
        source: impl Into<String>,
        display_path: impl Into<String>,
        diagnostic: Diagnostic,
    ) -> Self {
        Self {
            source: source.into(),
            display_path: display_path.into(),
            diagnostic,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn display_path(&self) -> &str {
        &self.display_path
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    pub fn print(&self) {
        self.diagnostic.print(&self.source, &self.display_path);
    }
}
