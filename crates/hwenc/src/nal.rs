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
    let mut out = Vec::with_capacity(n);
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
}
