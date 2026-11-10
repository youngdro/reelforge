//! What is done to a clip's sound beyond its volume: a changed voice, a moved pitch, noise taken
//! out.

use reelforge_time::{TICKS_PER_SECOND, Tick};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A ready-made changed voice.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum VoiceKind {
    /// Higher, a lighter voice.
    High,
    /// Lower, a deeper voice.
    Low,
    /// Much higher, like tape run fast.
    Chipmunk,
    /// Much lower and slow in the throat.
    Monster,
    /// A metallic buzz on every word.
    Robot,
    /// Thin, as down a telephone line.
    Phone,
    /// As through a hand-held loudspeaker: narrow and a little rough.
    Megaphone,
    /// Each word comes back a few times.
    Echo,
}

/// One part of what a changed voice does to the sound.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum VoicePart {
    /// Only what lies between two frequencies (Hz) is kept.
    Band(f32, f32),
    /// The sound is multiplied by a sine of a frequency (Hz), mixed in by an amount from 0 to 1.
    Ring(f32, f32),
    /// The sound comes back after a delay, each time quieter by a factor, a number of times.
    Echo(Tick, f32, u32),
}

impl VoiceKind {
    /// Every voice, in menu order.
    pub const ALL: [VoiceKind; 8] = [
        VoiceKind::High,
        VoiceKind::Low,
        VoiceKind::Chipmunk,
        VoiceKind::Monster,
        VoiceKind::Robot,
        VoiceKind::Phone,
        VoiceKind::Megaphone,
        VoiceKind::Echo,
    ];

    /// The name commands and files use.
    pub fn name(self) -> &'static str {
        match self {
            VoiceKind::High => "high",
            VoiceKind::Low => "low",
            VoiceKind::Chipmunk => "chipmunk",
            VoiceKind::Monster => "monster",
            VoiceKind::Robot => "robot",
            VoiceKind::Phone => "phone",
            VoiceKind::Megaphone => "megaphone",
            VoiceKind::Echo => "echo",
        }
    }

    /// The voice of that name.
    pub fn parse(name: &str) -> Option<VoiceKind> {
        VoiceKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// How many semitones the voice moves the pitch.
    pub fn pitch(self) -> f32 {
        match self {
            VoiceKind::High => 4.0,
            VoiceKind::Low => -4.0,
            VoiceKind::Chipmunk => 9.0,
            VoiceKind::Monster => -8.0,
            _ => 0.0,
        }
    }

    /// What else the voice does, in order.
    pub fn parts(self) -> &'static [VoicePart] {
        match self {
            VoiceKind::Robot => &[VoicePart::Ring(55.0, 0.85)],
            VoiceKind::Phone => &[VoicePart::Band(320.0, 3400.0)],
            VoiceKind::Megaphone => &[VoicePart::Band(550.0, 2800.0), VoicePart::Ring(1400.0, 0.12)],
            VoiceKind::Echo => &[VoicePart::Echo(Tick(TICKS_PER_SECOND * 22 / 100), 0.45, 4)],
            VoiceKind::Monster => &[VoicePart::Ring(28.0, 0.25)],
            _ => &[],
        }
    }
}

/// Background noise taken out of a clip's sound.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Denoise {
    /// How much is taken out, from 0 to 1.
    pub amount: f32,
    /// What the noise is like: its power density in dB in each of [`Denoise::BANDS`] bands (see
    /// RenderSpec's `AudioEffect::Denoise`), as measured from the clip's quietest moments.
    pub floor: Vec<f32>,
}

impl Denoise {
    /// How many bands a noise profile has.
    pub const BANDS: usize = 24;
    /// How much is taken out when nothing else is asked.
    pub const AMOUNT: f32 = 0.7;
}

/// A clip's sound, changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Voice {
    /// A ready-made voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<VoiceKind>,
    /// Semitones up (or, negative, down), on top of the voice's own.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pitch: f32,
    /// Noise taken out, before anything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denoise: Option<Denoise>,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

impl Voice {
    /// The furthest the pitch can be moved by hand, in semitones either way.
    pub const PITCH_RANGE: f32 = 12.0;

    /// Nothing is changed.
    pub fn is_empty(&self) -> bool {
        self.kind.is_none() && self.pitch == 0.0 && self.denoise.is_none()
    }

    /// The whole move in pitch: the voice's and the hand's.
    pub fn semitones(&self) -> f32 {
        self.kind.map_or(0.0, VoiceKind::pitch) + self.pitch
    }

    /// What is wrong with it, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        if !(self.pitch.is_finite() && self.pitch.abs() <= Voice::PITCH_RANGE) {
            return Some("pitch must be from -12 to 12 semitones");
        }
        if let Some(d) = &self.denoise {
            if !(d.amount.is_finite() && (0.0..=1.0).contains(&d.amount)) {
                return Some("denoise amount must be from 0 to 1");
            }
            if d.floor.len() != Denoise::BANDS || d.floor.iter().any(|v| !v.is_finite()) {
                return Some("a noise profile has 24 bands");
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_have_names_and_stay_in_range() {
        for k in VoiceKind::ALL {
            assert_eq!(VoiceKind::parse(k.name()), Some(k));
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(k.name()));
            let v = Voice { kind: Some(k), pitch: 12.0, denoise: None };
            assert!(v.semitones().abs() <= 24.0 && v.problem().is_none());
            assert!(k.pitch() != 0.0 || !k.parts().is_empty());
        }
        assert!(Voice::default().is_empty());
        assert!(Voice { pitch: 13.0, ..Voice::default() }.problem().is_some());
        let short = Denoise { amount: 0.5, floor: vec![-90.0; 3] };
        assert!(Voice { denoise: Some(short), ..Voice::default() }.problem().is_some());
    }
}
