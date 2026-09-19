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
        (
            "flask.jsonc",
            br#"{"name":"http","version":"1.0.0","authors":[]}"#,
        ),
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
        let config = json5::from_str(
            r#"{name:'app',version:'1.0.0',authors:[],dependencies:{http:{version:'^1'}}}"#,
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
        self.config.resolution = Some(self.client.resolve(&self.config, 310_000).unwrap());
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
    for bytes in [b"not a tar".to_vec(), archive(&[("flask.jsonc", br#"{"name":"other","version":"1.0.0","authors":[]}"#)]), archive(&[("flask.jsonc", br#"{"name":"http","version":"1.0.0","authors":[],"dependencies":{"local":{"path":"/tmp"}}}"#)])] {
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
