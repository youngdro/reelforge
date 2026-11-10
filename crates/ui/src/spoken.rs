//! Trimming speech: the pauses heard in the clips someone speaks in and the captions that are
//! only a filler word, listed to be ticked and cut out of the timeline together.

use reelforge_time::Tick;

/// What a stretch to cut is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Nothing is heard.
    Pause,
    /// A caption that is only filler words.
    Filler,
}

/// One stretch of the timeline that could go.
#[derive(Clone, Debug)]
pub(crate) struct Found {
    pub kind: Kind,
    pub from: Tick,
    pub to: Tick,
    /// A filler's words.
    pub text: String,
    /// Ticked: it goes when the cut is made.
    pub on: bool,
}

/// The window's state.
pub(crate) struct Spoken {
    pub open: bool,
    /// The shortest silence that counts as a pause, in seconds.
    pub min_pause: f32,
    /// How much of each pause stays beside the words, in seconds.
    pub keep: f32,
    /// Audio tracks none of the spoken clips is on are not cut.
    pub leave_music: bool,
    pub found: Vec<Found>,
    /// The clips that were listened to.
    pub clips: Vec<String>,
    /// The project's revision when `found` was found; it is good for that one only.
    pub revision: Option<u64>,
    /// The clips had no sound to hear pauses in.
    pub unheard: bool,
}

impl Default for Spoken {
    fn default() -> Spoken {
        Spoken {
            open: false,
            min_pause: 0.5,
            keep: 0.1,
            leave_music: true,
            found: Vec::new(),
            clips: Vec::new(),
            revision: None,
            unheard: false,
        }
    }
}

impl Spoken {
    /// Forgets what was found.
    pub(crate) fn clear(&mut self) {
        self.found.clear();
        self.clips.clear();
        self.revision = None;
        self.unheard = false;
    }

    /// The ticked stretches in order, with the ones that touch or overlap joined. So are two
    /// with less than a fifth of a second between them: that is the quiet left beside a filler
    /// word that goes too, not something said.
    pub(crate) fn chosen(&self) -> Vec<(Tick, Tick)> {
        let sliver = Tick(reelforge_time::TICKS_PER_SECOND / 5);
        let mut ranges: Vec<(Tick, Tick)> = self.found.iter().filter(|f| f.on).map(|f| (f.from, f.to)).collect();
        ranges.sort();
        let mut out: Vec<(Tick, Tick)> = Vec::new();
        for (a, b) in ranges {
            match out.last_mut() {
                Some(last) if a < last.1 + sliver => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        out
    }

    /// How many stretches are ticked, and how long they are together.
    pub(crate) fn total(&self) -> (usize, Tick) {
        let time = self.chosen().iter().fold(Tick::ZERO, |sum, (a, b)| sum + (*b - *a));
        (self.found.iter().filter(|f| f.on).count(), time)
    }
}
