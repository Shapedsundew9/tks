use candle_core::{Tensor, D};

use crate::decoder::TransformerDecoder;
use crate::encoder::ConformerEncoder;
use crate::tokenizer::Tokenizer;

pub fn transcribe(
    mel: &Tensor,
    encoder: &ConformerEncoder,
    decoder: &TransformerDecoder,
    tokenizer: &Tokenizer,
    language: &str,
    punctuation: bool,
    max_new_tokens: usize,
) -> anyhow::Result<String> {
    let t0 = std::time::Instant::now();
    tracing::debug!("Running encoder...");
    let encoder_hs = encoder.forward(mel)?;
    tracing::info!("Encoder: {:.2?}, shape: {:?}", t0.elapsed(), encoder_hs.dims());

    let t1 = std::time::Instant::now();
    let cross_kv = decoder.precompute_cross_kv(&encoder_hs)?;
    tracing::info!("Cross-KV precompute: {:.2?}", t1.elapsed());

    let prompt = tokenizer.special.build_prompt(language, punctuation)?;
    let n_prompt = prompt.len();
    tracing::debug!("Prompt IDs: {:?}", prompt);

    let (last_logits, mut self_kv_cache) = decoder.prefill(&prompt, &cross_kv)?;

    tracing::info!("Prompt priming: {:.2?} ({} tokens)", t1.elapsed(), n_prompt);
    let t2 = std::time::Instant::now();

    let eos_id = tokenizer.special.eos;
    let nospeech_id = tokenizer.special.nospeech;
    let mut generated: Vec<i64> = Vec::new();

    // GPU-side argmax — only pull the single token ID to CPU
    let mut next_token = gpu_argmax(&last_logits)?;
    let mut position = n_prompt as i64;

    while generated.len() < max_new_tokens {
        if next_token == eos_id || next_token == nospeech_id {
            break;
        }
        generated.push(next_token);

        let (logits, new_kv) = decoder.step(next_token, position, &self_kv_cache, &cross_kv)?;
        self_kv_cache = new_kv;
        position += 1;
        next_token = gpu_argmax(&logits)?;
    }

    tracing::info!(
        "Decoder: {:.2?} ({} tokens, {:.1?}/tok)",
        t2.elapsed(),
        generated.len(),
        t2.elapsed() / generated.len().max(1) as u32
    );

    let text = tokenizer.decode(&generated);
    Ok(text)
}

/// Argmax on GPU — returns a single i64 token ID. Only transfers 1 value to CPU.
fn gpu_argmax(logits: &Tensor) -> candle_core::Result<i64> {
    let idx = logits
        .to_dtype(candle_core::DType::F32)?
        .argmax(D::Minus1)?
        .to_dtype(candle_core::DType::I64)?;
    // Pull single scalar to CPU
    idx.to_scalar::<i64>()
}
