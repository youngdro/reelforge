//! Transitions between neighbouring clips.
//!
//! A transition belongs to the clip it leads into, and plays when that clip starts exactly where
//! the clip before it on the same picture track ends. It is centred on the cut: the outgoing clip
//! runs on for half its length and the incoming one starts half its length early, holding their
//! first and last frames where the media has nothing more. The compiler draws it with opacity and
//! placement keys, and a colour layer for the dips (`reelforge-compile`).

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anim::Offset;

/// How one clip gives way to the next.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransitionKind {
    /// The next clip fades in over the last.
    Dissolve,
    /// Down to black and up again.
    FadeBlack,
    /// A flash of white.
    FadeWhite,
    /// The next clip pushes the last out to the left.
    PushLeft,
    /// …to the right.
    PushRight,
    /// …upwards.
    PushUp,
    /// …downwards.
    PushDown,
    /// The next clip slides in from the right over the last.
    SlideLeft,
    /// …from the left.
    SlideRight,
    /// …from below.
    SlideUp,
    /// …from above.
    SlideDown,
    /// The last clip grows away as the next settles in.
    Zoom,
}

impl TransitionKind {
    /// Every kind, in menu order.
    pub const ALL: [TransitionKind; 12] = [
        TransitionKind::Dissolve,
        TransitionKind::FadeBlack,
        TransitionKind::FadeWhite,
        TransitionKind::PushLeft,
        TransitionKind::PushRight,
        TransitionKind::PushUp,
        TransitionKind::PushDown,
        TransitionKind::SlideLeft,
        TransitionKind::SlideRight,
        TransitionKind::SlideUp,
        TransitionKind::SlideDown,
        TransitionKind::Zoom,
    ];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            TransitionKind::Dissolve => "dissolve",
            TransitionKind::FadeBlack => "fadeBlack",
            TransitionKind::FadeWhite => "fadeWhite",
            TransitionKind::PushLeft => "pushLeft",
            TransitionKind::PushRight => "pushRight",
            TransitionKind::PushUp => "pushUp",
            TransitionKind::PushDown => "pushDown",
            TransitionKind::SlideLeft => "slideLeft",
            TransitionKind::SlideRight => "slideRight",
            TransitionKind::SlideUp => "slideUp",
            TransitionKind::SlideDown => "slideDown",
            TransitionKind::Zoom => "zoom",
        }
    }

    /// The kind a command names.
    pub fn parse(name: &str) -> Option<TransitionKind> {
        TransitionKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The colour a dip passes through, straight sRGB.
    pub fn dip(self) -> Option<[f32; 4]> {
        match self {
            TransitionKind::FadeBlack => Some([0.0, 0.0, 0.0, 1.0]),
            TransitionKind::FadeWhite => Some([1.0, 1.0, 1.0, 1.0]),
            _ => None,
        }
    }

    /// What the transition does to the incoming clip (`incoming`) or the outgoing one, `p` of the
    /// way through (0 to 1).
    pub fn offset(self, p: f32, incoming: bool) -> Offset {
        let p = p.clamp(0.0, 1.0);
        let e = p * p * (3.0 - 2.0 * p);
        let mut o = Offset::REST;
        // push: both move a whole frame; slide: only the incoming one does
        let (dx, dy) = match self {
            TransitionKind::PushLeft | TransitionKind::SlideLeft => (-1.0, 0.0),
            TransitionKind::PushRight | TransitionKind::SlideRight => (1.0, 0.0),
            TransitionKind::PushUp | TransitionKind::SlideUp => (0.0, -1.0),
            TransitionKind::PushDown | TransitionKind::SlideDown => (0.0, 1.0),
            _ => (0.0, 0.0),
        };
        let push = matches!(
            self,
            TransitionKind::PushLeft | TransitionKind::PushRight | TransitionKind::PushUp | TransitionKind::PushDown
        );
        match (self, incoming) {
            (TransitionKind::Dissolve, true) => o.opacity = p,
            (TransitionKind::FadeBlack | TransitionKind::FadeWhite, true) => {
                o.opacity = if p < 0.5 { 0.0 } else { 1.0 }
            }
            (TransitionKind::Zoom, true) => {
                o.opacity = p;
                o.scale = 1.3 - 0.3 * e;
            }
            (TransitionKind::Zoom, false) => o.scale = 1.0 + 0.4 * e,
            (_, true) if dx != 0.0 || dy != 0.0 => {
                o.dx = -dx * (1.0 - e);
                o.dy = -dy * (1.0 - e);
            }
            (_, false) if push => {
                o.dx = dx * e;
                o.dy = dy * e;
            }
            _ => {}
        }
        o
    }
}

/// A transition into a clip.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transition {
    /// Which kind.
    pub kind: TransitionKind,
    /// How long it takes, centred on the cut; positive.
    pub duration: Tick,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_end_at_rest_and_start_with_the_last_clip_on_screen() {
        for k in TransitionKind::ALL {
            assert_eq!(TransitionKind::parse(k.name()), Some(k));
            for incoming in [true, false] {
                let end = k.offset(1.0, incoming);
                if incoming {
                    assert!(
                        end.dx == 0.0 && end.dy == 0.0 && (end.scale - 1.0).abs() < 1e-6 && end.opacity == 1.0,
                        "{k:?}"
                    );
                }
                let start = k.offset(0.0, incoming);
                if !incoming {
                    assert_eq!(start, Offset::REST, "{k:?}");
                }
            }
            // at the start nothing of the incoming clip covers the outgoing one
            let s = k.offset(0.0, true);
            assert!(s.opacity == 0.0 || s.dx.abs() >= 1.0 || s.dy.abs() >= 1.0, "{k:?}");
        }
    }
}
