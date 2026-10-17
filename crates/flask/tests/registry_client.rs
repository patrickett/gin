use ed25519_dalek::SigningKey;
use flask::{
    FlaskConfig, GinHome, HttpsTransport, Registry, RegistryClient, RegistrySettings,
    RegistryTransport, Release, ReleaseStatus, SignedSnapshot, TrustRoot,
};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::BTreeMap, io, rc::Rc};

#[derive(Clone, Default)]
struct Transport {
    blobs: Rc<RefCell<BTreeMap<String, Vec<u8>>>>,
    calls: Rc<RefCell<Vec<String>>>,
}
impl RegistryTransport for Transport {
    fn fetch(&self, resource: &str, _: usize) -> io::Result<Vec<u8>> {
        self.calls.borrow_mut().push(resource.into());
        self.blobs
            .borrow()
            .get(resource)
            .cloned()
            .ok_or_else(|| io::Error::other("unexpected network request"))
    }
}
fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o777);
        header.set_cksum();
        builder.append_data(&mut header, name, *bytes).unwrap();
    }
    builder.into_inner().unwrap()
}
fn package_archive() -> Vec<u8> {
    archive(&[
        ("mod.flask", b"#settings\nmodule: 'http'\nversion: 1.0.0\n"),
        ("http.gin", b"let value = 1\n"),
    ])
}
struct Fixture {
    home: tempfile::TempDir,
    client: RegistryClient<Transport>,
    transport: Transport,
    registry: Registry,
    config: FlaskConfig,
}
impl Fixture {
    fn new(bytes: Vec<u8>) -> Self {
        let home = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[11; 32]);
        let trust = TrustRoot {
            registry: "flasks.io".into(),
            threshold: 1,
            keys: BTreeMap::from([("root".into(), format_hex(&key.verifying_key().to_bytes()))]),
        };
        let transport = Transport::default();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        transport
            .blobs
            .borrow_mut()
            .insert(format!("v1/blobs/{digest}"), bytes);
        let mut registry = Registry::default();
        registry
            .insert(Release {
                name: "http".into(),
                version: Version::new(1, 0, 0),
                archive_sha256: digest,
                interface_sha256: "b".repeat(64),
                published_at: 0,
                dependencies: BTreeMap::new(),
                minimum_dependencies: Some(BTreeMap::new()),
            })
            .unwrap();
        registry
            .set_status("http", &Version::new(1, 0, 0), ReleaseStatus::Eligible)
            .unwrap();
        let client = RegistryClient::new(
            GinHome::new(home.path().to_path_buf()),
            RegistrySettings {
                url: "https://flasks.io".into(),
                trust,
                cooldown_seconds: 259_200,
            },
            transport.clone(),
        );
        let config = FlaskConfig::parse_manifest(
            "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\nhttp: 1.0.0\n",
        )
        .unwrap();
        let fixture = Self {
            home,
            client,
            transport,
            registry,
            config,
        };
        fixture.publish(1);
        fixture
    }
    fn publish(&self, revision: u64) {
        let snapshot = self
            .registry
            .snapshot("flasks.io".into(), revision, 300_000, 400_000);
        let signed = SignedSnapshot::sign(
            &snapshot,
            &BTreeMap::from([("root".into(), SigningKey::from_bytes(&[11; 32]))]),
        )
        .unwrap();
        self.transport.blobs.borrow_mut().insert(
            "v1/snapshot.json".into(),
            serde_json::to_vec(&signed).unwrap(),
        );
    }
    fn resolve(&mut self) {
        self.client.resolve(&self.config, 310_000).unwrap();
    }
}
fn format_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn verified_download_is_shared_and_cached_verification_never_uses_network() {
    let mut f = Fixture::new(package_archive());
    f.resolve();
    f.client.fetch(&f.config, 310_000, false).unwrap();
    assert_eq!(f.transport.calls.borrow().len(), 2);
    f.client.fetch(&f.config, 310_000, false).unwrap();
    f.transport.blobs.borrow_mut().clear();
    f.client.verify_cached(&f.config, 310_000).unwrap();
    assert_eq!(f.transport.calls.borrow().len(), 2);
    let source = f.home.path().join("cache/deps/http@1.0.0/http.gin");
    assert_eq!(std::fs::read(&source).unwrap(), b"let value = 1\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&source).unwrap().permissions().mode() & 0o111,
            0
        );
    }
    std::fs::write(&source, "changed").unwrap();
    assert!(f.client.verify_cached(&f.config, 310_000).is_err());
    assert_eq!(f.transport.calls.borrow().len(), 2);
}

#[test]
fn malformed_archive_and_manifest_mismatch_never_install() {
    for bytes in [
        b"not a tar".to_vec(),
        minimum_archive("other", "1.0.0", ""),
        minimum_archive("http", "1.0.0", "local: './local'\n"),
    ] {
        let mut f = Fixture::new(bytes);
        f.resolve();
        assert!(f.client.fetch(&f.config, 310_000, false).is_err());
        assert!(!f.home.path().join("cache/deps/http@1.0.0").exists());
    }
}

#[test]
fn bad_blob_digest_and_tampered_cached_archive_fail_closed() {
    let mut f = Fixture::new(package_archive());
    f.resolve();
    let resource = f
        .transport
        .blobs
        .borrow()
        .keys()
        .find(|k| k.starts_with("v1/blobs/"))
        .unwrap()
        .clone();
    let original = f
        .transport
        .blobs
        .borrow_mut()
        .insert(resource.clone(), b"tampered".to_vec())
        .unwrap();
    assert!(f.client.fetch(&f.config, 310_000, false).is_err());
    f.transport
        .blobs
        .borrow_mut()
        .insert(resource.clone(), original);
    f.client.fetch(&f.config, 310_000, false).unwrap();
    std::fs::write(
        f.home
            .path()
            .join("cache/archives")
            .join(resource.strip_prefix("v1/blobs/").unwrap()),
        "tampered",
    )
    .unwrap();
    assert!(f.client.verify_cached(&f.config, 310_000).is_err());
}

#[test]
fn cached_dependencies_obey_freshness_and_new_signed_blocks() {
    let mut f = Fixture::new(package_archive());
    f.resolve();
    f.client.fetch(&f.config, 310_000, false).unwrap();
    assert!(f.client.verify_cached(&f.config, 400_000).is_err());
    f.registry
        .set_status(
            "http",
            &Version::new(1, 0, 0),
            ReleaseStatus::Blocked {
                reason: "malware".into(),
            },
        )
        .unwrap();
    f.publish(2);
    assert!(f.client.fetch(&f.config, 310_000, true).is_err());
    assert!(f.client.verify_cached(&f.config, 310_000).is_err());
}

#[test]
fn archive_links_are_rejected_without_touching_their_targets() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    builder.append_link(&mut header, "escape", "/tmp").unwrap();
    let mut f = Fixture::new(builder.into_inner().unwrap());
    f.resolve();
    assert!(f.client.fetch(&f.config, 310_000, false).is_err());
    assert!(!f.home.path().join("cache/deps/http@1.0.0").exists());
}

#[test]
fn transport_accepts_only_https_origins() {
    assert!(HttpsTransport::new("https://flasks.io").is_ok());
    for url in [
        "http://flasks.io",
        "https://secret@flasks.io",
        "https://flasks.io/path",
        "https://flasks.io/?query=1",
    ] {
        assert!(HttpsTransport::new(url).is_err(), "{url}");
    }
}

#[test]
fn archive_paths_cannot_escape_or_alias_the_package_root() {
    for path in [
        "../escape",
        "/tmp/escape",
        "nested/../escape",
        "./escape",
        "nested//escape",
        "nested\\escape",
        "C:escape",
    ] {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
        header.set_cksum();
        builder.append(&header, b"x".as_slice()).unwrap();
        let mut f = Fixture::new(builder.into_inner().unwrap());
        f.resolve();
        let error = f.client.fetch(&f.config, 310_000, false).unwrap_err();
        assert!(
            error.to_string().contains("escapes or aliases"),
            "{path}: {error}"
        );
        assert!(!f.home.path().join("cache/deps/http@1.0.0").exists());
    }
}

#[test]
fn audit_refreshes_security_without_downloading_sources_and_reports_the_reason() {
    let mut f = Fixture::new(package_archive());
    f.resolve();
    f.client.audit(&f.config, 310_000).unwrap();
    assert!(!f.home.path().join("cache/deps/http@1.0.0").exists());
    f.registry
        .set_status(
            "http",
            &Version::new(1, 0, 0),
            ReleaseStatus::Blocked {
                reason: "confirmed credential theft".into(),
            },
        )
        .unwrap();
    f.publish(2);
    let error = f.client.audit(&f.config, 310_000).unwrap_err();
    assert!(error.to_string().contains("confirmed credential theft"));
    assert!(
        f.transport
            .calls
            .borrow()
            .iter()
            .all(|resource| resource == "v1/snapshot.json")
    );
}

fn minimum_archive(name: &str, version: &str, dependencies: &str) -> Vec<u8> {
    let manifest =
        format!("#settings\nmodule: '{name}'\nversion: {version}\n#dependencies\n{dependencies}");
    archive(&[
        ("mod.flask", manifest.as_bytes()),
        ("lib.gin", b"Choice is Left or Right\n"),
    ])
}

fn add_minimum(f: &mut Fixture, name: &str, version: &str, dependencies: &str) {
    let bytes = minimum_archive(name, version, dependencies);
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let manifest =
        format!("#settings\nmodule: '{name}'\nversion: {version}\n#dependencies\n{dependencies}");
    let config = FlaskConfig::parse_manifest(&manifest).unwrap();
    let version = Version::parse(version).unwrap();
    f.registry
        .insert(Release {
            name: name.into(),
            version: version.clone(),
            archive_sha256: digest.clone(),
            interface_sha256: "b".repeat(64),
            published_at: 0,
            dependencies: BTreeMap::new(),
            minimum_dependencies: Some(config.registry_minimum_requirements().unwrap()),
        })
        .unwrap();
    f.registry
        .set_status(name, &version, ReleaseStatus::Eligible)
        .unwrap();
    f.transport
        .blobs
        .borrow_mut()
        .insert(format!("v1/blobs/{digest}"), bytes);
}

#[test]
fn signed_minimum_fetch_and_offline_reconstruction_need_no_project_resolution() {
    let mut f = Fixture::new(minimum_archive("http", "1.0.0", ""));
    add_minimum(&mut f, "json", "1.0.0", "http: 1.2.0\n");
    add_minimum(&mut f, "http", "1.2.0", "socket: 1.0.0\n");
    add_minimum(&mut f, "socket", "1.0.0", "");
    f.publish(2);
    f.config = FlaskConfig::parse_manifest(
        "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\nhttp: 1.0.0\njson: 1.0.0\n",
    )
    .unwrap();
    let before = serde_json::to_value(&f.config).unwrap();
    f.client.fetch(&f.config, 310_000, true).unwrap();
    let selected = f.client.verify_cached(&f.config, 310_000).unwrap();
    assert_eq!(selected.direct["http"], "http@1.2.0");
    assert_eq!(selected.packages.len(), 3);
    assert!(!f.home.path().join("cache/deps/http@1.0.0").exists());
    assert_eq!(serde_json::to_value(&f.config).unwrap(), before);
    assert!(f.config.resolution.is_none());
    add_minimum(&mut f, "http", "1.9.0", "");
    f.publish(3);
    f.client.fetch(&f.config, 310_000, true).unwrap();
    assert_eq!(
        f.client.verify_cached(&f.config, 310_000).unwrap(),
        selected
    );
    let calls = f.transport.calls.borrow().len();
    f.transport.blobs.borrow_mut().clear();
    assert_eq!(
        f.client.verify_cached(&f.config, 310_000).unwrap(),
        selected
    );
    assert_eq!(f.transport.calls.borrow().len(), calls);
    std::fs::write(
        f.home.path().join("cache/deps/http@1.2.0/lib.gin"),
        "changed",
    )
    .unwrap();
    assert!(f.client.verify_cached(&f.config, 310_000).is_err());
}

#[test]
fn signed_manifest_mismatch_legacy_and_duplicate_manifests_never_install() {
    for bytes in [
        minimum_archive("other", "1.0.0", ""),
        minimum_archive("http", "1.0.0", "socket: 1.0.0\n"),
        archive(&[("flask.jsonc", b"{}")]),
        archive(&[
            ("mod.flask", b"#settings\nmodule: 'http'\nversion: 1.0.0\n"),
            ("mod.flask", b"{}"),
        ]),
    ] {
        let f = Fixture::new(bytes);
        assert!(f.client.fetch(&f.config, 310_000, true).is_err());
        assert!(!f.home.path().join("cache/deps/http@1.0.0").exists());
    }
}

#[test]
fn minimum_security_block_fails_without_choosing_a_newer_release() {
    let mut f = Fixture::new(minimum_archive("http", "1.0.0", ""));
    f.client.fetch(&f.config, 310_000, true).unwrap();
    add_minimum(&mut f, "http", "1.1.0", "");
    f.registry
        .set_status(
            "http",
            &Version::new(1, 0, 0),
            ReleaseStatus::Blocked {
                reason: "compromised".into(),
            },
        )
        .unwrap();
    f.publish(2);
    assert!(
        f.client
            .fetch(&f.config, 310_000, true)
            .unwrap_err()
            .to_string()
            .contains("compromised")
    );
    let update = f.client.update(&f.config, 310_000).unwrap();
    assert_eq!(update.direct["http"], "http@1.1.0");
    let next = f.config.with_minimum_floors(&update).unwrap();
    assert!(next.resolution.is_none());
    f.client.fetch(&next, 310_000, false).unwrap();
    assert_eq!(f.client.verify_cached(&next, 310_000).unwrap(), update);
}

#[test]
fn minimum_publication_binds_manifest_archive_and_compiled_interface() {
    let bytes = minimum_archive("http", "1.0.0", "json: 1.0.0\n");
    let compiled = interface::PublicInterface::from_subject_and_declarations(
        interface::PackageInstanceId {
            package: "http".into(),
            version: "1.0.0".into(),
            source: "flasks.io".into(),
            instance: "http".into(),
        },
        vec![],
    );
    let mut release = Release {
        name: "http".into(),
        version: Version::new(1, 0, 0),
        archive_sha256: format!("{:x}", Sha256::digest(&bytes)),
        interface_sha256: format!("{:x}", Sha256::digest(compiled.canonical_bytes())),
        published_at: 0,
        dependencies: BTreeMap::new(),
        minimum_dependencies: Some(BTreeMap::new()),
    };
    let mut registry = Registry::default();
    assert!(
        registry
            .publish(release.clone(), &bytes, &compiled, None)
            .is_err()
    );
    release.minimum_dependencies = Some(BTreeMap::from([(
        "json".into(),
        flask::MinimumRequirement::parse("json", "1.0.0").unwrap(),
    )]));
    registry
        .publish(release.clone(), &bytes, &compiled, None)
        .unwrap();
    assert!(registry.publish(release, &bytes, &compiled, None).is_err());
}
