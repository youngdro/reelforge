//! The editor joined to a session a server holds (`reelforge serve`, or a desktop editor started with
//! `--serve`), so a person in a browser edits the same project as agents and other editors.
//!
//! The editor keeps working on its own [`SharedSession`], a mirror of the server's. An edit runs on
//! the mirror first, so the interface answers at once and gets its result (the new clip's id), and
//! is then sent to the server, in order, one at a time. Ids are minted from the project, so the same
//! edit on the same state names things the same on both sides.
//!
//! Whenever nothing is waiting to be sent, the mirror takes the server's project as it is (when it
//! differs): after the editor's own edits are answered, and when the server says someone else made
//! a change (a long poll on `/v1/events`). An edit the server refused, or one that met someone
//! else's, is thereby replaced by what the server holds. Undo and redo run on the server only: its
//! history holds everyone's edits.
//!
//! Media the project uses is fetched from the server by hash; a file dropped on the editor is sent
//! to the server first and imported by the path the server keeps it at.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use reelforge_engine::SharedSession;
use reelforge_media::Footage;
use reelforge_model::Project;
use serde_json::{Value, json};

/// How long a long poll for changes waits on the server, in seconds.
const POLL_SECONDS: u64 = 25;
/// How long to wait before trying again after the server could not be reached, in seconds.
const RETRY_SECONDS: f64 = 2.0;

/// Something for the status bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    /// The server's project arrived.
    Joined(String),
    /// The server could not be reached: its address, and why.
    Unreachable(String, String),
    /// Something the server refused or that failed: what (a command, a file; may be empty), why.
    Failed(String, String),
    /// A file the project uses that the server has no copy of: its name, and the HTTP status.
    NoCopy(String, String),
}

impl Note {
    /// The note in `lang`.
    pub fn text(&self, lang: crate::i18n::Lang) -> String {
        use crate::i18n::fill;
        let s = lang.strings();
        match self {
            Note::Joined(base) => fill(s.joined, &[base]),
            Note::Unreachable(base, why) => fill(s.unreachable, &[base, why]),
            Note::Failed(what, why) if what.is_empty() => why.clone(),
            Note::Failed(what, why) => format!("{what}: {why}"),
            Note::NoCopy(name, status) => fill(s.no_copy, &[name, status]),
        }
    }
}

/// A file sent to the server, ready to import.
#[derive(Clone, Debug, PartialEq)]
pub struct Uploaded {
    /// `media.import` parameters: the server's path, the name, kind, hash and measurements.
    pub import: Value,
    /// The file's name.
    pub name: String,
    /// Whether it goes on the timeline too (it was dropped), not only into the media (Import).
    pub place: bool,
}

/// A connection from an editor to a server's session.
#[derive(Clone)]
pub struct Remote {
    inner: Arc<Inner>,
}

struct Inner {
    base: String,
    token: Option<String>,
    /// How this client names itself to the server (`web`, `desktop`).
    client: String,
    mirror: Arc<SharedSession>,
    media: Arc<Footage>,
    wake: Box<dyn Fn() + Send + Sync>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Edits not yet sent, oldest first.
    queue: VecDeque<(String, Value)>,
    /// An edit is with the server.
    sending: bool,
    /// The project is being fetched.
    fetching: bool,
    /// The server changed while edits were pending: fetch once they are answered.
    stale: bool,
    /// A long poll is waiting on the server.
    polling: bool,
    /// The last server revision heard of.
    revision: u64,
    /// Who made the last change heard of, and with what.
    last: Option<(String, String)>,
    joined: bool,
    /// When to try the server again (seconds on the editor's clock), after it could not be reached.
    retry_at: Option<f64>,
    /// Media hashes the mirror's footage has, or is fetching.
    have: HashSet<String>,
    /// Bumped whenever footage arrives, so the preview redraws.
    media_epoch: u64,
    uploaded: Vec<Uploaded>,
    /// Messages for the status bar.
    notes: Vec<Note>,
}

impl Remote {
    /// Joins the session at `base` (`http://127.0.0.1:7878`): `mirror` takes the server's project
    /// and follows it; `media` gets the files it uses. `wake` is called when something arrived (on
    /// any thread): the editor should repaint, and call [`tick`](Self::tick).
    pub fn join(
        base: &str,
        token: Option<String>,
        client: &str,
        mirror: Arc<SharedSession>,
        media: Arc<Footage>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Remote {
        let remote = Remote {
            inner: Arc::new(Inner {
                base: base.trim_end_matches('/').to_string(),
                token: token.filter(|t| !t.is_empty()),
                client: client.to_string(),
                mirror,
                media,
                wake: Box::new(wake),
                state: Mutex::default(),
            }),
        };
        remote.fetch();
        remote
    }

    /// The server's address.
    pub fn base(&self) -> &str {
        &self.inner.base
    }

    /// Whether the server's project has arrived.
    pub fn joined(&self) -> bool {
        self.state().joined
    }

    /// Whether every edit has been answered and the mirror holds what the server held then.
    pub fn settled(&self) -> bool {
        let s = self.state();
        s.joined && s.queue.is_empty() && !s.sending && !s.fetching && !s.stale
    }

    /// Changes whenever footage arrives.
    pub fn media_epoch(&self) -> u64 {
        self.state().media_epoch
    }

    /// Status bar messages since the last call.
    pub fn take_notes(&self) -> Vec<Note> {
        std::mem::take(&mut self.state().notes)
    }

    /// Files sent to the server since the last call, to import.
    pub fn take_uploaded(&self) -> Vec<Uploaded> {
        std::mem::take(&mut self.state().uploaded)
    }

    /// Sends an edit that already ran on the mirror.
    pub fn send(&self, id: &str, params: &Value) {
        self.state().queue.push_back((id.to_string(), params.clone()));
        self.pump();
    }

    /// Restarts what stopped when the server could not be reached, once it is time. `now` is the
    /// editor's clock in seconds. Returns how long until it wants to be called again, if it does.
    pub fn tick(&self, now: f64) -> Option<f64> {
        let retry = {
            let mut s = self.state();
            match s.retry_at {
                // just failed: start counting now
                Some(at) if at.is_infinite() => {
                    s.retry_at = Some(now + RETRY_SECONDS);
                    return Some(RETRY_SECONDS);
                }
                Some(at) if now >= at => {
                    s.retry_at = None;
                    true
                }
                Some(at) => return Some(at - now),
                None => false,
            }
        };
        if retry {
            if self.joined() {
                self.pump();
                self.settle();
                self.poll();
            } else {
                self.fetch();
            }
        }
        None
    }

    /// Hands a file to the mirror's footage and sends it to the server; when it is there, it shows
    /// up in [`take_uploaded`](Self::take_uploaded).
    pub fn upload(&self, name: &str, bytes: Vec<u8>, place: bool) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        let probe = self.inner.media.insert_bytes(&sha, name, bytes.clone())?;
        self.state().have.insert(sha.clone());
        let kind = match probe.kind {
            reelforge_renderspec::AssetKind::Video => "video",
            reelforge_renderspec::AssetKind::Image => "image",
            reelforge_renderspec::AssetKind::Lut => "lut",
            reelforge_renderspec::AssetKind::Font => "font",
            _ => "audio",
        };
        // a font is called by its family's name
        let mut import = json!({"name": probe.family.as_deref().unwrap_or(name), "kind": kind, "sha256": sha});
        if let Some(size) = probe.size {
            import["size"] = json!(size);
        }
        if let Some(d) = probe.duration {
            import["duration"] = json!(d);
        }
        let url = format!("{}/v1/media/{sha}?name={}", self.inner.base, encode(name));
        let this = self.clone();
        let name = name.to_string();
        ehttp::fetch(self.request(ehttp::Request::put(url, bytes)), move |r| {
            match answer(r) {
                Ok(v) => {
                    import["path"] = v["path"].clone();
                    this.state().uploaded.push(Uploaded { import, name, place });
                }
                Err(e) => this.note(Note::Failed(name.clone(), e.to_string())),
            }
            (this.inner.wake)();
        });
        Ok(())
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn note(&self, msg: Note) {
        self.state().notes.push(msg);
    }

    fn request(&self, mut r: ehttp::Request) -> ehttp::Request {
        if let Some(t) = &self.inner.token {
            r.headers.insert("Authorization", format!("Bearer {t}"));
        }
        r.headers.insert("X-Reelforge-Client", &self.inner.client);
        r
    }

    /// The server could not be reached: try again in a moment.
    fn unreachable(&self, e: &str) {
        let first = {
            let mut s = self.state();
            let first = s.retry_at.is_none();
            // the editor's clock is not known here: tick() sets the real time on its next call
            s.retry_at = Some(f64::NEG_INFINITY);
            first
        };
        if first {
            self.note(Note::Unreachable(self.inner.base.clone(), e.to_string()));
        }
        (self.inner.wake)();
    }

    /// Sends the next edit, if none is with the server.
    fn pump(&self) {
        let next = {
            let mut s = self.state();
            if s.sending || !s.joined || s.retry_at.is_some() {
                return;
            }
            let Some(next) = s.queue.pop_front() else { return };
            s.sending = true;
            next
        };
        let (id, params) = next;
        let url = format!("{}/v1/ops/{id}", self.inner.base);
        let body = serde_json::to_vec(&params).expect("serialises");
        let mut req = ehttp::Request::post(url, body);
        req.headers.insert("Content-Type", "application/json");
        let this = self.clone();
        ehttp::fetch(self.request(req), move |r| {
            let lost = match answer(r) {
                Ok(_) => None,
                Err(Answer::Unreachable(e)) => Some((id, params, e)),
                Err(e) => {
                    // refused there: what the server holds replaces it when the queue drains
                    this.note(Note::Failed(id.clone(), e.to_string()));
                    None
                }
            };
            {
                let mut s = this.state();
                s.sending = false;
                s.stale = true;
                if let Some((id, params, _)) = &lost {
                    s.queue.push_front((id.clone(), params.clone()));
                }
            }
            match lost {
                Some((_, _, e)) => this.unreachable(&e),
                None => {
                    this.pump();
                    this.settle();
                }
            }
        });
    }

    /// Fetches the server's project once nothing is pending.
    fn settle(&self) {
        let go = {
            let s = self.state();
            s.stale && s.queue.is_empty() && !s.sending && !s.fetching
        };
        if go {
            self.fetch();
        }
    }

    fn fetch(&self) {
        {
            let mut s = self.state();
            if s.fetching {
                return;
            }
            s.fetching = true;
            s.stale = false;
        }
        let url = format!("{}/v1/project", self.inner.base);
        let this = self.clone();
        ehttp::fetch(self.request(ehttp::Request::get(url)), move |r| {
            let got = answer(r).and_then(|v| {
                let revision = v["result"]["revision"].as_u64().unwrap_or(0);
                serde_json::from_value::<Project>(v["result"]["project"].clone())
                    .map(|p| (revision, p))
                    .map_err(|e| Answer::Refused(format!("the server sent a project this editor cannot read: {e}")))
            });
            let (revision, project) = match got {
                Ok(x) => x,
                Err(e) => {
                    {
                        let mut s = this.state();
                        s.fetching = false;
                        s.stale = s.joined;
                    }
                    match e {
                        Answer::Unreachable(e) => this.unreachable(&e),
                        e => this.note(Note::Failed(String::new(), e.to_string())),
                    }
                    return;
                }
            };
            let (first, adopt, last) = {
                let mut s = this.state();
                s.fetching = false;
                let first = !s.joined;
                s.joined = true;
                s.revision = s.revision.max(revision);
                // edits made meanwhile ran on the mirror and are on their way: this is older
                let adopt = s.queue.is_empty() && !s.sending && !s.stale;
                (first, adopt, s.last.clone())
            };
            if adopt && *this.inner.mirror.project() != project {
                let (by, command) = last.unwrap_or_else(|| ("server".into(), "sync".into()));
                this.inner.mirror.adopt(project, &command, &by);
            }
            this.fetch_media();
            if first {
                this.note(Note::Joined(this.inner.base.clone()));
                this.poll();
                this.pump();
            }
            this.settle();
            (this.inner.wake)();
        });
    }

    /// Waits on the server for a change past the last revision heard of, forever.
    fn poll(&self) {
        let since = {
            let mut s = self.state();
            if s.polling || s.retry_at.is_some() {
                return;
            }
            s.polling = true;
            s.revision
        };
        let url = format!("{}/v1/events?since={since}&timeout={POLL_SECONDS}", self.inner.base);
        let mut req = self.request(ehttp::Request::get(url));
        req.timeout = Some(std::time::Duration::from_secs(POLL_SECONDS + 15));
        let this = self.clone();
        ehttp::fetch(req, move |r| {
            this.state().polling = false;
            match answer(r) {
                Ok(v) => {
                    let revision = v["revision"].as_u64().unwrap_or(0);
                    {
                        let mut s = this.state();
                        if revision > s.revision {
                            s.revision = revision;
                            s.stale = true;
                            let text = |k: &str| v[k].as_str().unwrap_or_default().to_string();
                            s.last = Some((text("by"), text("command")));
                        }
                    }
                    this.settle();
                    this.poll();
                }
                Err(Answer::Unreachable(e)) => this.unreachable(&e),
                Err(e) => {
                    this.note(Note::Failed(String::new(), e.to_string()));
                    this.unreachable("the change feed failed");
                }
            }
        });
    }

    /// Fetches the files the project uses that the mirror's footage lacks.
    fn fetch_media(&self) {
        let project = self.inner.mirror.project();
        let wanted: Vec<(String, String)> = {
            let mut s = self.state();
            project
                .media
                .values()
                // library sounds and stickers are made here, not kept by the server
                .filter(|m| !m.uri.starts_with(reelforge_media::library::SCHEME))
                // a name tells what kind of file it is; a font is called by its family
                .filter_map(|m| {
                    let font = m.kind == reelforge_model::MediaKind::Font;
                    Some((m.sha256.clone()?, if font { format!("{}.ttf", m.name) } else { m.name.clone() }))
                })
                .filter(|(sha, _)| s.have.insert(sha.clone()))
                .collect()
        };
        for (sha, name) in wanted {
            let url = format!("{}/v1/media/{sha}", self.inner.base);
            let this = self.clone();
            ehttp::fetch(self.request(ehttp::Request::get(url)), move |r| {
                match r {
                    Ok(r) if r.ok => match this.inner.media.insert_bytes(&sha, &name, r.bytes) {
                        Ok(_) => this.state().media_epoch += 1,
                        Err(e) => this.note(Note::Failed(name.clone(), e)),
                    },
                    Ok(r) => this.note(Note::NoCopy(name.clone(), format!("{} {}", r.status, r.status_text))),
                    Err(e) => {
                        // try again with the next change
                        this.state().have.remove(&sha);
                        this.note(Note::Failed(name.clone(), e));
                    }
                }
                (this.inner.wake)();
            });
        }
    }
}

/// How a request went wrong.
enum Answer {
    /// No answer: the server is down or the network is.
    Unreachable(String),
    /// The server answered no.
    Refused(String),
}

impl std::fmt::Display for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Answer::Unreachable(e) | Answer::Refused(e) => f.write_str(e),
        }
    }
}

/// The JSON of a successful answer.
fn answer(r: ehttp::Result<ehttp::Response>) -> Result<Value, Answer> {
    let r = r.map_err(Answer::Unreachable)?;
    let v: Value = serde_json::from_slice(&r.bytes).unwrap_or(Value::Null);
    if !r.ok {
        let why = v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("{} {}", r.status, r.status_text));
        return Err(Answer::Refused(why));
    }
    Ok(v)
}

/// Percent-encodes a query value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b => format!("%{b:02X}"),
        })
        .collect()
}
