// Fused LayerNorm CUDA kernel.
//
// Single kernel launch that computes:
//   mean = sum(x) / n_cols
//   var  = sum(x^2) / n_cols - mean^2
//   dst[i] = (x[i] - mean) / sqrt(var + eps) * weight[i] + bias[i]
//
// One block per row. Each thread handles a strided subset of columns.

#include <stdint.h>

#if __CUDA_ARCH__ >= 530
#include <cuda_fp16.h>
#endif
#if __CUDA_ARCH__ >= 800
#include <cuda_bf16.h>
#endif

#define WARP_SIZE 32

template <typename T>
__device__ float to_float(T v) { return static_cast<float>(v); }

template <typename T>
__device__ T from_float(float v) { return static_cast<T>(v); }

__device__ float warp_reduce_sum(float val) {
    for (int offset = WARP_SIZE / 2; offset > 0; offset >>= 1) {
        val += __shfl_xor_sync(0xffffffff, val, offset);
    }
    return val;
}

// Block-level reduction: warp reduce, then cross-warp via shared memory.
__device__ float block_reduce_sum(float val, int block_size) {
    val = warp_reduce_sum(val);
    if (block_size > WARP_SIZE) {
        __shared__ float s_buf[32]; // max 32 warps = 1024 threads
        int warp_id = threadIdx.x / WARP_SIZE;
        int lane_id = threadIdx.x % WARP_SIZE;
        if (lane_id == 0) {
            s_buf[warp_id] = val;
        }
        __syncthreads();
        val = (lane_id < (block_size / WARP_SIZE)) ? s_buf[lane_id] : 0.0f;
        val = warp_reduce_sum(val);
    }
    return val;
}

// One block per row. Each thread handles a strided subset of columns.
template <typename T>
__device__ void fused_layernorm(
    const T * __restrict__ x,
    const T * __restrict__ weight,
    const T * __restrict__ bias,
    T * __restrict__ dst,
    const int ncols,
    const int block_size,
    const float eps
) {
    const int row = blockIdx.x;
    const int tid = threadIdx.x;
    const int row_offset = row * ncols;

    // Pass 1: accumulate sum(x) and sum(x^2) in F32
    float sum_val = 0.0f;
    float sum_sq  = 0.0f;
    for (int col = tid; col < ncols; col += block_size) {
        float xi = to_float(x[row_offset + col]);
        sum_val += xi;
        sum_sq  += xi * xi;
    }

    // Block reduction for both sums
    sum_val = block_reduce_sum(sum_val, block_size);
    sum_sq  = block_reduce_sum(sum_sq, block_size);

    const float mean = sum_val / ncols;
    const float var  = sum_sq / ncols - mean * mean;
    const float inv_std = rsqrtf(var + eps);

    // Pass 2: normalize, scale, shift
    for (int col = tid; col < ncols; col += block_size) {
        float xi = to_float(x[row_offset + col]);
        float w  = to_float(weight[col]);
        float b  = to_float(bias[col]);
        float normed = (xi - mean) * inv_std * w + b;
        dst[row_offset + col] = from_float<T>(normed);
    }
}

#define FUSED_LAYERNORM_OP(TYPENAME, FN_NAME)     \
  extern "C" __global__ void FN_NAME(             \
      const TYPENAME *x,                          \
      const TYPENAME *weight,                     \
      const TYPENAME *bias,                       \
      TYPENAME *dst,                              \
      const int n_cols,                           \
      const int block_size,                       \
      const float eps) {                          \
    fused_layernorm<TYPENAME>(                    \
        x, weight, bias, dst, n_cols, block_size, eps); \
  }

#if __CUDA_ARCH__ >= 800
FUSED_LAYERNORM_OP(__nv_bfloat16, fused_layernorm_bf16)
#endif
#if __CUDA_ARCH__ >= 530
FUSED_LAYERNORM_OP(__half, fused_layernorm_f16)
#endif
FUSED_LAYERNORM_OP(float, fused_layernorm_f32)
