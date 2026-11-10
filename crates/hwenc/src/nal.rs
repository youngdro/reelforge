//! Reading the NAL units an encoder hands back.

use reelforge_renderspec::VideoCodec;

/// Whether length-prefixed (4-byte) NAL units hold a picture decoding can start at: an IDR slice
/// for H.264, an IRAP slice for HEVC.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn is_keyframe(codec: VideoCodec, mut data: &[u8]) -> bool {
    while data.len() > 4 {
        let len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let Some(nal) = data.get(4..4 + len) else { return false };
        if let Some(&head) = nal.first() {
            let key = match codec {
                VideoCodec::H264 => head & 0x1f == 5,
                VideoCodec::Hevc => (16..=21).contains(&((head >> 1) & 0x3f)),
                VideoCodec::Av1 => false,
            };
            if key {
                return true;
            }
        }
        data = &data[4 + len..];
    }
    false
}

/// The NAL units of an Annex B byte stream (start codes `00 00 01`, with any zero bytes before
/// them), without the start codes.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (n, &at) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(data.len());
        let mut unit = &data[at + 3..end];
        // zeros before the next start code (a four-byte one, or padding) are not this unit's:
        // a unit never ends in a zero byte
        while unit.last() == Some(&0) {
            unit = &unit[..unit.len() - 1];
        }
        if !unit.is_empty() {
            units.push(unit);
        }
    }
    units
}

/// What a NAL unit is, for the configuration record.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) enum Unit {
    Vps,
    Sps,
    Pps,
    /// An access unit delimiter: not needed in an MP4.
    Delimiter,
    Other,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn kind(codec: VideoCodec, unit: &[u8]) -> Unit {
    let Some(&head) = unit.first() else { return Unit::Other };
    match codec {
        VideoCodec::H264 => match head & 0x1f {
            7 => Unit::Sps,
            8 => Unit::Pps,
            9 => Unit::Delimiter,
            _ => Unit::Other,
        },
        VideoCodec::Hevc => match (head >> 1) & 0x3f {
            32 => Unit::Vps,
            33 => Unit::Sps,
            34 => Unit::Pps,
            35 => Unit::Delimiter,
            _ => Unit::Other,
        },
        VideoCodec::Av1 => Unit::Other,
    }
}

/// The parameter sets of a stream, as they are met.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct ParameterSets {
    pub(crate) vps: Vec<Vec<u8>>,
    pub(crate) sps: Vec<Vec<u8>>,
    pub(crate) pps: Vec<Vec<u8>>,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl ParameterSets {
    /// Takes an Annex B access unit apart: parameter sets are kept here (once each), delimiters
    /// dropped, and the rest comes back length-prefixed, as an MP4 sample holds it.
    pub(crate) fn sample(&mut self, codec: VideoCodec, annex: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(annex.len() + 8);
        for unit in annex_b(annex) {
            let keep = |list: &mut Vec<Vec<u8>>| {
                if !list.iter().any(|u| u == unit) {
                    list.push(unit.to_vec());
                }
            };
            match kind(codec, unit) {
                Unit::Vps => keep(&mut self.vps),
                Unit::Sps if codec == VideoCodec::H264 => match h264_sps_bt709(unit) {
                    Some(told) if !self.sps.contains(&told) => self.sps.push(told),
                    Some(_) => {}
                    None => keep(&mut self.sps),
                },
                Unit::Sps => keep(&mut self.sps),
                Unit::Pps => keep(&mut self.pps),
                Unit::Delimiter => {}
                Unit::Other => {
                    out.extend_from_slice(&(unit.len() as u32).to_be_bytes());
                    out.extend_from_slice(unit);
                }
            }
        }
        out
    }

    /// The `avcC` record of these sets, or `None` until there is an SPS and a PPS.
    pub(crate) fn avcc(&self) -> Option<Vec<u8>> {
        let sps = self.sps.first().filter(|s| s.len() >= 4)?;
        self.pps.first()?;
        let mut b = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe0 | self.sps.len().min(31) as u8];
        for s in self.sps.iter().take(31) {
            b.extend_from_slice(&(s.len() as u16).to_be_bytes());
            b.extend_from_slice(s);
        }
        b.push(self.pps.len().min(255) as u8);
        for p in self.pps.iter().take(255) {
            b.extend_from_slice(&(p.len() as u16).to_be_bytes());
            b.extend_from_slice(p);
        }
        // the High profiles say their chroma format and bit depths too: 4:2:0, 8 bits
        if matches!(sps[1], 100 | 110 | 122 | 144) {
            b.extend_from_slice(&[0xfc | 1, 0xf8, 0xf8, 0]);
        }
        Some(b)
    }

    /// The `hvcC` record of these sets (8-bit 4:2:0, what the encoders here write), or `None`
    /// until there is a VPS, an SPS and a PPS.
    pub(crate) fn hvcc(&self) -> Option<Vec<u8>> {
        self.vps.first()?;
        self.pps.first()?;
        // an SPS starts: 2 header bytes, one byte of ids and the sub-layer count, then the 12
        // bytes of the general profile, tier and level, which the record repeats
        let sps = unescaped(self.sps.first()?, 15);
        if sps.len() < 15 {
            return None;
        }
        let layers = ((sps[2] >> 1) & 7) + 1;
        let nested = sps[2] & 1;
        let mut b = vec![1];
        b.extend_from_slice(&sps[3..15]);
        b.extend_from_slice(&[0xf0, 0, 0xfc, 0xfc | 1, 0xf8, 0xf8, 0, 0]);
        b.push((layers << 3) | (nested << 2) | 3);
        b.push(3);
        for (kind, sets) in [(32u8, &self.vps), (33, &self.sps), (34, &self.pps)] {
            b.push(0x80 | kind);
            b.extend_from_slice(&(sets.len() as u16).to_be_bytes());
            for s in sets {
                b.extend_from_slice(&(s.len() as u16).to_be_bytes());
                b.extend_from_slice(s);
            }
        }
        Some(b)
    }
}

/// The first `n` bytes of a NAL unit's payload with the emulation prevention bytes (the `03` of
/// `00 00 03`) taken out.
fn unescaped(unit: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n.min(unit.len()));
    let mut zeros = 0;
    for &b in unit {
        if out.len() == n {
            break;
        }
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// An H.264 SPS that says what the pictures are: BT.709, limited range. Media Foundation's
/// software encoder leaves the colours unsaid, and a decoder then guesses from the size (BT.601
/// below HD, which shifts every colour). The rest of the SPS is kept bit for bit. `None` when
/// the SPS cannot be read; it is then used as it came.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn h264_sps_bt709(unit: &[u8]) -> Option<Vec<u8>> {
    let rbsp = unescaped(unit.get(1..)?, usize::MAX);
    // the payload ends at the stop bit: the last 1 in the last byte that has one
    let last = rbsp.iter().rposition(|b| *b != 0)?;
    let end = last * 8 + 7 - rbsp[last].trailing_zeros() as usize;
    let mut r = Bits { data: &rbsp, at: 0, end };
    let profile = r.take(8)?;
    r.take(16)?;
    r.ue()?;
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        let chroma = r.ue()?;
        if chroma == 3 {
            r.take(1)?;
        }
        r.ue()?;
        r.ue()?;
        r.take(1)?;
        if r.take(1)? == 1 {
            for list in 0..if chroma == 3 { 12 } else { 8 } {
                if r.take(1)? == 0 {
                    continue;
                }
                let (mut last, mut next) = (8i64, 8i64);
                for _ in 0..if list < 6 { 16 } else { 64 } {
                    if next != 0 {
                        next = (last + r.se()? + 256) % 256;
                    }
                    if next != 0 {
                        last = next;
                    }
                }
            }
        }
    }
    r.ue()?;
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.take(1)?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?;
    r.take(1)?;
    r.ue()?;
    r.ue()?;
    if r.take(1)? == 0 {
        r.take(1)?;
    }
    r.take(1)?;
    if r.take(1)? == 1 {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    let mut out = Writer::default();
    out.copy(&rbsp, 0, r.at);
    // video format 5 (unspecified), limited range, then primaries, transfer and matrix: 1 is BT.709
    let signal = |out: &mut Writer| {
        out.put(1, 1);
        out.put(5, 3);
        out.put(0, 1);
        out.put(1, 1);
        out.put(0x01_01_01, 24);
    };
    if r.take(1)? == 1 {
        out.put(1, 1);
        let from = r.at;
        if r.take(1)? == 1 && r.take(8)? == 255 {
            r.take(32)?;
        }
        if r.take(1)? == 1 {
            r.take(1)?;
        }
        out.copy(&rbsp, from, r.at);
        if r.take(1)? == 1 {
            r.take(4)?;
            if r.take(1)? == 1 {
                r.take(24)?;
            }
        }
        signal(&mut out);
        out.copy(&rbsp, r.at, end);
    } else {
        // a VUI with nothing else in it: no aspect ratio or overscan before the colours, and no
        // chroma location, timing, HRDs, picture structure or restrictions after
        out.put(1, 1);
        out.put(0, 2);
        signal(&mut out);
        out.put(0, 6);
    }
    out.put(1, 1);
    // back into a NAL unit: a 03 goes in wherever two zeros would be followed by 0 to 3
    let mut sps = vec![unit[0]];
    let mut zeros = 0;
    for b in out.bytes {
        if zeros >= 2 && b <= 3 {
            sps.push(3);
            zeros = 0;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        sps.push(b);
    }
    Some(sps)
}

/// Reads bits, most significant first, up to `end`.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    end: usize,
}

impl Bits<'_> {
    fn take(&mut self, n: usize) -> Option<u32> {
        if self.at + n > self.end {
            return None;
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from((self.data[self.at / 8] >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Some(v)
    }

    /// An unsigned Exp-Golomb number.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.take(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some(((1u64 << zeros) - 1 + u64::from(self.take(zeros)?)) as u32)
    }

    /// A signed Exp-Golomb number.
    fn se(&mut self) -> Option<i64> {
        let k = i64::from(self.ue()?);
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }
}

/// Writes bits, most significant first.
#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bits: usize,
}

impl Writer {
    fn put(&mut self, v: u32, n: usize) {
        for i in (0..n).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                *self.bytes.last_mut().expect("pushed above") |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    /// The bits of `data` from `from` up to `to`.
    fn copy(&mut self, data: &[u8], from: usize, to: usize) {
        for at in from..to {
            self.put(u32::from((data[at / 8] >> (7 - at % 8)) & 1), 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idr_slice_after_parameter_sets_is_a_keyframe() {
        let unit = |head: u8| [&[0, 0, 0, 2][..], &[head, 0x80]].concat();
        let key = [unit(0x67), unit(0x68), unit(0x65)].concat();
        assert!(is_keyframe(VideoCodec::H264, &key));
        assert!(!is_keyframe(VideoCodec::H264, &unit(0x41)));
        // HEVC: IDR_W_RADL is type 19, a trailing picture type 1
        assert!(is_keyframe(VideoCodec::Hevc, &unit(19 << 1)));
        assert!(!is_keyframe(VideoCodec::Hevc, &unit(1 << 1)));
        // a length running past the end is not read
        assert!(!is_keyframe(VideoCodec::H264, &[0, 0, 0, 9, 0x65]));
    }

    #[test]
    fn an_annex_b_access_unit_becomes_a_sample_and_a_record() {
        let sps = [0x67, 100, 0, 31, 0xac];
        let pps = [0x68, 0xee, 0x3c, 0x80];
        let idr = [0x65, 0x88, 0x84, 0x00, 0x33];
        let annex = [&[0, 0, 0, 1, 9, 0xf0][..], &[0, 0, 0, 1], &sps, &[0, 0, 1], &pps, &[0, 0, 0, 1], &idr].concat();
        assert_eq!(annex_b(&annex), [&[9, 0xf0][..], &sps, &pps, &idr]);
        let mut sets = ParameterSets::default();
        assert_eq!(sets.avcc(), None);
        let sample = sets.sample(VideoCodec::H264, &annex);
        assert_eq!(sample, [&[0, 0, 0, 5][..], &idr].concat());
        assert!(is_keyframe(VideoCodec::H264, &sample));
        // the same sets again are not kept twice
        sets.sample(VideoCodec::H264, &annex);
        let record = sets.avcc().expect("an SPS and a PPS");
        let want = [&[1, 100, 0, 31, 0xff, 0xe1, 0, 5][..], &sps, &[1, 0, 4], &pps, &[0xfd, 0xf8, 0xf8, 0]].concat();
        assert_eq!(record, want);
    }

    #[test]
    fn an_hevc_record_repeats_the_profile_from_the_sps() {
        // Main profile, level 4: the compatibility and constraint flags hold `00 00 03` escapes
        let sps = [
            0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x78,
            0xa0,
        ];
        let sets =
            ParameterSets { vps: vec![vec![0x40, 1, 0x0c]], sps: vec![sps.to_vec()], pps: vec![vec![0x44, 1, 0xc1]] };
        let r = sets.hvcc().expect("all three");
        assert_eq!(r[..13], [1, 0x01, 0x60, 0, 0, 0, 0x90, 0, 0, 0, 0, 0, 0x78]);
        assert_eq!(r[13..23], [0xf0, 0, 0xfc, 0xfd, 0xf8, 0xf8, 0, 0, 0x0f, 3]);
        assert_eq!(r[23..29], [0x80 | 32, 0, 1, 0, 3, 0x40]);
        assert_eq!(ParameterSets::default().hvcc(), None);
    }

    #[test]
    fn an_h264_sps_is_made_to_say_bt709() {
        use filmcraft_h264::params::Sps;
        let colours = |unit: &[u8]| {
            let sps = Sps::parse(&unescaped(&unit[1..], usize::MAX)).expect("an SPS");
            let v = sps.vui.clone().expect("a VUI");
            (sps, (v.full_range, v.colour_primaries, v.transfer_characteristics, v.matrix_coefficients))
        };
        // x264's, 1280×720 High: a VUI with timing and restrictions, and no colours
        let with: &[u8] = &[
            0x67, 0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x05, 0xbb, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00,
            0x00, 0x03, 0x03, 0xc0, 0xf1, 0x83, 0x19, 0x60,
        ];
        let before = Sps::parse(&unescaped(&with[1..], usize::MAX)).expect("an SPS");
        let told = h264_sps_bt709(with).expect("readable");
        let (after, said) = colours(&told);
        assert_eq!(said, (false, 1, 1, 1));
        assert_eq!((after.pic_width_in_mbs, after.pic_height_in_map_units), (80, 45));
        let (was, is) = (before.vui.expect("a VUI"), after.vui.expect("a VUI"));
        assert_eq!((was.timing, was.sar), (is.timing, is.sar));
        assert_eq!(
            was.bitstream_restriction.map(|b| (b.max_num_reorder_frames, b.max_dec_frame_buffering)),
            is.bitstream_restriction.map(|b| (b.max_num_reorder_frames, b.max_dec_frame_buffering))
        );
        // said once is said: nothing changes the second time
        assert_eq!(h264_sps_bt709(&told), Some(told));

        // Baseline, 640×360 (cropped from 368), and no VUI at all
        let mut w = Writer::default();
        w.put(0x42_00_1e, 24);
        for (v, n) in [(1, 1), (1, 1), (1, 1), (1, 1), (0b010, 3), (0, 1)] {
            w.put(v, n);
        }
        // 40 and 23 macroblocks, coded less one: ue(39), ue(22)
        w.put(40, 11);
        w.put(23, 9);
        // frames only, 8×8 direct, cropped: 0, 0, 0, 4 (ue(4) is 00101), and no VUI
        for (v, n) in [(1, 1), (1, 1), (1, 1), (1, 1), (1, 1), (1, 1), (0b00101, 5), (0, 1), (1, 1)] {
            w.put(v, n);
        }
        let without = [&[0x67][..], &w.bytes].concat();
        let before = Sps::parse(&without[1..]).expect("an SPS");
        assert!(before.vui.is_none());
        let told = h264_sps_bt709(&without).expect("readable");
        let (after, said) = colours(&told);
        assert_eq!(said, (false, 1, 1, 1));
        assert_eq!(after.frame_crop, Some((0, 0, 0, 4)));
        assert_eq!((after.pic_width_in_mbs, after.pic_height_in_map_units), (40, 23));
        // cut short, it is left alone
        assert_eq!(h264_sps_bt709(&with[..6]), None);
    }
}
