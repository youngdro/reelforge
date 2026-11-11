//! A Kokoro-shaped model far too small to say anything, with random weights, written to disk the
//! way the official one is published (a PyTorch checkpoint and voice files): for testing the
//! plumbing here and in crates that use this one without a download.

use std::io::Write;
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::{VarBuilder, VarMap};

use crate::kokoro::config::KokoroConfig;
use crate::kokoro::model::Network;

/// The settings: the real vocabulary, everything else shrunk.
fn config_json() -> String {
    let vocab: serde_json::Value = serde_json::from_str(include_str!("kokoro/vocab.json")).expect("vocab.json");
    serde_json::json!({
        "hidden_dim": 32, "style_dim": 8, "n_token": 178, "n_layer": 1, "dim_in": 8, "max_dur": 6,
        "decoder_width": 32,
        "plbert": { "hidden_size": 16, "num_attention_heads": 2, "num_hidden_layers": 1,
                    "intermediate_size": 32, "embedding_size": 8 },
        "istftnet": { "upsample_initial_channel": 16, "resblock_kernel_sizes": [3],
                      "resblock_dilation_sizes": [[1, 3, 5]] },
        "vocab": vocab,
    })
    .to_string()
}

/// Writes `config.json`, `kokoro-v1_0.pth` and `voices/zf_xiaoxiao.pt` into `dir`.
pub fn write(dir: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| e.to_string();
    let json = config_json();
    let config = KokoroConfig::from_bytes(&json)?;
    let style = config.full_style_dim();
    let vars = VarMap::new();
    Network::new(config, VarBuilder::from_varmap(&vars, DType::F32, &Device::Cpu))?;
    // weights a builder makes start at zero, which a weight-normalised layer divides by
    for var in vars.all_vars() {
        let noise = Tensor::randn(0f32, 0.2, var.shape(), &Device::Cpu).map_err(|e| e.to_string())?;
        var.set(&noise).map_err(|e| e.to_string())?;
    }
    // the checkpoint is a dictionary of the five parts, each saved through `DataParallel`
    let mut parts: Vec<(String, Vec<(String, Tensor)>)> = Vec::new();
    let data = vars.data().lock().expect("not poisoned");
    let mut names: Vec<&String> = data.keys().collect();
    names.sort();
    for name in names {
        let (top, rest) = name.split_once('.').expect("every weight is in a part");
        if parts.last().is_none_or(|(t, _)| t != top) {
            parts.push((top.to_string(), Vec::new()));
        }
        let t = data[name].as_tensor().clone();
        parts.last_mut().expect("just pushed").1.push((format!("module.{rest}"), t));
    }
    std::fs::create_dir_all(dir.join("voices")).map_err(err)?;
    std::fs::write(dir.join("config.json"), json).map_err(err)?;
    std::fs::write(dir.join("kokoro-v1_0.pth"), pth(Root::Parts(&parts))?).map_err(err)?;
    let voice = Tensor::randn(0f32, 0.5, (510, 1, style), &Device::Cpu).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("voices").join("zf_xiaoxiao.pt"), pth(Root::Bare(&voice))?).map_err(err)?;
    Ok(())
}

enum Root<'a> {
    Bare(&'a Tensor),
    Parts(&'a [(String, Vec<(String, Tensor)>)]),
}

/// A PyTorch zip checkpoint (pickle protocol 2) holding `root`.
fn pth(root: Root) -> Result<Vec<u8>, String> {
    let mut p = vec![0x80, 2];
    let mut storages: Vec<Vec<u8>> = Vec::new();
    match root {
        Root::Bare(t) => tensor(&mut p, &mut storages, t)?,
        Root::Parts(parts) => {
            p.extend(b"}(");
            for (top, weights) in parts {
                unicode(&mut p, top);
                p.extend(b"}(");
                for (name, t) in weights {
                    unicode(&mut p, name);
                    tensor(&mut p, &mut storages, t)?;
                }
                p.push(b'u');
            }
            p.push(b'u');
        }
    }
    p.push(b'.');
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let err = |e: zip::result::ZipError| e.to_string();
    zip.start_file("archive/data.pkl", opts).map_err(err)?;
    zip.write_all(&p).map_err(|e| e.to_string())?;
    for (i, s) in storages.iter().enumerate() {
        zip.start_file(format!("archive/data/{i}"), opts).map_err(err)?;
        zip.write_all(s).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(err)?.into_inner())
}

fn unicode(p: &mut Vec<u8>, s: &str) {
    p.push(b'X');
    p.extend((s.len() as u32).to_le_bytes());
    p.extend(s.as_bytes());
}

fn global(p: &mut Vec<u8>, module: &str, name: &str) {
    p.push(b'c');
    p.extend(format!("{module}\n{name}\n").as_bytes());
}

fn int(p: &mut Vec<u8>, v: usize) {
    p.push(b'J');
    p.extend((v as i32).to_le_bytes());
}

fn ints(p: &mut Vec<u8>, vs: &[usize]) {
    p.push(b'(');
    for &v in vs {
        int(p, v);
    }
    p.push(b't');
}

/// `torch._utils._rebuild_tensor_v2(storage, 0, size, stride, False, OrderedDict())`.
fn tensor(p: &mut Vec<u8>, storages: &mut Vec<Vec<u8>>, t: &Tensor) -> Result<(), String> {
    let values: Vec<f32> = t.flatten_all().and_then(|t| t.to_vec1()).map_err(|e| e.to_string())?;
    global(p, "torch._utils", "_rebuild_tensor_v2");
    p.push(b'(');
    p.push(b'(');
    unicode(p, "storage");
    global(p, "torch", "FloatStorage");
    unicode(p, &storages.len().to_string());
    unicode(p, "cpu");
    int(p, values.len());
    p.extend(b"tQ");
    int(p, 0);
    ints(p, t.dims());
    ints(p, t.stride());
    p.push(0x89);
    global(p, "collections", "OrderedDict");
    p.extend(b")R");
    p.extend(b"tR");
    storages.push(values.iter().flat_map(|v| v.to_le_bytes()).collect());
    Ok(())
}
