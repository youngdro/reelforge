//! What the browser's video encoders (WebCodecs) need from us: codec strings with a level that fits,
//! and an `av1C` record for encoders that give none. The encoder itself lives with the web editor.

use reelforge_renderspec::VideoCodec;

/// The WebCodecs codec string for `codec` at this size and rate (a level that fits it).
pub fn codec_string(codec: VideoCodec, [w, h]: [u32; 2], fps: f64) -> String {
    let rate = w as f64 * h as f64 * fps;
    let big = |px: f64, r: f64| (w as f64 * h as f64) <= px && rate <= r;
    match codec {
        // High profile; levels 4.0, 4.2, 5.1, 5.2
        VideoCodec::H264 => {
            let level = if big(2_228_224.0, 62_914_560.0) {
                0x28
            } else if big(2_228_224.0, 133_693_440.0) {
                0x2a
            } else if big(9_437_184.0, 251_658_240.0) {
                0x33
            } else {
                0x34
            };
            format!("avc1.6400{level:02x}")
        }
        // Main profile; levels 4.0, 4.1, 5.0, 5.1, 5.2 (level × 30)
        VideoCodec::Hevc => {
            let level = if big(2_228_224.0, 66_846_720.0) {
                120
            } else if big(2_228_224.0, 133_693_440.0) {
                123
            } else if big(8_912_896.0, 267_386_880.0) {
                150
            } else if big(8_912_896.0, 534_773_760.0) {
                153
            } else {
                156
            };
            format!("hvc1.1.6.L{level}.B0")
        }
        // Main profile, 8-bit; levels 4.0, 4.1, 5.0, 5.1, 5.2
        VideoCodec::Av1 => {
            let level = if big(2_228_224.0, 70_778_880.0) {
                8
            } else if big(2_228_224.0, 141_557_760.0) {
                9
            } else if big(8_912_896.0, 267_386_880.0) {
                12
            } else if big(8_912_896.0, 534_773_760.0) {
                13
            } else {
                14
            };
            format!("av01.0.{level:02}M.08")
        }
    }
}

/// An `av1C` record from the sequence header OBU at the start of a key frame, for encoders that
/// do not give one: profile, level and tier from the header, 8-bit 4:2:0.
pub fn av1c_from_obus(data: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0;
    while i < data.len() {
        let header = data[i];
        let kind = (header >> 3) & 0x0f;
        let ext = header & 0x04 != 0;
        let mut j = i + 1 + usize::from(ext);
        if header & 0x02 == 0 {
            return None; // no size: not the low-overhead format
        }
        let mut size = 0usize;
        for k in 0..8 {
            let b = *data.get(j)?;
            j += 1;
            size |= usize::from(b & 0x7f) << (7 * k);
            if b & 0x80 == 0 {
                break;
            }
        }
        let end = j.checked_add(size).filter(|e| *e <= data.len())?;
        if kind == 1 {
            let body = &data[j..end];
            let mut bits = Bits { data: body, pos: 0 };
            let profile = bits.take(3)? as u8;
            let _still = bits.take(1)?;
            let reduced = bits.take(1)? == 1;
            let (level, tier) = if reduced {
                (bits.take(5)? as u8, false)
            } else if bits.take(1)? == 1 {
                // timing info present: the level is past fields not worth parsing here
                (31, false)
            } else {
                let _delay_present = bits.take(1)?;
                let _ops = bits.take(5)?;
                let _idc = bits.take(12)?;
                let level = bits.take(5)? as u8;
                let tier = level > 7 && bits.take(1)? == 1;
                (level, tier)
            };
            let mut out = vec![0x81, (profile << 5) | (level & 0x1f), (u8::from(tier) << 7) | 0x0c, 0];
            out.extend_from_slice(&data[i..end]);
            return Some(out);
        }
        i = end;
    }
    None
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn take(&mut self, n: usize) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = (v << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(v)
    }
}

/// The H.264 video of an MP4 file, as the browser's decoder (WebCodecs `VideoDecoder`) takes it:
/// its configuration and its samples in decode order. Other codecs and containers are decoded in
/// Rust.
pub struct DecodableVideo {
    /// The WebCodecs codec string (`avc1.PPCCLL`).
    pub codec: String,
    /// The `avcC` record, WebCodecs' `description`.
    pub description: Vec<u8>,
    /// Coded size in pixels.
    pub size: [u32; 2],
    bytes: std::sync::Arc<[u8]>,
    track: filmcraft_isobmff::Track,
}

impl DecodableVideo {
    /// The file's first video track when it is H.264 the browser can take as it is (not rotated).
    pub fn open(bytes: std::sync::Arc<[u8]>) -> Option<DecodableVideo> {
        let file = filmcraft_isobmff::open(bytes.clone()).ok()?;
        let i = file.track_of_kind(filmcraft_isobmff::TrackKind::Video)?;
        let track = file.tracks.into_iter().nth(i)?;
        if track.display_rotation().is_some_and(|r| r != 0) || track.entries.len() != 1 || track.samples.is_empty() {
            return None;
        }
        let filmcraft_isobmff::CodecConfig::Avc(avc) = track.codec()? else { return None };
        let codec = format!("avc1.{:02x}{:02x}{:02x}", avc.profile, avc.compatibility, avc.level);
        let description = avc.to_bytes();
        let size = track.video().map_or([track.width, track.height], |v| [u32::from(v.width), u32::from(v.height)]);
        Some(DecodableVideo { codec, description, size, bytes, track })
    }

    /// How many samples.
    pub fn len(&self) -> usize {
        self.track.samples.len()
    }

    /// Whether it has no samples (never, once opened).
    pub fn is_empty(&self) -> bool {
        self.track.samples.is_empty()
    }

    /// The sample (decode order) whose picture shows at `at` in the file's own time, as the Rust
    /// decoder picks it: nearest in the track's timescale, the last one past the end.
    pub fn sample_at(&self, at: reelforge_time::Tick) -> usize {
        let ts = i64::from(self.track.timescale.max(1));
        let t = (i128::from(at.0) * i128::from(ts) + i128::from(reelforge_time::TICKS_PER_SECOND / 2))
            / i128::from(reelforge_time::TICKS_PER_SECOND);
        let t = i64::try_from(t).unwrap_or(i64::MAX);
        self.track.sample_at_presentation_time(t).unwrap_or(self.len() - 1)
    }

    /// The sync sample decoding must start from to reach sample `i`.
    pub fn sync_before(&self, i: usize) -> usize {
        self.track.sync_sample_before(i)
    }

    /// Sample `i`: its bytes, presentation time and length in microseconds, and whether decoding
    /// can start there.
    pub fn sample(&self, i: usize) -> Option<(&[u8], i64, i64, bool)> {
        let s = self.track.samples.get(i)?;
        let start = usize::try_from(s.offset).ok()?;
        let data = self.bytes.get(start..start.checked_add(s.size as usize)?)?;
        let ts = i128::from(self.track.timescale.max(1));
        let us = |v: i64| i64::try_from(i128::from(v) * 1_000_000 / ts).unwrap_or(0);
        let pts = self.track.presentation_pts(i)?;
        Some((data, us(pts), us(i64::from(s.duration)), s.is_sync))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_strings_pick_a_level_that_fits() {
        assert_eq!(codec_string(VideoCodec::H264, [1080, 1920], 30.0), "avc1.640028");
        assert_eq!(codec_string(VideoCodec::H264, [1080, 1920], 60.0), "avc1.64002a");
        assert_eq!(codec_string(VideoCodec::H264, [2160, 3840], 30.0), "avc1.640033");
        assert_eq!(codec_string(VideoCodec::Hevc, [720, 1280], 30.0), "hvc1.1.6.L120.B0");
        assert_eq!(codec_string(VideoCodec::Hevc, [2160, 3840], 60.0), "hvc1.1.6.L153.B0");
        assert_eq!(codec_string(VideoCodec::Av1, [1080, 1920], 30.0), "av01.0.08M.08");
    }

    #[test]
    fn an_av1_record_is_built_from_the_sequence_header() {
        // a temporal delimiter, then a sequence header: profile 0, one operating point at level 8
        let frame = [0x12, 0x00, 0x0a, 0x04, 0x00, 0x00, 0x00, 0x40, 0x32, 0x01, 0xff];
        let rec = av1c_from_obus(&frame).unwrap();
        assert_eq!(rec, [0x81, 0x08, 0x0c, 0x00, 0x0a, 0x04, 0x00, 0x00, 0x00, 0x40]);
        assert!(filmcraft_isobmff::Av1Config::parse(&rec).is_ok());
        assert_eq!(av1c_from_obus(&[0x32, 0x01, 0xff]), None);
    }
}
