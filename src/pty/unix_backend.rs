use std::fs::File;
use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail};

use super::PtySize;
use super::control::{CONTROL_FD, set_cloexec};

pub(super) struct UnixPtyBackend {
    master: File,
    child: Child,
}

impl UnixPtyBackend {
    pub(super) fn spawn(
        mut command: Command,
        size: PtySize,
        control_fd: Option<RawFd>,
    ) -> Result<Self> {
        let (master, slave) = openpty(size)?;
        set_cloexec(master.as_raw_fd(), true).context("failed to mark PTY master close-on-exec")?;

        let stdin = slave
            .try_clone()
            .context("failed to clone PTY slave stdin")?;
        let stdout = slave
            .try_clone()
            .context("failed to clone PTY slave stdout")?;
        let stderr = slave
            .try_clone()
            .context("failed to clone PTY slave stderr")?;
        command
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));

        unsafe {
            command.pre_exec(move || {
                reset_child_signal_state();

                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }

                if let Some(fd) = control_fd {
                    if fd != CONTROL_FD && libc::dup2(fd, CONTROL_FD) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let _ = libc::fcntl(CONTROL_FD, libc::F_SETFD, 0);
                    if fd != CONTROL_FD {
                        let _ = libc::close(fd);
                    }
                }

                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }

                Ok(())
            });
        }

        let child = command.spawn().context("failed to spawn backend shell")?;
        drop(slave);

        Ok(Self { master, child })
    }

    pub(super) fn clone_writer(&self) -> Result<File> {
        self.master
            .try_clone()
            .context("failed to clone PTY master writer")
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }

    pub(super) fn read_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        let mut buf = [0_u8; 4096];
        match self.master.read(&mut buf) {
            Ok(0) => Ok(None),
            Ok(n) => Ok(Some(buf[..n].to_vec())),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => Ok(Some(Vec::new())),
            Err(err) => Err(err).context("failed to read PTY"),
        }
    }

    pub(super) fn resize(&self, size: PtySize) -> Result<()> {
        let winsize = winsize_from_pty_size(size);
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &winsize) } == -1 {
            return Err(std::io::Error::last_os_error()).context("failed to resize PTY");
        }
        Ok(())
    }

    pub(super) fn size(&self) -> Result<PtySize> {
        let mut winsize = MaybeUninit::<libc::winsize>::uninit();
        if unsafe {
            libc::ioctl(
                self.master.as_raw_fd(),
                libc::TIOCGWINSZ as _,
                winsize.as_mut_ptr(),
            )
        } == -1
        {
            return Err(std::io::Error::last_os_error()).context("failed to read PTY size");
        }
        let winsize = unsafe { winsize.assume_init() };
        Ok(PtySize {
            rows: winsize.ws_row,
            cols: winsize.ws_col,
            pixel_width: winsize.ws_xpixel,
            pixel_height: winsize.ws_ypixel,
        })
    }
}

impl Drop for UnixPtyBackend {
    fn drop(&mut self) {
        let pgid = self.child.id() as libc::pid_t;
        unsafe {
            let _ = libc::kill(-pgid, libc::SIGTERM);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn openpty(size: PtySize) -> Result<(File, File)> {
    let mut master = -1;
    let mut slave = -1;
    let winsize = winsize_from_pty_size(size);
    let status = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &winsize,
        )
    };
    if status != 0 {
        bail!("failed to open PTY: {}", std::io::Error::last_os_error());
    }

    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    Ok((master, slave))
}

fn winsize_from_pty_size(size: PtySize) -> libc::winsize {
    libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: size.pixel_width,
        ws_ypixel: size.pixel_height,
    }
}

fn reset_child_signal_state() {
    for signal in [
        libc::SIGCHLD,
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGQUIT,
        libc::SIGTERM,
        libc::SIGALRM,
        libc::SIGTSTP,
    ] {
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
        }
    }

    let empty = MaybeUninit::<libc::sigset_t>::zeroed();
    unsafe {
        libc::sigprocmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut());
    }
}

impl Read for UnixPtyBackend {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.master.read(buf)
    }
}

impl Write for UnixPtyBackend {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.master.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.master.flush()
    }
}
