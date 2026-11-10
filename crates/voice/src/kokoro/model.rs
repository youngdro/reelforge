//! Kokoro-82M: phoneme ids and a voice's style → 24 kHz speech.
//!
//! PL-BERT (ALBERT) encodes the phonemes, the prosody predictor gives each one a length, a pitch
//! and a noise curve, and the ISTFTNet decoder makes the waveform. A voice is a `[510, 1, 256]`
//! table of style vectors, one per input length. Adapted from the MIT / Apache-2.0 `any-tts`
//! crate.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use candle_core::{DType, Device, IndexOp, Module, Tensor};
use candle_nn::VarBuilder;

use super::albert::Albert;
use super::config::KokoroConfig;
use super::decoder::IstftDecoder;
use super::prosody::ProsodyPredictor;
use super::text_encoder::TextEncoder;

/// The last row a voice table has.
const MAX_VOICE_OFFSET: usize = 509;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The loaded network.
pub struct Network {
    pub(crate) config: KokoroConfig,
    bert: Albert,
    bert_encoder: candle_nn::Linear,
    predictor: ProsodyPredictor,
    text_encoder: TextEncoder,
    decoder: IstftDecoder,
    device: Device,
}

impl Network {
    /// Builds the network from `vb` (weights named as in the official checkpoint).
    pub fn new(config: KokoroConfig, vb: VarBuilder) -> Result<Network, String> {
        let device = vb.device().clone();
        let c = &config;
        let bert = Albert::load(&c.plbert, vb.pp("bert"), &device).map_err(err)?;
        let bert_encoder = candle_nn::linear(c.plbert.hidden_size, c.hidden_dim, vb.pp("bert_encoder")).map_err(err)?;
        let predictor =
            ProsodyPredictor::load(c.style_dim, c.hidden_dim, c.n_layer, c.max_dur, vb.pp("predictor"), &device)
                .map_err(err)?;
        let text_encoder = TextEncoder::load(
            c.hidden_dim,
            c.text_encoder_kernel_size,
            c.n_layer,
            c.n_token,
            vb.pp("text_encoder"),
            &device,
        )
        .map_err(err)?;
        let i = &c.istftnet;
        let decoder = IstftDecoder::load(
            c.hidden_dim,
            c.style_dim,
            c.n_mels,
            &i.resblock_kernel_sizes,
            &i.upsample_rates,
            i.upsample_initial_channel,
            &i.resblock_dilation_sizes,
            &i.upsample_kernel_sizes,
            i.gen_istft_n_fft,
            i.gen_istft_hop_size,
            c.decoder_width,
            c.dim_in,
            vb.pp("decoder"),
            &device,
            DType::F32,
        )
        .map_err(err)?;
        Ok(Network { config, bert, bert_encoder, predictor, text_encoder, decoder, device })
    }

    /// The style row of `voice` (`[N, 1, 256]` or `[N, 256]`) for an input of `tokens` ids.
    pub fn style(voice: &Tensor, tokens: usize) -> Result<Tensor, String> {
        let voice = if voice.rank() == 3 { voice.squeeze(1).map_err(err)? } else { voice.clone() };
        let n = voice.dim(0).map_err(err)?;
        let row = tokens.saturating_sub(3).min(n.saturating_sub(1)).min(MAX_VOICE_OFFSET);
        voice.i(row).and_then(|r| r.unsqueeze(0)).map_err(err)
    }

    /// The waveform for `ids` (already padded with 0 at both ends) in the style `style`
    /// (`[1, 2 × style_dim]`), `speed` times as fast as the voice speaks.
    pub fn forward(&self, ids: &[u32], style: &Tensor, speed: f32) -> Result<Vec<f32>, String> {
        self.forward_inner(ids, style, speed).map_err(err)
    }

    fn forward_inner(&self, ids: &[u32], style: &Tensor, speed: f32) -> candle_core::Result<Vec<f32>> {
        let n = ids.len();
        let ids_t = Tensor::new(ids, &self.device)?.unsqueeze(0)?;
        let lengths = Tensor::new(&[n as u32], &self.device)?;
        let mask = Tensor::zeros((1, n), DType::U8, &self.device)?;
        let d_en = self.bert_encoder.forward(&self.bert.forward_unmasked(&ids_t)?)?.transpose(1, 2)?;
        // a style row is the decoder's half, then the predictor's
        let half = self.config.style_dim;
        let s_pred = style.narrow(1, half, half)?;
        let s_dec = style.narrow(1, 0, half)?;
        let d = self.predictor.text_encoder.forward(&d_en, &s_pred, &lengths, &mask)?;
        let duration = self.predictor.predict_duration(&d, &s_pred)?;
        let duration: Vec<f32> = (duration / speed as f64)?.squeeze(0)?.to_dtype(DType::F32)?.to_vec1()?;
        if duration.iter().any(|d| !d.is_finite()) {
            candle_core::bail!("the model predicted a length that is not a number");
        }
        // each phoneme lasts at least one frame
        let mut frames = Vec::new();
        for (i, d) in duration.iter().enumerate() {
            frames.extend(std::iter::repeat_n(i as u32, ((d + 2.5e-4).round().max(1.0)) as usize));
        }
        let frames = Tensor::from_vec(frames.clone(), frames.len(), &self.device)?;
        let en = d.transpose(1, 2)?.contiguous()?.index_select(&frames, 2)?;
        let (f0, noise) = self.predictor.f0_n_predict(&en, &s_pred)?;
        let t_en = self.text_encoder.forward(&ids_t, &lengths, &mask)?;
        let asr = t_en.contiguous()?.index_select(&frames, 2)?;
        let audio = self.decoder.forward(&asr, &f0, &noise, &s_dec)?;
        audio.flatten_all()?.to_dtype(DType::F32)?.to_vec1()
    }
}

/// Reads a PyTorch checkpoint (a zip of `data.pkl` and tensor payloads). With `key`, the tensors
/// of that entry of a top-level dictionary; a bare tensor comes back under the name `""`.
pub fn read_pth(bytes: &[u8], key: Option<&str>) -> Result<HashMap<String, Tensor>, String> {
    use candle_core::pickle::{Object, Stack};
    let open = || zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a PyTorch file: {e}"));
    let mut zip = open()?;
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let mut infos = Vec::new();
    for name in names.iter().filter(|n| n.ends_with("data.pkl")) {
        let dir = std::path::PathBuf::from(name.strip_suffix(".pkl").expect("ends with it"));
        let mut stack = Stack::empty();
        stack.read_loop(&mut std::io::BufReader::new(zip.by_name(name).map_err(err)?)).map_err(err)?;
        let mut obj = stack.finalize().map_err(err)?;
        if let Some(key) = key {
            obj = match obj {
                Object::Dict(kv) => kv
                    .into_iter()
                    .find(|(k, _)| *k == Object::Unicode(key.to_string()))
                    .map(|(_, v)| v)
                    .ok_or(format!("the checkpoint has no {key:?}"))?,
                other => other,
            };
        }
        match obj {
            Object::Dict(kv) => {
                for (k, v) in kv {
                    if let Ok(Some(info)) = v.into_tensor_info(k, &dir) {
                        infos.push(info);
                    }
                }
            }
            other => {
                if let Ok(Some(info)) = other.into_tensor_info(Object::Unicode(String::new()), &dir) {
                    infos.push(info);
                }
            }
        }
    }
    let mut out = HashMap::new();
    for info in infos {
        let mut zip = open()?;
        let mut r = zip.by_name(&info.path).map_err(err)?;
        // the offset is in bytes already
        let start = info.layout.start_offset();
        std::io::copy(&mut r.by_ref().take(start as u64), &mut std::io::sink()).map_err(err)?;
        let dims = info.layout.shape().dims().to_vec();
        let mut raw = vec![0u8; info.layout.shape().elem_count() * info.dtype.size_in_bytes()];
        r.read_exact(&mut raw).map_err(err)?;
        let fortran = info.layout.is_fortran_contiguous() && dims.len() > 1 && !info.layout.is_contiguous();
        let t = if fortran {
            let rev: Vec<usize> = dims.iter().rev().copied().collect();
            Tensor::from_raw_buffer(&raw, info.dtype, &rev, &Device::Cpu)
                .and_then(|t| t.permute((0..dims.len()).rev().collect::<Vec<_>>()))
        } else {
            Tensor::from_raw_buffer(&raw, info.dtype, &dims, &Device::Cpu)
        }
        .and_then(|t| t.to_dtype(DType::F32))
        .map_err(err)?;
        out.insert(info.name.clone(), t);
    }
    Ok(out)
}

/// The official checkpoint's weights (`bert`, `bert_encoder`, `predictor`, `text_encoder`,
/// `decoder`, each saved through `DataParallel`), as one variable store.
pub fn checkpoint(path: &Path) -> Result<VarBuilder<'static>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut all = HashMap::new();
    for top in ["bert", "bert_encoder", "predictor", "text_encoder", "decoder"] {
        for (name, t) in read_pth(&bytes, Some(top))? {
            all.insert(format!("{top}.{name}").replace(".module.", "."), t);
        }
    }
    if all.is_empty() {
        return Err(format!("{} holds no Kokoro weights", path.display()));
    }
    Ok(VarBuilder::from_tensors(all, DType::F32, &Device::Cpu))
}
