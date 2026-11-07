//! Traits across modules. A capsule has no AST, but it keeps the written text of each trait
//! declaration it defines (the declaration alone: its required methods, `#infix` markers and
//! default bodies). An importer that names such a trait in a `use` line parses that text into an
//! extra, non-exported file with its own file slot after the module's real files, so the trait
//! then behaves exactly as one declared in the importer: `Type has Trait` requires the
//! methods, default bodies are re-checked per providing type, and bodiless methods give the
//! `Trait.method` signatures a bound reads. Names a default body mentions resolve in the
//! importer, so it must import them too. A trait the importer does not name stays unknown.
use super::*;
use std::sync::Arc;

/// One trait a capsule defines: its name and the exact text of its declaration.
#[derive(Debug, Clone)]
pub(super) struct TraitSource {
    pub(super) name: String,
    /// The declaration with every type name that means something in the module that wrote it
    /// replaced by a unique spelling (`StepInCoreIter`), so that it means the same in any
    /// importer whatever the importer has in scope.
    pub(super) text: Arc<str>,
    /// Each such spelling and the layout (in the defining module's table) it stands for.
    pub(super) names: Vec<(String, usize)>,
}

/// The module's files followed by one parsed file per imported trait, and the work it took.
/// `None` when no `use` line names a trait, so the common module copies nothing.
pub(super) fn extend(
    files: &[SourceFile],
    dependencies: &[&modules::CheckedSourceModule],
) -> Result<(Option<Vec<SourceFile>>, usize, Vec<String>)> {
    let mut work = 0usize;
    let mut pulled: Vec<(usize, &TraitSource)> = Vec::new();
    for file in files {
        for item in &file.items {
            let Item::Import(import) = &item.value else {
                continue;
            };
            for entry in &import.entries {
                let ImportSource::Module(path) = &entry.source.value else {
                    continue;
                };
                work += path.len() + dependencies.len() + 1;
                let module = path
                    .iter()
                    .map(|name| name.value.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                let Some(index) = dependencies.iter().position(|d| d.name == module) else {
                    continue;
                };
                for member in &entry.members {
                    let [name] = &member.path[..] else {
                        continue;
                    };
                    let traits = &dependencies[index].traits;
                    work += traits.len() + 1;
                    let Some(found) = traits.iter().find(|t| t.name == name.value) else {
                        continue;
                    };
                    if !pulled.iter().any(|(d, t)| *d == index && t.name == found.name) {
                        pulled.push((index, found));
                    }
                }
            }
        }
    }
    if pulled.is_empty() {
        // Looking for trait names is a scan of the `use` lines; it costs nothing unless one is found,
        // so a module that imports no trait is charged what it always was.
        return Ok((None, 0, Vec::new()));
    }
    if files.len() + pulled.len() >= SpanId::FILES_MAX as usize {
        return Err(SourceCheckError::Budget);
    }
    let mut all = files.to_vec();
    let mut names = Vec::new();
    for (index, (_, found)) in pulled.into_iter().enumerate() {
        // Parsing and copying cost what the text and the files it joins weigh.
        work += found.text.len() + all.len();
        let slot = (files.len() + index) as u32;
        let parsed = parser::parse_strict_file(&found.text, slot)
            .map_err(|_| SourceCheckError::Malformed)?;
        names.push(found.name.clone());
        all.push(parsed);
    }
    Ok((Some(all), work, names))
}

impl<'a> Checker<'a> {
    /// The text of each trait this module declares itself (not those it imported), for the
    /// capsule to keep.
    pub(super) fn exported_traits(&mut self, own_files: usize, module: &str) -> Result<Vec<TraitSource>> {
        let mut traits = Vec::new();
        for index in 0..self.module.traits.len() {
            let (declaration, item) = self.module.traits[index];
            if item.file() as usize >= own_files {
                continue;
            }
            let text = self.span(item)?;
            self.charge(text.len() + declaration.name.value.len())?;
            let (text, names) = self.home_resolved(text, &declaration.name.value, module)?;
            traits.push(TraitSource {
                name: declaration.name.value.clone(),
                text: Arc::from(text),
                names,
            });
        }
        Ok(traits)
    }
}

impl<'a> Checker<'a> {
    /// `text` with each capitalized name that this module resolves to a type (its own or
    /// imported) respelled uniquely, and the table of those spellings. A name the trait declares
    /// itself, a trait, or anything this module does not know is left as written.
    fn home_resolved(&mut self, text: &str, own: &str, module: &str) -> Result<(String, Vec<(String, usize)>)> {
        // `core.iter` -> `CoreIter`: a name made of letters alone is a type name to the lexer.
        let suffix: String = module
            .split(['.', '_'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |first| first.to_ascii_uppercase().to_string() + chars.as_str())
            })
            .collect();
        let mut output = String::with_capacity(text.len());
        let mut names: Vec<(String, usize)> = Vec::new();
        let mut chars = text.char_indices().peekable();
        while let Some((start, c)) = chars.next() {
            if !(c.is_ascii_alphabetic() || c == '_') {
                output.push(c);
                continue;
            }
            let mut end = start + c.len_utf8();
            while let Some(&(next, n)) = chars.peek() {
                if n.is_ascii_alphanumeric() || n == '_' {
                    end = next + n.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let word = &text[start..end];
            let after_dot = text[..start].trim_end().ends_with('.');
            let known = c.is_ascii_uppercase()
                && !after_dot
                && word != own
                && !self.module.traits.iter().any(|(t, _)| t.name.value == word);
            self.charge(self.module.type_names.len() + 1)?;
            let layout = if known {
                self.module.type_names.iter().rev().find(|(n, _)| n == word).map(|(_, l)| *l)
            } else {
                None
            };
            match layout {
                Some(layout) => {
                    let mangled = format!("{word}In{suffix}");
                    if !names.iter().any(|(n, _)| *n == mangled) {
                        names.push((mangled.clone(), layout));
                    }
                    output.push_str(&mangled);
                }
                None => output.push_str(word),
            }
        }
        Ok((output, names))
    }
}
