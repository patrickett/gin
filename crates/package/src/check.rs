//! Checking a whole package: each module's files through `check_module_files`, in
//! dependency order, against the capsules of the modules it imports.

use std::path::PathBuf;

use typecheck::{
    CheckedSourceModule, Limits, SourceCheckFailure, check_module_files_diagnostic,
};

use crate::{GraphError, GraphLimits, Package, PackageGraph};

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error("module `{module}` failed to check: {failure:?}")]
    Check {
        module: String,
        /// The file the failure is in, when it is located.
        path: Option<PathBuf>,
        failure: SourceCheckFailure,
    },
    #[error("module `{module}` marks an `#entry`; only the root module may")]
    EntryOutsideRoot { module: String },
}

/// A package whose modules all checked; `capsules[i]` belongs to `graph.modules[i]`.
#[derive(Debug)]
pub struct CheckedPackage {
    pub graph: PackageGraph,
    pub capsules: Vec<CheckedSourceModule>,
}

impl CheckedPackage {
    /// The root module's `#entry` function as an index into its checked `functions`.
    pub fn entry(&self) -> Option<u32> {
        self.capsules.get(self.graph.root?)?.checked().entry
    }
}

impl Package {
    pub fn check(
        &self,
        graph_limits: GraphLimits,
        limits: Limits,
    ) -> Result<CheckedPackage, PackageError> {
        let graph = self.load_graph(graph_limits)?;
        self.check_graph(graph, limits)
    }

    /// Check the module at `segments` below the root as the package's root module.
    pub fn check_at(
        &self,
        segments: &[String],
        graph_limits: GraphLimits,
        limits: Limits,
    ) -> Result<CheckedPackage, PackageError> {
        let graph = self.load_graph_at(segments, graph_limits)?;
        self.check_graph(graph, limits)
    }

    /// Check every module of a library that has nothing at its root.
    pub fn check_all(
        &self,
        graph_limits: GraphLimits,
        limits: Limits,
    ) -> Result<CheckedPackage, PackageError> {
        let graph = self.load_graph_all(graph_limits)?;
        self.check_graph(graph, limits)
    }

    /// Check the modules of a loaded graph in order. Only the graph's root module may mark an
    /// `#entry`.
    pub fn check_graph(
        &self,
        graph: PackageGraph,
        limits: Limits,
    ) -> Result<CheckedPackage, PackageError> {
        let mut capsules: Vec<CheckedSourceModule> = Vec::with_capacity(graph.modules.len());
        for (index, module) in graph.modules.iter().enumerate() {
            let dependencies: Vec<&CheckedSourceModule> = graph.imports[index]
                .iter()
                .map(|&dependency| &capsules[dependency])
                .collect();
            let checked =
                check_module_files_diagnostic(&module.name, &module.files, &dependencies, limits)
                    .map_err(|failure| PackageError::Check {
                    module: module.name.clone(),
                    path: failure.file.map(|file| module.paths[file].clone()),
                    failure,
                })?;
            let root = graph.root == Some(index);
            if !root && checked.checked().entry.is_some() {
                return Err(PackageError::EntryOutsideRoot {
                    module: module.name.clone(),
                });
            }
            capsules.push(checked);
        }
        assert_eq!(capsules.len(), graph.modules.len());
        Ok(CheckedPackage { graph, capsules })
    }
}
