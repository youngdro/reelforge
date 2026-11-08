//! The browser's video encoders (WebCodecs), usually in hardware: H.264, HEVC and AV1. Frames go
//! in as I420 and come back asynchronously; [`WebEncoder`] is a [`VideoEncoder`] an
//! [`ExportWriter`](reelforge_media::ExportWriter) drives, its backlog above 0 until the last frames
//! are back.

use js_sys::{Promise, Uint8Array};
use reelforge_renderspec::VideoCodec;
use reelforge_time::FrameRate;
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
