//! The operating system's video encoders behind [`reelforge_media::VideoEncoder`]: VideoToolbox on
//! macOS. They usually run in hardware, several times faster than FilmCraft's pure-Rust H.264, and
//! they write HEVC too.
//!
//! [`register`] puts them in front of the software encoder; an app calls it once at start. When
//! the system has no encoder for a codec, or refuses the settings, the software one is used, so an
//! export never fails for want of hardware. `REELFORGE_ENCODER=software` turns them off.
//!
//! This is the one crate in the workspace that calls C interfaces, so it is the one that may use
//! `unsafe`. Each platform's module is compiled only there; elsewhere (the browser included, which
//! has WebCodecs) [`register`] does nothing.

use reelforge_media::VideoEncoder;
use reelforge_renderspec::VideoEncoding;
use reelforge_time::FrameRate;

#[cfg(target_os = "windows")]
mod mediafoundation;
mod nal;
#[cfg(target_os = "macos")]
mod videotoolbox;

/// Puts the system's encoders in front of the software one ([`reelforge_media::open_encoder`]).
/// Calling it again changes nothing.
pub fn register() {
    reelforge_media::register_video_encoder(open);
}

/// The system's encoder for these frames, or `None` (no such encoder here, or it is turned off).
pub fn open(size: [u32; 2], fps: FrameRate, venc: &VideoEncoding) -> Option<Box<dyn VideoEncoder>> {
    if std::env::var("REELFORGE_ENCODER").is_ok_and(|v| v.eq_ignore_ascii_case("software")) {
        return None;
    }
    system(size, fps, venc)
}

#[cfg(target_os = "macos")]
fn system(size: [u32; 2], fps: FrameRate, venc: &VideoEncoding) -> Option<Box<dyn VideoEncoder>> {
    videotoolbox::Encoder::open(size, fps, venc).ok().map(|e| Box::new(e) as Box<dyn VideoEncoder>)
}

#[cfg(target_os = "windows")]
fn system(size: [u32; 2], fps: FrameRate, venc: &VideoEncoding) -> Option<Box<dyn VideoEncoder>> {
    mediafoundation::Encoder::open(size, fps, venc).ok().map(|e| Box::new(e) as Box<dyn VideoEncoder>)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system(_: [u32; 2], _: FrameRate, _: &VideoEncoding) -> Option<Box<dyn VideoEncoder>> {
    None
}

/// The bitrate a hardware encoder aims for, in kbit/s: the one asked for, or one worked out from
/// the constant-quality number (hardware encoders have no such mode in common): about 0.1 bit a
/// pixel at `crf` 20, doubling every 6 steps better, as x264's does.
pub fn bitrate_kbps([w, h]: [u32; 2], fps: FrameRate, venc: &VideoEncoding) -> u32 {
    if let (None, Some(kbps)) = (venc.crf, venc.bitrate_kbps) {
        return kbps.max(1);
    }
    let crf = f64::from(venc.crf.unwrap_or(20));
    let per_pixel = 0.1 * 2f64.powf((20.0 - crf) / 6.0);
    let kbps = f64::from(w) * f64::from(h) * fps.fps() * per_pixel / 1000.0;
    kbps.clamp(200.0, 200_000.0).round() as u32
}

#[cfg(test)]
mod tests;
