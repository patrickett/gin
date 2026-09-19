use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Default)]
pub struct ExecutionSandbox {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
    network: bool,
}

impl ExecutionSandbox {
    pub fn with_read(mut self, path: PathBuf) -> Self {
        self.read.push(path);
        self
    }
    pub fn with_write(mut self, path: PathBuf) -> Self {
        self.write.push(path);
        self
    }
    pub fn with_network(mut self, allowed: bool) -> Self {
        self.network = allowed;
        self
    }

    pub fn command(&self, program: &Path, scratch: &Path) -> io::Result<Command> {
        let program = program.canonicalize()?;
        let scratch = scratch.canonicalize()?;
        let read = self
            .read
            .iter()
            .map(|path| path.canonicalize())
            .collect::<io::Result<Vec<_>>>()?;
        let write = self
            .write
            .iter()
            .map(|path| path.canonicalize())
            .collect::<io::Result<Vec<_>>>()?;
        let mut command = platform_command(&program, &scratch, &read, &write, self.network)?;
        command
            .env_clear()
            .env("HOME", &scratch)
            .env("TMPDIR", &scratch)
            .env("LANG", "C.UTF-8")
            .current_dir(&scratch);
        Ok(command)
    }
}

#[cfg(target_os = "macos")]
fn platform_command(
    program: &Path,
    scratch: &Path,
    read: &[PathBuf],
    write: &[PathBuf],
    network: bool,
) -> io::Result<Command> {
    fn quoted(path: &Path) -> io::Result<String> {
        let text = path
            .to_str()
            .ok_or_else(|| io::Error::other("sandbox paths must be UTF-8"))?;
        if text.chars().any(char::is_control) {
            return Err(io::Error::other(
                "sandbox paths cannot contain control characters",
            ));
        }
        Ok(format!(
            "\"{}\"",
            text.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    }
    let mut policy = String::from(
        "(version 1)(deny default)(allow process-fork)(allow signal (target self))(allow sysctl-read (sysctl-name-prefix \"hw.\") (sysctl-name \"kern.osrelease\") (sysctl-name \"kern.osversion\") (sysctl-name \"kern.ostype\") (sysctl-name \"kern.osproductversion\") (sysctl-name \"kern.argmax\") (sysctl-name \"kern.maxfilesperproc\") (sysctl-name \"kern.usrstack64\"))(allow file-read-metadata)(allow file-read* (literal \"/\"))",
    );
    policy.push_str(&format!(
        "(allow process-exec (literal {}))",
        quoted(program)?
    ));
    policy.push_str(&format!(
        "(allow file-read* file-map-executable (literal {}))",
        quoted(program)?
    ));
    for root in [
        "/System",
        "/usr/lib",
        "/usr/share",
        "/Library/Apple",
        "/opt/homebrew/lib",
        "/usr/local/lib",
    ] {
        policy.push_str(&format!(
            "(allow file-read* file-map-executable (subpath {}))",
            quoted(Path::new(root))?
        ));
    }
    policy.push_str("(allow file-read* (literal \"/dev/null\") (literal \"/dev/urandom\") (literal \"/dev/random\"))(allow file-write* (literal \"/dev/null\"))");
    for root in read {
        policy.push_str(&format!("(allow file-read* (subpath {}))", quoted(root)?));
    }
    for root in write
        .iter()
        .map(PathBuf::as_path)
        .chain(std::iter::once(scratch))
    {
        policy.push_str(&format!(
            "(allow file-read* file-write* (subpath {}))",
            quoted(root)?
        ));
    }
    if network {
        policy.push_str("(allow network*)(allow mach-lookup (global-name \"com.apple.bsd.dirhelper\") (global-name \"com.apple.SystemConfiguration.configd\") (global-name \"com.apple.mDNSResponder\"))");
    }
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command.arg("-p").arg(policy).arg(program);
    Ok(command)
}

#[cfg(target_os = "linux")]
fn platform_command(
    program: &Path,
    scratch: &Path,
    read: &[PathBuf],
    write: &[PathBuf],
    network: bool,
) -> io::Result<Command> {
    let mut command = Command::new("/usr/bin/bwrap");
    command.args(["--die-with-parent", "--new-session", "--unshare-all"]);
    if network {
        command.arg("--share-net");
        for root in ["/etc/resolv.conf", "/etc/hosts", "/etc/nsswitch.conf"] {
            if Path::new(root).exists() {
                command.arg("--ro-bind").arg(root).arg(root);
            }
        }
    }
    for root in ["/usr", "/lib", "/lib64", "/etc/ld.so.cache"] {
        if Path::new(root).exists() {
            command.arg("--ro-bind").arg(root).arg(root);
        }
    }
    command.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
    command.arg("--ro-bind").arg(program).arg(program);
    for root in read {
        command.arg("--ro-bind").arg(root).arg(root);
    }
    for root in write
        .iter()
        .map(PathBuf::as_path)
        .chain(std::iter::once(scratch))
    {
        command.arg("--bind").arg(root).arg(root);
    }
    command.arg("--").arg(program);
    Ok(command)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_command(
    _: &Path,
    _: &Path,
    _: &[PathBuf],
    _: &[PathBuf],
    _: bool,
) -> io::Result<Command> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "execution sandbox is not supported on this host",
    ))
}
