use std::io::{self, Read, Write};
use std::process::{Child, Command, Output, Stdio};

fn contextual_error(stage: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{stage}: {error}"))
}

fn terminate_and_reap(child: &mut Child) -> io::Result<()> {
    let kill_result = child.kill();
    match child.wait() {
        Ok(_) => Ok(()),
        Err(wait_error) => Err(io::Error::new(
            wait_error.kind(),
            match kill_result {
                Ok(()) => format!("waiting for terminated child: {wait_error}"),
                Err(kill_error) => {
                    format!("terminating child: {kill_error}; waiting for child: {wait_error}")
                }
            },
        )),
    }
}

fn cleanup_error(child: &mut Child, stage: &str, error: io::Error) -> io::Error {
    match terminate_and_reap(child) {
        Ok(()) => contextual_error(stage, error),
        Err(cleanup) => io::Error::new(
            error.kind(),
            format!("{stage}: {error}; cleanup failed: {cleanup}"),
        ),
    }
}

fn read_output(mut pipe: impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    pipe.read_to_end(&mut output)?;
    Ok(output)
}

pub(crate) fn run_with_input(command: &mut Command, input: &[u8]) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| contextual_error("spawning child", error))?;
    let mut stdin = child.stdin.take().expect("child stdin was piped");
    let stdout = child.stdout.take().expect("child stdout was piped");
    let stderr = child.stderr.take().expect("child stderr was piped");
    std::thread::scope(|scope| {
        let stdout = scope.spawn(|| read_output(stdout));
        let stderr = scope.spawn(|| read_output(stderr));
        let write_result = stdin.write_all(input);
        drop(stdin);
        let status = match write_result {
            Ok(()) => child
                .wait()
                .map_err(|error| cleanup_error(&mut child, "waiting for child", error)),
            Err(error) => Err(cleanup_error(&mut child, "writing child stdin", error)),
        };
        let stdout = stdout.join().expect("stdout reader must not panic");
        let stderr = stderr.join().expect("stderr reader must not panic");
        Ok(Output {
            status: status?,
            stdout: stdout.map_err(|error| contextual_error("reading child stdout", error))?,
            stderr: stderr.map_err(|error| contextual_error("reading child stderr", error))?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "process::tests::child_fixture", "--nocapture"])
            .env("GIN_CODEGEN_PROCESS_FIXTURE", mode);
        command
    }

    #[test]
    fn drains_output_while_writing_input() {
        let output = run_with_input(&mut fixture("pressure"), &vec![b'x'; 512 * 1024]).unwrap();
        assert!(output.status.success());
        assert!(output.stdout.len() >= 256 * 1024);
        assert!(output.stderr.len() >= 256 * 1024);
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("INPUT_BYTES=524288")
        );
    }

    #[test]
    fn retains_failed_command_status_and_stderr() {
        let output = run_with_input(&mut fixture("failure"), b"").unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("fixture failed")
        );
    }

    #[cfg(unix)]
    #[test]
    fn closed_stdin_terminates_child_before_waiting() {
        let start = std::time::Instant::now();
        let error =
            run_with_input(&mut fixture("close-input"), &vec![b'x'; 512 * 1024]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert!(error.to_string().contains("writing child stdin"));
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn child_fixture() {
        let Ok(mode) = std::env::var("GIN_CODEGEN_PROCESS_FIXTURE") else {
            return;
        };
        match mode.as_str() {
            "pressure" => {
                io::stdout().write_all(&vec![b'o'; 256 * 1024]).unwrap();
                io::stderr().write_all(&vec![b'e'; 256 * 1024]).unwrap();
                let input = read_output(io::stdin()).unwrap();
                println!("INPUT_BYTES={}", input.len());
            }
            "failure" => {
                eprintln!("fixture failed");
                std::process::exit(7);
            }
            #[cfg(unix)]
            "close-input" => {
                use std::os::fd::{FromRawFd, OwnedFd};
                #[allow(unsafe_code)]
                let stdin = unsafe { OwnedFd::from_raw_fd(0) };
                drop(stdin);
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            _ => panic!("unknown fixture mode"),
        }
    }
}
