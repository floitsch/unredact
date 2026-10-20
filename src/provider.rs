// Copyright (C) 2026 Toit contributors.
//! Bounded JSON subprocess transport shared by one-shot and incremental providers.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub const BYTE_LIMIT: usize = 8_000_000;

/// Opt-in JSON-lines transport. Each frame carries an increasing request ID;
/// each response is bounded independently, stderr is bounded for the session.
pub struct Worker {
    process: Option<ProcessGroup>,
    input: mpsc::Sender<Vec<u8>>,
    events: mpsc::Receiver<Event>,
    request_id: u64,
}

impl Worker {
    pub fn start(executable: &Path, args: &[String]) -> Result<Self> {
        let mut process = ProcessGroup(
            Command::new(executable)
                .args(args)
                .arg("--persistent")
                .process_group(0)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("Cannot start persistent context provider")?,
        );
        let mut stdin = process.0.stdin.take().unwrap();
        let stdout = process.0.stdout.take().unwrap();
        let stderr = process.0.stderr.take().unwrap();
        let (events, receiver) = mpsc::sync_channel(2);
        let (input, requests) = mpsc::channel::<Vec<u8>>();
        let writer = events.clone();
        std::thread::spawn(move || {
            while let Ok(bytes) = requests.recv() {
                let result = stdin.write_all(&bytes).and_then(|()| stdin.flush());
                let failed = result.is_err();
                if writer.send(Event::Input(result)).is_err() || failed {
                    break;
                }
            }
        });
        let output = events.clone();
        std::thread::spawn(move || {
            let mut stream = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let result = (&mut stream)
                    .take(BYTE_LIMIT as u64 + 2)
                    .read_until(b'\n', &mut bytes);
                let result = result.and_then(|_| {
                    if bytes.last() != Some(&b'\n') {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "Persistent provider response exceeds limit or lacks newline",
                        ));
                    }
                    bytes.pop();
                    Ok(bytes)
                });
                let failed = result.is_err();
                let oversized = result.as_ref().is_ok_and(|b| b.len() > BYTE_LIMIT);
                if output.send(Event::Output(result)).is_err() || failed || oversized {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let _ = events.send(Event::Error(read_bounded(stderr)));
        });
        Ok(Self {
            process: Some(process),
            input,
            events: receiver,
            request_id: 0,
        })
    }

    pub fn call(&mut self, request: &Value, timeout: Duration) -> Result<Value> {
        let result = self.exchange(request, timeout);
        if result.is_err() {
            // A partial exchange cannot safely be retried on the same stream.
            self.process.take();
        }
        result
    }

    fn exchange(&mut self, request: &Value, timeout: Duration) -> Result<Value> {
        ensure!(!timeout.is_zero(), "Provider timeout must be positive");
        let started = Instant::now();
        ensure!(
            self.process.is_some(),
            "Persistent provider session has failed"
        );
        self.request_id += 1;
        let mut bytes = serde_json::to_vec(&serde_json::json!({
            "worker_protocol_version": 1, "request_id": self.request_id, "request": request
        }))?;
        ensure!(bytes.len() <= BYTE_LIMIT, "Provider request too large");
        bytes.push(b'\n');
        self.input
            .send(bytes)
            .context("Persistent provider input worker stopped")?;
        let (mut input_done, mut response) = (false, None);
        loop {
            let remaining = timeout
                .checked_sub(started.elapsed())
                .context("Persistent provider timed out")?;
            match self
                .events
                .recv_timeout(remaining.min(Duration::from_millis(10)))
            {
                Ok(Event::Input(result)) => {
                    result.context("Provider failed to read request")?;
                    ensure!(
                        !input_done,
                        "Unexpected persistent provider input acknowledgement"
                    );
                    input_done = true;
                }
                Ok(Event::Output(result)) => {
                    let bytes = result.context("Cannot read persistent provider output")?;
                    ensure!(bytes.len() <= BYTE_LIMIT, "Provider response too large");
                    let frame: Value = serde_json::from_slice(&bytes)
                        .context("Persistent provider must return a JSON object per line")?;
                    ensure!(
                        frame["worker_protocol_version"] == 1
                            && frame["request_id"].as_u64() == Some(self.request_id)
                            && frame["response"].is_object(),
                        "Invalid persistent provider response envelope"
                    );
                    ensure!(response.is_none(), "Duplicate persistent provider response");
                    response = Some(frame["response"].clone());
                }
                Ok(Event::Error(result)) => {
                    let bytes = result.context("Cannot read provider stderr")?;
                    ensure!(bytes.len() <= BYTE_LIMIT, "Provider stderr too large");
                    ensure!(
                        bytes.is_empty(),
                        "Persistent provider closed stderr: {}",
                        String::from_utf8_lossy(&bytes)
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("Persistent provider I/O workers stopped")
                }
            }
            if input_done && let Some(response) = response.take() {
                return Ok(response);
            }
            if let Some(status) = self.process.as_mut().unwrap().0.try_wait()? {
                bail!("Persistent provider exited before completing request: {status}");
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn worker(script: &str) -> (tempfile::TempDir, Worker) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worker.py");
        std::fs::write(&path, script).unwrap();
        let worker = Worker::start(Path::new("python3"), &[path.display().to_string()]).unwrap();
        (directory, worker)
    }

    #[test]
    fn persistent_worker_rejects_wrong_request_id_and_cannot_reuse_failed_session() {
        let (_dir, mut worker) = worker(
            r#"import json,sys
for line in sys.stdin:
 frame=json.loads(line)
 print(json.dumps(dict(worker_protocol_version=1,request_id=frame['request_id']+1,response={})),flush=True)
"#,
        );
        let error = worker.call(&json!({}), Duration::from_secs(2)).unwrap_err();
        assert!(error.to_string().contains("envelope"), "{error}");
        assert!(
            worker
                .call(&json!({}), Duration::from_secs(2))
                .unwrap_err()
                .to_string()
                .contains("session has failed")
        );
    }

    #[test]
    fn persistent_worker_bounds_output_and_stderr_without_waiting_for_exit() {
        for stream in ["stdout", "stderr"] {
            let script = format!(
                "import sys,time\nsys.{stream}.write('x'*8_000_002)\nsys.{stream}.flush()\ntime.sleep(10)\n"
            );
            let (_dir, mut worker) = worker(&script);
            let started = Instant::now();
            let error = worker.call(&json!({}), Duration::from_secs(2)).unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains("limit") || message.contains("too large"),
                "{message}"
            );
            assert!(started.elapsed() < Duration::from_secs(2));
        }
    }

    #[test]
    fn dropping_worker_kills_descendants_that_keep_pipes_open() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("escaped");
        let script = format!(
            r#"import json,sys,subprocess
subprocess.Popen([sys.executable,'-c', 'import time,pathlib;time.sleep(.4);pathlib.Path({:?}).touch()'])
for line in sys.stdin:
 frame=json.loads(line)
 print(json.dumps(dict(worker_protocol_version=1,request_id=frame['request_id'],response={{}})),flush=True)
"#,
            marker.display().to_string()
        );
        let (_dir, mut worker) = worker(&script);
        worker.call(&json!({}), Duration::from_secs(2)).unwrap();
        drop(worker);
        std::thread::sleep(Duration::from_millis(650));
        assert!(!marker.exists());
    }
}
