//! How a project is exported: picture size, frame rate, quality and codec. Kept with the project so
//! every client (editor, CLI, agents) exports the same way.

use reelforge_time::FrameRate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Canvas;

/// Export size, by the picture's shorter side.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Resolution {
    /// The canvas's own size.
    #[default]
    Canvas,
    /// 720 pixels.
    #[serde(rename = "720p")]
    P720,
    /// 1080 pixels.
    #[serde(rename = "1080p")]
    P1080,
    /// 1440 pixels (2K).
    #[serde(rename = "2k")]
    P1440,
    /// 2160 pixels (4K).
    #[serde(rename = "4k")]
    P2160,
}

impl Resolution {
    /// Every size, in menu order.
    pub const ALL: [Resolution; 5] =
        [Resolution::Canvas, Resolution::P720, Resolution::P1080, Resolution::P1440, Resolution::P2160];

    /// The id commands use.
    pub fn name(self) -> &'static str {
        match self {
            Resolution::Canvas => "canvas",
            Resolution::P720 => "720p",
            Resolution::P1080 => "1080p",
            Resolution::P1440 => "2k",
            Resolution::P2160 => "4k",
        }
    }

    /// The size with this id.
    pub fn parse(s: &str) -> Option<Resolution> {
        Resolution::ALL.into_iter().find(|r| r.name() == s)
    }

    /// The shorter side in pixels, or `None` for the canvas's.
    pub fn short_side(self) -> Option<u32> {
        match self {
            Resolution::Canvas => None,
            Resolution::P720 => Some(720),
            Resolution::P1080 => Some(1080),
            Resolution::P1440 => Some(1440),
            Resolution::P2160 => Some(2160),
        }
    }
}

/// How much picture detail the bitrate buys.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    /// Smaller files.
    Low,
    /// What platforms expect.
    #[default]
    Recommended,
    /// Larger files, finer detail.
    High,
}

impl Quality {
    /// Every quality, in menu order.
    pub const ALL: [Quality; 3] = [Quality::Low, Quality::Recommended, Quality::High];

    /// The id commands use.
    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Recommended => "recommended",
            Quality::High => "high",
        }
    }

    /// The quality with this id.
    pub fn parse(s: &str) -> Option<Quality> {
        Quality::ALL.into_iter().find(|q| q.name() == s)
    }

    /// Constant-quality factor for encoders that have one (H.264's CRF scale).
    pub fn crf(self) -> u8 {
        match self {
            Quality::Low => 26,
            Quality::Recommended => 21,
            Quality::High => 17,
        }
    }

    /// Bits per pixel per frame of H.264, for encoders that take a bitrate.
    fn bits_per_pixel(self) -> f64 {
        match self {
            Quality::Low => 0.05,
            Quality::Recommended => 0.1,
            Quality::High => 0.18,
        }
    }
}

/// Video codecs.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Codec {
    /// H.264: plays everywhere.
    #[default]
    H264,
    /// H.265 / HEVC: about 40 % smaller.
    Hevc,
    /// AV1: smallest, newer players only.
    Av1,
}

impl Codec {
    /// Every codec, in menu order.
    pub const ALL: [Codec; 3] = [Codec::H264, Codec::Hevc, Codec::Av1];

    /// The id commands use.
    pub fn name(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
            Codec::Av1 => "av1",
        }
    }

    /// The codec with this id.
    pub fn parse(s: &str) -> Option<Codec> {
        Codec::ALL.into_iter().find(|c| c.name() == s)
    }

    /// Bits needed for the same picture as H.264.
    fn efficiency(self) -> f64 {
        match self {
            Codec::H264 => 1.0,
            Codec::Hevc => 0.6,
            Codec::Av1 => 0.5,
        }
    }
}

/// The frame rates offered.
pub const FRAME_RATES: [u32; 5] = [24, 25, 30, 50, 60];

/// How a project is exported.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ExportSettings {
    /// Picture size.
    #[serde(skip_serializing_if = "is_canvas")]
    pub resolution: Resolution,
    /// Frames per second; 0 keeps the canvas's rate.
    #[serde(skip_serializing_if = "is_zero")]
    pub fps: u32,
    /// Picture quality.
    #[serde(skip_serializing_if = "is_recommended")]
    pub quality: Quality,
    /// Video codec.
    #[serde(skip_serializing_if = "is_h264")]
    pub codec: Codec,
}

fn is_canvas(r: &Resolution) -> bool {
    *r == Resolution::Canvas
}
fn is_zero(v: &u32) -> bool {
    *v == 0
}
fn is_recommended(q: &Quality) -> bool {
    *q == Quality::Recommended
}
fn is_h264(c: &Codec) -> bool {
    *c == Codec::H264
}

/// Ready-made settings: `(id, settings)`.
pub const PRESETS: [(&str, ExportSettings); 4] = [
    ("draft", ExportSettings { resolution: Resolution::P720, fps: 30, quality: Quality::Low, codec: Codec::H264 }),
    (
        "standard",
        ExportSettings { resolution: Resolution::P1080, fps: 30, quality: Quality::Recommended, codec: Codec::H264 },
    ),
    ("smooth", ExportSettings { resolution: Resolution::P1080, fps: 60, quality: Quality::High, codec: Codec::H264 }),
    ("4k", ExportSettings { resolution: Resolution::P2160, fps: 30, quality: Quality::High, codec: Codec::Hevc }),
];

impl ExportSettings {
    /// Whether these are the defaults.
    pub fn is_default(&self) -> bool {
        *self == ExportSettings::default()
    }

    /// The preset these settings match.
    pub fn preset(&self) -> Option<&'static str> {
        PRESETS.iter().find(|(_, s)| s == self).map(|(id, _)| *id)
    }

    /// The preset with this id.
    pub fn of_preset(id: &str) -> Option<ExportSettings> {
        PRESETS.iter().find(|(p, _)| *p == id).map(|(_, s)| *s)
    }

    /// The frame rate the export runs at.
    pub fn frame_rate(&self, canvas: &Canvas) -> FrameRate {
        if self.fps == 0 { canvas.fps } else { FrameRate { num: self.fps, den: 1 } }
    }

    /// The encoded picture size: the canvas scaled so its shorter side is the resolution's, both
    /// sides even and the aspect kept to the pixel (`height == round(canvas.height × width ÷
    /// canvas.width)`). Falls back to the canvas's size when no such size is near.
    pub fn size(&self, canvas: &Canvas) -> [u32; 2] {
        let (cw, ch) = (canvas.width, canvas.height);
        let Some(short) = self.resolution.short_side() else { return [cw, ch] };
        let k = short as f64 / cw.min(ch) as f64;
        let start = ((cw as f64 * k).round() as u32) & !1;
        // look for an even width whose height comes out even too, nearest first
        for d in 0..16u32 {
            for w in [start.saturating_sub(2 * d), start + 2 * d] {
                if w < 2 {
                    continue;
                }
                let h = scaled_height(cw, ch, w);
                if h.is_multiple_of(2) && h >= 2 && w <= 8192 && h <= 8192 {
                    return [w, h];
                }
            }
        }
        [cw, ch]
    }

    /// The bitrate in kbit/s, for encoders that take one.
    pub fn bitrate_kbps(&self, canvas: &Canvas) -> u32 {
        let [w, h] = self.size(canvas);
        let fps = self.frame_rate(canvas).fps();
        // more frames each look more like the last, so a doubled rate costs about 1.5×
        let motion = (fps / 30.0).powf(0.6) * 30.0;
        let bits = w as f64 * h as f64 * motion * self.quality.bits_per_pixel() * self.codec.efficiency();
        (bits / 1000.0).round().max(100.0) as u32
    }
}

/// The height a `width`-wide picture of a `cw` by `ch` canvas has, rounded as renderers round it.
pub fn scaled_height(cw: u32, ch: u32, width: u32) -> u32 {
    (ch as f32 * (width as f32 / cw as f32)).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_even_and_keep_the_aspect() {
        let tall = Canvas::default();
        let s = |r, c: &Canvas| ExportSettings { resolution: r, ..Default::default() }.size(c);
        assert_eq!(s(Resolution::P720, &tall), [720, 1280]);
        assert_eq!(s(Resolution::P2160, &tall), [2160, 3840]);
        assert_eq!(s(Resolution::Canvas, &tall), [1080, 1920]);
        let odd = Canvas { width: 1000, height: 750, ..Canvas::default() };
        for r in Resolution::ALL {
            let [w, h] = s(r, &odd);
            assert!(w % 2 == 0 && h % 2 == 0 && h == scaled_height(1000, 750, w), "{r:?}: {w}x{h}");
        }
        for (id, p) in PRESETS {
            assert_eq!(ExportSettings::of_preset(id), Some(p));
            assert_eq!(p.preset(), Some(id));
        }
        let std = ExportSettings::of_preset("standard").unwrap();
        let kbps = std.bitrate_kbps(&tall);
        assert!((5000..8000).contains(&kbps), "{kbps}");
        assert!(ExportSettings { codec: Codec::Hevc, ..std }.bitrate_kbps(&tall) < kbps);
        // the document spells them as commands do
        for r in Resolution::ALL {
            assert_eq!(serde_json::to_value(r).unwrap(), serde_json::json!(r.name()));
        }
    }
}
