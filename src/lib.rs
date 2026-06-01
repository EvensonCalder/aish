pub mod ai;
pub mod app;
pub mod commands;
pub mod completion;
pub mod completion_worker;
pub mod config;
pub mod context;
mod display_width;
pub mod editor;
pub mod encrypted_writer;
pub mod encryption;
mod env_name;
mod git_remote;
pub mod history;
pub mod input;
pub mod keybindings;
pub mod log;
pub mod modes;
pub mod paste;
pub mod picker;
mod process_control;
pub mod pty;
pub mod shell_completion;
pub mod shell_integration;
pub mod sync;
pub mod templates;
pub mod terminal;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Mutex, MutexGuard};

    static SHELL_PROCESS_TEST_MUTEX: Mutex<()> = Mutex::new(());

    pub(crate) struct ShellProcessTestGuard {
        _mutex: MutexGuard<'static, ()>,
        #[cfg(unix)]
        lock_file: std::fs::File,
    }

    pub(crate) fn shell_process_test_guard() -> ShellProcessTestGuard {
        let mutex = SHELL_PROCESS_TEST_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        #[cfg(unix)]
        {
            let path = std::env::temp_dir().join("aish-shell-process-test.lock");
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(path)
                .expect("failed to open shell process test lock");
            lock_shell_process_file(&lock_file);
            ShellProcessTestGuard {
                _mutex: mutex,
                lock_file,
            }
        }
        #[cfg(not(unix))]
        {
            ShellProcessTestGuard { _mutex: mutex }
        }
    }

    #[cfg(unix)]
    fn lock_shell_process_file(file: &std::fs::File) {
        use std::os::fd::AsRawFd;

        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        assert_eq!(
            result,
            0,
            "failed to acquire shell process test lock: {}",
            std::io::Error::last_os_error()
        );
    }

    #[cfg(unix)]
    impl Drop for ShellProcessTestGuard {
        fn drop(&mut self) {
            use std::os::fd::AsRawFd;

            let _ = unsafe { libc::flock(self.lock_file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    app::run()
}
