//! Kokoro-82M on candle (CPU), adapted from the MIT / Apache-2.0 `any-tts` crate
//! (<https://github.com/TM9657/any-tts>): the network, the official checkpoint and voice files.

// ported mostly as is: some knobs and accessors go unused here
#[allow(dead_code)]
pub(crate) mod albert;
#[allow(dead_code)]
pub(crate) mod config;
#[allow(dead_code)]
pub(crate) mod decoder;
#[allow(dead_code)]
pub(crate) mod model;
#[allow(dead_code)]
pub(crate) mod prosody;
#[allow(dead_code)]
pub(crate) mod text_encoder;
