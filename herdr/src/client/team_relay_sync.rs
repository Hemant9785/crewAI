//! Keeps the local team mailbox current while the Herdr workspace is open.
//!
//! Sync runs through the existing `relay sync` command in a child process so
//! it never blocks terminal rendering or input. Relay credentials stay in the
//! process environment and are never persisted or placed on the command line.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{debug, warn};

const BASE_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(5 * 60);
const NO_CONFIGURATION_INTERVAL: Duration = Duration::from_secs(10);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_SYNC_RUNTIME: Duration = Duration::from_secs(60);

pub(super) struct BackgroundRelaySync {
    stopped: Arc<AtomicBool>,
    credential_sink: RelayCredentialSink,
    worker: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub(crate) struct RelayCredentialSink {
    credentials: Arc<Mutex<Option<(String, String)>>>,
    worker_thread: Arc<Mutex<Option<thread::Thread>>>,
}

impl RelayCredentialSink {
    pub(super) fn set(&self, endpoint: String, token: String) -> bool {
        if endpoint.trim().is_empty() || token.trim().is_empty() {
            return false;
        }
        let Ok(mut credentials) = self.credentials.lock() else {
            return false;
        };
        *credentials = Some((endpoint, token));
        if let Ok(worker_thread) = self.worker_thread.lock() {
            if let Some(worker_thread) = worker_thread.as_ref() {
                worker_thread.unpark();
            }
        }
        true
    }

    pub(crate) fn get(&self) -> Option<(String, String)> {
        self.credentials.lock().ok()?.clone()
    }
}

impl BackgroundRelaySync {
    pub(super) fn start() -> Option<Self> {
        let executable = std::env::current_exe().ok()?;
        let database = crate::team_view::default_database_path();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let credential_sink = RelayCredentialSink {
            credentials: Arc::new(Mutex::new(None)),
            worker_thread: Arc::new(Mutex::new(None)),
        };
        let worker_credentials = credential_sink.clone();
        let worker = thread::Builder::new()
            .name("herdr-team-relay-sync".into())
            .spawn(move || {
                if let Ok(mut worker_thread) = worker_credentials.worker_thread.lock() {
                    *worker_thread = Some(thread::current());
                }
                run_worker(worker_stopped, executable, database, worker_credentials)
            })
            .ok()?;
        Some(Self {
            stopped,
            credential_sink,
            worker: Some(worker),
        })
    }

    pub(super) fn credential_sink(&self) -> RelayCredentialSink {
        self.credential_sink.clone()
    }
}

impl Drop for BackgroundRelaySync {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

fn run_worker(
    stopped: Arc<AtomicBool>,
    executable: PathBuf,
    database: PathBuf,
    credential_sink: RelayCredentialSink,
) {
    let mut retry_interval = BASE_RETRY_INTERVAL;
    while !stopped.load(Ordering::Acquire) {
        let Some((endpoint, token)) = credential_sink.get().or_else(relay_configuration) else {
            if wait_or_stop(&stopped, NO_CONFIGURATION_INTERVAL) {
                break;
            }
            continue;
        };

        let Some(team_id) = load_team_id(&database) else {
            if wait_or_stop(&stopped, NO_CONFIGURATION_INTERVAL) {
                break;
            }
            continue;
        };

        if sync_once(
            &stopped,
            &executable,
            &database,
            &team_id,
            &endpoint,
            &token,
        ) {
            retry_interval = BASE_RETRY_INTERVAL;
        } else {
            warn!("background team relay sync failed; it will retry with backoff");
            retry_interval = next_retry_interval(retry_interval);
        }

        if wait_or_stop(&stopped, retry_interval) {
            break;
        }
    }
}

fn relay_configuration() -> Option<(String, String)> {
    let endpoint = std::env::var("HERDR_RELAY_URL").ok()?;
    let token = std::env::var("HERDR_RELAY_TOKEN").ok()?;
    if endpoint.trim().is_empty() || token.trim().is_empty() {
        return None;
    }
    Some((endpoint, token))
}

fn load_team_id(database: &Path) -> Option<String> {
    crate::team_view::load_from_path(database)
        .ok()?
        .teams
        .into_iter()
        .next()
        .map(|team| team.team_id)
}

fn next_retry_interval(current: Duration) -> Duration {
    current
        .saturating_mul(2)
        .min(MAX_RETRY_INTERVAL)
        .max(BASE_RETRY_INTERVAL)
}

fn wait_or_stop(stopped: &AtomicBool, duration: Duration) -> bool {
    if stopped.load(Ordering::Acquire) {
        return true;
    }
    thread::park_timeout(duration);
    stopped.load(Ordering::Acquire)
}

fn sync_once(
    stopped: &AtomicBool,
    executable: &Path,
    database: &Path,
    team_id: &str,
    endpoint: &str,
    token: &str,
) -> bool {
    if stopped.load(Ordering::Acquire) {
        return false;
    }

    let database_arg = database.to_string_lossy().into_owned();
    let mut command = Command::new(executable);
    command
        .args([
            "relay",
            "sync",
            "--db",
            database_arg.as_str(),
            "--team",
            team_id,
            "--url",
            endpoint,
        ])
        .env(crate::team_view::TEAM_DB_ENV, database)
        .env("HERDR_RELAY_URL", endpoint)
        .env("HERDR_RELAY_TOKEN", token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            debug!(%error, "could not start background relay sync command");
            return false;
        }
    };

    let deadline = Instant::now() + MAX_SYNC_RUNTIME;
    loop {
        if stopped.load(Ordering::Acquire) {
            stop_child(&mut child);
            return false;
        }
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(CHILD_POLL_INTERVAL);
            }
            Ok(None) => {
                warn!("background team relay sync exceeded its time limit");
                stop_child(&mut child);
                return false;
            }
            Err(error) => {
                debug!(%error, "could not check background relay sync process");
                stop_child(&mut child);
                return false;
            }
        }
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_interval_doubles_and_caps() {
        assert_eq!(
            next_retry_interval(BASE_RETRY_INTERVAL),
            Duration::from_secs(60)
        );
        assert_eq!(next_retry_interval(MAX_RETRY_INTERVAL), MAX_RETRY_INTERVAL);
        assert_eq!(
            next_retry_interval(Duration::from_secs(1)),
            BASE_RETRY_INTERVAL
        );
    }

    #[test]
    fn stop_interrupts_the_worker_wait() {
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = ready_tx.send(thread::current());
            wait_or_stop(&worker_stopped, Duration::from_secs(60))
        });
        let worker_thread = ready_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker started");
        stopped.store(true, Ordering::Release);
        worker_thread.unpark();
        assert!(worker.join().expect("worker exited"));
    }

    #[test]
    fn dashboard_credentials_wake_and_update_the_worker_in_memory() {
        let sink = RelayCredentialSink {
            credentials: Arc::new(Mutex::new(None)),
            worker_thread: Arc::new(Mutex::new(None)),
        };
        let worker_sink = sink.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            if let Ok(mut worker_thread) = worker_sink.worker_thread.lock() {
                *worker_thread = Some(thread::current());
            }
            let _ = ready_tx.send(());
            thread::park();
            worker_sink.get()
        });
        ready_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker parked");

        assert!(sink.set("https://relay.example".into(), "secret-token".into()));
        assert_eq!(
            worker.join().expect("worker woke"),
            Some(("https://relay.example".into(), "secret-token".into()))
        );
    }

    #[test]
    fn empty_dashboard_credentials_are_rejected_without_replacing_existing_values() {
        let sink = RelayCredentialSink {
            credentials: Arc::new(Mutex::new(Some((
                "https://relay.example".into(),
                "secret-token".into(),
            )))),
            worker_thread: Arc::new(Mutex::new(None)),
        };

        assert!(!sink.set(" ".into(), "replacement".into()));
        assert_eq!(
            sink.get(),
            Some(("https://relay.example".into(), "secret-token".into()))
        );
    }
}
