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
use crate::fx::{FxChannel, FxSample};
use crate::look::{Mask, MaskKind};

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
    /// The next clip is uncovered by an edge travelling leftwards, soft along the edge.
    WipeLeft,
    /// …rightwards.
    WipeRight,
    /// …upwards.
    WipeUp,
    /// …downwards.
    WipeDown,
    /// The next clip shows through a circle that opens from the middle.
    Circle,
    /// …a box, shaped like the picture.
    Box,
    /// …a diamond.
    Diamond,
    /// The last clip turns away in space about its vertical like a card, and the next is on its
    /// back.
    Flip,
    /// …about its horizontal.
    FlipUp,
    /// The next clip is turned in from the right edge like the page of a book.
    PageTurn,
    /// A warm light passes over the picture as the next clip fades in (漏光).
    LightLeak,
    /// The picture tears into bands that jump sideways, its colours apart, and the next clip is
    /// there when it settles (故障).
    Glitch,
    /// The camera goes into the last clip out of focus and comes back out of the next one.
    ZoomBlur,
}

impl TransitionKind {
    /// Every kind, in menu order.
    pub const ALL: [TransitionKind; 25] = [
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
        TransitionKind::WipeLeft,
        TransitionKind::WipeRight,
        TransitionKind::WipeUp,
        TransitionKind::WipeDown,
        TransitionKind::Circle,
        TransitionKind::Box,
        TransitionKind::Diamond,
        TransitionKind::Flip,
        TransitionKind::FlipUp,
        TransitionKind::PageTurn,
        TransitionKind::LightLeak,
        TransitionKind::Glitch,
        TransitionKind::ZoomBlur,
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
            TransitionKind::WipeLeft => "wipeLeft",
            TransitionKind::WipeRight => "wipeRight",
            TransitionKind::WipeUp => "wipeUp",
            TransitionKind::WipeDown => "wipeDown",
            TransitionKind::Circle => "circle",
            TransitionKind::Box => "box",
            TransitionKind::Diamond => "diamond",
            TransitionKind::Flip => "flip",
            TransitionKind::FlipUp => "flipUp",
            TransitionKind::PageTurn => "pageTurn",
            TransitionKind::LightLeak => "lightLeak",
            TransitionKind::Glitch => "glitch",
            TransitionKind::ZoomBlur => "zoomBlur",
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

    /// Whether the next clip is uncovered through a shape ([`reveal`](Self::reveal)).
    pub fn reveals(self) -> bool {
        matches!(
            self,
            TransitionKind::WipeLeft
                | TransitionKind::WipeRight
                | TransitionKind::WipeUp
                | TransitionKind::WipeDown
                | TransitionKind::Circle
                | TransitionKind::Box
                | TransitionKind::Diamond
        )
    }

    /// The shape the incoming clip shows through `p` of the way through (0 to 1), on a picture
    /// `size` wide and high in units of its shorter side: nothing of it at 0, all of it at 1.
    /// `None` for the kinds that uncover nothing.
    pub fn reveal(self, p: f32, size: [f32; 2]) -> Option<Mask> {
        let p = p.clamp(0.0, 1.0);
        let e = p * p * (3.0 - 2.0 * p);
        let [w, h] = size;
        // an edge is soft; it starts and ends clear of the picture
        let wipe = |turn: f32, x: f32, y: f32| Mask {
            kind: MaskKind::Linear,
            center: [x, y],
            size: [1.0, 1.0],
            rotation: turn,
            feather: 0.08,
            invert: false,
        };
        let (near, far) = (-0.06, 1.06);
        let along = near + (far - near) * e;
        let open = |kind: MaskKind, turn: f32, full: [f32; 2]| Mask {
            kind,
            center: [0.5, 0.5],
            size: [(full[0] + 0.05) * e, (full[1] + 0.05) * e],
            rotation: turn,
            feather: 0.04,
            invert: false,
        };
        let across = (w * w + h * h).sqrt();
        Some(match self {
            // a line mask shows what is above it, as it lies; turned, what is to one side
            TransitionKind::WipeDown => wipe(0.0, 0.5, along),
            TransitionKind::WipeUp => wipe(180.0, 0.5, 1.0 - along),
            TransitionKind::WipeRight => wipe(-90.0, along, 0.5),
            TransitionKind::WipeLeft => wipe(90.0, 1.0 - along, 0.5),
            TransitionKind::Circle => open(MaskKind::Circle, 0.0, [across, across]),
            TransitionKind::Box => open(MaskKind::Rectangle, 0.0, [w, h]),
            // a square on its corner reaches a corner of the picture when its side is this
            TransitionKind::Diamond => {
                let side = (w + h) / std::f32::consts::SQRT_2;
                open(MaskKind::Rectangle, 45.0, [side, side])
            }
            _ => return None,
        })
    }

    /// Whether the transition changes `channel` of a clip's look ([`look`](Self::look)).
    pub fn touches(self, channel: FxChannel) -> bool {
        match self {
            TransitionKind::Glitch => {
                matches!(channel, FxChannel::Glitch | FxChannel::GlitchSeed | FxChannel::Split)
            }
            TransitionKind::ZoomBlur => channel == FxChannel::Blur,
            TransitionKind::LightLeak => channel == FxChannel::Exposure,
            _ => false,
        }
    }

    /// What the transition does to the look of both clips `p` of the way through: nothing at
    /// either end, most at the cut.
    pub fn look(self, p: f32) -> FxSample {
        let p = p.clamp(0.0, 1.0);
        let most = (std::f32::consts::PI * p).sin();
        let mut s = FxSample::REST;
        match self {
            TransitionKind::Glitch => {
                // a new tear a dozen times on the way
                s.glitch = 0.2 * most;
                s.glitch_seed = (p * 12.0).floor() + 1.0;
                s.split = 0.025 * most;
            }
            // soft for a moment about the cut only, so the move in and out is seen
            TransitionKind::ZoomBlur => s.blur = 0.055 * most * most,
            TransitionKind::LightLeak => s.exposure = 0.7 * most,
            _ => {}
        }
        s
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
            // edge on at the cut: the last clip turns away, the next turns to face
            (TransitionKind::Flip | TransitionKind::FlipUp, _) => {
                let turn = if incoming { -90.0 * (1.0 - p) * (1.0 - p) * 4.0 } else { 90.0 * p * p * 4.0 };
                let (turn, shows) = match (incoming, p < 0.5) {
                    (false, true) | (true, false) => (turn.clamp(-90.0, 90.0), 1.0),
                    (false, false) => (90.0, 0.0),
                    (true, true) => (-90.0, 0.0),
                };
                if self == TransitionKind::Flip {
                    o.turn_y = turn;
                } else {
                    o.turn_x = turn;
                }
                o.opacity = shows;
            }
            // hinged on the frame's right edge: its middle travels half a frame as it closes
            (TransitionKind::PageTurn, true) => {
                let open = 90.0 * (1.0 - e);
                o.turn_y = -open;
                o.dx = 0.5 * (1.0 - open.to_radians().cos());
                o.opacity = (p * 20.0).min(1.0);
            }
            (TransitionKind::LightLeak, true) => o.opacity = e,
            // what a shape uncovers is there from the first instant on, and no sooner
            (_, true) if self.reveals() => o.opacity = (p * 30.0).min(1.0),
            // there once the picture is torn most
            (TransitionKind::Glitch, true) => o.opacity = if p < 0.5 { 0.0 } else { 1.0 },
            // the camera goes into the last clip and comes back out of this one (which so never
            // shows its edges), and they change places while most out of focus
            (TransitionKind::ZoomBlur, true) => {
                o.opacity = ((p - 0.35) / 0.3).clamp(0.0, 1.0);
                o.scale = 1.0 + 1.2 * (1.0 - p).powi(3);
            }
            (TransitionKind::ZoomBlur, false) => o.scale = 1.0 + 1.2 * p * p * p,
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
            // and at the end all of it shows, and nothing is left of the transition's look
            if let Some(m) = k.reveal(1.0, [1.0, 1.778]) {
                assert!(k.reveals());
                for at in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.5, 0.5]] {
                    assert_eq!(covered(&m, at), 1.0, "{k:?} {at:?}");
                }
            }
            assert_eq!((k.look(0.0).blur, k.look(0.0).glitch, k.look(0.0).exposure), (0.0, 0.0, 0.0));
            assert!(k.look(1.0).blur < 1e-6 && k.look(1.0).glitch < 1e-6 && k.look(1.0).exposure < 1e-6);
        }
    }

    /// How much of a 1 by 1.778 picture `m` shows at `uv`, as the renderer works it out.
    fn covered(m: &Mask, uv: [f32; 2]) -> f32 {
        let (dx, dy) = (uv[0] - m.center[0], (uv[1] - m.center[1]) * 1.778);
        let (sin, cos) = (-m.rotation.to_radians()).sin_cos();
        let (x, y) = (dx * cos - dy * sin, dx * sin + dy * cos);
        let (hw, hh) = (m.size[0] / 2.0, m.size[1] / 2.0);
        let d = match m.kind {
            MaskKind::Linear => y,
            MaskKind::Mirror => y.abs() - hh,
            MaskKind::Circle => {
                let (rx, ry) = (hw.max(1e-6), hh.max(1e-6));
                (((x / rx) * (x / rx) + (y / ry) * (y / ry)).sqrt() - 1.0) * rx.min(ry)
            }
            MaskKind::Rectangle => {
                let (qx, qy) = (x.abs() - hw, y.abs() - hh);
                (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt() + qx.max(qy).min(0.0)
            }
        };
        (0.5 - d / m.feather).clamp(0.0, 1.0)
    }

    #[test]
    fn wipes_cross_the_picture_and_cards_are_edge_on_at_the_cut() {
        // half way, a wipe to the right has uncovered the left half
        let m = TransitionKind::WipeRight.reveal(0.5, [1.0, 1.778]).unwrap();
        assert_eq!((covered(&m, [0.2, 0.5]), covered(&m, [0.8, 0.5])), (1.0, 0.0));
        let m = TransitionKind::WipeUp.reveal(0.5, [1.0, 1.778]).unwrap();
        assert_eq!((covered(&m, [0.5, 0.8]), covered(&m, [0.5, 0.2])), (1.0, 0.0));
        // a circle half open shows the middle and not the corners
        let m = TransitionKind::Circle.reveal(0.5, [1.0, 1.778]).unwrap();
        assert_eq!((covered(&m, [0.5, 0.5]), covered(&m, [0.02, 0.02])), (1.0, 0.0));
        // a card: the last clip turns to edge on and is gone, then the next turns to face
        let k = TransitionKind::Flip;
        assert_eq!((k.offset(0.25, false).opacity, k.offset(0.25, true).opacity), (1.0, 0.0));
        assert_eq!((k.offset(0.75, false).opacity, k.offset(0.75, true).opacity), (0.0, 1.0));
        assert!((k.offset(0.5, false).turn_y - 90.0).abs() < 1e-3 && (k.offset(0.5, true).turn_y + 90.0).abs() < 1e-3);
        assert!(k.offset(0.25, false).turn_y > 10.0 && k.offset(0.75, true).turn_y < -10.0);
        assert_eq!(TransitionKind::FlipUp.offset(0.25, false).turn_y, 0.0);
        // a page closes from the right edge: open, it stands at the edge; shut, it lies flat
        let page = TransitionKind::PageTurn;
        assert!((page.offset(0.0, true).dx - 0.5).abs() < 1e-6 && (page.offset(0.0, true).turn_y + 90.0).abs() < 1e-4);
        assert_eq!(page.offset(1.0, true), Offset::REST);
        // the look is most at the cut
        assert!(TransitionKind::Glitch.look(0.5).glitch > 0.15 && TransitionKind::ZoomBlur.look(0.5).blur > 0.05);
        assert!(TransitionKind::Glitch.touches(FxChannel::Split) && !TransitionKind::Zoom.touches(FxChannel::Blur));
    }
}
