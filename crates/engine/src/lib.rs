//! The engine: one session, one command registry, one dispatcher.
//!
//! Every way of changing a project goes through [`Session::execute`]: the interface's buttons and
//! drags, the command line, an MCP agent, a script. A command is a [`CommandSpec`] with a stable id
//! (`clip.split`), a label, a parameter description that agents read, an `enabled` check that
//! explains itself, and a `run` function. Nothing about what an edit means lives in a client, so
//! two clients cannot disagree, and a command added here reaches all of them.
//!
//! Edits are copy-on-write: the project is an `Arc<Project>`, an edit clones it, applies the change,
//! checks the invariants and only then commits, pushing the old snapshot onto the undo stack. A
//! reader (a preview, an export) holds a snapshot and never blocks an edit.

mod beat;
mod canvas;
mod capcut;
mod captions;
mod commands;
mod compound;
mod cut;
mod decor;
mod layout;
pub mod listen;
mod outline;
mod params;
mod pieces;
mod promo;
mod remix;
mod schema;
mod script;
mod shared;
pub mod space;
mod storyboard;
mod templates;
mod voiceover;

use std::sync::Arc;

use reelforge_model::Project;
use serde_json::{Value, json};
use thiserror::Error;

pub use beat::{MOVES as BEAT_MOVES, Move as BeatMove};
pub use canvas::RATIOS;
pub use captions::{CAPTIONS_TRACK, CAPTIONS_TRACK_2, Cue, format_srt, parse_srt};
pub use commands::{CommandSpec, media_uses, registry};
pub use layout::{LAYOUTS, Layout};
pub use params::parse_time;
pub use pieces::{Category, PROMOS, Promo};
pub use promo::made_of;
pub use remix::{TimelineTemplate, is_empty_slot, slot_colour, slot_states, timeline_template, timeline_templates};
pub use schema::input_schema;
pub use shared::{Change, SharedSession};
pub use storyboard::{Field, Lang, Said, Scene, Storyboard, field_label, scene_label, storyboards};
pub use templates::{TitleTemplate, title_template, title_templates};
pub use voiceover::{Spoken, VOICEOVER_TRACK, spoken_lines};

/// How many undo steps a session keeps.
pub const UNDO_DEPTH: usize = 200;

/// Why a command failed. Nothing changed.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EngineError {
    /// No command has this id.
    #[error("unknown command {0:?}")]
    Unknown(String),
    /// The command cannot run now; the message says why.
    #[error("{0}")]
    Disabled(String),
    /// The parameters are wrong; the message names the field.
    #[error("invalid parameters: {0}")]
    Params(String),
    /// The edit would break the project.
    #[error("{0}")]
    Refused(String),
}

/// Result of a command.
pub type CmdResult = Result<Value, EngineError>;

/// An open project and its history.
pub struct Session {
    project: Arc<Project>,
    undo: Vec<(String, Arc<Project>)>,
    redo: Vec<(String, Arc<Project>)>,
    revision: u64,
}

impl Default for Session {
    fn default() -> Session {
        Session::new(Project::default())
    }
}

impl Session {
    /// A session on `project`.
    pub fn new(project: Project) -> Session {
        Session { project: Arc::new(project), undo: Vec::new(), redo: Vec::new(), revision: 0 }
    }

    /// The current project snapshot. Cheap; hold it as long as needed.
    pub fn project(&self) -> Arc<Project> {
        self.project.clone()
    }

    /// Bumped by every committed change (edits, undo, redo). Clients compare it to know when to
    /// redraw, and a remote client sends the revision it last saw.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Labels of the undo stack, oldest first.
    pub fn history(&self) -> Vec<String> {
        self.undo.iter().map(|(l, _)| l.clone()).collect()
    }

    /// Runs the command `id` with `params` (an object, or null for none).
    pub fn execute(&mut self, id: &str, params: &Value) -> CmdResult {
        let spec = registry().iter().find(|c| c.id == id).ok_or_else(|| EngineError::Unknown(id.to_string()))?;
        let empty = json!({});
        let params = if params.is_null() { &empty } else { params };
        if !params.is_object() {
            return Err(EngineError::Params("parameters must be a JSON object".into()));
        }
        schema::check_names(id, params).map_err(EngineError::Params)?;
        (spec.enabled)(self).map_err(EngineError::Disabled)?;
        (spec.run)(self, params)
    }

    /// Applies `change` to a copy of the project; commits it as one undo step named `label` only if
    /// it succeeds and the result passes [`Project::check`].
    pub fn edit<T>(
        &mut self,
        label: &str,
        change: impl FnOnce(&mut Project) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let mut next = (*self.project).clone();
        let out = change(&mut next)?;
        next.sort_clips();
        next.drop_stray_scripts();
        next.pack_magnetic();
        next.check().map_err(|e| EngineError::Refused(e.to_string()))?;
        if let Some(t) = next.changed_locked_track(&self.project) {
            return Err(EngineError::Refused(format!("track {:?} is locked", t.name)));
        }
        let old = std::mem::replace(&mut self.project, Arc::new(next));
        self.undo.push((label.to_string(), old));
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision += 1;
        Ok(out)
    }

    /// Takes `project` as the current state, from the session this one mirrors (an editor joined
    /// to a server). Not an edit: it has no undo step, and the history is dropped, since the
    /// session it came from keeps the real one.
    pub fn adopt(&mut self, project: Project) {
        self.project = Arc::new(project);
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
    }

    fn swap(&mut self, undo: bool) -> CmdResult {
        let (from, to) = if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some((label, snapshot)) = from.pop() else {
            return Err(EngineError::Disabled(format!("nothing to {}", if undo { "undo" } else { "redo" })));
        };
        let current = std::mem::replace(&mut self.project, snapshot);
        to.push((label.clone(), current));
        self.revision += 1;
        Ok(json!({ "label": label }))
    }
}

/// The captions track, for tests.
#[cfg(test)]
fn captions_track_for_tests(s: &Session) -> String {
    captions::captions_track(&s.project()).expect("a text track")
}

#[cfg(test)]
mod tests;
