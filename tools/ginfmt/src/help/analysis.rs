use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use ast::ParamConvention;
use ast::ty::{PackageInstanceKey, Ty};
use diagnostic::Category;
use flask::CompileTarget;
use parser::query::SourceParseExt;
use resolve::{
    GinPackageExt, PackageFrontEndOptions, PackageFrontEndOutput, ParsedFile, ParsedModuleCache,
    run_package_front_end,
};
use typecheck::transform::PackageTransformOptions;
use typecheck::{DefId, TypedExprKind, TypedFileAst};

#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) enum CallTarget {
    Definition(DefId),
    Specialization {
        definition: DefId,
        parameters: Vec<(Ty, ParamConvention)>,
        result: Ty,
    },
}

pub(super) struct Analysis {
    pub parsed: ParsedFile,
    pub(super) output: PackageFrontEndOutput,
}

impl Analysis {
    pub fn load(path: &Path, source: String, target: CompileTarget) -> Result<Self, String> {
        let parsed = ParsedFile {
            path: path.to_path_buf(),
            output: source.parse_source_full(),
            source,
        };
        if parsed
            .output
            .symptoms
            .iter()
            .any(|symptom| symptom.category == Category::Flaw)
        {
            return Err(format!(
                "{}: fix parse errors before applying help",
                path.display()
            ));
        }
        let output = run_package_front_end(
            vec![parsed.clone()],
            &path.flask_path_dependencies(),
            ParsedModuleCache::default(),
            PackageFrontEndOptions {
                gin_home: None,
                compile_target: target,
                transform: PackageTransformOptions::IDE,
                resolve_imports: true,
                package_fallback: PackageInstanceKey::workspace("formatter", "0"),
                transform_paths: None,
            },
        );
        Ok(Self { parsed, output })
    }

    pub fn typed(&self) -> &TypedFileAst {
        self.output
            .files
            .iter()
            .find(|file| file.parsed.path == self.parsed.path)
            .and_then(|file| file.typed.as_ref())
            .expect("front end retains the requested file")
    }

    pub fn flaws(&self) -> BTreeMap<(PathBuf, String, String), usize> {
        let mut flaws = BTreeMap::new();
        for file in &self.output.files {
            let typed = file
                .typed
                .as_ref()
                .map(|typed| typed.all_flaws())
                .unwrap_or_default();
            for flaw in file
                .parsed
                .output
                .symptoms
                .iter()
                .chain(&file.import_diagnostics)
                .chain(&file.prepare_diagnostics)
                .chain(typed.iter().map(|(_, flaw)| *flaw))
            {
                if flaw.category == Category::Flaw {
                    *flaws
                        .entry((
                            file.parsed.path.clone(),
                            flaw.code.to_string(),
                            flaw.message.clone(),
                        ))
                        .or_default() += 1;
                }
            }
        }
        flaws
    }

    pub fn calls(&self) -> HashMap<CallTarget, usize> {
        let mut calls = HashMap::new();
        for kind in &self.typed().exprs.kind {
            if let TypedExprKind::FnCall {
                target,
                args: Some(_),
                ..
            } = kind
            {
                *calls.entry(self.call_target(*target)).or_default() += 1;
            }
        }
        calls
    }

    fn call_target(&self, target: DefId) -> CallTarget {
        if let Some((name, _)) = target.0.split_once("$specialized$")
            && let Some(bind) = self
                .output
                .files
                .iter()
                .find_map(|file| file.typed.as_ref()?.defs.get(&target))
        {
            return CallTarget::Specialization {
                definition: target.with_name(internment::Intern::from_ref(name)),
                parameters: bind
                    .params
                    .iter()
                    .zip(&bind.param_conventions)
                    .map(|((_, ty), convention)| (ty.clone(), *convention))
                    .collect(),
                result: bind.return_type.clone(),
            };
        }
        CallTarget::Definition(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_fixtures::TempPackage;

    #[test]
    fn call_verification_preserves_specialized_types_and_overload_identity() {
        let package = TempPackage::new("help_call_verification");
        package.write_flask("help_call_verification");
        let source = "First has\n    release(self):\n        return\nSecond has\n    release(self):\n        return\nconsume(a t): return a.release\nrun: consume(First())\n";
        let path = package.write("main.gin", source).canonicalize().unwrap();
        let analyze =
            |source: String| Analysis::load(&path, source, CompileTarget::Library).unwrap();
        let mut before = analyze(source.to_owned());
        let inferred = analyze(source.replace("a t", "a"));
        assert!(before.flaws().is_empty(), "{:?}", before.flaws());
        assert_eq!(before.flaws(), inferred.flaws());
        assert_eq!(before.calls(), inferred.calls());

        let other_type = analyze(source.replace("consume(First())", "consume(Second())"));
        let specialization = |analysis: &Analysis| {
            analysis
                .calls()
                .into_iter()
                .filter(|(target, _)| matches!(target, CallTarget::Specialization { .. }))
                .collect::<HashMap<_, _>>()
        };
        assert!(!specialization(&before).is_empty());
        assert_ne!(specialization(&before), specialization(&other_type));

        let target = before
            .typed()
            .defs
            .keys()
            .find(|target| target.0.contains("$specialized$"))
            .copied()
            .unwrap();
        let other_overload = DefId::member(target.0, target.1 + 1);
        let file = before
            .output
            .files
            .iter_mut()
            .find_map(|file| {
                file.typed
                    .as_mut()
                    .filter(|typed| typed.defs.contains_key(&target))
            })
            .unwrap();
        file.defs.insert(other_overload, file.defs[&target].clone());
        assert_ne!(
            before.call_target(target),
            before.call_target(other_overload)
        );
    }
}
