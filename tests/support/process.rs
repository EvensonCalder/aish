#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) struct RunLimiter {
    active: Mutex<usize>,
    available: Condvar,
    max: usize,
}

pub(crate) struct RunPermit {
    limiter: &'static RunLimiter,
}

pub(crate) struct ScriptOutput {
    pub(crate) output: Output,
    pub(crate) timed_out: bool,
}

impl RunLimiter {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            active: Mutex::new(0),
            available: Condvar::new(),
            max,
        }
    }

    pub(crate) fn acquire(&'static self) -> RunPermit {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *active >= self.max {
            active = self
                .available
                .wait(active)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *active += 1;
        RunPermit { limiter: self }
    }
}

impl Drop for RunPermit {
    fn drop(&mut self) {
        let mut active = self
            .limiter
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *active = active.saturating_sub(1);
        self.limiter.available.notify_one();
    }
}

pub(crate) fn default_test_jobs(env_name: &str) -> usize {
    if let Ok(raw) = std::env::var(env_name)
        && let Ok(value) = raw.parse::<usize>()
    {
        return value.max(1);
    }

    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().saturating_div(2).clamp(1, 4))
        .unwrap_or(2)
}

pub(crate) fn script_timeout(env_name: &str, default_seconds: u64) -> Duration {
    let seconds = std::env::var(env_name)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default_seconds)
        .max(1);
    Duration::from_secs(seconds)
}

pub(crate) fn run_with_timeout<F>(
    mut command: Command,
    timeout: Duration,
    mut on_timeout: F,
) -> std::io::Result<ScriptOutput>
where
    F: FnMut(),
{
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let start = Instant::now();

    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map(|output| ScriptOutput {
                output,
                timed_out: false,
            });
        }

        if start.elapsed() >= timeout {
            on_timeout();
            signal_process_tree(child.id(), "TERM");
            std::thread::sleep(Duration::from_millis(500));
            if child.try_wait()?.is_none() {
                signal_process_tree(child.id(), "KILL");
                let _ = child.kill();
            }
            return child.wait_with_output().map(|output| ScriptOutput {
                output,
                timed_out: true,
            });
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn signal_process_tree(pid: u32, signal: &str) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
