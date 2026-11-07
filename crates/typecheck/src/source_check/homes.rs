//! Where each layout and function of a capsule was defined, so that lowering can name a type or a
//! function across modules without the instance ids or table indexes of any one module.
use super::*;

/// Parallel to the capsule's full `layouts` and `functions` tables (own, then imported).
#[derive(Debug, Default)]
pub(super) struct Homes {
    /// The names of the dependencies, in the order the capsule was checked against them.
    pub dependencies: Vec<String>,
    /// For each layout, the module that declared it.
    pub layouts: Vec<String>,
    /// For each function, the module that defined it and its index in that module's own table.
    pub functions: Vec<(String, u32)>,
    /// The origin of each imported function, as recorded while importing: (dependency, index).
    pub imported: Vec<(u32, u32)>,
    /// Stable names retained by the artifact reader, whose capsule carries only own functions.
    /// Keys use the body's dependency slot and the defining module's own function index.
    pub artifact_callees:
        std::collections::HashMap<(u32, u32), (interface_codec::ModuleId, String)>,
    /// How many functions the module defines itself; the imported ones follow.
    pub own: usize,
}

impl Homes {
    pub(super) fn of(
        module: &CheckedSourceModule,
        name: &str,
        dependencies: &[&CheckedSourceModule],
    ) -> Self {
        let layouts = module
            .layouts
            .iter()
            .map(|layout| {
                dependencies
                    .iter()
                    .find_map(|dependency| {
                        let position = dependency
                            .layouts
                            .iter()
                            .position(|other| other.origin == layout.origin)?;
                        dependency.homes.layouts.get(position).cloned()
                    })
                    .unwrap_or_else(|| name.to_owned())
            })
            .collect();
        let own = module.homes.own;
        let functions = (0..module.functions.len())
            .map(|index: usize| match index.checked_sub(own) {
                None => (name.to_owned(), index as u32),
                Some(imported) => {
                    let (slot, position) = module.homes.imported[imported];
                    dependencies[slot as usize].homes.functions[position as usize].clone()
                }
            })
            .collect();
        Self {
            dependencies: dependencies
                .iter()
                .map(|dependency| dependency.name.clone())
                .collect(),
            layouts,
            functions,
            imported: module.homes.imported.clone(),
            artifact_callees: module.homes.artifact_callees.clone(),
            own,
        }
    }
}

impl CheckedSourceModule {
    /// The module that declared layout `layout` (an index into `checked().aggregates`).
    pub fn layout_home(&self, layout: usize) -> Option<&str> {
        self.homes.layouts.get(layout).map(String::as_str)
    }

    /// The module that defined function `function` of the full table a body's callees index (own
    /// functions first, then imported ones), and its index in that module's own functions.
    pub fn function_home(&self, function: usize) -> Option<(&str, u32)> {
        self.homes
            .functions
            .get(function)
            .map(|(module, index)| (module.as_str(), *index))
    }

    /// The name of the dependency a `CheckedSourceCallee::Dependency` of this module's bodies names.
    pub fn dependency_name(&self, dependency: usize) -> Option<&str> {
        self.homes.dependencies.get(dependency).map(String::as_str)
    }
}
