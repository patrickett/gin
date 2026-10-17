use std::collections::HashSet;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{Dependency, DependencyKind, FlaskConfig, MinimumRequirement};

pub const MANIFEST_NAME: &str = "mod.flask";
const MANIFEST_BYTES_MAX: usize = 1024 * 1024;
const LINE_BYTES_MAX: usize = 16 * 1024;

#[derive(Debug, Error)]
#[error("mod.flask:{line}: {message}")]
pub struct ManifestError {
    pub line: u32,
    pub message: String,
}

impl ManifestError {
    fn new(line: usize, message: impl ToString) -> Self {
        Self {
            line: u32::try_from(line).expect("manifest line count is bounded"),
            message: message.to_string(),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Section {
    Settings,
    Dependencies,
}

impl FlaskConfig {
    pub fn parse_manifest(raw: &str) -> Result<Self, ManifestError> {
        if raw.len() > MANIFEST_BYTES_MAX {
            return Err(ManifestError::new(1, "manifest exceeds 1 MiB"));
        }
        let mut config = Self::new(String::new(), String::new());
        config.minimum_version_selection = true;
        let mut section = None;
        let mut sections = HashSet::new();
        let mut keys = HashSet::new();
        for (index, raw_line) in raw.lines().enumerate() {
            let line = index + 1;
            if raw_line.len() > LINE_BYTES_MAX {
                return Err(ManifestError::new(line, "line exceeds 16 KiB"));
            }
            let text = manifest_line(raw_line).map_err(|e| ManifestError::new(line, e))?;
            if text.is_empty() {
                continue;
            }
            if text.starts_with('#') {
                if !sections.insert(text.to_owned()) {
                    return Err(ManifestError::new(line, "duplicate section"));
                }
                section = Some(match text {
                    "#settings" => Section::Settings,
                    "#dependencies" => Section::Dependencies,
                    _ => return Err(ManifestError::new(line, "unknown section")),
                });
                continue;
            }
            let current =
                section.ok_or_else(|| ManifestError::new(line, "entry needs a section"))?;
            let (key, value) = text
                .split_once(':')
                .ok_or_else(|| ManifestError::new(line, "expected key: value"))?;
            let (key, value) = (key.trim(), value.trim());
            let identity = (current == Section::Dependencies, key.to_owned());
            if !keys.insert(identity) {
                return Err(ManifestError::new(line, format!("duplicate key `{key}`")));
            }
            let result = match current {
                Section::Settings => config.manifest_setting(key, value),
                Section::Dependencies => config.manifest_dependency(key, value),
            };
            result.map_err(|e| ManifestError::new(line, e))?;
        }
        if config.name.is_empty() || config.version.is_empty() {
            return Err(ManifestError::new(
                1,
                "module and version settings are required",
            ));
        }
        Ok(config)
    }

    fn manifest_setting(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "module" => {
                let name = manifest_string(value)?;
                if !crate::valid_package_name(&name) {
                    return Err("invalid module name".into());
                }
                self.name = name;
            }
            "version" => {
                let version = semver::Version::parse(value).map_err(|e| e.to_string())?;
                if !version.pre.is_empty() || !version.build.is_empty() {
                    return Err("expected a stable version literal".into());
                }
                self.version = version.to_string();
            }
            "target" => {
                let target = manifest_string(value)?;
                if target != "library" {
                    crate::TargetTriple::parse(&target).map_err(|e| e.to_string())?;
                }
                self.target = Some(target);
            }
            "description" => self.description = Some(manifest_string(value)?),
            "repository" => self.repository = Some(manifest_string(value)?),
            "bugs" => {
                self.bugs = Some(crate::config::BugInfo {
                    url: Some(manifest_string(value)?),
                })
            }
            "authors" => {
                self.authors = manifest_strings(value)?
                    .into_iter()
                    .map(crate::config::Author)
                    .collect()
            }
            "keywords" => self.keywords = Some(manifest_strings(value)?),
            "license" => self.license = Some(manifest_strings(value)?),
            "funding" => self.funding = Some(manifest_strings(value)?),
            _ => return Err(format!("unknown setting `{key}`")),
        }
        Ok(())
    }

    fn manifest_dependency(&mut self, alias: &str, value: &str) -> Result<(), String> {
        if !crate::valid_package_name(alias) {
            return Err("invalid dependency alias".into());
        }
        let kind = if value.starts_with(['\'', '"']) {
            let path = manifest_string(value)?;
            if !path.starts_with("./") && !path.starts_with("../") {
                return Err("local paths must start with ./ or ../".into());
            }
            DependencyKind::Path { path }
        } else {
            let requirement = MinimumRequirement::parse(alias, value).map_err(|e| e.to_string())?;
            DependencyKind::Version {
                version: requirement.manifest_value(alias),
            }
        };
        self.dependencies
            .insert(alias.to_owned(), Dependency::new(kind));
        Ok(())
    }

    pub fn format_manifest(&self) -> Result<String, ManifestError> {
        let quote = |value: &str| serde_json::to_string(value).expect("strings serialize");
        let mut raw = format!(
            "#settings\nmodule: {}\nversion: {}\n",
            quote(&self.name),
            self.version
        );
        if let Some(target) = &self.target {
            raw.push_str(&format!("target: {}\n", quote(target)));
        }
        if let Some(description) = &self.description {
            raw.push_str(&format!("description: {}\n", quote(description)));
        }
        let metadata = serde_json::to_value(self).expect("configuration serializes");
        for key in ["authors", "repository", "keywords", "license", "funding"] {
            let value = &metadata[key];
            if !value.is_null()
                && !(key == "authors" && value.as_array().is_some_and(Vec::is_empty))
            {
                raw.push_str(&format!("{key}: {value}\n"));
            }
        }
        if let Some(url) = self.bugs.as_ref().and_then(|bugs| bugs.url.as_ref()) {
            raw.push_str(&format!("bugs: {}\n", quote(url)));
        }
        raw.push_str("\n#dependencies\n");
        let mut dependencies: Vec<_> = self.dependencies.iter().collect();
        dependencies.sort_by_key(|(alias, _)| *alias);
        for (alias, dependency) in dependencies {
            let value = match &dependency.kind {
                DependencyKind::Version { version } => version.clone(),
                DependencyKind::Path { path } => quote(path),
                _ => return Err(ManifestError::new(1, "unsupported dependency kind")),
            };
            raw.push_str(&format!("{alias}: {value}\n"));
        }
        Self::parse_manifest(&raw)?;
        Ok(raw)
    }

    pub fn manifest_path(root: &Path) -> PathBuf {
        root.join(MANIFEST_NAME)
    }
}

fn manifest_strings(value: &str) -> Result<Vec<String>, String> {
    if !value.starts_with('[') {
        return Err("expected a list of quoted strings".into());
    }
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or("expected a list of quoted strings")?;
    let mut remaining = inner.trim();
    let mut strings = Vec::new();
    while !remaining.is_empty() {
        let (string, rest) = quoted_string(remaining)?;
        strings.push(string);
        remaining = rest.trim_start();
        if remaining.is_empty() {
            break;
        }
        remaining = remaining
            .strip_prefix(',')
            .ok_or("expected a comma between strings")?
            .trim_start();
    }
    Ok(strings)
}

pub(crate) fn manifest_line(raw: &str) -> Result<&str, &'static str> {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote.is_some() && character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
        } else {
            if character == '\'' || character == '"' {
                quote = Some(character);
            }
            if raw[index..].starts_with("--") {
                return Ok(raw[..index].trim());
            }
            if raw[index..].starts_with("//") || raw[index..].starts_with("/*") {
                return Err("comments use --");
            }
        }
    }
    if quote.is_some() {
        return Err("unterminated string");
    }
    Ok(raw.trim())
}

fn manifest_string(value: &str) -> Result<String, String> {
    if !value.starts_with(['\'', '"']) {
        return Err("string values must be quoted".into());
    }
    let (string, rest) = quoted_string(value)?;
    if !rest.trim().is_empty() {
        return Err("unexpected text after string".into());
    }
    Ok(string)
}

fn quoted_string(value: &str) -> Result<(String, &str), String> {
    let quote = value
        .chars()
        .next()
        .filter(|c| matches!(c, '\'' | '"'))
        .ok_or("string values must be quoted")?;
    let mut encoded = String::from("\"");
    let mut characters = value.char_indices().skip(1);
    while let Some((index, character)) = characters.next() {
        if character == quote {
            encoded.push('"');
            let decoded = serde_json::from_str(&encoded).map_err(|e| e.to_string())?;
            return Ok((decoded, &value[index + character.len_utf8()..]));
        }
        if character == '\\' {
            let (_, escaped) = characters.next().ok_or("unterminated escape")?;
            if escaped == '\'' && quote == '\'' {
                encoded.push('\'');
            } else {
                encoded.push('\\');
                encoded.push(escaped);
            }
        } else {
            if character == '"' {
                encoded.push('\\');
            }
            encoded.push(character);
        }
    }
    Err("unterminated string".into())
}

pub(crate) fn set_entry(
    raw: &str,
    section: &str,
    key: &str,
    value: &str,
) -> Result<String, String> {
    FlaskConfig::parse_manifest(raw).map_err(|e| e.to_string())?;
    let mut active = false;
    let mut insertion = None;
    let mut offset = 0;
    for line in raw.split_inclusive('\n') {
        let content = manifest_line(line).map_err(str::to_owned)?;
        if content.starts_with('#') {
            if active {
                break;
            }
            active = content == section;
            if active {
                insertion = Some(offset + line.len());
            }
        } else {
            if active {
                if let Some((found, _)) = content.split_once(':')
                    && found.trim() == key
                {
                    let start = offset + line.find(content).expect("content belongs to line");
                    let end = start + content.len();
                    return Ok(format!("{}{key}: {value}{}", &raw[..start], &raw[end..]));
                }
                insertion = Some(offset + line.len());
            }
        }
        offset += line.len();
    }
    let entry = format!("{key}: {value}\n");
    match insertion {
        Some(index) => {
            let newline = if raw[..index].ends_with('\n') {
                ""
            } else {
                "\n"
            };
            Ok(format!(
                "{}{newline}{entry}{}",
                &raw[..index],
                &raw[index..]
            ))
        }
        None => Ok(format!("{}\n{section}\n{entry}", raw.trim_end())),
    }
}
