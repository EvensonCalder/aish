use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};

use anyhow::{Context, Result};

pub(super) const CONTROL_FD: RawFd = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ControlChannelClosed;

impl std::fmt::Display for ControlChannelClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PTY control channel closed")
    }
}

impl std::error::Error for ControlChannelClosed {}

pub(super) struct ControlChannel {
    read: File,
}

pub(super) struct ChildControlFd {
    file: File,
}

impl ControlChannel {
    pub(super) fn create() -> Result<(Self, ChildControlFd)> {
        let mut fds = [-1; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("failed to create control pipe");
        }

        let read = unsafe { File::from_raw_fd(fds[0]) };
        let write = unsafe { File::from_raw_fd(fds[1]) };
        set_cloexec(read.as_raw_fd(), true)
            .context("failed to mark control reader close-on-exec")?;
        set_cloexec(write.as_raw_fd(), true)
            .context("failed to mark control writer close-on-exec")?;
        set_nonblocking(read.as_raw_fd(), true)
            .context("failed to set control reader nonblocking")?;
        let child_write = duplicate_cloexec(write.as_raw_fd(), CONTROL_FD)
            .context("failed to move control writer away from stdio setup fds")?;
        drop(write);

        Ok((Self { read }, ChildControlFd { file: child_write }))
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.read.as_raw_fd()
    }

    pub(super) fn read_available(&mut self) -> Result<Vec<Vec<u8>>> {
        let mut chunks = Vec::new();
        let mut buf = [0_u8; 4096];
        loop {
            match self.read.read(&mut buf) {
                Ok(0) => return Err(ControlChannelClosed.into()),
                Ok(n) => chunks.push(buf[..n].to_vec()),
                Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(chunks),
                Err(err) if err.kind() == ErrorKind::Interrupted => {}
                Err(err) => return Err(err).context("failed to read PTY control channel"),
            }
        }
    }
}

impl ChildControlFd {
    pub(super) fn raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

pub(super) fn set_cloexec(fd: RawFd, enabled: bool) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to read fd flags");
    }
    let next = if enabled {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, next) } < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to update fd flags");
    }
    Ok(())
}

pub(super) fn set_nonblocking(fd: RawFd, enabled: bool) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to read fd status flags");
    }
    let next = if enabled {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFL, next) } < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to update fd status flags");
    }
    Ok(())
}

fn duplicate_cloexec(fd: RawFd, min_fd: RawFd) -> Result<File> {
    let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, min_fd) };
    if duplicated < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to duplicate fd");
    }
    Ok(unsafe { File::from_raw_fd(duplicated) })
}
