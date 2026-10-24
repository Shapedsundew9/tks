use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use std::collections::HashMap;
use std::path::Path;

pub struct Weights {
    tensors: HashMap<String, Tensor>,
    device: Device,
}

impl Weights {
    /// Load weights, keeping them in FP16 on CUDA for speed (tensor core acceleration).
    /// On CPU, converts to FP32 since CPU has no FP16 compute benefit.
    pub fn load(path: impl AsRef<Path>, device: &Device) -> Result<Self> {
        tracing::info!("Loading weights from {:?}", path.as_ref());

        // Stage on the CPU and upload each final (cast/transposed) tensor once; staging on the
        // GPU kept both the original and the transposed copy resident (EA-0 change).
        let tensors = candle_core::safetensors::load(path.as_ref(), &Device::Cpu)
            .with_context(|| format!("Cannot load safetensors at {:?}", path.as_ref()))?;

        let use_f16 = device.is_cuda();
        let target_dtype = if use_f16 { DType::F16 } else { DType::F32 };
        tracing::info!(
            "Converting weights to {:?} (CUDA={})",
            target_dtype,
            use_f16
        );

        let tensors: HashMap<String, Tensor> = tensors
            .into_iter()
            .map(|(name, tensor)| {
                let tensor = if tensor.dtype() != target_dtype {
                    tensor.to_dtype(target_dtype).unwrap_or(tensor)
                } else {
                    tensor
                };
                (name, tensor)
            })
            .collect();

        tracing::info!("Loaded {} tensors ({:?})", tensors.len(), target_dtype);
        Ok(Self {
            tensors,
            device: device.clone(),
        })
    }

    fn staged(&self, name: &str) -> Result<&Tensor> {
        self.tensors
            .get(name)
            .with_context(|| format!("Missing weight: '{}'", name))
    }

    pub fn get(&self, name: &str) -> Result<Tensor> {
        Ok(self.staged(name)?.to_device(&self.device)?)
    }

    /// Get a weight pre-transposed and contiguous for use in linear layers.
    pub fn get_t(&self, name: &str) -> Result<Tensor> {
        let t = self.staged(name)?;
        Ok(t.t()?.contiguous()?.to_device(&self.device)?)
    }

    /// Get a weight pre-transposed and cast to F32 (for encoder which runs in F32).
    pub fn get_t_f32(&self, name: &str) -> Result<Tensor> {
        let t = self.staged(name)?;
        Ok(t.t()?.contiguous()?.to_dtype(DType::F32)?.to_device(&self.device)?)
    }

    /// Get a weight cast to F32 (for encoder norms/convs which run in F32).
    pub fn get_f32(&self, name: &str) -> Result<Tensor> {
        let t = self.staged(name)?;
        Ok(t.to_dtype(DType::F32)?.to_device(&self.device)?)
    }
}
