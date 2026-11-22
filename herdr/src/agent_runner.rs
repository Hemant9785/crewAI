//! Separate-process runner for bounded team-agent requests.
//!
//! This module owns process lifecycle only. Repository authorization and
//! filesystem isolation are separate concerns and must be checked before a
//! caller constructs a request.

use crate::noninteractive_process;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const MAX_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug)]
pub(crate) struct AgentRequest {
    pub request_id: String,
    pub repository_id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub timeout: Duration,
    pub output_limit_bytes: usize,
}

impl AgentRequest {
    pub(crate) fn validate(&self) -> Result<(), RunnerError> {
        if self.request_id.trim().is_empty() {
            return Err(RunnerError::InvalidRequest("request ID"));
        }
        if self.repository_id.trim().is_empty() {
            return Err(RunnerError::InvalidRequest("repository ID"));
        }
        if self.program.as_os_str().is_empty() {
            return Err(RunnerError::InvalidRequest("program"));
        }
        if self.timeout.is_zero() || self.timeout > MAX_TIMEOUT {
            return Err(RunnerError::InvalidRequest("timeout outside allowed range"));
        }
        if self.output_limit_bytes == 0 || self.output_limit_bytes > MAX_OUTPUT_LIMIT_BYTES {
            return Err(RunnerError::InvalidRequest(
                "output limit outside allowed range",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentRunState {
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentResult {
    pub request_id: String,
    pub repository_id: String,
    pub state: AgentRunState,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub output_truncated: bool,
    pub started_at: u64,
    pub duration_ms: u128,
}

pub(crate) fn run_agent(
    request: AgentRequest,
    cancellation: &CancellationToken,
) -> Result<AgentResult, RunnerError> {
    request.validate()?;
    if cancellation.is_cancelled() {
        return Ok(cancelled_result(&request, current_unix_seconds(), 0));
    }
    let started_at = current_unix_seconds();
    let started = Instant::now();
    let mut command = noninteractive_process::command(&request.program);
    command
        .args(&request.args)
        .env_remove("HERDR_RELAY_TOKEN")
        .env_remove("HERDR_RELAY_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = &request.working_directory {
        command.current_dir(directory);
    }
    let mut child = command.spawn().map_err(RunnerError::Spawn)?;
    let stdout = child
        .stdout
        .take()
        .ok_or(RunnerError::MissingPipe("stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or(RunnerError::MissingPipe("stderr"))?;
    let stdout_reader = spawn_capped_reader(stdout, request.output_limit_bytes);
    let stderr_reader = spawn_capped_reader(stderr, request.output_limit_bytes);

    let state = loop {
        if cancellation.is_cancelled() {
            terminate_child(&mut child);
            break AgentRunState::Cancelled;
        }
        if started.elapsed() >= request.timeout {
            terminate_child(&mut child);
            break AgentRunState::TimedOut;
        }
        if child.try_wait().map_err(RunnerError::Wait)?.is_some() {
            break AgentRunState::Completed;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let status = child.wait().map_err(RunnerError::Wait)?;
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    let output_truncated = stdout.truncated || stderr.truncated;
    let state = if state == AgentRunState::Completed && !status.success() {
        AgentRunState::Failed
    } else {
        state
    };
    Ok(AgentResult {
        request_id: request.request_id,
        repository_id: request.repository_id,
        state,
        exit_code: exit_code(status),
        stdout: stdout.text,
        stderr: stderr.text,
        output_truncated,
        started_at,
        duration_ms: started.elapsed().as_millis(),
    })
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn cancelled_result(request: &AgentRequest, started_at: u64, duration_ms: u128) -> AgentResult {
    AgentResult {
        request_id: request.request_id.clone(),
        repository_id: request.repository_id.clone(),
        state: AgentRunState::Cancelled,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        output_truncated: false,
        started_at,
        duration_ms,
    }
}

struct CappedOutput {
    text: String,
    truncated: bool,
}

fn spawn_capped_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> thread::JoinHandle<CappedOutput> {
    thread::spawn(move || {
        let mut stored = Vec::with_capacity(limit.min(8192));
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if stored.len() < limit {
                        let keep = (limit - stored.len()).min(read);
                        stored.extend_from_slice(&buffer[..keep]);
                        if keep < read {
                            truncated = true;
                        }
                    } else {
                        truncated = true;
                    }
                }
                Err(_) => {
                    truncated = true;
                    break;
                }
            }
        }
        CappedOutput {
            text: String::from_utf8_lossy(&stored).into_owned(),
            truncated,
        }
    })
}

fn join_reader(reader: thread::JoinHandle<CappedOutput>) -> Result<CappedOutput, RunnerError> {
    reader.join().map_err(|_| RunnerError::ReaderPanicked)
}

fn exit_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Debug)]
pub(crate) enum RunnerError {
    InvalidRequest(&'static str),
    Spawn(std::io::Error),
    Wait(std::io::Error),
    MissingPipe(&'static str),
    ReaderPanicked,
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(field) => write!(formatter, "invalid agent request: {field}"),
            Self::Spawn(error) => write!(formatter, "agent process could not start: {error}"),
            Self::Wait(error) => write!(formatter, "agent process wait failed: {error}"),
            Self::MissingPipe(stream) => write!(formatter, "agent process {stream} pipe missing"),
            Self::ReaderPanicked => formatter.write_str("agent output reader panicked"),
        }
    }
}

impl std::error::Error for RunnerError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AgentRequest {
        AgentRequest {
            request_id: "request-1".into(),
            repository_id: "repo-1".into(),
            program: std::env::current_exe().unwrap(),
            args: vec!["--help".into()],
            working_directory: None,
            timeout: Duration::from_secs(5),
            output_limit_bytes: 1024,
        }
    }

    #[test]
    fn runner_is_a_separate_process_and_bounds_output() {
        let result = run_agent(request(), &CancellationToken::default()).unwrap();
        assert_eq!(result.state, AgentRunState::Completed);
        assert!(result.exit_code == Some(0) || result.exit_code == Some(101));
        assert!(result.stdout.len() <= 1024);
        assert_eq!(result.request_id, "request-1");
    }

    #[test]
    fn cancellation_before_start_does_not_spawn_a_process() {
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let result = run_agent(request(), &cancellation).unwrap();
        assert_eq!(result.state, AgentRunState::Cancelled);
        assert!(result.stdout.is_empty());
    }

    #[test]
    fn request_limits_are_rejected_before_process_start() {
        let mut request = request();
        request.timeout = Duration::ZERO;
        assert!(matches!(
            run_agent(request, &CancellationToken::default()),
            Err(RunnerError::InvalidRequest("timeout outside allowed range"))
        ));
    }
}
