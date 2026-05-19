use std::env;
use std::ffi::OsString;

pub(crate) struct EnvVarGuard {
    name: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub(crate) fn set(name: &'static str, value: impl Into<OsString>) -> Self {
        let previous = env::var_os(name);
        unsafe {
            env::set_var(name, value.into());
        }
        Self { name, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe {
                env::set_var(self.name, value);
            },
            None => unsafe {
                env::remove_var(self.name);
            },
        }
    }
}
