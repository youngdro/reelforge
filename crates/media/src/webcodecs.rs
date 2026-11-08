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
