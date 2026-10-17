use diagnostic::code::PackageCode;
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
            // package-unresolved-git-revision
            Self::GitRevision { url } => Diagnostic::new(
                PackageCode::UnresolvedGitRevision,
                format!("Git dependency `{alias}` has no resolved revision"),
            )
            .with_arg("alias", alias)
            .with_arg("url", url),
            // package-unresolved-registry-instance
            Self::RegistryInstance { version } => Diagnostic::new(
                PackageCode::UnresolvedRegistryInstance,
                format!("registry dependency `{alias}` has no resolved registry instance"),
            )
            .with_arg("alias", alias)
            .with_arg("version", version),
            // package-invalid-path-dependency
            Self::InvalidPath { path } => Diagnostic::new(
                PackageCode::InvalidPathDependency,
                format!("path dependency `{alias}` cannot be resolved from `{path}`"),
            )
            .with_arg("alias", alias)
            .with_arg("path", path),
            // package-dependency-cycle
            Self::DependencyCycle { path } => Diagnostic::new(
                PackageCode::DependencyCycle,
                format!("path dependency `{alias}` forms a package cycle through `{path}`"),
            )
            .with_arg("alias", alias)
            .with_arg("path", path),
            // package-conflicting-resolved-instance
            Self::ConflictingInstance { instance } => Diagnostic::new(
                PackageCode::ConflictingResolvedInstance,
                format!(
                    "dependency `{alias}` conflicts with an existing resolved package instance"
                ),
            )
            .with_arg("alias", alias)
            .with_arg("instance", format!("{instance:?}")),
            // package-manifest-identity-mismatch
            Self::ManifestIdentityMismatch { expected } => Diagnostic::new(
                PackageCode::ManifestIdentityMismatch,
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
    registry: Option<(flask::GinHome, String, flask::PackageResolution)>,
    instances: HashMap<PackageInstanceKey, ResolvedPackageId>,
    roots: HashMap<PathBuf, ResolvedPackageId>,
}

impl ResolvedPackageGraph {
    pub fn discover(entry_path: &Path) -> Self {
        Self::discover_with_home(entry_path, None)
    }

    pub fn discover_with_home(entry_path: &Path, home: Option<&flask::GinHome>) -> Self {
        let (config, root) = match FlaskConfig::discover(entry_path) {
            Ok(Some(found)) => found,
            result => {
                let root = entry_path.parent().unwrap_or(entry_path).to_path_buf();
                let mut graph =
                    Self::from_config(FlaskConfig::new("anonymous".into(), "0".into()), root);
                if let Err(error) = result {
                    graph
                        .diagnostics
                        // package-invalid-manifest
                        .push(Diagnostic::new(PackageCode::InvalidManifest, error));
                }
                return graph;
            }
        };
        let root = if config.minimum_version_selection {
            root.canonicalize().unwrap_or(root)
        } else {
            normalize_operational(&root)
        };
        let mut graph = Self::from_config(config, root);
        let workspace_config = match graph.nodes[0]
            .config
            .with_workspace_requirements(&graph.nodes[0].root)
        {
            Ok(config) => config,
            Err(error) => {
                // package-invalid-workspace
                graph.diagnostics.push(Diagnostic::new(
                    PackageCode::InvalidWorkspace,
                    error.to_string(),
                ));
                return graph;
            }
        };
        if workspace_config
            .dependencies()
            .values()
            .any(|dep| matches!(dep.kind, DependencyKind::Version { .. }))
        {
            let verified = Self::verified_registry(&workspace_config, home);
            match verified {
                Ok(context) => graph.registry = Some(context),
                Err(error) => {
                    // package-registry-verification-failed
                    graph.diagnostics.push(Diagnostic::new(
                        PackageCode::RegistryVerificationFailed,
                        error.to_string(),
                    ));
                    return graph;
                }
            }
        }
        graph.resolve_all_dependencies();
        graph
    }

    fn from_config(config: FlaskConfig, root: PathBuf) -> Self {
        let instance = PackageInstanceKey::workspace(config.name(), config.version());
        Self {
            instances: HashMap::from([(instance.clone(), ResolvedPackageId(0))]),
            roots: HashMap::from([(root.clone(), ResolvedPackageId(0))]),
            nodes: vec![ResolvedPackageNode {
                instance,
                root,
                config,
                dependencies: Vec::new(),
            }],
            root: ResolvedPackageId(0),
            diagnostics: Vec::new(),
            registry: None,
        }
    }

    fn verified_registry(
        config: &FlaskConfig,
        home: Option<&flask::GinHome>,
    ) -> std::io::Result<(flask::GinHome, String, flask::PackageResolution)> {
        let home = home
            .cloned()
            .map(Ok)
            .unwrap_or_else(flask::GinHome::discover)?;
        let client = flask::RegistryClient::load(home.clone())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_secs();
        let resolution = client.verify_cached(config, now)?;
        Ok((home, client.registry_name().to_owned(), resolution))
    }

    fn resolve_all_dependencies(&mut self) {
        if self.nodes[0].config.minimum_version_selection {
            let mut index = 0;
            while index < self.nodes.len() {
                if self.nodes.len() > 4352 {
                    // package-graph-limit
                    self.diagnostics.push(Diagnostic::new(
                        PackageCode::GraphLimit,
                        "too many package instances",
                    ));
                    break;
                }
                self.resolve_dependencies(ResolvedPackageId(index), &mut HashSet::new());
                index += 1;
            }
        } else {
            let mut active = HashSet::from([self.nodes[0].root.clone()]);
            self.resolve_dependencies(ResolvedPackageId(0), &mut active);
        }
    }

    pub fn node(&self, id: ResolvedPackageId) -> &ResolvedPackageNode {
        &self.nodes[id.0]
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
        if parent == self.root
            && let Err(error) = self.nodes[parent.0].config.validate_resolution()
        {
            // package-invalid-resolution
            self.diagnostics.push(Diagnostic::new(
                PackageCode::InvalidResolution,
                error.to_string(),
            ));
            return;
        }
        let mut declarations: Vec<_> = self.nodes[parent.0]
            .config
            .dependencies()
            .iter()
            .map(|(alias, dependency)| (alias.clone(), dependency.kind.clone()))
            .collect();
        declarations.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let mut edges = Vec::with_capacity(declarations.len());

        for (alias, dependency) in declarations {
            let state = match dependency {
                DependencyKind::Path { path } => self.resolve_path(parent, &alias, &path, active),
                DependencyKind::Git { url } => {
                    ResolvedDependencyState::Unresolved(UnresolvedDependency::GitRevision { url })
                }
                DependencyKind::Version { version } => {
                    self.resolve_registry(parent, &alias, &version, active)
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

    fn registry_key<'a>(
        &self,
        parent: ResolvedPackageId,
        alias: &str,
        requirement: &str,
        resolution: &'a flask::PackageResolution,
    ) -> Option<&'a String> {
        if self.nodes[self.root.0].config.minimum_version_selection {
            let requirement = if self.nodes[parent.0].config.minimum_version_selection {
                flask::MinimumRequirement::parse(alias, requirement).ok()?
            } else {
                flask::MinimumRequirement::from_legacy(alias, requirement).ok()?
            };
            return resolution.minimum_key(&requirement);
        }
        if parent == self.root {
            resolution.direct.get(alias)
        } else {
            resolution
                .packages
                .get(self.nodes[parent.0].instance.instance.as_str())?
                .dependencies
                .get(alias)
        }
    }

    fn package_root(&self, path: &Path) -> PathBuf {
        if self.nodes[self.root.0].config.minimum_version_selection {
            path.canonicalize()
                .unwrap_or_else(|_| normalize_operational(path))
        } else {
            normalize_operational(path)
        }
    }

    fn insert_node(
        &mut self,
        instance: PackageInstanceKey,
        root: PathBuf,
        config: FlaskConfig,
    ) -> ResolvedPackageId {
        let id = ResolvedPackageId(self.nodes.len());
        self.instances.insert(instance.clone(), id);
        self.roots.insert(root.clone(), id);
        self.nodes.push(ResolvedPackageNode {
            instance,
            root,
            config,
            dependencies: Vec::new(),
        });
        id
    }

    fn resolve_registry(
        &mut self,
        parent: ResolvedPackageId,
        alias: &str,
        requirement: &str,
        active: &mut HashSet<PathBuf>,
    ) -> ResolvedDependencyState {
        let unresolved = || {
            ResolvedDependencyState::Unresolved(UnresolvedDependency::RegistryInstance {
                version: requirement.to_owned(),
            })
        };
        let Some((home, registry, resolution)) = &self.registry else {
            return unresolved();
        };
        let Some(key) = self.registry_key(parent, alias, requirement, resolution) else {
            return unresolved();
        };
        let Some(package) = resolution.packages.get(key) else {
            return unresolved();
        };
        let Ok(root) = home.dependency(&package.name, &package.version) else {
            return unresolved();
        };
        let root = self.package_root(&root);
        if active.contains(&root) && !self.nodes[self.root.0].config.minimum_version_selection {
            return ResolvedDependencyState::Unresolved(UnresolvedDependency::DependencyCycle {
                path: root.display().to_string(),
            });
        }
        let instance = PackageInstanceKey {
            name: Intern::new(package.name.clone()),
            version: Intern::new(package.version.to_string()),
            source: PackageSourceKey::Registry {
                registry: Intern::new(registry.clone()),
            },
            instance: Intern::new(key.clone()),
        };
        if let Some(id) = self.instances.get(&instance) {
            return ResolvedDependencyState::Resolved {
                package: *id,
                import_root: root,
            };
        }
        let Some((config, manifest_root)) = FlaskConfig::find_package_config(&root) else {
            return unresolved();
        };
        if self.package_root(&manifest_root) != root
            || config.name() != package.name
            || config.version() != package.version.to_string()
        {
            return ResolvedDependencyState::Unresolved(
                UnresolvedDependency::ManifestIdentityMismatch { expected: instance },
            );
        }
        let id = self.insert_node(instance, root.clone(), config);
        if !self.nodes[self.root.0].config.minimum_version_selection {
            active.insert(root.clone());
            self.resolve_dependencies(id, active);
            active.remove(&root);
        }
        ResolvedDependencyState::Resolved {
            package: id,
            import_root: root,
        }
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
        let operational_root = self.package_root(&self.nodes[parent.0].root.join(&relative_path));
        let Some((config, manifest_root)) = FlaskConfig::find_package_config(&operational_root)
        else {
            return ResolvedDependencyState::Unresolved(UnresolvedDependency::InvalidPath {
                path: declared_path.to_string(),
            });
        };

        let manifest_root = self.package_root(&manifest_root);
        if self.nodes[self.root.0].config.minimum_version_selection {
            if let Some(id) = self.roots.get(&manifest_root) {
                return ResolvedDependencyState::Resolved {
                    package: *id,
                    import_root: operational_root,
                };
            }
        }
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

        if let Some(id) = self.instances.get(&instance) {
            return if self.nodes[id.0].root == manifest_root {
                ResolvedDependencyState::Resolved {
                    package: *id,
                    import_root: operational_root,
                }
            } else {
                ResolvedDependencyState::Unresolved(UnresolvedDependency::ConflictingInstance {
                    instance,
                })
            };
        }

        let id = self.insert_node(instance, manifest_root.clone(), config);
        if !self.nodes[self.root.0].config.minimum_version_selection {
            active.insert(manifest_root.clone());
            self.resolve_dependencies(id, active);
            active.remove(&manifest_root);
        }
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
mod resolution_tests {
    use super::*;
    use test_fixtures::TempPackage;

    #[test]
    fn embedded_resolutions_are_rejected_in_root_and_local_manifests() {
        for relative in ["mod.flask", "dep/mod.flask"] {
            let pkg = TempPackage::new("embedded_resolution");
            pkg.write_flask_with_deps(
                "consumer",
                r#"dep: "./dep"
"#,
            );
            pkg.write_nested_flask("dep", "dep");
            pkg.write(
                relative,
                "#settings\nmodule: 'pkg'\nversion: 1.0.0\nresolution: {}\n",
            );
            let graph = ResolvedPackageGraph::discover(pkg.path());
            assert!(!graph.diagnostics.is_empty(), "{relative}");
        }
    }
}
