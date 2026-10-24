/// Conformer encoder implementing the architecture from modeling_cohere_asr.py,
/// translated to the candle ML framework.
use anyhow::Context as AnyhowContext;
use candle_core::{Device, Result, Tensor, D};
use candle_nn::ops;

use crate::config::ModelConfig;
use crate::weights::Weights;

/// F16-safe matmul: flattens higher dims to 2D, matmuls, reshapes back.
/// Candle's CUDA F16 matmul requires matching rank for batched ops.
fn matmul_2d(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    let a_dims = a.dims().to_vec();
    if a_dims.len() <= 2 {
        return a.matmul(b);
    }
    let m: usize = a_dims[..a_dims.len() - 1].iter().product();
    let k = *a_dims.last().unwrap();
    let a2d = a.reshape(&[m, k])?.contiguous()?;
    let out2d = a2d.matmul(b)?;
    let mut out_shape = a_dims[..a_dims.len() - 1].to_vec();
    out_shape.push(out2d.dims()[1]);
    out2d.reshape(out_shape.as_slice())
}

fn linear(x: &Tensor, w: &Tensor, b: &Tensor) -> Result<Tensor> {
    if w.dtype() != x.dtype() {
        let w = w.to_dtype(x.dtype())?;
        let b = b.to_dtype(x.dtype())?;
        matmul_2d(x, &w)?.broadcast_add(&b)
    } else {
        matmul_2d(x, w)?.broadcast_add(b)
    }
}


fn batch_norm_eval(
    x: &Tensor,
    weight: &Tensor,
    bias: &Tensor,
    running_mean: &Tensor,
    running_var: &Tensor,
) -> Result<Tensor> {
    let eps = 1e-5;
    let c = running_mean.dim(0)?;
    // BatchNorm stats are always F32 from training — cast to match input if needed
    let cast = |t: &Tensor| -> Result<Tensor> {
        if t.dtype() != x.dtype() { t.to_dtype(x.dtype()) } else { Ok(t.clone()) }
    };
    let inv_std = (cast(running_var)? + eps)?.sqrt()?.recip()?;
    let rm = cast(running_mean)?.reshape((1, c, 1))?;
    let rv_inv = inv_std.reshape((1, c, 1))?;
    let w = cast(weight)?.reshape((1, c, 1))?;
    let b = cast(bias)?.reshape((1, c, 1))?;
    let x_norm = x.broadcast_sub(&rm)?.broadcast_mul(&rv_inv)?;
    x_norm.broadcast_mul(&w)?.broadcast_add(&b)
}

fn conv2d_with_bias(
    x: &Tensor,
    w: &Tensor,
    b: &Tensor,
    padding: usize,
    stride: usize,
    dilation: usize,
    groups: usize,
) -> Result<Tensor> {
    let (w, b) = if w.dtype() != x.dtype() {
        (w.to_dtype(x.dtype())?, b.to_dtype(x.dtype())?)
    } else {
        (w.clone(), b.clone())
    };
    let out = x.conv2d(&w, padding, stride, dilation, groups)?;
    let out_channels = b.dim(0)?;
    let b_reshaped = b.reshape((1, out_channels, 1, 1))?;
    out.broadcast_add(&b_reshaped)
}

fn conv1d_with_bias(
    x: &Tensor,
    w: &Tensor,
    b: &Tensor,
    padding: usize,
    stride: usize,
    dilation: usize,
    groups: usize,
) -> Result<Tensor> {
    let (w, b) = if w.dtype() != x.dtype() {
        (w.to_dtype(x.dtype())?, b.to_dtype(x.dtype())?)
    } else {
        (w.clone(), b.clone())
    };
    let out = x.conv1d(&w, padding, stride, dilation, groups)?;
    let out_channels = b.dim(0)?;
    let b_reshaped = b.reshape((1, out_channels, 1))?;
    out.broadcast_add(&b_reshaped)
}

struct ConvSubsampling {
    c0_w: Tensor,
    c0_b: Tensor,
    c2_w: Tensor,
    c2_b: Tensor,
    c3_w: Tensor,
    c3_b: Tensor,
    c5_w: Tensor,
    c5_b: Tensor,
    c6_w: Tensor,
    c6_b: Tensor,
    out_w: Tensor,
    out_b: Tensor,
    conv_channels: usize,
}

impl ConvSubsampling {
    fn load(weights: &Weights, prefix: &str) -> anyhow::Result<Self> {
        let w = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get(&format!("{}{}", prefix, n))?.clone())
        };
        let wt = |n: &str| -> anyhow::Result<Tensor> {
            weights.get_t(&format!("{}{}", prefix, n))
        };
        Ok(Self {
            c0_w: w("conv.0.weight")?,
            c0_b: w("conv.0.bias")?,
            c2_w: w("conv.2.weight")?,
            c2_b: w("conv.2.bias")?,
            c3_w: w("conv.3.weight")?,
            c3_b: w("conv.3.bias")?,
            c5_w: w("conv.5.weight")?,
            c5_b: w("conv.5.bias")?,
            c6_w: w("conv.6.weight")?,
            c6_b: w("conv.6.bias")?,
            out_w: wt("out.weight")?,
            out_b: w("out.bias")?,
            conv_channels: 256,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<(Tensor, usize)> {
        // x: (B, T, F) -> (B, 1, T, F)
        let x = x.transpose(1, 2)?.unsqueeze(1)?;

        // Conv block 1: standard conv2d, stride 2, pad 1, dilation 1
        let x = conv2d_with_bias(&x, &self.c0_w, &self.c0_b, 1, 2, 1, 1)?.relu()?;

        // Conv block 2: depthwise + pointwise, stride 2
        let x = conv2d_with_bias(&x, &self.c2_w, &self.c2_b, 1, 2, 1, self.conv_channels)?;
        let x = conv2d_with_bias(&x, &self.c3_w, &self.c3_b, 0, 1, 1, 1)?.relu()?;

        // Conv block 3: depthwise + pointwise, stride 2
        let x = conv2d_with_bias(&x, &self.c5_w, &self.c5_b, 1, 2, 1, self.conv_channels)?;
        let x = conv2d_with_bias(&x, &self.c6_w, &self.c6_b, 0, 1, 1, 1)?.relu()?;

        // Reshape: (B, C, T', F') -> (B, T', C*F')
        let dims = x.dims4()?;
        let (b, c, t, f) = (dims.0, dims.1, dims.2, dims.3);
        let x = x.transpose(1, 2)?.reshape(&[b, t, c * f])?;

        let out = linear(&x, &self.out_w, &self.out_b)?;
        Ok((out, t))
    }
}

fn rel_positional_encoding(length: usize, d_model: usize, device: &Device) -> Result<Tensor> {
    let n_pos = 2 * length - 1;
    let mut pe = vec![0.0f32; n_pos * d_model];
    for i in 0..n_pos {
        let pos = (length as i64 - 1 - i as i64) as f64;
        for k in (0..d_model).step_by(2) {
            let div = ((k as f64) * -(10000.0f64.ln()) / d_model as f64).exp();
            pe[i * d_model + k] = (pos * div).sin() as f32;
            if k + 1 < d_model {
                pe[i * d_model + k + 1] = (pos * div).cos() as f32;
            }
        }
    }
    Tensor::from_vec(pe, (1, n_pos, d_model), device)
}

struct FeedForward {
    l1_w: Tensor,
    l1_b: Tensor,
    l2_w: Tensor,
    l2_b: Tensor,
}

impl FeedForward {
    fn load(weights: &Weights, prefix: &str) -> anyhow::Result<Self> {
        let w = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get(&format!("{}{}", prefix, n))?.clone())
        };
        let wt = |n: &str| -> anyhow::Result<Tensor> {
            weights.get_t(&format!("{}{}", prefix, n))
        };
        Ok(Self {
            l1_w: wt("linear1.weight")?,
            l1_b: w("linear1.bias")?,
            l2_w: wt("linear2.weight")?,
            l2_b: w("linear2.bias")?,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let h = ops::silu(&linear(x, &self.l1_w, &self.l1_b)?)?;
        linear(&h, &self.l2_w, &self.l2_b)
    }
}

struct ConformerConv {
    pw1_w: Tensor,
    pw1_b: Tensor,
    dw_w: Tensor,
    dw_b: Tensor,
    bn_w: Tensor,
    bn_b: Tensor,
    bn_rm: Tensor,
    bn_rv: Tensor,
    pw2_w: Tensor,
    pw2_b: Tensor,
    d_model: usize,
}

impl ConformerConv {
    fn load(weights: &Weights, prefix: &str, d_model: usize) -> anyhow::Result<Self> {
        let w = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get(&format!("{}{}", prefix, n))?.clone())
        };
        Ok(Self {
            pw1_w: w("pointwise_conv1.weight")?,
            pw1_b: w("pointwise_conv1.bias")?,
            dw_w: w("depthwise_conv.weight")?,
            dw_b: w("depthwise_conv.bias")?,
            bn_w: w("batch_norm.weight")?,
            bn_b: w("batch_norm.bias")?,
            bn_rm: w("batch_norm.running_mean")?,
            bn_rv: w("batch_norm.running_var")?,
            pw2_w: w("pointwise_conv2.weight")?,
            pw2_b: w("pointwise_conv2.bias")?,
            d_model,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        // x: (B, T, D) -> (B, D, T)
        let x = x.transpose(1, 2)?;

        let kernel_size = self.dw_w.dims()[2];

        // Pointwise conv1: expands to 2*d_model
        let x = conv1d_with_bias(&x, &self.pw1_w, &self.pw1_b, 0, 1, 1, 1)?;

        // GLU gate
        let a = x.narrow(1, 0, self.d_model)?;
        let b = x.narrow(1, self.d_model, self.d_model)?;
        let x = (a * ops::sigmoid(&b)?)?;

        // Depthwise conv
        let pad = (kernel_size - 1) / 2;
        let x = conv1d_with_bias(&x, &self.dw_w, &self.dw_b, pad, 1, 1, self.d_model)?;

        // Batch norm (eval mode)
        let x = batch_norm_eval(&x, &self.bn_w, &self.bn_b, &self.bn_rm, &self.bn_rv)?;

        // SiLU activation
        let x = ops::silu(&x)?;

        // Pointwise conv2
        let x = conv1d_with_bias(&x, &self.pw2_w, &self.pw2_b, 0, 1, 1, 1)?;

        // (B, D, T) -> (B, T, D)
        x.transpose(1, 2)
    }
}

struct RelPosAttn {
    q_w: Tensor,
    q_b: Tensor,
    k_w: Tensor,
    k_b: Tensor,
    v_w: Tensor,
    v_b: Tensor,
    pos_w: Tensor,
    out_w: Tensor,
    out_b: Tensor,
    pos_bias_u: Tensor,
    pos_bias_v: Tensor,
    n_heads: usize,
    d_k: usize,
    scale: f64,
}

impl RelPosAttn {
    fn load(
        weights: &Weights,
        prefix: &str,
        n_heads: usize,
        d_model: usize,
    ) -> anyhow::Result<Self> {
        let d_k = d_model / n_heads;
        let w = |n: &str| -> anyhow::Result<Tensor> {
            Ok(weights.get(&format!("{}{}", prefix, n))?.clone())
        };
        let wt = |n: &str| -> anyhow::Result<Tensor> {
            weights.get_t(&format!("{}{}", prefix, n))
        };
        Ok(Self {
            q_w: wt("linear_q.weight")?,
            q_b: w("linear_q.bias")?,
            k_w: wt("linear_k.weight")?,
            k_b: w("linear_k.bias")?,
            v_w: wt("linear_v.weight")?,
            v_b: w("linear_v.bias")?,
            pos_w: wt("linear_pos.weight")?,
            out_w: wt("linear_out.weight")?,
            out_b: w("linear_out.bias")?,
            pos_bias_u: w("pos_bias_u")?,
            pos_bias_v: w("pos_bias_v")?,
            n_heads,
            d_k,
            scale: (d_k as f64).powf(-0.5),
        })
    }

    fn rel_shift(&self, x: &Tensor) -> Result<Tensor> {
        let dims = x.dims4()?;
        let (b, h, t, n_pos) = (dims.0, dims.1, dims.2, dims.3);
        // Pad last dimension with one zero on the left
        let zeros = Tensor::zeros((b, h, t, 1), x.dtype(), x.device())?;
        let x = Tensor::cat(&[&zeros, x], 3)?;
        // Transformer-XL shift as in NeMo: view as (n_pos+1, t), drop the first row,
        // view back as (t, n_pos) and keep the first t relative positions.
        // (EA-0 fix: upstream took rows 1..t+1 of the (n_pos+1, t) view, which only
        // equals the shift when n_pos == t.)
        let x = x.reshape((b, h, n_pos + 1, t))?;
        x.narrow(2, 1, n_pos)?
            .contiguous()?
            .reshape((b, h, t, n_pos))?
            .narrow(3, 0, t)
    }

    fn forward(&self, x: &Tensor, pos_emb: &Tensor) -> Result<Tensor> {
        let dims = x.dims3()?;
        let (b, t, _d) = (dims.0, dims.1, dims.2);

        let reshape_head =
            |z: Tensor| -> Result<Tensor> { z.reshape((b, t, self.n_heads, self.d_k))?.transpose(1, 2) };

        let q = reshape_head(linear(x, &self.q_w, &self.q_b)?)?;
        let k = reshape_head(linear(x, &self.k_w, &self.k_b)?)?;
        let v = reshape_head(linear(x, &self.v_w, &self.v_b)?)?;

        // Positional projection (no bias — the original uses zero bias)
        let n_pos = pos_emb.dims()[1];
        let pos_w = self.pos_w.to_dtype(pos_emb.dtype())?;
        let p = matmul_2d(pos_emb, &pos_w)?
            .reshape((1, n_pos, self.n_heads, self.d_k))?
            .transpose(1, 2)?;

        let dt = q.dtype();
        let u = self.pos_bias_u.to_dtype(dt)?.reshape((1, self.n_heads, 1, self.d_k))?;
        let v_bias = self.pos_bias_v.to_dtype(dt)?.reshape((1, self.n_heads, 1, self.d_k))?;

        let q_with_u = q.broadcast_add(&u)?;
        let q_with_v = q.broadcast_add(&v_bias)?;

        let matrix_ac = q_with_u.matmul(&k.transpose(2, 3)?.contiguous()?)?;
        let matrix_bd = q_with_v.matmul(&p.transpose(2, 3)?.contiguous()?)?;
        let matrix_bd = self.rel_shift(&matrix_bd)?;

        let scores = ((matrix_ac + matrix_bd)? * self.scale)?;
        let attn = if scores.dtype() == candle_core::DType::F32 {
            ops::softmax(&scores, D::Minus1)?
        } else {
            ops::softmax(&scores.to_dtype(candle_core::DType::F32)?, D::Minus1)?
                .to_dtype(scores.dtype())?
        };

        let out = attn.contiguous()?.matmul(&v.contiguous()?)?;
        let out = out
            .transpose(1, 2)?
            .contiguous()?
            .reshape((b, t, self.n_heads * self.d_k))?;
        linear(&out, &self.out_w, &self.out_b)
    }
}

struct ConformerLayer {
    norm_ff1: crate::fused_ops::FusedLayerNorm,
    ff1: FeedForward,
    norm_self_att: crate::fused_ops::FusedLayerNorm,
    self_attn: RelPosAttn,
    norm_conv: crate::fused_ops::FusedLayerNorm,
    conv: ConformerConv,
    norm_ff2: crate::fused_ops::FusedLayerNorm,
    ff2: FeedForward,
    norm_out: crate::fused_ops::FusedLayerNorm,
}

impl ConformerLayer {
    fn load(
        weights: &Weights,
        prefix: &str,
        n_heads: usize,
        d_model: usize,
    ) -> anyhow::Result<Self> {
        let fused_norm = |n: &str| -> anyhow::Result<crate::fused_ops::FusedLayerNorm> {
            let key = format!("{}{}", prefix, n);
            let w = weights.get(&format!("{}.weight", key))?.clone();
            let b = weights.get(&format!("{}.bias", key))?.clone();
            Ok(crate::fused_ops::FusedLayerNorm::new(w, b, 1e-5))
        };
        Ok(Self {
            norm_ff1: fused_norm("norm_feed_forward1")?,
            ff1: FeedForward::load(weights, &format!("{}feed_forward1.", prefix))?,
            norm_self_att: fused_norm("norm_self_att")?,
            self_attn: RelPosAttn::load(
                weights,
                &format!("{}self_attn.", prefix),
                n_heads,
                d_model,
            )?,
            norm_conv: fused_norm("norm_conv")?,
            conv: ConformerConv::load(weights, &format!("{}conv.", prefix), d_model)?,
            norm_ff2: fused_norm("norm_feed_forward2")?,
            ff2: FeedForward::load(weights, &format!("{}feed_forward2.", prefix))?,
            norm_out: fused_norm("norm_out")?,
        })
    }

    fn forward(&self, x: &Tensor, pos_emb: &Tensor) -> Result<Tensor> {
        // x = x + 0.5 * ff1(fused_norm(x))
        let ff1_out = self.ff1.forward(&self.norm_ff1.forward(x)?)?;
        let x = (x + (ff1_out * 0.5)?)?;

        // x = x + self_attn(fused_norm(x), pos_emb)
        let attn_out = self
            .self_attn
            .forward(&self.norm_self_att.forward(&x)?, pos_emb)?;
        let x = (&x + attn_out)?;

        // x = x + conv(fused_norm(x))
        let conv_out = self.conv.forward(&self.norm_conv.forward(&x)?)?;
        let x = (&x + conv_out)?;

        // x = x + 0.5 * ff2(fused_norm(x))
        let ff2_out = self.ff2.forward(&self.norm_ff2.forward(&x)?)?;
        let x = (&x + (ff2_out * 0.5)?)?;

        // Final fused norm
        self.norm_out.forward(&x)
    }
}

pub struct ConformerEncoder {
    pre_encode: ConvSubsampling,
    layers: Vec<ConformerLayer>,
    enc_dec_proj_w: Option<Tensor>,
    enc_dec_proj_b: Option<Tensor>,
    d_model: usize,
}

impl ConformerEncoder {
    pub fn load(weights: &Weights, cfg: &ModelConfig) -> anyhow::Result<Self> {
        let enc = &cfg.encoder;
        let d_model = enc.d_model;
        let n_heads = enc.n_heads;

        let pre_encode = ConvSubsampling::load(weights, "encoder.pre_encode.")?;

        let mut layers = Vec::with_capacity(enc.n_layers);
        for i in 0..enc.n_layers {
            let prefix = format!("encoder.layers.{}.", i);
            let layer = ConformerLayer::load(weights, &prefix, n_heads, d_model)
                .with_context(|| format!("Loading ConformerLayer {}", i))?;
            layers.push(layer);
        }

        let (enc_dec_proj_w, enc_dec_proj_b) = if let (Ok(w), Ok(b)) = (
            weights.get_t("encoder_decoder_proj.weight"),
            weights.get("encoder_decoder_proj.bias"),
        ) {
            (Some(w), Some(b.clone()))
        } else {
            (None, None)
        };

        Ok(Self {
            pre_encode,
            layers,
            enc_dec_proj_w,
            enc_dec_proj_b,
            d_model,
        })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        // Run encoder in native dtype (F16 on CUDA for tensor core acceleration).
        // FusedLayerNorm handles F32 precision internally in a single kernel.
        let (x, t_prime) = self.pre_encode.forward(x)?;

        let pos_emb =
            rel_positional_encoding(t_prime, self.d_model, x.device())?
                .to_dtype(x.dtype())?;

        let mut x = x;
        for layer in &self.layers {
            x = layer.forward(&x, &pos_emb)?;
        }

        if let (Some(w), Some(b)) = (&self.enc_dec_proj_w, &self.enc_dec_proj_b) {
            x = linear(&x, w, b)?;
        }

        Ok(x)
    }
}
