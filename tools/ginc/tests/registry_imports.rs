use ed25519_dalek::SigningKey;
use flask::{
    FlaskConfig, GinHome, Registry, RegistryClient, RegistrySettings, RegistryTransport, Release,
    ReleaseStatus, SignedSnapshot, TrustRoot,
};
use ginc::{
    cli::{Args, Emit},
    compile::{CompileResult, GinCompiler},
};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io};

struct Transport(BTreeMap<String, Vec<u8>>);
impl RegistryTransport for Transport {
    fn fetch(&self, resource: &str, _: usize) -> io::Result<Vec<u8>> {
        self.0
            .get(resource)
            .cloned()
            .ok_or_else(|| io::Error::other("unexpected request"))
    }
}

#[test]
fn compiler_loads_signed_minimum_sources_without_a_project_lock() {
    let home = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    let manifest_name = "mod.flask";
    let manifest = b"#settings\nmodule: 'http'\nversion: 1.0.0\n#dependencies\n".as_slice();
    for (path, bytes) in [
        (manifest_name, manifest),
        ("http.gin", b"Choice is Left or Right\n".as_slice()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes).unwrap();
    }
    let bytes = archive.into_inner().unwrap();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut registry = Registry::default();
    registry
        .insert(Release {
            name: "http".into(),
            version: Version::new(1, 0, 0),
            archive_sha256: digest.clone(),
            interface_sha256: "b".repeat(64),
            published_at: now - 300_000,
            dependencies: BTreeMap::new(),
            minimum_dependencies: Some(BTreeMap::new()),
        })
        .unwrap();
    registry
        .set_status("http", &Version::new(1, 0, 0), ReleaseStatus::Eligible)
        .unwrap();
    let key = SigningKey::from_bytes(&[13; 32]);
    let trust = TrustRoot {
        registry: "flasks.io".into(),
        threshold: 1,
        keys: BTreeMap::from([(
            "root".into(),
            key.verifying_key()
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        )]),
    };
    let signed = SignedSnapshot::sign(
        &registry.snapshot("flasks.io".into(), 1, now, now + 86400),
        &BTreeMap::from([("root".into(), key)]),
    )
    .unwrap();
    let settings = RegistrySettings {
        url: "https://flasks.io".into(),
        trust,
        cooldown_seconds: 259_200,
    };
    std::fs::write(
        home.path().join("registry.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let client = RegistryClient::new(
        GinHome::new(home.path().to_path_buf()),
        settings,
        Transport(BTreeMap::from([
            (
                "v1/snapshot.json".into(),
                serde_json::to_vec(&signed).unwrap(),
            ),
            (format!("v1/blobs/{digest}"), bytes),
        ])),
    );
    let config = FlaskConfig::parse_manifest(
        "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\nhttp: 1.0.0\n",
    )
    .unwrap();
    client.resolve(&config, now).unwrap();
    client.fetch(&config, now, false).unwrap();
    std::fs::write(
        source.path().join(manifest_name),
        config.format_manifest().unwrap(),
    )
    .unwrap();
    std::fs::write(
        source.path().join("main.gin"),
        "use http Choice\nidentity(value Choice) Choice: return value\n",
    )
    .unwrap();
    let mut args = Args {
        input: source.path().to_path_buf(),
        emit: Emit::Interface,
        gin_home: Some(GinHome::new(home.path().to_path_buf())),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(
        matches!(result, CompileResult::Success { output: Some(_) }),
        "{result:?}"
    );
    std::fs::write(
        home.path().join("cache/deps/http@1.0.0/http.gin"),
        "Choice is Different\n",
    )
    .unwrap();
    let result = GinCompiler::compile(&mut args);
    assert!(matches!(result, CompileResult::Failed(_)), "{result:?}");
    assert_eq!(std::fs::read_dir(source.path()).unwrap().count(), 2);
}

fn minimum_package(
    registry: &mut Registry,
    resources: &mut BTreeMap<String, Vec<u8>>,
    name: &str,
    version: &str,
    dependencies: &str,
    now: u64,
) {
    let raw =
        format!("#settings\nmodule: '{name}'\nversion: {version}\n#dependencies\n{dependencies}");
    let config = FlaskConfig::parse_manifest(&raw).unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    for (path, bytes) in [
        ("mod.flask", raw.as_bytes()),
        ("lib.gin", b"Choice is Left or Right\n".as_slice()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes).unwrap();
    }
    let bytes = archive.into_inner().unwrap();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let version = Version::parse(version).unwrap();
    registry
        .insert(Release {
            name: name.into(),
            version: version.clone(),
            archive_sha256: digest.clone(),
            interface_sha256: "b".repeat(64),
            published_at: now - 300_000,
            dependencies: BTreeMap::new(),
            minimum_dependencies: Some(config.registry_minimum_requirements().unwrap()),
        })
        .unwrap();
    registry
        .set_status(name, &version, ReleaseStatus::Eligible)
        .unwrap();
    resources.insert(format!("v1/blobs/{digest}"), bytes);
}

fn minimum_client(home: &std::path::Path, now: u64) -> RegistryClient<Transport> {
    let mut registry = Registry::default();
    let mut resources = BTreeMap::new();
    for (name, version, dependencies) in [
        ("http", "1.0.0", ""),
        ("http", "1.2.0", ""),
        ("http", "2.0.0", ""),
        ("json", "1.0.0", "http: 1.2.0\n"),
    ] {
        minimum_package(
            &mut registry,
            &mut resources,
            name,
            version,
            dependencies,
            now,
        );
    }
    let key = SigningKey::from_bytes(&[17; 32]);
    let settings = RegistrySettings {
        url: "https://flasks.io".into(),
        cooldown_seconds: 259_200,
        trust: TrustRoot {
            registry: "flasks.io".into(),
            threshold: 1,
            keys: BTreeMap::from([(
                "root".into(),
                key.verifying_key()
                    .to_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            )]),
        },
    };
    let signed = SignedSnapshot::sign(
        &registry.snapshot("flasks.io".into(), 1, now, now + 86400),
        &BTreeMap::from([("root".into(), key)]),
    )
    .unwrap();
    resources.insert(
        "v1/snapshot.json".into(),
        serde_json::to_vec(&signed).unwrap(),
    );
    std::fs::write(
        home.join("registry.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    RegistryClient::new(
        GinHome::new(home.to_owned()),
        settings,
        Transport(resources),
    )
}

#[test]
fn local_transitives_share_the_global_selection_and_distinct_major_instances_compile() {
    let home = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let client = minimum_client(home.path(), now);
    std::fs::create_dir(source.path().join("core")).unwrap();
    std::fs::write(source.path().join("mod.flask"), "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\nhttp: 1.0.0\nhttp2: http@2.0.0\ncore: './core'\n").unwrap();
    std::fs::write(
        source.path().join("core/mod.flask"),
        "#settings\nmodule: 'core'\nversion: 0.1.0\n#dependencies\njson: 1.0.0\n",
    )
    .unwrap();
    std::fs::write(
        source.path().join("main.gin"),
        "use http Choice\nidentity(value Choice) Choice: return value\n",
    )
    .unwrap();
    let config = FlaskConfig::discover(source.path()).unwrap().unwrap().0;
    let workspace = config.with_workspace_requirements(source.path()).unwrap();
    client.fetch(&workspace, now, true).unwrap();
    let gin_home = GinHome::new(home.path().to_owned());
    let graph = resolve::ResolvedPackageGraph::discover_with_home(source.path(), Some(&gin_home));
    assert!(graph.diagnostics.is_empty(), "{:?}", graph.diagnostics);
    assert_eq!(graph.nodes.len(), 5);
    let roots = graph.node(graph.root).resolved_dependency_roots();
    assert!(roots["http"].ends_with("http@1.2.0"));
    assert!(roots["http2"].ends_with("http@2.0.0"));
    let json = graph
        .nodes
        .iter()
        .find(|node| node.config.name() == "json")
        .unwrap();
    assert_eq!(json.resolved_dependency_roots()["http"], roots["http"]);
    assert!(!home.path().join("cache/deps/http@1.0.0").exists());
    let mut args = Args {
        input: source.path().to_owned(),
        emit: Emit::Interface,
        gin_home: Some(gin_home),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(
        matches!(result, CompileResult::Success { output: Some(_) }),
        "{result:?}"
    );
    assert_eq!(std::fs::read_dir(source.path()).unwrap().count(), 3);
}
