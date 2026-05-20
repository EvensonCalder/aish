use std::io;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) fn configure_new_process_group(process: &mut Command) {
    configure_new_process_group_impl(process);
}

pub(crate) fn wait_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
    poll_interval: Duration,
    termination_grace: Duration,
) -> io::Result<(Option<ExitStatus>, bool)> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((Some(status), false));
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            terminate_child_process_group(child, termination_grace, poll_interval)?;
            return child.wait().map(|status| (Some(status), true));
        }
        let remaining = timeout.saturating_sub(elapsed);
        thread::sleep(remaining.min(poll_interval));
    }
}

pub(crate) fn terminate_child_process_group(
    child: &mut Child,
    grace: Duration,
    poll_interval: Duration,
) -> io::Result<()> {
    terminate_child_process_group_impl(child, grace, poll_interval)
}

#[cfg(unix)]
fn configure_new_process_group_impl(process: &mut Command) {
    use std::os::unix::process::CommandExt;

    process.process_group(0);
}

#[cfg(not(unix))]
fn configure_new_process_group_impl(_process: &mut Command) {}

#[cfg(unix)]
fn terminate_child_process_group_impl(
    child: &mut Child,
    grace: Duration,
    poll_interval: Duration,
) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    let pgid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }
    let started = Instant::now();
    while started.elapsed() < grace {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        let remaining = grace.saturating_sub(started.elapsed());
        thread::sleep(remaining.min(poll_interval));
    }
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.kill();
    Ok(())
}

#[cfg(not(unix))]
fn terminate_child_process_group_impl(
    child: &mut Child,
    _grace: Duration,
    _poll_interval: Duration,
) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    let _ = child.kill();
    Ok(())
}
