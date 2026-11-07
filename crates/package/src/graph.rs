//! Import graph of one package: which folder modules a module names, in checking order.
//!
//! Imports resolve by exact dotted name: `<package>.<folder>.<folder>` is the folder module
//! below the root. The walk is iterative and bounded, so cycles and runaway graphs fail
//! with an error instead of recursing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ast::{ImportSource, Item};

use crate::{DependencyRoot, LoadError, LoadedModule, Package, PackageLimits};

/// Bounds on one package walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphLimits {
    pub modules_max: u32,
    pub module: PackageLimits,
}

impl Default for GraphLimits {
    fn default() -> Self {
        Self {
            modules_max: 1024,
            module: PackageLimits::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error("module `{from}` imports `{import}`, which is not inside package `{package}`")]
    ForeignModule {
        from: String,
        import: String,
        package: String,
    },
    #[error("package `{package}` names a {kind} dependency `{name}`, which is not supported")]
    DependencyUnsupported {
        package: String,
        name: String,
        kind: &'static str,
    },
    #[error(
        "package `{package}` names dependency `{key}`, but the package at {path} is called `{declared}`"
    )]
    DependencyName {
        package: String,
        key: String,
        declared: String,
        path: PathBuf,
    },
    #[error("dependency `{name}` is at {first} for one package and {second} for another")]
    DependencyConflict {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
    #[error("import cycle: {}", .path.join(" -> "))]
    Cycle { path: Vec<String> },
    #[error("package has more than {max} modules")]
    TooManyModules { max: u32 },
    #[error("module limits are invalid: {message}")]
    LimitsInvalid { message: &'static str },
}

/// Modules in dependency order: every module appears after all modules it imports.
#[derive(Debug)]
pub struct PackageGraph {
    pub modules: Vec<LoadedModule>,
    /// `imports[i]` lists indices into `modules` that module `i` imports, each below `i`.
    pub imports: Vec<Vec<usize>>,
    /// The index of the root module when the walk began at exactly one module (always the
    /// last); `None` when it began at every module of a library with nothing at its root.
    pub root: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    Active,
    Done,
}

struct Frame {
    /// The package the module belongs to: the root, or a dependency.
    package: String,
    module: LoadedModule,
    wanted: Vec<String>,
    next: usize,
    resolved: Vec<usize>,
}

impl Package {
    /// Load the root module and everything it imports, transitively, in checking order.
    /// An import whose first segment names a dependency of the importing package loads that
    /// package's module; every other first segment must be the importing package's own.
    pub fn load_graph(&self, limits: GraphLimits) -> Result<PackageGraph, GraphError> {
        self.load_graph_roots(&[Vec::new()], limits)
    }

    /// Like [`Self::load_graph`], with the module at `segments` below the root as the root.
    pub fn load_graph_at(
        &self,
        segments: &[String],
        limits: GraphLimits,
    ) -> Result<PackageGraph, GraphError> {
        self.load_graph_roots(&[segments.to_vec()], limits)
    }

    /// Like [`Self::load_graph`], beginning at every module the package holds: the way to
    /// check a library whose root has no `.gin` files of its own.
    pub fn load_graph_all(&self, limits: GraphLimits) -> Result<PackageGraph, GraphError> {
        let roots = self.module_segments(limits.modules_max)?;
        self.load_roots(&roots, false, limits)
    }

    /// Load each of `roots` (folder segments below the package root) and everything they
    /// import. A module reached twice is loaded once. No roots is an empty graph: a package
    /// with no source has nothing to check.
    pub fn load_graph_roots(
        &self,
        roots: &[Vec<String>],
        limits: GraphLimits,
    ) -> Result<PackageGraph, GraphError> {
        self.load_roots(roots, roots.len() == 1, limits)
    }

    /// `program`: the single root is the module the run is about.
    fn load_roots(
        &self,
        roots: &[Vec<String>],
        program: bool,
        limits: GraphLimits,
    ) -> Result<PackageGraph, GraphError> {
        assert!(!program || roots.len() == 1);
        if limits.modules_max == 0 {
            return Err(GraphError::LimitsInvalid {
                message: "modules_max must be at least 1",
            });
        }
        let mut visits: HashMap<String, (Visit, usize)> = HashMap::new();
        let mut packages: HashMap<String, Package> = HashMap::new();
        packages.insert(self.name.clone(), self.clone());
        let mut graph = PackageGraph {
            modules: Vec::new(),
            imports: Vec::new(),
            root: None,
        };
        let mut last_root = String::new();
        for segments in roots {
            let name = self.module_name(&segments.iter().map(String::as_str).collect::<Vec<_>>());
            last_root = name.clone();
            if visits.contains_key(&name) {
                continue;
            }
            if visits.len() >= limits.modules_max as usize {
                return Err(GraphError::TooManyModules {
                    max: limits.modules_max,
                });
            }
            let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
            let mut stack = vec![self.frame(self, &segments, limits)?];
            visits.insert(name, (Visit::Active, 0));
            self.walk(&mut stack, &mut visits, &mut packages, &mut graph, limits)?;
        }
        assert_eq!(graph.modules.len(), graph.imports.len());
        if program {
            assert!(graph.modules.last().is_some_and(|m| m.name == last_root));
            graph.root = Some(graph.modules.len() - 1);
        }
        Ok(graph)
    }

    fn walk(
        &self,
        stack: &mut Vec<Frame>,
        visits: &mut HashMap<String, (Visit, usize)>,
        packages: &mut HashMap<String, Package>,
        graph: &mut PackageGraph,
        limits: GraphLimits,
    ) -> Result<(), GraphError> {
        while let Some(top) = stack.last_mut() {
            if top.next == top.wanted.len() {
                let frame = stack.pop().expect("a frame was just observed");
                let index = graph.modules.len();
                visits.insert(frame.module.name.clone(), (Visit::Done, index));
                graph.modules.push(frame.module);
                graph.imports.push(frame.resolved);
                if let Some(parent) = stack.last_mut() {
                    parent.resolved.push(index);
                }
                continue;
            }
            let wanted = top.wanted[top.next].clone();
            top.next += 1;
            // Resolved before the module is looked up, because a module already loaded
            // under this name does not make a second path for its package acceptable.
            let owner = top.package.clone();
            let from = top.module.name.clone();
            let (package, segments) = self.target(packages, &owner, &from, &wanted)?;
            match visits.get(&wanted) {
                Some(&(Visit::Done, index)) => top.resolved.push(index),
                Some(&(Visit::Active, _)) => return Err(cycle(stack, &wanted)),
                None => {
                    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
                    if visits.len() >= limits.modules_max as usize {
                        return Err(GraphError::TooManyModules {
                            max: limits.modules_max,
                        });
                    }
                    let frame = self.frame(&packages[&package], &segments, limits)?;
                    visits.insert(wanted, (Visit::Active, 0));
                    stack.push(frame);
                }
            }
        }
        Ok(())
    }

    /// Whether the module at `segments` has `.gin` files of its own.
    pub fn has_sources_at(&self, segments: &[String]) -> bool {
        let mut directory = self.root.clone();
        directory.extend(segments);
        std::fs::read_dir(directory).is_ok_and(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                let path = entry.path();
                path.is_file() && path.extension().is_some_and(|ext| ext == crate::SOURCE_EXTENSION)
            })
        })
    }

    /// Every folder module below the root that has `.gin` files, in sorted order, as folder
    /// segments. Folders named `target` and hidden folders are not modules. At most `max`
    /// are returned; more is an error.
    pub fn module_segments(&self, max: u32) -> Result<Vec<Vec<String>>, GraphError> {
        let mut found: Vec<Vec<String>> = Vec::new();
        let mut pending: Vec<Vec<String>> = vec![Vec::new()];
        let mut visited = 0u32;
        while let Some(segments) = pending.pop() {
            visited += 1;
            if visited > max.saturating_mul(4).max(64) {
                return Err(GraphError::TooManyModules { max });
            }
            if self.has_sources_at(&segments) {
                if found.len() >= max as usize {
                    return Err(GraphError::TooManyModules { max });
                }
                found.push(segments.clone());
            }
            let mut directory = self.root.clone();
            directory.extend(&segments);
            let read = |error: std::io::Error| LoadError::Read {
                path: directory.clone(),
                message: error.to_string(),
            };
            let mut children: Vec<String> = Vec::new();
            for entry in std::fs::read_dir(&directory).map_err(read)? {
                let entry = entry.map_err(read)?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() && !name.starts_with('.') && name != "target" {
                    children.push(name);
                }
            }
            children.sort();
            // Pushed in reverse so the smallest name is visited first.
            for name in children.into_iter().rev() {
                let mut child = segments.clone();
                child.push(name);
                pending.push(child);
            }
        }
        Ok(found)
    }

    fn frame(
        &self,
        package: &Package,
        segments: &[&str],
        limits: GraphLimits,
    ) -> Result<Frame, GraphError> {
        let module = package.load_module(segments, limits.module)?;
        let wanted = imported_modules(&module);
        Ok(Frame {
            package: package.name.clone(),
            module,
            wanted,
            next: 0,
            resolved: Vec::new(),
        })
    }

    /// The package and the folder segments an import names, as seen from `owner`: its own
    /// package, or one of the dependencies its manifest names. A dependency is opened the
    /// first time it is needed and must be the package it is called.
    fn target(
        &self,
        packages: &mut HashMap<String, Package>,
        owner: &str,
        from: &str,
        import: &str,
    ) -> Result<(String, Vec<String>), GraphError> {
        let (head, rest) = import.split_once('.').unwrap_or((import, ""));
        let segments: Vec<String> = if rest.is_empty() {
            Vec::new()
        } else {
            rest.split('.').map(str::to_owned).collect()
        };
        if head == owner {
            return Ok((owner.to_owned(), segments));
        }
        let importer = &packages[owner];
        let foreign = || GraphError::ForeignModule {
            from: from.to_owned(),
            import: import.to_owned(),
            package: owner.to_owned(),
        };
        let root = match importer.dependency(head).ok_or_else(foreign)? {
            DependencyRoot::Unsupported(kind) => {
                return Err(GraphError::DependencyUnsupported {
                    package: owner.to_owned(),
                    name: head.to_owned(),
                    kind,
                });
            }
            DependencyRoot::Path(root) => root.clone(),
        };
        let wanted = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        if let Some(known) = packages.get(head) {
            let held =
                std::fs::canonicalize(known.root()).unwrap_or_else(|_| known.root().to_owned());
            if held != wanted {
                return Err(GraphError::DependencyConflict {
                    name: head.to_owned(),
                    first: held,
                    second: wanted,
                });
            }
            return Ok((head.to_owned(), segments));
        }
        let opened = Package::open(&root)?.with_overlay(Arc::clone(&importer.overlay));
        if opened.name() != head {
            return Err(GraphError::DependencyName {
                package: owner.to_owned(),
                key: head.to_owned(),
                declared: opened.name().to_owned(),
                path: root,
            });
        }
        packages.insert(head.to_owned(), opened);
        Ok((head.to_owned(), segments))
    }
}

/// The active stack from the first occurrence of `target`, closed back onto it.
fn cycle(stack: &[Frame], target: &str) -> GraphError {
    let start = stack
        .iter()
        .position(|frame| frame.module.name == target)
        .unwrap_or(0);
    let mut path: Vec<String> = stack[start..]
        .iter()
        .map(|frame| frame.module.name.clone())
        .collect();
    path.push(target.to_owned());
    assert!(path.len() >= 2);
    GraphError::Cycle { path }
}

/// Dotted module names written in `use` items, first-seen order, without duplicates.
fn imported_modules(module: &LoadedModule) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for item in module.files.iter().flat_map(|file| &file.items) {
        let Item::Import(import) = &item.value else {
            continue;
        };
        for entry in &import.entries {
            let ImportSource::Module(path) = &entry.source.value else {
                continue;
            };
            let dotted: Vec<&str> = path.iter().map(|name| name.value.as_str()).collect();
            let dotted = dotted.join(".");
            if !names.contains(&dotted) {
                names.push(dotted);
            }
        }
    }
    names
}
