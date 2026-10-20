// Copyright (C) 2026 Toit contributors.
//! Bounded JSON subprocess transport shared by one-shot and incremental providers.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub const BYTE_LIMIT: usize = 8_000_000;

struct ProcessGroup(Child);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // A provider can exit while a descendant still holds its pipe open.
        // Kill only the new process group created for this invocation.
        unsafe {
            libc::kill(-(self.0.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}
enum Event {
    Input(std::io::Result<()>),
    Output(std::io::Result<Vec<u8>>),
    Error(std::io::Result<Vec<u8>>),
}
fn read_bounded(stream: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.take(BYTE_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn call(
    executable: &Path,
    args: &[String],
    request: &Value,
    timeout: Duration,
) -> Result<Value> {
    ensure!(!timeout.is_zero(), "Provider timeout must be positive");
    let input = serde_json::to_vec(request)?;
    ensure!(input.len() <= BYTE_LIMIT, "Provider request too large");
    let mut process = ProcessGroup(
        Command::new(executable)
            .args(args)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Cannot start candidate/AI provider")?,
    );
    let mut stdin = process.0.stdin.take().unwrap();
    let stdout = process.0.stdout.take().unwrap();
    let stderr = process.0.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let writer = tx.clone();
    std::thread::spawn(move || {
        let _ = writer.send(Event::Input(stdin.write_all(&input)));
    });
    let reader = tx.clone();
    std::thread::spawn(move || {
        let _ = reader.send(Event::Output(read_bounded(stdout)));
    });
    std::thread::spawn(move || {
        let _ = tx.send(Event::Error(read_bounded(stderr)));
    });
    let started = Instant::now();
    let (mut input_done, mut output, mut error, mut status) = (false, None, None, None);
    loop {
        if status.is_none() {
            status = process.0.try_wait()?;
        }
        if input_done && output.is_some() && error.is_some() && status.is_some() {
            break;
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .context("Provider timed out while running or closing output pipes")?;
        match rx.recv_timeout(remaining.min(Duration::from_millis(10))) {
            Ok(Event::Input(result)) => {
                result.context("Provider failed to read request")?;
                input_done = true;
            }
            Ok(Event::Output(result)) => {
                let bytes = result.context("Cannot read provider output")?;
                ensure!(bytes.len() <= BYTE_LIMIT, "Provider response too large");
                output = Some(bytes);
            }
            Ok(Event::Error(result)) => {
                let bytes = result.context("Cannot read provider stderr")?;
                ensure!(bytes.len() <= BYTE_LIMIT, "Provider stderr too large");
                error = Some(bytes);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !input_done || output.is_none() || error.is_none() {
                    bail!("Provider I/O worker stopped unexpectedly");
                }
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
        }
    }
    ensure!(
        status.unwrap().success(),
        "Provider failed: {}",
        String::from_utf8_lossy(&error.unwrap())
    );
    serde_json::from_slice(&output.unwrap()).context("Provider must return one JSON object")
}
