//! A two-package build lowers to the same MLIR whether the dependency's capsule was checked
//! from source or read back from its interface artifact.
use typecheck::{
    ArtifactExpectation, ArtifactIdentity, ArtifactReader, CheckedSourceModule, ModuleId,
    write_artifact,
};

const LIB: &str = "\
#unrestricted
Digit is in 0 to 9
#unrestricted
Pair has
    value Digit
    #private
    helper(self) in 0 to 18: return self.value + self.value
    sum(self) in 0 to 18: return self.helper()
double(d Digit) in 0 to 18: return d + d
pick(element Type, x element) element: return x
seven() Digit: return 7
";

const APP: &str = "\
use lib Digit, Pair, double, pick, seven
twice() in 0 to 18: return double(seven())
same(d Digit) Digit: return pick(Digit, d)
total(d Digit) in 0 to 18: return double(d)
paired(p Pair) in 0 to 18: return p.sum()
";

fn check(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    let source = parser::parse_strict(text).unwrap();
    typecheck::check_module(
        name,
        &source,
        dependencies,
        typecheck::Limits::default(),
    )
    .unwrap()
}

fn lib_identity() -> ArtifactIdentity {
    ArtifactIdentity {
        module: ModuleId {
            package: "lib-pkg".into(),
            version: "1.0.0".into(),
            source: [8; 32],
            module: "lib".into(),
        },
        dependencies: Vec::new(),
        target: "test-target".into(),
        compiler: "ginc test".into(),
    }
}

fn read_back(lib: &CheckedSourceModule) -> CheckedSourceModule {
    let identity = lib_identity();
    let expected = ArtifactExpectation {
        module: identity.module.clone(),
        target: identity.target.clone(),
        compiler: identity.compiler.clone(),
    };
    ArtifactReader::new()
        .read(&write_artifact(lib, &identity).unwrap(), &expected)
        .unwrap()
}

#[test]
fn the_artifact_capsule_lowers_to_the_same_mlir_as_the_source_capsule() {
    let lib = check("lib", LIB, &[]);
    let app = check("app", APP, &[&lib]);
    let from_source = ginc::lower_capsules(&[&lib, &app], 1).unwrap();
    let read = read_back(&lib);
    let app_read = check("app", APP, &[&read]);
    let from_artifact = ginc::lower_capsules(&[&read, &app_read], 1).unwrap();
    assert_eq!(from_source.len(), 2);
    for ((name, a), (_, b)) in from_source.iter().zip(&from_artifact) {
        assert_eq!(a, b, "module {name}");
    }
    // The template is instantiated in the importer, and the private helper is lowered in `lib`.
    let (_, app_text) = &from_source[1];
    let (_, lib_text) = &from_source[0];
    assert!(app_text.contains("pick"), "{app_text}");
    assert!(lib_text.contains("helper"), "{lib_text}");
}

/// Negative control: a different library body gives different MLIR, so the comparison above
/// is not vacuous.
#[test]
fn a_changed_dependency_body_changes_the_mlir() {
    let lib = check("lib", LIB, &[]);
    let other = check(
        "lib",
        &LIB.replace(
            "double(d Digit) in 0 to 18: return d + d",
            "double(d Digit) in 0 to 18: return d * 2",
        ),
        &[],
    );
    let a = ginc::lower_capsules(&[&lib, &check("app", APP, &[&lib])], 1).unwrap();
    let read = read_back(&other);
    let b = ginc::lower_capsules(&[&read, &check("app", APP, &[&read])], 1).unwrap();
    assert_ne!(a, b);
}

#[test]
fn capsule_lowering_requires_a_root_inside_the_module_inventory() {
    let library = check("lib", LIB, &[]);
    assert!(ginc::lower_capsules(&[&library], 1).is_err());
}
