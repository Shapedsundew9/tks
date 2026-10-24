//! EA-0: minimal Whisper large-v3-turbo greedy decoder on candle (D-131 fallback).
//!
//! English transcription only, no timestamps, single 30 s window (E-A utterances are
//! endpointed by the VAD and are far shorter than 30 s).

use std::collections::HashMap;
use std::path::Path;

use candle_core::{D, DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as m, Config, audio, model::Whisper};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub struct WhisperAsr {
    model: Whisper,
    config: Config,
    filters: Vec<f32>,
    device: Device,
    dtype: DType,
    prompt: Vec<u32>,
    eot: u32,
    suppress: Tensor,
    id_to_token: HashMap<u32, String>,
    special: std::collections::HashSet<u32>,
}

/// librosa-compatible Slaney mel filterbank (`htk=False, norm="slaney"`), row-major
/// `[n_mels][n_fft/2 + 1]`, matching the filters Whisper was trained with.
fn slaney_filters(sr: f64, n_fft: usize, n_mels: usize) -> Vec<f32> {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    let hz_to_mel = |f: f64| {
        if f >= min_log_hz {
            min_log_mel + (f / min_log_hz).ln() / logstep
        } else {
            f / f_sp
        }
    };
    let mel_to_hz = |mel: f64| {
        if mel >= min_log_mel {
            min_log_hz * (logstep * (mel - min_log_mel)).exp()
        } else {
            f_sp * mel
        }
    };
    let n_freqs = n_fft / 2 + 1;
    let fft_freqs: Vec<f64> = (0..n_freqs).map(|k| k as f64 * sr / n_fft as f64).collect();
    let (mel_lo, mel_hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let hz: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(mel_lo + (mel_hi - mel_lo) * i as f64 / (n_mels + 1) as f64))
        .collect();
    let mut out = vec![0f32; n_mels * n_freqs];
    for i in 0..n_mels {
        let enorm = 2.0 / (hz[i + 2] - hz[i]);
        for (k, &f) in fft_freqs.iter().enumerate() {
            let lower = (f - hz[i]) / (hz[i + 1] - hz[i]);
            let upper = (hz[i + 2] - f) / (hz[i + 2] - hz[i + 1]);
            out[i * n_freqs + k] = (lower.min(upper).max(0.0) * enorm) as f32;
        }
    }
    out
}

/// GPT-2 byte-level BPE: map the printable stand-in characters back to raw bytes.
fn byte_decoder() -> HashMap<char, u8> {
    let mut bs: Vec<u32> = (u32::from(b'!')..=u32::from(b'~'))
        .chain(0xA1..=0xAC)
        .chain(0xAE..=0xFF)
        .collect();
    let mut cs = bs.clone();
    let mut n = 0;
    for b in 0..=255u32 {
        if !bs.contains(&b) {
            bs.push(b);
            cs.push(256 + n);
            n += 1;
        }
    }
    bs.into_iter()
        .zip(cs)
        .filter_map(|(b, c)| char::from_u32(c).map(|c| (c, b as u8)))
        .collect()
}

impl WhisperAsr {
    pub fn load(dir: &Path, device: &Device) -> Res<Self> {
        let config: Config = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
        let tok: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("tokenizer.json"))?)?;
        let mut token_to_id: HashMap<String, u32> = HashMap::new();
        if let Some(vocab) = tok["model"]["vocab"].as_object() {
            for (k, v) in vocab {
                token_to_id.insert(k.clone(), v.as_u64().unwrap_or_default() as u32);
            }
        }
        let mut special = std::collections::HashSet::new();
        for added in tok["added_tokens"].as_array().into_iter().flatten() {
            let id = added["id"].as_u64().unwrap_or_default() as u32;
            token_to_id.insert(added["content"].as_str().unwrap_or("").to_string(), id);
            special.insert(id);
        }
        let get = |t: &str| -> Res<u32> {
            token_to_id
                .get(t)
                .copied()
                .ok_or_else(|| format!("missing whisper token {t}").into())
        };
        let no_ts = get(m::NO_TIMESTAMPS_TOKEN)?;
        let prompt = vec![
            get(m::SOT_TOKEN)?,
            get("<|en|>")?,
            get(m::TRANSCRIBE_TOKEN)?,
            no_ts,
        ];
        let eot = get(m::EOT_TOKEN)?;

        let generation: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("generation_config.json"))?)?;
        let suppressed: std::collections::HashSet<u32> = generation["suppress_tokens"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_u64().map(|x| x as u32))
            .collect();
        // Suppress the generation-config list plus every timestamp token (no-timestamp mode).
        let mask: Vec<f32> = (0..config.vocab_size as u32)
            .map(|i| {
                if suppressed.contains(&i) || i > no_ts {
                    f32::NEG_INFINITY
                } else {
                    0.0
                }
            })
            .collect();
        // candle-transformers 0.9.2 builds the encoder's sinusoidal positions in F32
        // regardless of the VarBuilder dtype, so F16 fails with a dtype mismatch.
        let dtype = m::DTYPE;
        let suppress = Tensor::new(mask.as_slice(), device)?.to_dtype(dtype)?;

        // SAFETY: the weights file is a read-only local cache entry not modified while mapped.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], dtype, device)?
        };
        let model = Whisper::load(&vb, config.clone())?;
        let filters = slaney_filters(m::SAMPLE_RATE as f64, m::N_FFT, config.num_mel_bins);
        let id_to_token = token_to_id.into_iter().map(|(k, v)| (v, k)).collect();
        Ok(Self {
            model,
            config,
            filters,
            device: device.clone(),
            dtype,
            prompt,
            eot,
            suppress,
            id_to_token,
            special,
        })
    }

    fn decode_tokens(&self, ids: &[u32]) -> String {
        let bd = byte_decoder();
        let mut bytes = Vec::new();
        for id in ids {
            if self.special.contains(id) {
                continue;
            }
            if let Some(tok) = self.id_to_token.get(id) {
                bytes.extend(tok.chars().filter_map(|c| bd.get(&c).copied()));
            }
        }
        String::from_utf8_lossy(&bytes).trim().to_string()
    }

    pub fn transcribe(&mut self, pcm: &[f32]) -> Res<String> {
        let mel = audio::pcm_to_mel(&self.config, pcm, &self.filters);
        let n_mels = self.config.num_mel_bins;
        let frames = mel.len() / n_mels;
        let mel = Tensor::from_vec(mel, (1, n_mels, frames), &self.device)?
            .narrow(2, 0, m::N_FRAMES.min(frames))?
            .to_dtype(self.dtype)?;
        let features = self.model.encoder.forward(&mel, true)?;
        let mut tokens = self.prompt.clone();
        let max_len = self.config.max_target_positions / 2;
        for i in 0..max_len {
            let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
            let ys = self.model.decoder.forward(&input, &features, i == 0)?;
            let (_, seq_len, _) = ys.dims3()?;
            let logits = self
                .model
                .decoder
                .final_linear(&ys.i((..1, seq_len - 1..))?)?
                .i(0)?
                .i(0)?
                .broadcast_add(&self.suppress)?;
            let next = logits.argmax(D::Minus1)?.to_scalar::<u32>()?;
            if next == self.eot || tokens.len() >= self.config.max_target_positions {
                break;
            }
            tokens.push(next);
        }
        Ok(self.decode_tokens(&tokens[self.prompt.len()..]))
    }
}
