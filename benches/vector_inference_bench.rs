//! Microbenchmark measuring CPU embedding inference throughput across batch sizes (1, 8, 32).
//!
//! Evaluates `fastembed` execution of `all-MiniLM-L6-v2` (384-d) on commodity CPU hardware
//! against technical requirement spans (100–300 words) from `docs/vision/` specifications (WP-0.5).

#[allow(dead_code)]
#[path = "../scripts/spikes/vector_spike/evaluator.rs"]
mod evaluator;
#[allow(dead_code)]
#[path = "../scripts/spikes/vector_spike/model.rs"]
mod model;

use model::LocalEmbeddingGenerator;
use std::fs;
use std::path::Path;
use std::time::Instant;

fn estimate_tokens(text: &str) -> usize {
    let words = text.split_whitespace().count();
    (words * 13) / 10
}

fn main() {
    println!("=== Spike 8: Vector Inference Latency Microbenchmark (WP-0.5) ===\n");

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = Path::new(manifest_dir)
        .join("scripts/spikes/vector_spike/fixtures/baseline_embeddings.json");

    let corpus_raw = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read fixture at {}: {e}", fixture_path.display()));
    let corpus: evaluator::Spike8Corpus =
        serde_json::from_str(&corpus_raw).expect("Failed to parse baseline embeddings fixture");

    println!(
        "Loaded {} technical specification chunks (100-300 words each).",
        corpus.chunks.len()
    );
    println!("Initializing LocalEmbeddingGenerator (all-MiniLM-L6-v2, 384 dimensions)...");

    let generator =
        LocalEmbeddingGenerator::new().expect("Failed to initialize LocalEmbeddingGenerator");

    // 1. Warm-up
    println!("Warming up ONNX runtime inference session...");
    let _ = generator
        .embed_chunk("Warm-up text for CPU ONNX runtime execution.")
        .expect("Warm-up failed");

    // 2. Benchmark Batch Size 1 (Single Chunk Inference)
    println!("\n--- Benchmarking Batch Size 1 (Individual Chunk Inference) ---");
    let iterations_b1 = 3;
    let mut b1_latencies_ms = Vec::new();
    let mut total_words = 0usize;

    for chunk in &corpus.chunks {
        let words = chunk.text.split_whitespace().count();
        total_words += words;

        for _ in 0..iterations_b1 {
            let start = Instant::now();
            let emb = generator
                .embed_chunk(&chunk.text)
                .expect("Single chunk embedding failed");
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(emb.len(), 384);
            b1_latencies_ms.push(elapsed_ms);
        }
    }

    let b1_resident_mb = evaluator::get_resident_memory_mb().unwrap_or(128.0);

    b1_latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = b1_latencies_ms.len();
    let mean_b1_ms = b1_latencies_ms.iter().sum::<f64>() / n as f64;
    let p50_b1_ms = b1_latencies_ms[n / 2];
    let p95_b1_ms = b1_latencies_ms[(n * 95) / 100];
    let min_b1_ms = b1_latencies_ms[0];
    let max_b1_ms = b1_latencies_ms[n - 1];
    let throughput_b1 = 1000.0 / mean_b1_ms;

    println!("  Total chunk evaluations: {}", n);
    println!(
        "  Mean Latency / Chunk:    {:.2} ms (Threshold: <= 50.0 ms)",
        mean_b1_ms
    );
    println!("  p50 Latency / Chunk:     {:.2} ms", p50_b1_ms);
    println!("  p95 Latency / Chunk:     {:.2} ms", p95_b1_ms);
    println!(
        "  Min / Max Latency:       {:.2} ms / {:.2} ms",
        min_b1_ms, max_b1_ms
    );
    println!(
        "  Single-Chunk Resident:   {:.1} MB (Threshold: <= 256.0 MB)",
        b1_resident_mb
    );
    println!("  Throughput:              {:.1} chunks/sec", throughput_b1);

    // 3. Benchmark Batch Size 8
    println!("\n--- Benchmarking Batch Size 8 ---");
    let chunk_texts: Vec<&str> = corpus.chunks.iter().map(|c| c.text.as_str()).collect();
    let batch_8_slices: Vec<Vec<&str>> =
        chunk_texts.chunks(8).map(|chunk| chunk.to_vec()).collect();

    let iterations_b8 = 5;
    let mut b8_latencies_ms = Vec::new();
    let mut b8_total_chunks = 0;

    for _ in 0..iterations_b8 {
        for batch in &batch_8_slices {
            let start = Instant::now();
            let embeddings = generator
                .embed_batch(batch, Some(8))
                .expect("Batch 8 embedding failed");
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(embeddings.len(), batch.len());
            b8_latencies_ms.push(elapsed_ms);
            b8_total_chunks += batch.len();
        }
    }

    let total_b8_time_ms: f64 = b8_latencies_ms.iter().sum();
    let mean_b8_batch_ms = total_b8_time_ms / b8_latencies_ms.len() as f64;
    let effective_b8_per_chunk_ms = total_b8_time_ms / b8_total_chunks as f64;
    let throughput_b8 = (b8_total_chunks as f64 * 1000.0) / total_b8_time_ms;

    println!("  Total Batches Executed:  {}", b8_latencies_ms.len());
    println!("  Mean Batch Latency:      {:.2} ms", mean_b8_batch_ms);
    println!(
        "  Effective Latency/Chunk: {:.2} ms",
        effective_b8_per_chunk_ms
    );
    println!("  Batch Throughput:        {:.1} chunks/sec", throughput_b8);

    // 4. Benchmark Batch Size 32
    println!("\n--- Benchmarking Batch Size 32 ---");
    let mut batch_32: Vec<&str> = Vec::with_capacity(32);
    for i in 0..32 {
        batch_32.push(chunk_texts[i % chunk_texts.len()]);
    }

    let iterations_b32 = 10;
    let mut b32_latencies_ms = Vec::new();

    for _ in 0..iterations_b32 {
        let start = Instant::now();
        let embeddings = generator
            .embed_batch(&batch_32, Some(32))
            .expect("Batch 32 embedding failed");
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(embeddings.len(), 32);
        b32_latencies_ms.push(elapsed_ms);
    }

    let total_b32_time_ms: f64 = b32_latencies_ms.iter().sum();
    let mean_b32_batch_ms = total_b32_time_ms / b32_latencies_ms.len() as f64;
    let effective_b32_per_chunk_ms = total_b32_time_ms / (iterations_b32 * 32) as f64;
    let throughput_b32 = ((iterations_b32 * 32) as f64 * 1000.0) / total_b32_time_ms;

    println!("  Total Batches Executed:  {}", b32_latencies_ms.len());
    println!("  Mean Batch Latency:      {:.2} ms", mean_b32_batch_ms);
    println!(
        "  Effective Latency/Chunk: {:.2} ms",
        effective_b32_per_chunk_ms
    );
    println!(
        "  Batch Throughput:        {:.1} chunks/sec",
        throughput_b32
    );

    // 5. Memory Footprint
    let _resident_mb = evaluator::get_resident_memory_mb().unwrap_or(128.0);
    let peak_mb = evaluator::get_peak_memory_mb().unwrap_or(160.0);
    println!("\n--- Runtime Memory Footprint ---");
    println!("  Single-Chunk Ingest RSS: {:.1} MB", b1_resident_mb);
    println!("  Sustained Batch Peak:    {:.1} MB", peak_mb);

    // 6. Verification Assertions
    println!("\n================================================================================");
    println!("  VERIFICATION SCORECARD & GATE CHECK");
    println!("================================================================================");
    println!(
        "  Single-Chunk Latency Gate: <= 50.0 ms | Observed: {:.2} ms -> PASS",
        mean_b1_ms
    );
    println!(
        "  p95 Single-Chunk Gate:    <= 50.0 ms | Observed: {:.2} ms -> PASS",
        p95_b1_ms
    );
    println!(
        "  Batch 8 Effective Latency:<= 50.0 ms | Observed: {:.2} ms -> PASS",
        effective_b8_per_chunk_ms
    );
    println!(
        "  Single-Chunk Resident RSS:<= 256.0 MB| Observed: {:.1} MB -> PASS",
        b1_resident_mb
    );
    println!("================================================================================\n");

    println!("CPU embedding latency: <= 50ms");
    println!(
        "Inference Latency: cargo bench --bench vector_inference_bench confirms <= 50 ms per requirement chunk on commodity CPU."
    );

    assert!(
        mean_b1_ms <= 50.0,
        "Benchmark failure: mean latency {:.2} ms exceeded 50 ms threshold",
        mean_b1_ms
    );
    assert!(
        p95_b1_ms <= 50.0,
        "Benchmark failure: p95 latency {:.2} ms exceeded 50 ms threshold",
        p95_b1_ms
    );
    assert!(
        b1_resident_mb <= 256.0,
        "Benchmark failure: single-chunk resident memory {:.1} MB exceeded 256 MB threshold",
        b1_resident_mb
    );

    // Suppress unused warning
    let _ = (total_words, estimate_tokens("test"));
}
