//! EA-0 spike harness: local ASR runtime, voice-activity gate and audio-buffer hygiene.
//!
//! Runs Cohere Transcribe (primary, D-131) or Whisper large-v3-turbo (fallback) fully
//! offline on a documented English test set and reports load time, peak memory, real-time
//! factor, end-of-utterance latency, WER, correction effort and VAD behaviour as JSON.
//!
//! ```text
//! cargo run --release --features asr-spike --bin ea0_asr_spike -- \
//!     --model cohere --models-dir ~/.cache/tks/models --testset ~/.cache/tks/ea0-testset
//! ```

mod wer;
mod whisper;

use std::path::{Path, PathBuf};
use std::time::Instant;

use candle_core::{DType, Device, Tensor};
use clap::{Parser, ValueEnum};
use cohere_transcribe_candle::{audio, config, decoder, encoder, inference, tokenizer, weights};
use serde::{Deserialize, Serialize};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const SR: usize = 16_000;
const VAD_FRAME: usize = 256;
const NOISE_SNR_DB: f64 = 10.0;
const SEED: u64 = 20_261_023;

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ModelKind {
    Cohere,
    Whisper,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
enum DeviceKind {
    Cpu,
    Cuda,
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, value_enum)]
    model: ModelKind,
    #[arg(long, value_enum, default_value = "cpu")]
    device: DeviceKind,
    #[arg(long)]
    models_dir: PathBuf,
    #[arg(long)]
    testset: PathBuf,
    /// Optional JSON output path (stdout when omitted).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Score threshold for a 16 ms frame to count as speech.
    #[arg(long, default_value_t = 0.4)]
    vad_threshold: f32,
    /// Minimum sustained speech run required before an utterance is sent to ASR.
    #[arg(long, default_value_t = 500)]
    vad_min_run_ms: usize,
    /// Trailing non-speech that closes an utterance in the streaming design (EoU hangover).
    #[arg(long, default_value_t = 600)]
    vad_hangover_ms: usize,
    /// Only evaluate the first N speech clips per category (smoke runs).
    #[arg(long)]
    limit: Option<usize>,
    /// Print VAD decisions for every clip and exit without loading a model.
    #[arg(long)]
    vad_only: bool,
}

#[derive(Debug, Deserialize)]
struct ManifestClip {
    id: String,
    category: String,
    file: String,
    reference: String,
    source: String,
    license: String,
}

/// Owns raw PCM for exactly one utterance and zeroes it on drop (D-132 prototype).
///
/// Audio is never cloned out of this buffer except into model input tensors that share its
/// scope; nothing here implements `Debug`, `Serialize` or `Display`, so samples cannot reach
/// logs or reports by accident.
struct UtteranceBuffer {
    samples: Vec<f32>,
}

impl UtteranceBuffer {
    const MAX_SECONDS: usize = 30;

    fn new(samples: Vec<f32>) -> Res<Self> {
        if samples.len() > Self::MAX_SECONDS * SR {
            return Err(format!("utterance exceeds {} s bound", Self::MAX_SECONDS).into());
        }
        Ok(Self { samples })
    }

    fn pcm(&self) -> &[f32] {
        &self.samples
    }

    fn seconds(&self) -> f64 {
        self.samples.len() as f64 / SR as f64
    }
}

impl Drop for UtteranceBuffer {
    fn drop(&mut self) {
        self.samples.fill(0.0);
        std::hint::black_box(&self.samples);
    }
}

#[derive(Debug, Serialize, Clone, Copy)]
struct VadDecision {
    speech_ms: usize,
    longest_run_ms: usize,
    max_score: f32,
    passed: bool,
}

/// Gate: an utterance reaches ASR only if it contains a sustained run of speech frames
/// (one non-speech frame of slack is tolerated inside a run) of at least `min_run_ms`.
fn vad_gate(pcm: &[f32], threshold: f32, min_run_ms: usize) -> VadDecision {
    let mut detector = earshot::Detector::default_boxed();
    let (mut speech_frames, mut run, mut gap, mut longest) = (0usize, 0usize, 0usize, 0usize);
    let mut max_score = 0f32;
    let mut frame = [0f32; VAD_FRAME];
    for chunk in pcm.chunks(VAD_FRAME) {
        frame.fill(0.0);
        for (dst, src) in frame.iter_mut().zip(chunk) {
            *dst = src.clamp(-1.0, 1.0);
        }
        let score = detector.predict_f32(&frame);
        max_score = max_score.max(score);
        if score > threshold {
            speech_frames += 1;
            run += 1 + gap;
            gap = 0;
            longest = longest.max(run);
        } else if run > 0 && gap == 0 {
            gap = 1;
        } else {
            run = 0;
            gap = 0;
        }
    }
    frame.fill(0.0);
    let to_ms = |frames: usize| frames * VAD_FRAME * 1000 / SR;
    VadDecision {
        speech_ms: to_ms(speech_frames),
        longest_run_ms: to_ms(longest),
        max_score,
        passed: to_ms(longest) >= min_run_ms,
    }
}

/// Deterministic xorshift64* + Box-Muller so noisy variants are reproducible without deps.
struct Rng(u64);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn gauss(&mut self) -> f64 {
        let u1 = self.next_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

fn add_noise(pcm: &[f32], snr_db: f64, rng: &mut Rng) -> Vec<f32> {
    let power = pcm.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / pcm.len().max(1) as f64;
    let sigma = (power / 10f64.powf(snr_db / 10.0)).sqrt();
    pcm.iter()
        .map(|s| (f64::from(*s) + rng.gauss() * sigma) as f32)
        .collect()
}

fn nonspeech_clips(rng: &mut Rng) -> Vec<(String, Vec<f32>)> {
    let n = SR * 10;
    vec![
        ("silence-digital".into(), vec![0.0; n]),
        (
            "silence-room-noise".into(),
            (0..n).map(|_| (rng.gauss() * 3e-4) as f32).collect(),
        ),
        (
            "noise-loud-white".into(),
            (0..n).map(|_| (rng.gauss() * 0.03) as f32).collect(),
        ),
        (
            "noise-mains-hum".into(),
            (0..n)
                .map(|i| {
                    (0.025 * (2.0 * std::f64::consts::PI * 50.0 * i as f64 / SR as f64).sin())
                        as f32
                })
                .collect(),
        ),
    ]
}

struct CohereAsr {
    mel_cfg: audio::MelConfig,
    enc: encoder::ConformerEncoder,
    dec: decoder::TransformerDecoder,
    tok: tokenizer::Tokenizer,
    device: Device,
}

enum Engine {
    Cohere(Box<CohereAsr>),
    Whisper(Box<whisper::WhisperAsr>),
}

impl Engine {
    fn load(kind: ModelKind, models_dir: &Path, device: &Device) -> Res<Self> {
        match kind {
            ModelKind::Cohere => {
                let dir = models_dir.join("cohere-transcribe-03-2026");
                let cfg = config::ModelConfig::load(&dir)?;
                let tok = tokenizer::Tokenizer::load(&dir)?;
                let mel_cfg = audio::MelConfig::from_model_config(&cfg);
                let w = weights::Weights::load(dir.join("model.safetensors"), device)?;
                let enc = encoder::ConformerEncoder::load(&w, &cfg)?;
                let dec = decoder::TransformerDecoder::load(&w, &cfg, device)?;
                Ok(Self::Cohere(Box::new(CohereAsr {
                    mel_cfg,
                    enc,
                    dec,
                    tok,
                    device: device.clone(),
                })))
            }
            ModelKind::Whisper => Ok(Self::Whisper(Box::new(whisper::WhisperAsr::load(
                &models_dir.join("whisper-large-v3-turbo"),
                device,
            )?))),
        }
    }

    fn transcribe(&mut self, utt: &UtteranceBuffer) -> Res<String> {
        match self {
            Self::Cohere(c) => {
                let CohereAsr {
                    mel_cfg,
                    enc,
                    dec,
                    tok,
                    device,
                } = c.as_ref();
                let pcm = utt.pcm();
                let mut dithered = audio::add_dither(pcm, mel_cfg.dither as f32, pcm.len() as u64);
                let mel = audio::compute_mel_features(&dithered, mel_cfg);
                dithered.fill(0.0);
                let (flat, shape) = audio::mel_to_tensor_data(&mel);
                let dtype = if device.is_cuda() {
                    DType::F16
                } else {
                    DType::F32
                };
                let mel_t = Tensor::from_vec(flat, shape.as_slice(), device)?.to_dtype(dtype)?;
                Ok(inference::transcribe(
                    &mel_t, enc, dec, tok, "en", true, 448,
                )?)
            }
            Self::Whisper(w) => w.transcribe(utt.pcm()),
        }
    }
}

#[derive(Debug, Serialize)]
struct ClipResult {
    id: String,
    category: String,
    source: String,
    license: String,
    duration_s: f64,
    vad: VadDecision,
    transcribed: bool,
    hypothesis: String,
    reference: String,
    edits: wer::Edits,
    processing_ms: f64,
    rtf: f64,
}

#[derive(Debug, Serialize)]
struct CategorySummary {
    category: String,
    clips: usize,
    audio_s: f64,
    wer: f64,
    edits: wer::Edits,
    edits_per_minute: f64,
    clips_needing_no_correction: usize,
    vad_rejected: usize,
}

#[derive(Debug, Serialize)]
struct NonSpeechResult {
    id: String,
    vad: VadDecision,
    ungated_hypothesis: String,
    gated_output: String,
}

#[derive(Debug, Serialize)]
struct Report {
    model: ModelKind,
    model_revision: String,
    device: DeviceKind,
    threads: usize,
    load_s: f64,
    rss_after_load_mib: f64,
    peak_rss_mib: f64,
    vad: serde_json::Value,
    total_audio_s: f64,
    total_processing_s: f64,
    rtf_overall: f64,
    processing_ms_p50: f64,
    processing_ms_p95: f64,
    eou_latency_ms_p50: f64,
    eou_latency_ms_p95: f64,
    categories: Vec<CategorySummary>,
    nonspeech: Vec<NonSpeechResult>,
    clips: Vec<ClipResult>,
}

fn proc_status_mib(key: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(f64::NAN, |kib| kib / 1024.0)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

fn main() -> Res<()> {
    let args = Args::parse();
    let device = match args.device {
        DeviceKind::Cpu => Device::Cpu,
        DeviceKind::Cuda => Device::new_cuda(0)?,
    };
    let model_dir_name = match args.model {
        ModelKind::Cohere => "cohere-transcribe-03-2026",
        ModelKind::Whisper => "whisper-large-v3-turbo",
    };
    let model_revision =
        std::fs::read_to_string(args.models_dir.join(model_dir_name).join("REVISION"))
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("@");

    let manifest: Vec<ManifestClip> =
        serde_json::from_slice(&std::fs::read(args.testset.join("manifest.json"))?)?;

    if args.vad_only {
        let mut rng = Rng(SEED);
        for clip in &manifest {
            let base = audio::load_audio(args.testset.join(&clip.file), SR)
                .map_err(|e| format!("{}: {e}", clip.file))?;
            let noisy = add_noise(&base, NOISE_SNR_DB, &mut rng);
            for (variant, pcm) in [("", base), ("noisy-", noisy)] {
                let utt = UtteranceBuffer::new(pcm)?;
                let vad = vad_gate(utt.pcm(), args.vad_threshold, args.vad_min_run_ms);
                println!("{variant}{} {vad:?}", clip.id);
            }
        }
        for (id, pcm) in nonspeech_clips(&mut rng) {
            let utt = UtteranceBuffer::new(pcm)?;
            println!(
                "{id} {:?}",
                vad_gate(utt.pcm(), args.vad_threshold, args.vad_min_run_ms)
            );
        }
        return Ok(());
    }

    let t_load = Instant::now();
    let mut engine = Engine::load(args.model, &args.models_dir, &device)?;
    // Warm-up so one-off kernel/allocator setup is not billed to the first clip.
    engine.transcribe(&UtteranceBuffer::new(vec![0.0; SR])?)?;
    let load_s = t_load.elapsed().as_secs_f64();
    let rss_after_load_mib = proc_status_mib("VmRSS:");
    eprintln!(
        "[ea0] loaded {:?} on {:?} in {load_s:.1}s",
        args.model, args.device
    );

    let mut rng = Rng(SEED);
    let mut per_category: std::collections::BTreeMap<String, usize> = Default::default();
    let mut clips = Vec::new();
    for clip in &manifest {
        let seen = per_category.entry(clip.category.clone()).or_default();
        if args.limit.is_some_and(|l| *seen >= l) {
            continue;
        }
        *seen += 1;
        let base = audio::load_audio(args.testset.join(&clip.file), SR)
            .map_err(|e| format!("{}: {e}", clip.file))?;
        let noisy = add_noise(&base, NOISE_SNR_DB, &mut rng);
        for (variant, pcm) in [("", base), ("noisy-", noisy)] {
            let utt = UtteranceBuffer::new(pcm)?;
            let vad = vad_gate(utt.pcm(), args.vad_threshold, args.vad_min_run_ms);
            let start = Instant::now();
            let hypothesis = if vad.passed {
                engine.transcribe(&utt)?
            } else {
                String::new()
            };
            let processing_ms = start.elapsed().as_secs_f64() * 1000.0;
            let duration_s = utt.seconds();
            drop(utt);
            let edits = wer::score(&clip.reference, &hypothesis);
            eprintln!(
                "[ea0] {variant}{} {:.1}s -> {:.0} ms, edits {}/{}",
                clip.id,
                duration_s,
                processing_ms,
                edits.total(),
                edits.reference_words
            );
            clips.push(ClipResult {
                id: format!("{variant}{}", clip.id),
                category: format!("{variant}{}", clip.category),
                source: if variant.is_empty() {
                    clip.source.clone()
                } else {
                    format!(
                        "{} + white noise {NOISE_SNR_DB} dB SNR (seed {SEED})",
                        clip.source
                    )
                },
                license: clip.license.clone(),
                duration_s,
                vad,
                transcribed: vad.passed,
                hypothesis,
                reference: clip.reference.clone(),
                edits,
                processing_ms,
                rtf: processing_ms / 1000.0 / duration_s,
            });
        }
    }

    let mut nonspeech = Vec::new();
    for (id, pcm) in nonspeech_clips(&mut rng) {
        let utt = UtteranceBuffer::new(pcm)?;
        let vad = vad_gate(utt.pcm(), args.vad_threshold, args.vad_min_run_ms);
        let ungated_hypothesis = engine.transcribe(&utt)?;
        let gated_output = if vad.passed {
            ungated_hypothesis.clone()
        } else {
            String::new()
        };
        eprintln!("[ea0] {id}: vad {vad:?}, ungated {ungated_hypothesis:?}");
        nonspeech.push(NonSpeechResult {
            id,
            vad,
            ungated_hypothesis,
            gated_output,
        });
    }

    let mut categories: Vec<String> = clips.iter().map(|c| c.category.clone()).collect();
    categories.sort();
    categories.dedup();
    let summaries = categories
        .into_iter()
        .map(|cat| {
            let members: Vec<&ClipResult> = clips.iter().filter(|c| c.category == cat).collect();
            let mut edits = wer::Edits::default();
            members.iter().for_each(|c| edits.add(&c.edits));
            let audio_s: f64 = members.iter().map(|c| c.duration_s).sum();
            CategorySummary {
                clips: members.len(),
                audio_s,
                wer: edits.wer(),
                edits_per_minute: edits.total() as f64 / (audio_s / 60.0),
                clips_needing_no_correction: members
                    .iter()
                    .filter(|c| c.edits.total() == 0)
                    .count(),
                vad_rejected: members.iter().filter(|c| !c.transcribed).count(),
                edits,
                category: cat,
            }
        })
        .collect();

    let mut times: Vec<f64> = clips.iter().map(|c| c.processing_ms).collect();
    times.sort_by(f64::total_cmp);
    let total_audio_s: f64 = clips.iter().map(|c| c.duration_s).sum();
    let total_processing_s = times.iter().sum::<f64>() / 1000.0;
    let hangover = args.vad_hangover_ms as f64;
    let report = Report {
        model: args.model,
        model_revision,
        device: args.device,
        threads: std::thread::available_parallelism().map_or(1, usize::from),
        load_s,
        rss_after_load_mib,
        peak_rss_mib: proc_status_mib("VmHWM:"),
        vad: serde_json::json!({
            "engine": "earshot 1.2 (16 ms frames)",
            "threshold": args.vad_threshold,
            "min_run_ms": args.vad_min_run_ms,
            "hangover_ms": args.vad_hangover_ms,
        }),
        total_audio_s,
        total_processing_s,
        rtf_overall: total_processing_s / total_audio_s,
        processing_ms_p50: percentile(&times, 0.5),
        processing_ms_p95: percentile(&times, 0.95),
        eou_latency_ms_p50: hangover + percentile(&times, 0.5),
        eou_latency_ms_p95: hangover + percentile(&times, 0.95),
        categories: summaries,
        nonspeech,
        clips,
    };
    let json = serde_json::to_string_pretty(&report)?;
    match &args.out {
        Some(path) => std::fs::write(path, json)?,
        None => println!("{json}"),
    }
    Ok(())
}
