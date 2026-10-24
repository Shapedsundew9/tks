//! Fused LayerNorm CUDA kernel with CPU fallback.
//!
//! On CUDA (with `cuda` feature), launches a custom PTX kernel that fuses:
//! cast to F32, compute mean/variance, normalize, scale, shift, cast back.
//! On CPU/Metal, falls back to sequential operations with identical results.

use candle_core::{DType, Tensor};

/// Fused LayerNorm that uses a custom CUDA kernel when available.
///
/// Drop-in replacement for the standard LayerNorm pattern:
/// `(x - mean) / sqrt(var + eps) * weight + bias`
pub struct FusedLayerNorm {
    weight: Tensor,
    bias: Tensor,
    eps: f32,
}

impl FusedLayerNorm {
    pub fn new(weight: Tensor, bias: Tensor, eps: f64) -> Self {
        Self {
            weight,
            bias,
            eps: eps as f32,
        }
    }

    pub fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        #[cfg(feature = "cuda")]
        {
            if x.device().is_cuda() {
                return self.forward_fused(x);
            }
        }
        self.forward_cpu(x)
    }

    fn forward_cpu(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let orig_dtype = x.dtype();
        let x = x.to_dtype(DType::F32)?;
        let w = self.weight.to_dtype(DType::F32)?;
        let b = self.bias.to_dtype(DType::F32)?;
        let mean = x.mean_keepdim(candle_core::D::Minus1)?;
        let diff = x.broadcast_sub(&mean)?;
        let var = diff.sqr()?.mean_keepdim(candle_core::D::Minus1)?;
        let std = (var + self.eps as f64)?.sqrt()?;
        let out = diff
            .broadcast_div(&std)?
            .broadcast_mul(&w)?
            .broadcast_add(&b)?;
        out.to_dtype(orig_dtype)
    }

    #[cfg(feature = "cuda")]
    fn forward_fused(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let x = x.contiguous()?;
        let shape = x.shape().clone();
        let dims = shape.dims();
        let n_cols = *dims.last().unwrap();
        let n_rows = shape.elem_count() / n_cols;

        // Ensure weight/bias match input dtype (encoder runs F32 but weights may be F16)
        let weight = self.weight.to_dtype(x.dtype())?;
        let bias = self.bias.to_dtype(x.dtype())?;

        let op = FusedLayerNormOp {
            weight,
            bias,
            eps: self.eps,
            n_rows,
            n_cols,
        };
        let out = x.apply_op1_no_bwd(&op)?;
        // Kernel returns flat [n_rows, n_cols] — reshape back to original
        out.reshape(dims)
    }
}

#[cfg(feature = "cuda")]
static FUSED_LAYERNORM_PTX: &str = include_str!("../kernels/fused_layernorm.ptx");

#[cfg(feature = "cuda")]
struct FusedLayerNormOp {
    weight: Tensor,
    bias: Tensor,
    eps: f32,
    n_rows: usize,
    n_cols: usize,
}

#[cfg(feature = "cuda")]
impl FusedLayerNormOp {
    fn launch_kernel<
        T: candle_core::cuda_backend::CudaDType
            + candle_core::WithDType
            + candle_core::cuda_backend::cudarc::driver::DeviceRepr,
    >(
        &self,
        s: &candle_core::CudaStorage,
        l: &candle_core::Layout,
    ) -> candle_core::Result<candle_core::cuda_backend::CudaStorageSlice> {
        use candle_core::backend::BackendStorage;
        use candle_core::cuda_backend::cudarc::driver::{LaunchConfig, PushKernelArg};
        use candle_core::cuda_backend::WrapErr;

        let dev = s.device();

        let x = s.as_cuda_slice::<T>()?;
        let x = match l.contiguous_offsets() {
            None => candle_core::bail!("input must be contiguous"),
            Some((o1, o2)) => x.slice(o1..o2),
        };

        let kernel_name = match T::DTYPE {
            DType::BF16 => "fused_layernorm_bf16",
            DType::F16 => "fused_layernorm_f16",
            DType::F32 => "fused_layernorm_f32",
            dt => candle_core::bail!("fused-layernorm unsupported dtype {dt:?}"),
        };

        let func =
            dev.get_or_load_custom_func(kernel_name, "fused_layernorm", FUSED_LAYERNORM_PTX)?;

        // Extract weight slice
        let weight_guard = self.weight.storage_and_layout();
        let weight_cuda = match &*weight_guard.0 {
            candle_core::Storage::Cuda(s) => s,
            _ => candle_core::bail!("weight must be on CUDA"),
        };
        let w = weight_cuda.as_cuda_slice::<T>()?;

        // Extract bias slice
        let bias_guard = self.bias.storage_and_layout();
        let bias_cuda = match &*bias_guard.0 {
            candle_core::Storage::Cuda(s) => s,
            _ => candle_core::bail!("bias must be on CUDA"),
        };
        let b = bias_cuda.as_cuda_slice::<T>()?;

        // Block size: min(1024, round up n_cols to nearest 32)
        let block_size: u32 = std::cmp::min(1024, ((self.n_cols as u32 + 31) / 32) * 32);
        let cfg = LaunchConfig {
            grid_dim: (self.n_rows as u32, 1, 1),
            block_dim: (block_size, 1, 1),
            shared_mem_bytes: 0,
        };

        let out_el = self.n_rows * self.n_cols;
        // SAFETY: Filled by kernel before read.
        let dst = unsafe { dev.alloc::<T>(out_el)? };

        let mut builder = func.builder();
        builder.arg(&x);
        builder.arg(w);
        builder.arg(b);
        builder.arg(&dst);
        candle_core::builder_arg!(builder, self.n_cols as i32, block_size as i32, self.eps);
        // SAFETY: kernel launch
        unsafe { builder.launch(cfg) }.w()?;

        Ok(T::wrap_cuda_slice(dst, dev.clone()).slice)
    }
}

#[cfg(feature = "cuda")]
impl candle_core::CustomOp1 for FusedLayerNormOp {
    fn name(&self) -> &'static str {
        "fused-layernorm"
    }

    fn cpu_fwd(
        &self,
        _s: &candle_core::CpuStorage,
        _l: &candle_core::Layout,
    ) -> candle_core::Result<(candle_core::CpuStorage, candle_core::Shape)> {
        candle_core::bail!("fused-layernorm is CUDA-only; use forward_cpu")
    }

    fn cuda_fwd(
        &self,
        s: &candle_core::CudaStorage,
        l: &candle_core::Layout,
    ) -> candle_core::Result<(candle_core::CudaStorage, candle_core::Shape)> {
        use candle_core::backend::BackendStorage;

        let dev = s.device();
        let out_shape = candle_core::Shape::from_dims(&[self.n_rows, self.n_cols]);

        let slice = match s.dtype() {
            DType::BF16 => self.launch_kernel::<half::bf16>(s, l)?,
            DType::F16 => self.launch_kernel::<half::f16>(s, l)?,
            DType::F32 => self.launch_kernel::<f32>(s, l)?,
            dt => candle_core::bail!("fused-layernorm unsupported dtype {dt:?}"),
        };

        let dst = candle_core::CudaStorage {
            slice,
            device: dev.clone(),
        };
        Ok((dst, out_shape))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::Device;

    #[test]
    fn test_fused_layernorm_cpu() {
        let device = Device::Cpu;
        let hidden = 64;
        let eps = 1e-5;

        let weight = Tensor::ones(hidden, DType::F32, &device).unwrap();
        let bias = Tensor::zeros(hidden, DType::F32, &device).unwrap();
        let norm = FusedLayerNorm::new(weight, bias, eps);

        let x = Tensor::randn(0.0f32, 1.0, (2, 10, hidden), &device).unwrap();
        let out = norm.forward(&x).unwrap();
        assert_eq!(out.dims(), &[2, 10, hidden]);

        // With unit weight and zero bias, output should have ~zero mean per row
        let mean = out
            .mean_keepdim(candle_core::D::Minus1)
            .unwrap()
            .abs()
            .unwrap()
            .max(0)
            .unwrap()
            .max(0)
            .unwrap()
            .max(0)
            .unwrap()
            .to_scalar::<f32>()
            .unwrap();
        assert!(mean < 1e-5, "mean should be ~0, got {mean}");
    }

    #[test]
    fn test_fused_layernorm_cpu_with_weight_bias() {
        let device = Device::Cpu;
        let hidden = 32;
        let eps = 1e-5;

        let weight = Tensor::new(vec![2.0f32; hidden], &device).unwrap();
        let bias = Tensor::new(vec![0.5f32; hidden], &device).unwrap();
        let norm = FusedLayerNorm::new(weight, bias, eps);

        let x = Tensor::randn(0.0f32, 1.0, (1, 4, hidden), &device).unwrap();
        let out = norm.forward(&x).unwrap();
        assert_eq!(out.dims(), &[1, 4, hidden]);
    }
}
