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
}
