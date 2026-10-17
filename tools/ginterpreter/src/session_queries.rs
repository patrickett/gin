use analysis::{HoverContent, ScopeSymbol, SemanticQueries, SymbolKind};
use ast::prelude::Intern;
use ginc::driver::{CheckedPackage, CompilerDriver};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use typecheck::DefId;

use super::{
    InterpreterSession, QueryOutcome, ScopeEntry, Submission, has_flaws, package_diagnostics,
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchMatch {
    pub name: String,
    pub kind: &'static str,
    pub signature: String,
    pub documentation: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InterpreterCompletion {
    pub label: String,
    pub kind: &'static str,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: Option<String>,
    pub is_snippet: bool,
}

pub(super) fn is_query_command(input: &str) -> bool {
    let name = input
        .trim_start()
        .strip_prefix('#')
        .and_then(|command| command.split_whitespace().next());
    matches!(
        name,
        Some(
            "type"
                | "inspect"
                | "scope"
                | "doc"
                | "search"
                | "complete"
                | "queries"
                | "suggest"
                | "frames"
                | "frame"
                | "tasks"
                | "help"
        )
    )
}

pub(super) struct AcceptedImage {
    checked: CheckedPackage,
}

struct PackageSymbol {
    symbol: ScopeSymbol,
    location: String,
    documentation: String,
}

impl AcceptedImage {
    pub(super) fn new(checked: CheckedPackage) -> Self {
        Self { checked }
    }

    fn queries(&self) -> SemanticQueries<'_> {
        let file = self.checked.primary_file();
        SemanticQueries::new(
            file.source(),
            &file.parsed().output.ast,
            file.typed(),
            self.checked.package_index(),
        )
    }

    fn package_symbols(&self, root: &Path) -> Vec<PackageSymbol> {
        let mut symbols = Vec::new();
        for file in self.checked.files() {
            if file.path() == self.checked.primary_path() || file.path().parent() != Some(root) {
                continue;
            }
            let queries = SemanticQueries::new(
                file.source(),
                &file.parsed().output.ast,
                file.typed(),
                self.checked.package_index(),
            );
            for symbol in queries.scope() {
                if matches!(symbol.kind, SymbolKind::Imported | SymbolKind::Module) {
                    continue;
                }
                let (line, column) =
                    diagnostic::source_position(file.source(), symbol.span.start());
                let location = format!("{}:{line}:{column}", file.path().display());
                let documentation = queries
                    .hover(symbol.span.start())
                    .map(|hover| hover.markdown)
                    .unwrap_or_else(|| symbol_documentation(&symbol));
                symbols.push(PackageSymbol {
                    symbol,
                    documentation: format!("{documentation}\n\nSource: {location}"),
                    location,
                });
            }
        }
        symbols
    }
}

impl InterpreterSession {
    pub fn search(&self, text: &str) -> Vec<SearchMatch> {
        let filter = crate::catalog::normalize_filter(text);
        if filter.is_empty() {
            return Vec::new();
        }
        let symbols = self.symbols();
        let session_names: HashSet<_> = symbols.iter().map(|symbol| symbol.name.clone()).collect();
        let mut matches: Vec<_> = symbols
            .into_iter()
            .map(|symbol| {
                let documentation = self
                    .hover_at(symbol.span.start())
                    .map(|hover| hover.markdown);
                let mut entry = symbol_search_match(symbol, None);
                entry.documentation = documentation.or(entry.documentation);
                entry
            })
            .chain(
                self.package_symbols()
                    .into_iter()
                    .filter(|entry| !session_names.contains(&entry.symbol.name))
                    .map(|entry| symbol_search_match(entry.symbol, Some(entry.location))),
            )
            .filter(|entry| search_match_contains(entry, &filter))
            .collect();
        matches.extend(
            crate::language_recipes()
                .iter()
                .filter(|recipe| recipe.matches(&filter))
                .map(recipe_search_match),
        );
        matches.extend(
            crate::catalog::capabilities()
                .iter()
                .filter(|entry| entry.matches(&filter))
                .map(|entry| SearchMatch {
                    name: format!("#{}", entry.name),
                    kind: capability_kind(entry.kind),
                    signature: entry.syntax.to_owned(),
                    documentation: Some(format!("{}\n{}", entry.summary, entry.prerequisites)),
                    location: None,
                }),
        );
        matches.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.signature.cmp(&right.signature))
        });
        matches
    }

    pub fn completions(&mut self, input: &str) -> Vec<InterpreterCompletion> {
        if let Some(completions) = catalog_completions(input) {
            return completions;
        }
        let source = self.source.candidate(input);
        let cursor = source.len() - 1;
        let checked = self.check(&source);
        let file = checked.primary_file();
        let queries = SemanticQueries::new(
            file.source(),
            &file.parsed().output.ast,
            file.typed(),
            checked.package_index(),
        );
        let prefix = completion_prefix(input);
        let mut completions: Vec<_> = queries
            .completions(cursor)
            .into_iter()
            .filter(|entry| entry.label.starts_with(prefix) && !entry.label.starts_with("__gin_"))
            .map(InterpreterCompletion::from)
            .collect();
        if input.trim().is_empty() {
            completions.extend(crate::language_recipes().iter().map(recipe_completion));
        }
        let prefix_start = input.len() - prefix.len();
        if !input[..prefix_start].ends_with(['.', '#']) {
            let session_names: HashSet<_> =
                self.symbols().into_iter().map(|entry| entry.name).collect();
            completions.extend(
                self.package_symbols()
                    .into_iter()
                    .filter(|entry| {
                        entry.symbol.name.starts_with(prefix)
                            && !session_names.contains(&entry.symbol.name)
                    })
                    .map(|entry| InterpreterCompletion {
                        label: entry.symbol.name,
                        kind: symbol_kind(entry.symbol.kind),
                        detail: Some(entry.symbol.signature),
                        documentation: Some(entry.documentation),
                        insert_text: None,
                        is_snippet: false,
                    }),
            );
        }
        completions.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then(left.detail.cmp(&right.detail))
        });
        completions
            .dedup_by(|left, right| left.label == right.label && left.detail == right.detail);
        completions
    }

    pub(super) fn search_report(&self, text: Option<&str>) -> Submission {
        let text = text.unwrap_or_default().trim();
        let text = if text.is_empty() {
            "usage: #search <text>".to_owned()
        } else {
            let entries = self.search(text);
            if entries.is_empty() {
                "no matching symbols or capabilities".to_owned()
            } else {
                entries
                    .iter()
                    .map(|entry| {
                        let mut line =
                            format!("{} [{}]: {}", entry.name, entry.kind, entry.signature);
                        if let Some(documentation) = &entry.documentation {
                            line.push_str(&format!("\n{documentation}"));
                        }
                        if let Some(location) = &entry.location {
                            line.push_str(&format!("\nSource: {location}"));
                        }
                        line
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            }
        };
        Submission::Answered { text }
    }

    pub(super) fn completions_report(&mut self, source: Option<&str>) -> Submission {
        let Some(source) = source.filter(|source| !source.is_empty()) else {
            return Submission::Answered {
                text: "usage: #complete <source>".to_owned(),
            };
        };
        let entries = self.completions(source);
        let text = if entries.is_empty() {
            "no completions".to_owned()
        } else {
            entries
                .iter()
                .map(|entry| match &entry.detail {
                    Some(detail) => format!("{} [{}]: {detail}", entry.label, entry.kind),
                    None => format!("{} [{}]", entry.label, entry.kind),
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Submission::Answered { text }
    }

    pub fn symbols(&self) -> Vec<ScopeSymbol> {
        self.image
            .as_ref()
            .map(|image| image.queries().scope())
            .unwrap_or_default()
    }

    pub fn scope(&self) -> Vec<ScopeEntry> {
        let symbols = self.symbols();
        let session_names: HashSet<_> = symbols.iter().map(|symbol| symbol.name.clone()).collect();
        let mut entries: Vec<_> = symbols
            .into_iter()
            .map(|symbol| ScopeEntry {
                name: symbol.name,
                signature: symbol.signature,
            })
            .collect();
        entries.extend(
            self.package_symbols()
                .into_iter()
                .filter(|entry| !session_names.contains(&entry.symbol.name))
                .map(|entry| ScopeEntry {
                    name: entry.symbol.name,
                    signature: format!("{} [{}]", entry.symbol.signature, entry.location),
                }),
        );
        entries.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.signature.cmp(&right.signature))
        });
        entries
    }

    pub fn hover_at(&self, byte_offset: usize) -> Option<HoverContent> {
        let image = self.image.as_ref()?;
        image
            .queries()
            .hover_with_imports(image.checked.primary_path(), byte_offset, &|path| {
                let file = image.checked.file(path)?;
                Some(file.parsed().clone())
            })
    }

    pub fn documentation(&self, name: &str) -> Option<String> {
        let entries = self
            .symbols()
            .into_iter()
            .filter(|symbol| symbol.name == name)
            .map(|symbol| {
                self.hover_at(symbol.span.start())
                    .map(|hover| hover.markdown)
                    .unwrap_or_else(|| symbol_documentation(&symbol))
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            return Some(entries.join("\n\n"));
        }
        let entries: Vec<_> = self
            .package_symbols()
            .into_iter()
            .filter(|entry| entry.symbol.name == name)
            .map(|entry| entry.documentation)
            .collect();
        (!entries.is_empty()).then(|| entries.join("\n\n"))
    }

    fn package_symbols(&self) -> Vec<PackageSymbol> {
        if self.context.package().sources().is_empty() {
            return Vec::new();
        }
        let image = self.image.as_ref().unwrap_or_else(|| {
            self.package_image.get_or_init(|| {
                let checked = self
                    .context
                    .check_source(
                        &mut CompilerDriver::default(),
                        "",
                        self.context.package().sources().to_vec(),
                    )
                    .expect("package queries include their primary session source");
                AcceptedImage::new(checked)
            })
        });
        image.package_symbols(self.context.package().root())
    }

    pub(super) fn documentation_report(&self, arguments: Option<&str>) -> Submission {
        let name = arguments.unwrap_or_default().trim();
        let text = if name.is_empty() || name.chars().any(char::is_whitespace) {
            "usage: #doc <symbol>".to_owned()
        } else {
            self.documentation(name)
                .unwrap_or_else(|| format!("unknown symbol `{name}`; use #scope to list names"))
        };
        Submission::Answered { text }
    }

    pub fn type_of(&mut self, expression: &str) -> QueryOutcome {
        let expression = expression.trim_end();
        if expression.trim().is_empty() {
            return QueryOutcome::Empty;
        }
        let evaluation = self.source.evaluation_candidate_with_dispatches(
            expression,
            self.next_generation,
            &HashMap::new(),
        );
        let checked = self.check(evaluation.source());
        let diagnostics = package_diagnostics(&checked, Some(&evaluation));
        if checked.has_front_end_flaws() || checked.has_typecheck_flaws() || has_flaws(&diagnostics)
        {
            return QueryOutcome::Rejected(diagnostics);
        }
        let file = checked.primary_file();
        let queries = SemanticQueries::new(
            file.source(),
            &file.parsed().output.ast,
            file.typed(),
            checked.package_index(),
        );
        let definition = DefId::named(Intern::from_ref(evaluation.symbol()));
        match queries.return_type(definition) {
            Some(ty) => QueryOutcome::Answered(ty.format_for_hover()),
            None => QueryOutcome::Empty,
        }
    }
}

fn symbol_documentation(symbol: &ScopeSymbol) -> String {
    match &symbol.documentation {
        Some(documentation) => format!("{}\n\n{documentation}", symbol.signature),
        None => symbol.signature.clone(),
    }
}

fn recipe_search_match(recipe: &crate::LanguageRecipe) -> SearchMatch {
    SearchMatch {
        name: recipe.name.to_owned(),
        kind: "language_recipe",
        signature: format!("#help {}", recipe.name),
        documentation: Some(recipe.summary.to_owned()),
        location: None,
    }
}

fn recipe_completion(recipe: &crate::LanguageRecipe) -> InterpreterCompletion {
    InterpreterCompletion {
        label: recipe.name.to_owned(),
        kind: "language_recipe",
        detail: Some(format!("#help {}: {}", recipe.name, recipe.title)),
        documentation: Some(recipe.summary.to_owned()),
        insert_text: Some(recipe.steps[0].source.to_owned()),
        is_snippet: false,
    }
}

fn symbol_search_match(symbol: ScopeSymbol, location: Option<String>) -> SearchMatch {
    SearchMatch {
        name: symbol.name,
        kind: symbol_kind(symbol.kind),
        signature: symbol.signature,
        documentation: symbol.documentation,
        location,
    }
}

fn symbol_kind(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Value => "value",
        SymbolKind::Function => "function",
        SymbolKind::Type => "type",
        SymbolKind::Module => "module",
        SymbolKind::Imported => "imported",
    }
}

fn capability_kind(kind: crate::catalog::CapabilityKind) -> &'static str {
    use crate::catalog::CapabilityKind;
    match kind {
        CapabilityKind::SessionCommand => "session_command",
        CapabilityKind::CompilerQuery => "compiler_query",
        CapabilityKind::DeclarationAttribute => "declaration_attribute",
        CapabilityKind::ExpressionDirective => "expression_directive",
    }
}

fn search_match_contains(entry: &SearchMatch, filter: &str) -> bool {
    entry.name.to_lowercase().contains(filter)
        || entry.signature.to_lowercase().contains(filter)
        || entry
            .documentation
            .as_ref()
            .is_some_and(|text| text.to_lowercase().contains(filter))
}

fn completion_prefix(source: &str) -> &str {
    let start = source
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_alphanumeric() && *character != '_')
        .map_or(0, |(offset, character)| offset + character.len_utf8());
    &source[start..]
}

fn catalog_completions(source: &str) -> Option<Vec<InterpreterCompletion>> {
    let prefix = source.trim_start().strip_prefix('#')?;
    if prefix.chars().any(char::is_whitespace) || prefix.contains('(') {
        return None;
    }
    if let Some(field_prefix) = prefix.strip_prefix("target.") {
        let target = crate::catalog::capabilities()
            .iter()
            .find(|entry| entry.name == "target")?;
        return Some(
            target
                .examples
                .iter()
                .filter_map(|example| example.source.strip_prefix("#target."))
                .filter(|field| field.starts_with(field_prefix))
                .map(|field| InterpreterCompletion {
                    label: field.to_owned(),
                    kind: "field",
                    detail: Some("target field (text)".to_owned()),
                    documentation: None,
                    insert_text: None,
                    is_snippet: false,
                })
                .collect(),
        );
    }
    Some(
        crate::catalog::capabilities()
            .iter()
            .filter(|entry| entry.name.starts_with(prefix))
            .map(|entry| InterpreterCompletion {
                label: entry.name.to_owned(),
                kind: capability_kind(entry.kind),
                detail: Some(entry.syntax.to_owned()),
                documentation: Some(entry.summary.to_owned()),
                insert_text: None,
                is_snippet: false,
            })
            .collect(),
    )
}

impl From<typecheck::CompletionCandidate> for InterpreterCompletion {
    fn from(candidate: typecheck::CompletionCandidate) -> Self {
        use typecheck::CompletionKind;
        Self {
            label: candidate.label,
            kind: match candidate.kind {
                CompletionKind::Function => "function",
                CompletionKind::Variable => "value",
                CompletionKind::Tag => "type",
                CompletionKind::Keyword => "keyword",
            },
            detail: candidate.detail,
            documentation: candidate.documentation,
            insert_text: candidate.insert_text,
            is_snippet: candidate.is_snippet,
        }
    }
}

#[cfg(test)]
#[path = "tests/discovery_tests.rs"]
mod discovery_tests;
