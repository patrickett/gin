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
