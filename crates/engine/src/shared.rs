//! One session edited by several clients at once: the editor, MCP agents, HTTP callers.
//!
//! Commands still run one at a time on one [`Session`]; this adds who made each change, a way to
//! hear about it (listeners, for the editor to repaint and a server to save), and a way to wait for
//! it (long polling, for remote clients).

use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use reelforge_model::Project;
use serde_json::Value;

use crate::{CmdResult, Session};

/// A committed change, as clients are told about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// The session revision after it.
    pub revision: u64,
    /// The command that made it.
    pub command: String,
    /// Who ran it: `editor`, `agent <MCP client name>`, `http`, `cli`.
    pub by: String,
}

type Listener = Box<dyn Fn(&Change, &Arc<Project>) + Send + Sync>;

/// A [`Session`] behind a lock, with change notifications.
pub struct SharedSession {
    session: Mutex<Session>,
    last: Mutex<Change>,
    changed: Condvar,
    listeners: Mutex<Vec<Listener>>,
}

impl SharedSession {
    /// Shares `session`.
    pub fn new(session: Session) -> Arc<SharedSession> {
        let revision = session.revision();
        Arc::new(SharedSession {
            session: Mutex::new(session),
            last: Mutex::new(Change { revision, ..Change::default() }),
            changed: Condvar::new(),
            listeners: Mutex::default(),
        })
    }

    /// The session itself, for reads. Hold the guard briefly: every client waits on it. Edits made
    /// through it directly are not announced; use [`execute`](Self::execute).
    pub fn lock(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The current project snapshot.
    pub fn project(&self) -> Arc<Project> {
        self.lock().project()
    }

    /// The current revision.
    pub fn revision(&self) -> u64 {
        self.lock().revision()
    }

    /// The last change (revision 0 and no command before any).
    pub fn last_change(&self) -> Change {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Runs a command for `by`, and announces the change if it made one.
    pub fn execute(&self, id: &str, params: &Value, by: &str) -> CmdResult {
        let (out, change) = {
            let mut s = self.lock();
            let before = s.revision();
            let out = s.execute(id, params)?;
            let after = s.revision();
            let change = (after != before)
                .then(|| (Change { revision: after, command: id.to_string(), by: by.to_string() }, s.project()));
            if let Some((c, _)) = &change {
                // still under the session lock, so a waiter that sees the revision sees this too
                *self.last.lock().unwrap_or_else(|e| e.into_inner()) = c.clone();
            }
            (out, change)
        };
        if let Some((change, project)) = change {
            self.changed.notify_all();
            // outside the session lock, so a listener may read the session
            for l in self.listeners.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                l(&change, &project);
            }
        }
        Ok(out)
    }

    /// Calls `f` after every change, with the project as it now is. It runs on the thread that made
    /// the change, so it should be quick (request a repaint, save a file).
    pub fn on_change(&self, f: impl Fn(&Change, &Arc<Project>) + Send + Sync + 'static) {
        self.listeners.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(f));
    }

    /// Waits up to `timeout` for a revision after `since`; returns the last change either way.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_for_change(&self, since: u64, timeout: std::time::Duration) -> Change {
        let guard = self.lock();
        let _guard = self
            .changed
            .wait_timeout_while(guard, timeout, |s| s.revision() <= since)
            .map(|(g, _)| g)
            .unwrap_or_else(|e| e.into_inner().0);
        self.last_change()
    }
}
