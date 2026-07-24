//! Manages the dcs-app serve child process.
//!
//! Wraps a single `dcs-app serve` subprocess, providing async JSON
//! request/response exchange over stdin/stdout.

use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Manages a single `dcs-app serve` child process.
///
/// Wraps stdin/stdout for JSON request/response exchange. The process is
/// spawned with `kill_on_drop(true)` so it is cleaned up when this struct
/// is dropped.
#[derive(Clone)]
pub struct GameProcess {
    inner: Arc<Mutex<GameProcessInner>>,
}

struct GameProcessInner {
    child: Child,
    stdin: BufWriter<tokio::process::ChildStdin>,
    reader: BufReader<tokio::process::ChildStdout>,
}

impl GameProcess {
    /// Spawn a new `dcs-app serve` process.
    ///
    /// # Errors
    ///
    /// Returns an error if the process fails to spawn.
    pub async fn spawn() -> Result<Self> {
        let child = Command::new("cargo")
            .args(["run", "--bin", "dcs-app", "--", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to spawn dcs-app serve")?;

        let stdin = child.stdin.unwrap();
        let stdout = child.stdout.unwrap();
        let stderr = child.stderr.take().unwrap();

        // Log stderr output from the game server
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "dcs-server", "{}", line);
            }
        });

        let inner = GameProcessInner {
            child,
            stdin: BufWriter::new(stdin),
            reader: BufReader::new(stdout),
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    /// Send a JSON request and receive a JSON response.
    ///
    /// Writes the request as a single line to stdin, reads one line from
    /// stdout, and parses the JSON response.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, I/O, or parsing fails.
    pub async fn send_request(&self, request: &Value) -> Result<Value> {
        let mut guard = self.inner.lock().await;

        // Serialize request as single line
        let mut line =
            serde_json::to_string(request).context("Failed to serialize request")?;
        line.push('\n');

        guard
            .stdin
            .write_all(line.as_bytes())
            .await
            .context("Failed to write request to stdin")?;
        guard
            .stdin
            .flush()
            .await
            .context("Failed to flush stdin")?;

        // Read response line
        let mut response_line = String::new();
        guard
            .reader
            .read_line(&mut response_line)
            .await
            .context("Failed to read response from stdout")?;

        if response_line.is_empty() {
            anyhow::bail!("Game process exited (EOF on stdout)");
        }

        let response: Value = serde_json::from_str(response_line.trim())
            .context(format!("Failed to parse response: {}", response_line.trim()))?;

        Ok(response)
    }

    /// Kill the current process and spawn a new one.
    ///
    /// # Errors
    ///
    /// Returns an error if the respawn fails.
    pub async fn restart(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;

        // Kill existing process
        let _ = guard.child.kill().await;

        // Wait for it to exit
        let _ = guard.child.wait().await;

        // Spawn new process
        let child = Command::new("cargo")
            .args(["run", "--bin", "dcs-app", "--", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to respawn dcs-app serve")?;

        let stdin = child.stdin.unwrap();
        let stdout = child.stdout.unwrap();
        let stderr = child.stderr.take().unwrap();

        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "dcs-server", "{}", line);
            }
        });

        guard.child = child;
        guard.stdin = BufWriter::new(stdin);
        guard.reader = BufReader::new(stdout);

        Ok(())
    }
}
