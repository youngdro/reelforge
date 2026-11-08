//! The browser's video codecs (WebCodecs), usually in hardware.
//!
//! Encoding: H.264, HEVC and AV1. Frames go in as I420 and come back asynchronously;
//! [`WebEncoder`] is a [`VideoEncoder`] an [`ExportWriter`](reelforge_media::ExportWriter) drives,
//! its backlog above 0 until the last frames are back.
//!
//! Decoding: [`WebDecoder`] decodes the H.264 footage of one clip ahead of an export, so that the
//! frames are ready before the renderer asks for them.

use std::sync::Arc;

use js_sys::{Promise, Uint8Array};
use reelforge_media::webcodecs::DecodableVideo;
use reelforge_render::Image;
use reelforge_renderspec::VideoCodec;
use reelforge_time::{FrameRate, Tick};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use reelforge_media::webcodecs::{av1c_from_obus, codec_string};
use reelforge_media::{EncodedFrame, ExportError, VideoConfig, VideoEncoder, Yuv420};

#[wasm_bindgen(inline_js = r#"
function rfConfig(codec, width, height, bitrate, fps, hw) {
  const c = { codec, width, height, bitrate, framerate: fps, hardwareAcceleration: hw, bitrateMode: 'variable', latencyMode: 'quality' };
  if (codec.startsWith('avc1')) c.avc = { format: 'avc' };
  if (codec.startsWith('hvc1') || codec.startsWith('hev1')) c.hevc = { format: 'hevc' };
  return c;
}
export async function rf_supported(codec, width, height, bitrate, fps) {
  if (typeof VideoEncoder === 'undefined' || typeof VideoFrame === 'undefined') return '';
  for (const hw of ['prefer-hardware', 'no-preference']) {
    try {
      const r = await VideoEncoder.isConfigSupported(rfConfig(codec, width, height, bitrate, fps, hw));
      if (r.supported) return hw;
    } catch (e) {}
  }
  return '';
}
export function rf_sleep(ms) { return new Promise(r => setTimeout(r, ms)); }
export class RfEncoder {
  constructor(codec, width, height, bitrate, fps, hw) {
    this.chunks = []; this.out = []; this.desc = null; this.err = null;
    this.enc = new VideoEncoder({
      output: (chunk, meta) => {
        const d = meta && meta.decoderConfig && meta.decoderConfig.description;
        if (d && !this.desc) {
          this.desc = d instanceof ArrayBuffer ? new Uint8Array(d.slice(0)) : new Uint8Array(d.buffer.slice(d.byteOffset, d.byteOffset + d.byteLength));
        }
        const data = new Uint8Array(chunk.byteLength);
        chunk.copyTo(data);
        this.chunks.push({ data, ts: chunk.timestamp, key: chunk.type === 'key' });
      },
      error: (e) => { this.err = String((e && e.message) || e); },
    });
    this.enc.configure(rfConfig(codec, width, height, bitrate, fps, hw));
  }
  encode(y, u, v, width, height, ts, key) {
    const cw = (width + 1) >> 1;
    const buf = new Uint8Array(y.length + u.length + v.length);
    buf.set(y, 0); buf.set(u, y.length); buf.set(v, y.length + u.length);
    const frame = new VideoFrame(buf, {
      format: 'I420', codedWidth: width, codedHeight: height, timestamp: ts,
      layout: [{ offset: 0, stride: width }, { offset: y.length, stride: cw }, { offset: y.length + u.length, stride: cw }],
    });
    try { this.enc.encode(frame, { keyFrame: key }); } finally { frame.close(); }
  }
  take() { this.out = this.chunks; this.chunks = []; return this.out.length; }
  data(i) { return this.out[i].data; }
  ts(i) { return this.out[i].ts; }
  key(i) { return this.out[i].key; }
  description() { return this.desc; }
  error() { return this.err; }
  busy() { return this.enc.encodeQueueSize + (this.flushing ? 1 : 0); }
  startFlush() {
    this.flushing = true;
    this.enc.flush().then(() => { this.flushing = false; }, (e) => { this.err = String((e && e.message) || e); this.flushing = false; });
  }
  close() { try { this.enc.close(); } catch (e) {} }
}
export async function rf_decodable(codec, width, height, description) {
  if (typeof VideoDecoder === 'undefined' || typeof OffscreenCanvas === 'undefined') return false;
  try {
    const r = await VideoDecoder.isConfigSupported({ codec, codedWidth: width, codedHeight: height, description });
    return !!r.supported;
  } catch (e) { return false; }
}
export class RfDecoder {
  constructor(codec, width, height, description) {
    this.cfg = { codec, codedWidth: width, codedHeight: height, description: description.slice(), hardwareAcceleration: 'no-preference' };
    this.frames = new Map(); this.err = null; this.flushing = false; this.gen = 0; this.cx = null;
    this.dec = new VideoDecoder({
      output: (f) => this.got(f),
      error: (e) => { this.err = String((e && e.message) || e); },
    });
    this.dec.configure(this.cfg);
  }
  got(f) {
    try {
      const w = f.displayWidth, h = f.displayHeight;
      if (!this.cx || this.cx.canvas.width !== w || this.cx.canvas.height !== h) {
        this.cx = new OffscreenCanvas(w, h).getContext('2d', { willReadFrequently: true });
      }
      this.cx.drawImage(f, 0, 0, w, h);
      this.frames.set(f.timestamp, { w, h, data: this.cx.getImageData(0, 0, w, h).data });
    } catch (e) { this.err = String((e && e.message) || e); } finally { f.close(); }
  }
  decode(data, ts, dur, key) {
    this.dec.decode(new EncodedVideoChunk({ type: key ? 'key' : 'delta', timestamp: ts, duration: dur, data }));
  }
  queue() { return this.dec.decodeQueueSize + (this.flushing ? 1 : 0); }
  startFlush() {
    const gen = this.gen;
    this.flushing = true;
    this.dec.flush().then(() => { if (gen === this.gen) this.flushing = false; },
      (e) => { if (gen === this.gen) { this.err = String((e && e.message) || e); this.flushing = false; } });
  }
  reset() {
    this.gen++; this.flushing = false; this.frames.clear();
    this.dec.reset(); this.dec.configure(this.cfg);
  }
  has(ts) { return this.frames.has(ts); }
  count() { return this.frames.size; }
  width(ts) { return this.frames.get(ts).w; }
  height(ts) { return this.frames.get(ts).h; }
  pixels(ts) { const d = this.frames.get(ts).data; return new Uint8Array(d.buffer, d.byteOffset, d.byteLength); }
  dropBelow(ts) { for (const k of [...this.frames.keys()]) if (k < ts) this.frames.delete(k); }
  error() { return this.err; }
  close() { try { this.dec.close(); } catch (e) {} this.frames.clear(); }
}
"#)]
extern "C" {
    fn rf_supported(codec: &str, width: u32, height: u32, bitrate: f64, fps: f64) -> Promise;
    fn rf_sleep(ms: u32) -> Promise;

    type RfEncoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(codec: &str, width: u32, height: u32, bitrate: f64, fps: f64, hw: &str) -> Result<RfEncoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn encode(
        this: &RfEncoder,
        y: &[u8],
        u: &[u8],
        v: &[u8],
        width: u32,
        height: u32,
        ts: f64,
        key: bool,
    ) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn take(this: &RfEncoder) -> u32;
    #[wasm_bindgen(method)]
    fn data(this: &RfEncoder, i: u32) -> Uint8Array;
    #[wasm_bindgen(method)]
    fn ts(this: &RfEncoder, i: u32) -> f64;
    #[wasm_bindgen(method)]
    fn key(this: &RfEncoder, i: u32) -> bool;
    #[wasm_bindgen(method)]
    fn description(this: &RfEncoder) -> Option<Uint8Array>;
    #[wasm_bindgen(method)]
    fn error(this: &RfEncoder) -> Option<String>;
    #[wasm_bindgen(method)]
    fn busy(this: &RfEncoder) -> u32;
    #[wasm_bindgen(method, js_name = startFlush)]
    fn start_flush(this: &RfEncoder);
    #[wasm_bindgen(method)]
    fn close(this: &RfEncoder);

    fn rf_decodable(codec: &str, width: u32, height: u32, description: &[u8]) -> Promise;

    type RfDecoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(codec: &str, width: u32, height: u32, description: &[u8]) -> Result<RfDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn decode(this: &RfDecoder, data: &[u8], ts: f64, dur: f64, key: bool) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn queue(this: &RfDecoder) -> u32;
    #[wasm_bindgen(method, js_name = startFlush)]
    fn start_flush(this: &RfDecoder);
    #[wasm_bindgen(method, catch)]
    fn reset(this: &RfDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn has(this: &RfDecoder, ts: f64) -> bool;
    #[wasm_bindgen(method)]
    fn count(this: &RfDecoder) -> u32;
    #[wasm_bindgen(method)]
    fn width(this: &RfDecoder, ts: f64) -> u32;
    #[wasm_bindgen(method)]
    fn height(this: &RfDecoder, ts: f64) -> u32;
    #[wasm_bindgen(method)]
    fn pixels(this: &RfDecoder, ts: f64) -> Uint8Array;
    #[wasm_bindgen(method, js_name = dropBelow)]
    fn drop_below(this: &RfDecoder, ts: f64);
    #[wasm_bindgen(method)]
    fn error(this: &RfDecoder) -> Option<String>;
    #[wasm_bindgen(method)]
    fn close(this: &RfDecoder);
}

/// A WebCodecs video encoder.
pub struct WebEncoder {
    enc: RfEncoder,
    codec: VideoCodec,
    codec_string: String,
    hardware: bool,
    size: [u32; 2],
    fps: FrameRate,
    keyint: u64,
    /// Frames out so far, for decode times.
    out: i64,
    config: Option<VideoConfig>,
}

impl WebEncoder {
    /// Opens an encoder for `codec` at `size` and `fps`, aiming at `kbps`, in hardware when the
    /// browser has it. Fails when the browser cannot encode it at all.
    pub async fn open(codec: VideoCodec, size: [u32; 2], fps: FrameRate, kbps: u32) -> Result<WebEncoder, String> {
        let codec_string = codec_string(codec, size, fps.fps());
        let bitrate = kbps as f64 * 1000.0;
        let hw = JsFuture::from(rf_supported(&codec_string, size[0], size[1], bitrate, fps.fps()))
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        if hw.is_empty() {
            return Err(format!("this browser cannot encode {codec_string}"));
        }
        let enc = RfEncoder::new(&codec_string, size[0], size[1], bitrate, fps.fps(), &hw).map_err(js_err)?;
        Ok(WebEncoder {
            enc,
            codec,
            codec_string,
            hardware: hw == "prefer-hardware",
            size,
            fps,
            keyint: (fps.fps() * 2.0).round().max(1.0) as u64,
            out: 0,
            config: None,
        })
    }

    fn check(&self) -> Result<(), ExportError> {
        match self.enc.error() {
            Some(e) => Err(ExportError::Encode(e)),
            None => Ok(()),
        }
    }
}

/// Waits `ms` milliseconds without blocking the page.
pub async fn sleep(ms: u32) {
    let _ = JsFuture::from(rf_sleep(ms)).await;
}

fn js_err(e: JsValue) -> String {
    e.as_string()
        .or_else(|| js_sys::JSON::stringify(&e).ok().and_then(|s| s.as_string()))
        .unwrap_or_else(|| "error".into())
}

impl Drop for WebEncoder {
    fn drop(&mut self) {
        self.enc.close();
    }
}

impl VideoEncoder for WebEncoder {
    fn encode(&mut self, f: &Yuv420, index: u64) -> Result<(), ExportError> {
        self.check()?;
        let ts = index as f64 * 1e6 * f64::from(self.fps.den) / f64::from(self.fps.num);
        let key = index.is_multiple_of(self.keyint);
        self.enc
            .encode(&f.y, &f.u, &f.v, self.size[0], self.size[1], ts.round(), key)
            .map_err(|e| ExportError::Encode(js_err(e)))
    }

    fn take(&mut self) -> Result<Vec<EncodedFrame>, ExportError> {
        self.check()?;
        let n = self.enc.take();
        let den = i64::from(self.fps.den);
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let data = self.enc.data(i).to_vec();
            let index = (self.enc.ts(i) * f64::from(self.fps.num) / (1e6 * den as f64)).round() as i64;
            if self.config.is_none() {
                self.config = match (self.codec, self.enc.description()) {
                    (VideoCodec::H264, Some(d)) => Some(VideoConfig::Avc(d.to_vec())),
                    (VideoCodec::Hevc, Some(d)) => Some(VideoConfig::Hevc(d.to_vec())),
                    (VideoCodec::Av1, Some(d)) => Some(VideoConfig::Av1(d.to_vec())),
                    (VideoCodec::Av1, None) => av1c_from_obus(&data).map(VideoConfig::Av1),
                    _ => None,
                };
            }
            out.push(EncodedFrame { data, pts: index * den, dts: self.out * den, keyframe: self.enc.key(i) });
            self.out += 1;
        }
        Ok(out)
    }

    fn config(&self) -> Option<VideoConfig> {
        self.config.clone()
    }

    fn finish(&mut self) -> Result<(), ExportError> {
        self.check()?;
        self.enc.start_flush();
        Ok(())
    }

    fn backlog(&self) -> u32 {
        // an error ends the wait; the next call reports it
        if self.enc.error().is_some() { 0 } else { self.enc.busy() }
    }

    fn name(&self) -> String {
        format!("{} ({})", self.codec_string, if self.hardware { "hardware" } else { "browser" })
    }
}

/// The browser decoding one clip's footage, for an export that walks it forwards.
pub struct WebDecoder {
    video: DecodableVideo,
    dec: RfDecoder,
    /// Samples fed so far: from `base` up to `next`, in decode order.
    base: usize,
    next: usize,
    /// Pictures before this were let go.
    floor: i64,
    flushed: bool,
    /// Restarts from a key frame; a clip that keeps going backwards is left to the software decoder.
    resets: u32,
}

impl WebDecoder {
    /// A decoder for these file bytes, if they hold H.264 the browser can decode.
    pub async fn open(bytes: Arc<[u8]>) -> Option<WebDecoder> {
        let video = DecodableVideo::open(bytes)?;
        let [w, h] = video.size;
        let ok = JsFuture::from(rf_decodable(&video.codec, w, h, &video.description)).await.ok()?;
        if ok.as_bool() != Some(true) {
            return None;
        }
        let dec = RfDecoder::new(&video.codec, w, h, &video.description).ok()?;
        Some(WebDecoder { video, dec, base: 0, next: 0, floor: i64::MIN, flushed: false, resets: 0 })
    }

    /// The picture shown at `at` in the footage; `None` when the browser fails at it, and the
    /// software decoder should draw it.
    pub async fn picture(&mut self, at: Tick) -> Option<Arc<Image>> {
        const PATIENCE: u32 = 8;
        /// Samples in the browser's queue at most, and twice as many pictures waiting.
        const AHEAD: u32 = 4;
        if self.resets > PATIENCE || self.video.is_empty() {
            return None;
        }
        let target = self.video.sample_at(at);
        let pts = self.video.sample(target)?.1;
        let ts = pts as f64;
        if !self.dec.has(ts) {
            let start = self.video.sync_before(target);
            // behind what was decoded, or past a key frame further on
            if pts < self.floor || target < self.base || start > self.next {
                self.resets += 1;
                if self.resets > PATIENCE {
                    return None;
                }
                self.dec.reset().ok()?;
                (self.base, self.next, self.floor, self.flushed) = (start, start, i64::MIN, false);
            }
        }
        while !self.dec.has(ts) {
            if self.dec.error().is_some() {
                self.resets = PATIENCE + 1;
                return None;
            }
            if self.next < self.video.len() && self.dec.queue() < AHEAD {
                self.feed()?;
                continue;
            }
            if self.next >= self.video.len() {
                if !self.flushed {
                    self.dec.start_flush();
                    self.flushed = true;
                } else if self.dec.queue() == 0 {
                    // everything is out, and not this picture
                    return None;
                }
            }
            sleep(0).await;
        }
        self.dec.drop_below(ts);
        self.floor = pts;
        // keep the browser decoding while this frame is drawn
        while self.next < self.video.len() && self.dec.queue() < AHEAD && self.dec.count() < 2 * AHEAD {
            self.feed()?;
        }
        let (w, h) = (self.dec.width(ts), self.dec.height(ts));
        Some(Arc::new(Image::from_srgba8(w, h, &self.dec.pixels(ts).to_vec())))
    }
}

impl WebDecoder {
    /// Hands the browser the next sample.
    fn feed(&mut self) -> Option<()> {
        let (data, pts, dur, key) = self.video.sample(self.next)?;
        self.dec.decode(data, pts as f64, dur as f64, key).ok()?;
        self.next += 1;
        Some(())
    }
}

impl Drop for WebDecoder {
    fn drop(&mut self) {
        self.dec.close();
    }
}
