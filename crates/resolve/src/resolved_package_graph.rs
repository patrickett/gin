use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use ast::ty::{PackageInstanceKey, PackageSourceKey};
use derive_more::From;
use diagnostic::Diagnostic;
use flask::{DependencyKind, FlaskConfig};
use internment::Intern;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct ResolvedPackageId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedDependency {
    GitRevision { url: String },
    RegistryInstance { version: String },
    InvalidPath { path: String },
    DependencyCycle { path: String },
    ConflictingInstance { instance: PackageInstanceKey },
    ManifestIdentityMismatch { expected: PackageInstanceKey },
}

impl UnresolvedDependency {
    fn diagnostic(&self, alias: &str) -> Diagnostic {
        match self {
            Self::GitRevision { url } => Diagnostic::new(
                "package-unresolved-git-revision",
                format!("Git dependency `{alias}` has no resolved revision"),
            )
            .with_arg("alias", alias)
            .with_arg("url", url),
            Self::RegistryInstance { version } => Diagnostic::new(
                "package-unresolved-registry-instance",
                format!("registry dependency `{alias}` has no resolved registry instance"),
            )
            .with_arg("alias", alias)
            .with_arg("version", version),
            Self::InvalidPath { path } => Diagnostic::new(
                "package-invalid-path-dependency",
                format!("path dependency `{alias}` cannot be resolved from `{path}`"),
            )
            .with_arg("alias", alias)
            .with_arg("path", path),
            Self::DependencyCycle { path } => Diagnostic::new(
                "package-dependency-cycle",
                format!("path dependency `{alias}` forms a package cycle through `{path}`"),
            )
            .with_arg("alias", alias)
            .with_arg("path", path),
            Self::ConflictingInstance { instance } => Diagnostic::new(
                "package-conflicting-resolved-instance",
                format!(
                    "dependency `{alias}` conflicts with an existing resolved package instance"
                ),
            )
            .with_arg("alias", alias)
            .with_arg("instance", format!("{instance:?}")),
            Self::ManifestIdentityMismatch { expected } => Diagnostic::new(
                "package-manifest-identity-mismatch",
                format!(
                    "dependency `{alias}` manifest does not match its supplied package identity"
                ),
            )
            .with_arg("alias", alias)
            .with_arg("expected", format!("{expected:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedDependencyState {
    Resolved {
        package: ResolvedPackageId,
        import_root: PathBuf,
    },
    Unresolved(UnresolvedDependency),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDependencyEdge {
    pub alias: String,
    pub state: ResolvedDependencyState,
}

#[derive(Debug, Clone)]
pub struct ResolvedPackageNode {
    pub instance: PackageInstanceKey,
    pub root: PathBuf,
    pub config: FlaskConfig,
    pub dependencies: Vec<ResolvedDependencyEdge>,
}

impl ResolvedPackageNode {
    pub fn resolved_dependency_roots(&self) -> HashMap<String, PathBuf> {
        let mut roots: HashMap<_, _> = self
            .dependencies
            .iter()
            .filter_map(|edge| match &edge.state {
                ResolvedDependencyState::Resolved { import_root, .. } => {
                    Some((edge.alias.clone(), import_root.clone()))
                }
                ResolvedDependencyState::Unresolved(_) => None,
            })
            .collect();
        roots.insert(self.config.name().to_string(), self.root.clone());
        roots
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedPackageGraph {
    pub nodes: Vec<ResolvedPackageNode>,
    pub root: ResolvedPackageId,
    pub diagnostics: Vec<Diagnostic>,
}

impl ResolvedPackageGraph {
    pub fn discover(entry_path: &Path) -> Self {
        let Some((config, root)) = FlaskConfig::find_package_config(entry_path) else {
            let root = entry_path.parent().unwrap_or(entry_path).to_path_buf();
            return Self {
                nodes: vec![ResolvedPackageNode {
                    instance: PackageInstanceKey::workspace("anonymous", "0"),
                    root,
                    config: FlaskConfig::new("anonymous".to_string(), "0".to_string()),
                    dependencies: Vec::new(),
                }],
                root: ResolvedPackageId(0),
                diagnostics: Vec::new(),
            };
        };

        let root = normalize_operational(&root);
        let mut graph = Self {
            nodes: vec![ResolvedPackageNode {
                instance: PackageInstanceKey::workspace(config.name(), config.version()),
                root,
                config,
                dependencies: Vec::new(),
            }],
            root: ResolvedPackageId(0),
            diagnostics: Vec::new(),
        };
        let mut active = HashSet::from([graph.nodes[0].root.clone()]);
        graph.resolve_dependencies(ResolvedPackageId(0), &mut active);
        graph
    }

    pub fn node(&self, id: ResolvedPackageId) -> &ResolvedPackageNode {
        &self.nodes[id.0]
    }

    pub fn validate_instance_metadata(
        &mut self,
        package: ResolvedPackageId,
        supplied: &PackageInstanceKey,
    ) -> Result<(), UnresolvedDependency> {
        if self.nodes[package.0].instance == *supplied {
            return Ok(());
        }
        let error = UnresolvedDependency::ManifestIdentityMismatch {
            expected: self.nodes[package.0].instance.clone(),
        };
        self.diagnostics
            .push(error.diagnostic(self.nodes[package.0].config.name()));
        Err(error)
    }

    pub fn package_for_file(&self, path: &Path) -> Option<ResolvedPackageId> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| path.starts_with(&node.root))
            .max_by_key(|(_, node)| node.root.components().count())
            .map(|(index, _)| ResolvedPackageId(index))
    }

    pub fn package_for_import(
        &self,
        parent: ResolvedPackageId,
        path: &Path,
    ) -> Option<ResolvedPackageId> {
        self.nodes[parent.0]
            .dependencies
            .iter()
            .filter_map(|edge| match &edge.state {
                ResolvedDependencyState::Resolved {
                    package,
                    import_root,
                } if path.starts_with(import_root) => Some((*package, import_root)),
                _ => None,
            })
            .max_by_key(|(_, root)| root.components().count())
            .map(|(package, _)| package)
            .or_else(|| self.package_for_file(path))
    }

    pub fn module_path(&self, package: ResolvedPackageId, file: &Path) -> Vec<Intern<String>> {
        file.parent()
            .and_then(|parent| parent.strip_prefix(&self.nodes[package.0].root).ok())
            .into_iter()
            .flat_map(Path::components)
            .filter_map(|component| component.as_os_str().to_str())
            .map(|component| Intern::new(component.to_string()))
            .collect()
    }

    fn resolve_dependencies(&mut self, parent: ResolvedPackageId, active: &mut HashSet<PathBuf>) {
        let declarations: Vec<_> = self.nodes[parent.0]
            .config
            .dependencies()
            .iter()
            .map(|(alias, dependency)| (alias.clone(), dependency.kind.clone()))
            .collect();
        let mut edges = Vec::with_capacity(declarations.len());

        for (alias, dependency) in declarations {
            let state = match dependency {
                DependencyKind::Path { path } => self.resolve_path(parent, &alias, &path, active),
                DependencyKind::Git { url } => {
                    ResolvedDependencyState::Unresolved(UnresolvedDependency::GitRevision { url })
                }
                DependencyKind::Version { version } => {
                    ResolvedDependencyState::Unresolved(UnresolvedDependency::RegistryInstance {
                        version,
                    })
                }
                _ => ResolvedDependencyState::Unresolved(UnresolvedDependency::RegistryInstance {
                    version: "unsupported".to_string(),
                }),
            };
            if let ResolvedDependencyState::Unresolved(error) = &state {
                self.diagnostics.push(error.diagnostic(&alias));
            }
            edges.push(ResolvedDependencyEdge { alias, state });
        }
        self.nodes[parent.0].dependencies = edges;
    }

    fn resolve_path(
        &mut self,
        parent: ResolvedPackageId,
        _alias: &str,
        declared_path: &str,
        active: &mut HashSet<PathBuf>,
    ) -> ResolvedDependencyState {
        let Ok(relative_path) = normalize_manifest_relative(declared_path) else {
            return ResolvedDependencyState::Unresolved(UnresolvedDependency::InvalidPath {
                path: declared_path.to_string(),
            });
        };
        let operational_root =
            normalize_operational(&self.nodes[parent.0].root.join(&relative_path));
        let Some((config, manifest_root)) = FlaskConfig::find_package_config(&operational_root)
        else {
            return ResolvedDependencyState::Unresolved(UnresolvedDependency::InvalidPath {
                path: declared_path.to_string(),
            });
        };

        let manifest_root = normalize_operational(&manifest_root);
        if manifest_root == self.nodes[parent.0].root {
            return ResolvedDependencyState::Resolved {
                package: parent,
                import_root: operational_root,
            };
        }
        if active.contains(&manifest_root) {
            return ResolvedDependencyState::Unresolved(UnresolvedDependency::DependencyCycle {
                path: relative_path,
            });
        }

        let parent_instance = self.nodes[parent.0].instance.instance;
        let instance_name = Intern::new(format!("{}/{}", parent_instance.as_str(), relative_path));
        let instance = PackageInstanceKey {
            name: Intern::new(config.name().to_string()),
            version: Intern::new(config.version().to_string()),
            source: PackageSourceKey::Path {
                parent_instance,
                relative_path: Intern::new(relative_path),
            },
            instance: instance_name,
        };

        if let Some((index, existing)) = self
            .nodes
            .iter()
            .enumerate()
            .find(|(_, node)| node.instance == instance)
        {
            return if existing.root == manifest_root {
                ResolvedDependencyState::Resolved {
                    package: ResolvedPackageId(index),
                    import_root: operational_root,
                }
            } else {
                ResolvedDependencyState::Unresolved(UnresolvedDependency::ConflictingInstance {
                    instance,
                })
            };
        }

        let id = ResolvedPackageId(self.nodes.len());
        self.nodes.push(ResolvedPackageNode {
            instance,
            root: manifest_root.clone(),
            config,
            dependencies: Vec::new(),
        });
        active.insert(manifest_root.clone());
        self.resolve_dependencies(id, active);
        active.remove(&manifest_root);
        ResolvedDependencyState::Resolved {
            package: id,
            import_root: operational_root,
        }
    }
}

fn normalize_manifest_relative(path: &str) -> Result<String, ()> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(());
    }
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.last().is_some_and(|part| part != "..") {
                    parts.pop();
                } else {
                    parts.push("..".to_string());
                }
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::RootDir | Component::Prefix(_) => return Err(()),
        }
    }
    Ok(if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    })
}

fn normalize_operational(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}
#[cfg(test)]
#[path = "../tests/resolved_package_graph_tests.rs"]
mod tests;
