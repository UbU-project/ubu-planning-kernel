//! Owns and reaps one local repository worker, reused within a bounded session.
use crate::{
    invalid, read_frame, reference_message, request_message, write_frame, PlanningStreamFrame,
    PlanningTransport,
};
use serde_json::{json, Value};
use std::{
    io,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use ubu_planning_core::{PlanningRequest, PlanningResponse};

pub struct WorkerSession {
    child: Child,
    input: Option<ChildStdin>,
    output: mpsc::Receiver<io::Result<Value>>,
    reader: Option<thread::JoinHandle<()>>,
    timeout: Duration,
}
impl WorkerSession {
    /// The only executable is a caller-selected Python interpreter; its module
    /// and import root are fixed to this repository. No shell or user payload.
    pub fn spawn(python: &str, timeout: Duration) -> io::Result<Self> {
        if timeout.is_zero() || timeout > Duration::from_secs(30) {
            return Err(invalid("worker timeout must be in (0, 30s]"));
        }
        let module_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gpu-advisory/src");
        let mut child = Command::new(python)
            .args(["-B", "-u", "-m", "ubu_planning_worker.main"])
            .env_clear()
            .env("PYTHONPATH", module_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| invalid("missing stdout"))?;
        let (sender, output) = mpsc::channel();
        let mut session = Self {
            child,
            input,
            output,
            reader: None,
            timeout,
        };
        // Owner guard exists before any fallible thread creation or later panic.
        session.reader = Some(
            thread::Builder::new()
                .name("planning-worker-reader".into())
                .spawn(move || {
                    let mut reader = stdout;
                    loop {
                        match read_frame(&mut reader) {
                            Ok(Some(value)) => {
                                if sender.send(Ok(value)).is_err() {
                                    break;
                                }
                            }
                            Ok(None) => {
                                let _ = sender.send(Err(io::Error::new(
                                    io::ErrorKind::UnexpectedEof,
                                    "worker exited",
                                )));
                                break;
                            }
                            Err(error) => {
                                let _ = sender.send(Err(error));
                                break;
                            }
                        }
                    }
                })?,
        );
        Ok(session)
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn send(&mut self, value: &Value) -> io::Result<()> {
        write_frame(
            self.input
                .as_mut()
                .ok_or_else(|| invalid("worker closed"))?,
            value,
        )
    }
    pub fn receive(&mut self) -> io::Result<PlanningStreamFrame> {
        let value = match self.output.recv_timeout(self.timeout) {
            Ok(value) => value?,
            Err(_) => {
                self.stop();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "worker response timeout",
                ));
            }
        };
        let frame: PlanningStreamFrame = serde_json::from_value(value).map_err(invalid)?;
        frame.validate().map_err(invalid)?;
        Ok(frame)
    }
    pub fn cancel(
        &mut self,
        request: &PlanningRequest,
        reference: &PlanningResponse,
    ) -> io::Result<PlanningStreamFrame> {
        self.send(&request_message(request, reference)?)?;
        self.send(&json!({"kind":"cancel", "payload":{"request_id":request.request_id}}))?;
        self.receive()
    }
    /// Terminates only this owned child. Drop always waits and joins the reader.
    pub fn stop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl PlanningTransport for WorkerSession {
    fn exchange(
        &mut self,
        request: &PlanningRequest,
        reference: &PlanningResponse,
    ) -> io::Result<PlanningStreamFrame> {
        let result = (|| {
            self.send(&request_message(request, reference)?)?;
            self.send(&reference_message(reference)?)?;
            self.receive()
        })();
        if result.is_err() {
            self.stop();
        }
        result
    }
}
impl Drop for WorkerSession {
    fn drop(&mut self) {
        self.stop();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
