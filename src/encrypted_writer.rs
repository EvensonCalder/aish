use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;

use crate::encryption::{existing_jsonl_bytes, jsonl_bytes, rewrite_encrypted_jsonl_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptedWriteOperation {
    AppendJsonl,
    RewriteJsonl,
}

impl EncryptedWriteOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppendJsonl => "append-jsonl",
            Self::RewriteJsonl => "rewrite-jsonl",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedWriteEvent {
    pub operation: EncryptedWriteOperation,
    pub path: PathBuf,
    pub error: Option<String>,
}

enum EncryptedWriteJob {
    AppendJsonl {
        path: PathBuf,
        item_json: Vec<u8>,
    },
    RewriteJsonl {
        path: PathBuf,
        bytes: Vec<u8>,
    },
    ReplaceCache {
        entries: HashMap<PathBuf, Vec<u8>>,
    },
    Invalidate {
        paths: Vec<PathBuf>,
    },
    Flush {
        reply: mpsc::Sender<std::result::Result<(), String>>,
    },
    Stop,
}

pub struct EncryptedWriteQueue {
    sender: mpsc::Sender<EncryptedWriteJob>,
    events: mpsc::Receiver<EncryptedWriteEvent>,
    handle: Option<JoinHandle<()>>,
}

impl fmt::Debug for EncryptedWriteQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedWriteQueue")
            .finish_non_exhaustive()
    }
}

impl EncryptedWriteQueue {
    pub fn start(
        gpg_program: String,
        recipient: String,
        initial_cache: HashMap<PathBuf, Vec<u8>>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (event_sender, events) = mpsc::channel();
        let handle = thread::spawn(move || {
            run_worker(
                receiver,
                event_sender,
                gpg_program,
                recipient,
                initial_cache,
            );
        });
        Self {
            sender,
            events,
            handle: Some(handle),
        }
    }

    pub fn enqueue_append_jsonl<T: Serialize>(&self, path: &Path, item: &T) -> Result<()> {
        let mut item_json = Vec::new();
        serde_json::to_writer(&mut item_json, item).with_context(|| {
            format!(
                "failed to serialize encrypted JSONL item for {}",
                path.display()
            )
        })?;
        self.sender
            .send(EncryptedWriteJob::AppendJsonl {
                path: path.to_path_buf(),
                item_json,
            })
            .context("encrypted write queue is not running")
    }

    pub fn enqueue_rewrite_jsonl<T: Serialize>(&self, path: &Path, items: &[T]) -> Result<()> {
        let bytes = jsonl_bytes(items, path)?;
        self.enqueue_rewrite_jsonl_bytes(path, bytes)
    }

    pub fn enqueue_rewrite_jsonl_bytes(&self, path: &Path, bytes: Vec<u8>) -> Result<()> {
        self.sender
            .send(EncryptedWriteJob::RewriteJsonl {
                path: path.to_path_buf(),
                bytes,
            })
            .context("encrypted write queue is not running")
    }

    pub fn invalidate(&self, paths: Vec<PathBuf>) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.sender
            .send(EncryptedWriteJob::Invalidate { paths })
            .context("encrypted write queue is not running")
    }

    pub fn replace_cache(&self, entries: HashMap<PathBuf, Vec<u8>>) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        self.sender
            .send(EncryptedWriteJob::ReplaceCache { entries })
            .context("encrypted write queue is not running")
    }

    pub fn flush(&self) -> Result<()> {
        let (reply, receiver) = mpsc::channel();
        self.sender
            .send(EncryptedWriteJob::Flush { reply })
            .context("encrypted write queue is not running")?;
        match receiver
            .recv()
            .context("encrypted write queue stopped before flush completed")?
        {
            Ok(()) => Ok(()),
            Err(error) => Err(anyhow!(error)),
        }
    }

    pub fn drain_events(&self) -> Vec<EncryptedWriteEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for EncryptedWriteQueue {
    fn drop(&mut self) {
        let _ = self.sender.send(EncryptedWriteJob::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_worker(
    receiver: mpsc::Receiver<EncryptedWriteJob>,
    events: mpsc::Sender<EncryptedWriteEvent>,
    gpg_program: String,
    recipient: String,
    mut cache: HashMap<PathBuf, Vec<u8>>,
) {
    let mut failure: Option<String> = None;
    while let Ok(job) = receiver.recv() {
        match job {
            EncryptedWriteJob::AppendJsonl { path, item_json } => {
                let result = if let Some(error) = &failure {
                    Err(anyhow!(
                        "encrypted writer is stopped after a previous failure: {error}"
                    ))
                } else {
                    append_jsonl_item_with_cache(
                        &gpg_program,
                        &recipient,
                        &mut cache,
                        &path,
                        &item_json,
                    )
                };
                send_event(
                    &events,
                    EncryptedWriteOperation::AppendJsonl,
                    path,
                    result,
                    &mut failure,
                );
            }
            EncryptedWriteJob::RewriteJsonl { path, bytes } => {
                let result = if let Some(error) = &failure {
                    Err(anyhow!(
                        "encrypted writer is stopped after a previous failure: {error}"
                    ))
                } else {
                    rewrite_encrypted_jsonl_bytes(gpg_program.clone(), &recipient, &path, &bytes)
                        .map(|()| {
                            cache.insert(path.clone(), bytes);
                        })
                };
                send_event(
                    &events,
                    EncryptedWriteOperation::RewriteJsonl,
                    path,
                    result,
                    &mut failure,
                );
            }
            EncryptedWriteJob::Invalidate { paths } => {
                for path in paths {
                    cache.remove(&path);
                }
            }
            EncryptedWriteJob::ReplaceCache { entries } => {
                for (path, bytes) in entries {
                    cache.insert(path, bytes);
                }
            }
            EncryptedWriteJob::Flush { reply } => {
                let _ = reply.send(match &failure {
                    Some(error) => Err(error.clone()),
                    None => Ok(()),
                });
            }
            EncryptedWriteJob::Stop => break,
        }
    }
}

fn append_jsonl_item_with_cache(
    gpg_program: &str,
    recipient: &str,
    cache: &mut HashMap<PathBuf, Vec<u8>>,
    path: &Path,
    item_json: &[u8],
) -> Result<()> {
    let mut bytes = match cache.get(path) {
        Some(bytes) => bytes.clone(),
        None => existing_jsonl_bytes(gpg_program, path)?,
    };
    bytes.extend_from_slice(item_json);
    bytes.push(b'\n');
    rewrite_encrypted_jsonl_bytes(gpg_program.to_string(), recipient, path, &bytes)?;
    cache.insert(path.to_path_buf(), bytes);
    Ok(())
}

fn send_event(
    events: &mpsc::Sender<EncryptedWriteEvent>,
    operation: EncryptedWriteOperation,
    path: PathBuf,
    result: Result<()>,
    failure: &mut Option<String>,
) {
    let error = result.err().map(|error| error.to_string());
    if failure.is_none() {
        *failure = error.clone();
    }
    let _ = events.send(EncryptedWriteEvent {
        operation,
        path,
        error,
    });
}
