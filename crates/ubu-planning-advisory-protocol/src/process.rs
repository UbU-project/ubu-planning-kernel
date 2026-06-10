use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::{GpuAdvisoryRequest, GpuAdvisoryResponse};

pub fn request_via_process(
    executable: &str,
    request: &GpuAdvisoryRequest,
) -> Result<GpuAdvisoryResponse> {
    let mut child = Command::new(executable)
        .arg("-m")
        .arg("ubu_gpu_advisory.main")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn advisory process '{executable}'"))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .context("failed to open advisory stdin")?;
        serde_json::to_writer(&mut stdin, request).context("failed to write advisory request")?;
        stdin
            .write_all(b"\n")
            .context("failed to finish advisory request")?;
    }

    let output = child
        .wait_with_output()
        .context("failed to read advisory response")?;
    if !output.status.success() {
        anyhow::bail!("advisory process exited with {}", output.status);
    }

    serde_json::from_slice(&output.stdout).context("failed to decode advisory response")
}
