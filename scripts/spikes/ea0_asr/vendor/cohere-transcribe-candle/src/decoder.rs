/// Transformer decoder for the Cohere Transcribe ASR model,
/// translated to the candle ML framework with optional flash attention.
use candle_core::{DType, Device, Result, Tensor, D};
#[cfg(not(feature = "flash-attn"))]
use candle_nn::ops;

use crate::config::ModelConfig;
use crate::weights::Weights;

fn matmul_2d(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    let a_dims = a.dims().to_vec();
    let b = b.contiguous()?;
    if a_dims.len() <= 2 {
        return a.matmul(&b);
    }
    let m: usize = a_dims[..a_dims.len() - 1].iter().product();
    let k = *a_dims.last().unwrap();
    let a2d = a.reshape(&[m, k])?.contiguous()?;
    let out2d = a2d.matmul(&b)?;
    let mut out_shape = a_dims[..a_dims.len() - 1].to_vec();
    out_shape.push(out2d.dims()[1]);
    out2d.reshape(out_shape.as_slice())
}

fn linear(x: &Tensor, w: &Tensor, b: &Tensor) -> Result<Tensor> {
    matmul_2d(x, w)?.broadcast_add(b)
}

fn layer_norm(x: &Tensor, w: &Tensor, b: &Tensor) -> Result<Tensor> {
    let eps = 1e-5;
    let orig_dtype = x.dtype();
    let x = x.to_dtype(candle_core::DType::F32)?;
    let mean = x.mean_keepdim(D::Minus1)?;
    let diff = x.broadcast_sub(&mean)?;
    let var = diff.sqr()?.mean_keepdim(D::Minus1)?;
    let std = (var + eps)?.sqrt()?;
    let w = w.to_dtype(candle_core::DType::F32)?;
    let b = b.to_dtype(candle_core::DType::F32)?;
    let out = diff.broadcast_div(&std)?.broadcast_mul(&w)?.broadcast_add(&b)?;
    out.to_dtype(orig_dtype)
}

#[cfg(feature = "flash-attn")]
fn sdpa(q: &Tensor, k: &Tensor, v: &Tensor, _mask: Option<&Tensor>) -> Result<Tensor> {
    let scale = 1.0 / (q.dim(D::Minus1)? as f64).sqrt();
    // flash_attn expects (B, T, H, D) — our input is (B, H, T, D)
    let q = q.transpose(1, 2)?.contiguous()?;
    let k = k.transpose(1, 2)?.contiguous()?;
    let v = v.transpose(1, 2)?.contiguous()?;
    let out = candle_flash_attn::flash_attn(&q, &k, &v, scale as f32, false)?;
    out.transpose(1, 2) // back to (B, H, T, D)
}

#[cfg(not(feature = "flash-attn"))]
fn sdpa(q: &Tensor, k: &Tensor, v: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
    let d_k = *q.dims().last().unwrap();
    let scale = 1.0 / (d_k as f64).sqrt();
    let scores = q.contiguous()?.matmul(&k.transpose(2, 3)?.contiguous()?)?.affine(scale, 0.0)?;
    let scores = match mask {
        Some(m) => scores.broadcast_add(m)?,
        None => scores,
    };
    let attn = ops::softmax(&scores.to_dtype(candle_core::DType::F32)?, D::Minus1)?
        .to_dtype(scores.dtype())?;
    attn.contiguous()?.matmul(&v.contiguous()?)
}

struct DecoderAttn {
    q_w: Tensor,
    q_b: Tensor,
    k_w: Tensor,
    k_b: Tensor,
    v_w: Tensor,
    v_b: Tensor,
    out_w: Tensor,
    out_b: Tensor,
    n_heads: usize,
    head_dim: usize,
    hidden: usize,
}

impl DecoderAttn {
    fn load(
        weights: &Weights,
        prefix: &str,
        n_heads: usize,
        hidden: usize,
    ) -> anyhow::Result<Self> {
        let head_dim = hidden / n_heads;
        let b = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get(&format!("{}{}", prefix, n))?.clone())
        };
        let wt = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get_t(&format!("{}{}", prefix, n))?)
        };
        Ok(Self {
            q_w: wt("query_net.weight")?,
            q_b: b("query_net.bias")?,
            k_w: wt("key_net.weight")?,
            k_b: b("key_net.bias")?,
            v_w: wt("value_net.weight")?,
            v_b: b("value_net.bias")?,
            out_w: wt("out_projection.weight")?,
            out_b: b("out_projection.bias")?,
            n_heads,
            head_dim,
            hidden,
        })
    }

    fn project_qkv(
        &self,
        hidden_states: &Tensor,
        source: &Tensor,
    ) -> Result<(Tensor, Tensor, Tensor)> {
        let (b, t, _) = hidden_states.dims3()?;
        let s = source.dims()[1];
        let reshape_q = |z: Tensor| -> Result<Tensor> {
            z.reshape(&[b, t, self.n_heads, self.head_dim])?
                .transpose(1, 2)
        };
        let reshape_kv = |z: Tensor, seq: usize| -> Result<Tensor> {
            z.reshape(&[b, seq, self.n_heads, self.head_dim])?
                .transpose(1, 2)
        };
        let q = reshape_q(linear(hidden_states, &self.q_w, &self.q_b)?)?;
        let k = reshape_kv(linear(source, &self.k_w, &self.k_b)?, s)?;
        let v = reshape_kv(linear(source, &self.v_w, &self.v_b)?, s)?;
        Ok((q, k, v))
    }
}

struct DecoderFFN {
    dense_in_w: Tensor,
    dense_in_b: Tensor,
    dense_out_w: Tensor,
    dense_out_b: Tensor,
}

impl DecoderFFN {
    fn load(weights: &Weights, prefix: &str) -> anyhow::Result<Self> {
        let b = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get(&format!("{}{}", prefix, n))?.clone())
        };
        let wt = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get_t(&format!("{}{}", prefix, n))?)
        };
        Ok(Self {
            dense_in_w: wt("dense_in.weight")?,
            dense_in_b: b("dense_in.bias")?,
            dense_out_w: wt("dense_out.weight")?,
            dense_out_b: b("dense_out.bias")?,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let h = linear(x, &self.dense_in_w, &self.dense_in_b)?.relu()?;
        linear(&h, &self.dense_out_w, &self.dense_out_b)
    }
}

pub struct DecoderLayer {
    norm1: (Tensor, Tensor),
    self_attn: DecoderAttn,
    norm2: (Tensor, Tensor),
    cross_attn: DecoderAttn,
    norm3: (Tensor, Tensor),
    ffn: DecoderFFN,
}

impl DecoderLayer {
    fn load(
        weights: &Weights,
        prefix: &str,
        n_heads: usize,
        hidden: usize,
    ) -> anyhow::Result<Self> {
        let norm = |n: &str| -> anyhow::Result<(Tensor, Tensor)> {
            let key = format!("{}{}", prefix, n);
            Ok((
                weights.get(&format!("{}.weight", key))?.clone(),
                weights.get(&format!("{}.bias", key))?.clone(),
            ))
        };
        Ok(Self {
            norm1: norm("layer_norm_1")?,
            self_attn: DecoderAttn::load(
                weights,
                &format!("{}first_sub_layer.", prefix),
                n_heads,
                hidden,
            )?,
            norm2: norm("layer_norm_2")?,
            cross_attn: DecoderAttn::load(
                weights,
                &format!("{}second_sub_layer.", prefix),
                n_heads,
                hidden,
            )?,
            norm3: norm("layer_norm_3")?,
            ffn: DecoderFFN::load(weights, &format!("{}third_sub_layer.", prefix))?,
        })
    }

    fn forward_cached(
        &self,
        hidden: &Tensor,
        self_k_cache: Option<&Tensor>,
        self_v_cache: Option<&Tensor>,
        cross_k: &Tensor,
        cross_v: &Tensor,
        self_attn_mask: Option<&Tensor>,
    ) -> Result<(Tensor, Tensor, Tensor)> {
        // Self-attention
        let normed = layer_norm(hidden, &self.norm1.0, &self.norm1.1)?;
        let (b, t, _) = normed.dims3()?;
        let (q_new, k_new, v_new) = self.self_attn.project_qkv(&normed, &normed)?;
        let (k_full, v_full) = match (self_k_cache, self_v_cache) {
            (Some(kc), Some(vc)) => (
                Tensor::cat(&[kc, &k_new], 2)?,
                Tensor::cat(&[vc, &v_new], 2)?,
            ),
            _ => (k_new.clone(), v_new.clone()),
        };
        let self_out = sdpa(&q_new, &k_full, &v_full, self_attn_mask)?;
        let self_out = self_out
            .transpose(1, 2)?
            .contiguous()?
            .reshape(&[b, t, self.self_attn.hidden])?;
        let self_out = linear(&self_out, &self.self_attn.out_w, &self.self_attn.out_b)?;
        let hidden = (hidden + self_out)?;

        // Cross-attention
        let normed2 = layer_norm(&hidden, &self.norm2.0, &self.norm2.1)?;
        let cross_q = linear(&normed2, &self.cross_attn.q_w, &self.cross_attn.q_b)?
            .reshape(&[b, t, self.cross_attn.n_heads, self.cross_attn.head_dim])?
            .transpose(1, 2)?;
        let cross_out = sdpa(&cross_q, cross_k, cross_v, None)?;
        let cross_out = cross_out
            .transpose(1, 2)?
            .contiguous()?
            .reshape(&[b, t, self.cross_attn.hidden])?;
        let cross_out = linear(
            &cross_out,
            &self.cross_attn.out_w,
            &self.cross_attn.out_b,
        )?;
        let hidden = (hidden + cross_out)?;

        // FFN
        let normed3 = layer_norm(&hidden, &self.norm3.0, &self.norm3.1)?;
        let ffn_out = self.ffn.forward(&normed3)?;
        let hidden = (hidden + ffn_out)?;

        Ok((hidden, k_full, v_full))
    }
}

struct FixedPosEnc {
    pos_enc: Tensor,
}

impl FixedPosEnc {
    fn load(weights: &Weights, prefix: &str) -> anyhow::Result<Self> {
        Ok(Self {
            pos_enc: weights
                .get(&format!("{}position_embedding.pos_enc", prefix))?
                .clone(),
        })
    }

    fn forward(&self, position_ids: &[i64], device: &Device) -> Result<Tensor> {
        let indices = Tensor::new(position_ids, device)?;
        self.pos_enc.index_select(&indices, 0)
    }
}

fn causal_mask(seq_len: usize, dtype: DType, device: &Device) -> Result<Tensor> {
    // Build upper-triangular mask: 0 on diagonal and below, -inf above
    let mut data = vec![0f32; seq_len * seq_len];
    for i in 0..seq_len {
        for j in (i + 1)..seq_len {
            data[i * seq_len + j] = f32::NEG_INFINITY;
        }
    }
    let mask = Tensor::new(data.as_slice(), device)?.reshape((seq_len, seq_len))?;
    mask.to_dtype(dtype)?.unsqueeze(0)?.unsqueeze(0) // (1, 1, N, N)
}

pub struct TransformerDecoder {
    token_emb: Tensor,
    pos_enc: FixedPosEnc,
    emb_norm_w: Tensor,
    emb_norm_b: Tensor,
    pub layers: Vec<DecoderLayer>,
    final_ln_w: Tensor,
    final_ln_b: Tensor,
    head_w: Tensor,
    head_b: Tensor,
    n_heads: usize,
    head_dim: usize,
    device: Device,
}

impl TransformerDecoder {
    pub fn load(weights: &Weights, cfg: &ModelConfig, device: &Device) -> anyhow::Result<Self> {
        let dec = &cfg.transf_decoder.config_dict;
        let hidden = dec.hidden_size;
        let n_heads = dec.num_attention_heads;
        let head_dim = hidden / n_heads;
        let emb_prefix = "transf_decoder._embedding.";
        let dec_prefix = "transf_decoder._decoder.";

        let token_emb = weights
            .get(&format!("{}token_embedding.weight", emb_prefix))?
            .clone();
        let pos_enc = FixedPosEnc::load(weights, emb_prefix)?;
        let emb_norm_w = weights
            .get(&format!("{}layer_norm.weight", emb_prefix))?
            .clone();
        let emb_norm_b = weights
            .get(&format!("{}layer_norm.bias", emb_prefix))?
            .clone();

        let mut layers = Vec::with_capacity(dec.num_layers);
        for i in 0..dec.num_layers {
            let prefix = format!("{}layers.{}.", dec_prefix, i);
            layers.push(DecoderLayer::load(weights, &prefix, n_heads, hidden)?);
        }

        let final_ln_w = weights
            .get(&format!("{}final_layer_norm.weight", dec_prefix))?
            .clone();
        let final_ln_b = weights
            .get(&format!("{}final_layer_norm.bias", dec_prefix))?
            .clone();
        let head_w = weights.get_t("log_softmax.mlp.layer0.weight")?;
        let head_b = weights.get("log_softmax.mlp.layer0.bias")?.clone();

        Ok(Self {
            token_emb,
            pos_enc,
            emb_norm_w,
            emb_norm_b,
            layers,
            final_ln_w,
            final_ln_b,
            head_w,
            head_b,
            n_heads,
            head_dim,
            device: device.clone(),
        })
    }

    pub fn precompute_cross_kv(&self, encoder_hs: &Tensor) -> Result<Vec<(Tensor, Tensor)>> {
        let (b, s, _) = encoder_hs.dims3()?;
        let mut result = Vec::with_capacity(self.layers.len());
        for layer in &self.layers {
            let k = linear(encoder_hs, &layer.cross_attn.k_w, &layer.cross_attn.k_b)?
                .reshape(&[b, s, self.n_heads, self.head_dim])?
                .transpose(1, 2)?;
            let v = linear(encoder_hs, &layer.cross_attn.v_w, &layer.cross_attn.v_b)?
                .reshape(&[b, s, self.n_heads, self.head_dim])?
                .transpose(1, 2)?;
            result.push((k, v));
        }
        Ok(result)
    }

    /// Process multiple tokens at once (prompt prefill).
    /// Returns (logits_for_last_token, kv_cache).
    pub fn prefill(
        &self,
        token_ids: &[i64],
        cross_kv: &[(Tensor, Tensor)],
    ) -> Result<(Tensor, Vec<(Option<Tensor>, Option<Tensor>)>)> {
        let n = token_ids.len();
        let ids = Tensor::new(token_ids, &self.device)?;
        let emb = self.token_emb.index_select(&ids, 0)?.unsqueeze(0)?; // (1, N, hidden)

        let position_ids: Vec<i64> = (0..n as i64).collect();
        let pe = self.pos_enc.forward(&position_ids, &self.device)?.unsqueeze(0)?; // (1, N, hidden)

        let x = layer_norm(&(emb + pe)?, &self.emb_norm_w, &self.emb_norm_b)?;

        // Causal mask: upper triangular -inf, shape (1, 1, N, N)
        let mask = causal_mask(n, x.dtype(), &self.device)?;

        let mut new_kv: Vec<(Option<Tensor>, Option<Tensor>)> =
            Vec::with_capacity(self.layers.len());
        let mut hidden = x;

        for (i, layer) in self.layers.iter().enumerate() {
            let (ck, cv) = &cross_kv[i];
            let (new_hidden, k_full, v_full) =
                layer.forward_cached(&hidden, None, None, ck, cv, Some(&mask))?;
            hidden = new_hidden;
            new_kv.push((Some(k_full), Some(v_full)));
        }

        let hidden = layer_norm(&hidden, &self.final_ln_w, &self.final_ln_b)?;
        // Take last token only: narrow to position n-1
        let last_hidden = hidden.narrow(1, n - 1, 1)?;
        let logits = linear(&last_hidden.squeeze(1)?, &self.head_w, &self.head_b)?;
        let logits_squeezed = logits.squeeze(0)?;
        Ok((logits_squeezed, new_kv))
    }

    pub fn step(
        &self,
        token_id: i64,
        position: i64,
        self_kv_cache: &[(Option<Tensor>, Option<Tensor>)],
        cross_kv: &[(Tensor, Tensor)],
    ) -> Result<(Tensor, Vec<(Option<Tensor>, Option<Tensor>)>)> {
        let ids = Tensor::new(&[token_id], &self.device)?;
        let emb = self.token_emb.index_select(&ids, 0)?.unsqueeze(0)?;
        let pe = self.pos_enc.forward(&[position], &self.device)?.unsqueeze(0)?;
        let x = layer_norm(&(emb + pe)?, &self.emb_norm_w, &self.emb_norm_b)?;

        let mut new_kv: Vec<(Option<Tensor>, Option<Tensor>)> =
            Vec::with_capacity(self.layers.len());
        let mut hidden = x;

        for (i, layer) in self.layers.iter().enumerate() {
            let (kc, vc) = &self_kv_cache[i];
            let (ck, cv) = &cross_kv[i];
            let (new_hidden, k_full, v_full) =
                layer.forward_cached(&hidden, kc.as_ref(), vc.as_ref(), ck, cv, None)?;
            hidden = new_hidden;
            new_kv.push((Some(k_full), Some(v_full)));
        }

        let hidden = layer_norm(&hidden, &self.final_ln_w, &self.final_ln_b)?;
        let logits = linear(&hidden.squeeze(1)?, &self.head_w, &self.head_b)?;
        let logits_squeezed = logits.squeeze(0)?;
        Ok((logits_squeezed, new_kv))
    }
}
