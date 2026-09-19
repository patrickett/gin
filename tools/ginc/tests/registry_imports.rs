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
fn compiler_loads_only_verified_registry_sources_from_its_explicit_home() {
    let home = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    for (path, bytes) in [
        (
            "flask.jsonc",
            br#"{"name":"http","version":"1.0.0","authors":[]}"#.as_slice(),
        ),
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
    let mut config = FlaskConfig::new("app".into(), "1.0.0".into());
    config = serde_json::from_value(serde_json::json!({"name":config.name,"version":config.version,"authors":[],"dependencies":{"http":{"version":"^1"}}})).unwrap();
    config.resolution = Some(client.resolve(&config, now).unwrap());
    client.fetch(&config, now, false).unwrap();
    std::fs::write(
        source.path().join("flask.jsonc"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    std::fs::write(
        source.path().join("main.gin"),
        "use http Choice\nidentity(value Choice) Choice: value\n",
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
