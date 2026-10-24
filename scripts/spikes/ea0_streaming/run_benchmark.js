#!/usr/bin/env node
// EA-0.2: Automated Benchmark Harness for Client-Side Streaming Speech Recognition.
// Evaluates candidate streaming engines on the exact 60-clip EA-0 testset.

const fs = require('fs');
const path = require('path');
const { performance } = require('perf_hooks');
const sherpa = require('sherpa-onnx');
const { normalize, score } = require('./wer.js');
const { SR, SEED, Rng, addNoise, nonSpeechClips } = require('./audio_util.js');

const TESTSET_DIR = process.env.TKS_EA0_TESTSET || '/workspaces/.tks-cache/ea0-testset';
const MODELS_CACHE = process.env.TKS_MODEL_CACHE || '/workspaces/.tks-cache/models';

function getMemoryRssMb() {
  const mem = process.memoryUsage();
  return (mem.rss / (1024 * 1024)).toFixed(1);
}

// -----------------------------------------------------------------------------
// Engine 1 & 2: Sherpa-ONNX Zipformer (Transducer streaming)
// -----------------------------------------------------------------------------

function createSherpaEngine(modelDir, encoderFile, decoderFile, joinerFile, tokensFile, name) {
  const encPath = path.join(modelDir, encoderFile);
  const decPath = path.join(modelDir, decoderFile);
  const joinPath = path.join(modelDir, joinerFile);
  const tokPath = path.join(modelDir, tokensFile);

  // Compute model download/asset size
  const totalBytes = [encPath, decPath, joinPath, tokPath].reduce(
    (acc, p) => acc + (fs.existsSync(p) ? fs.statSync(p).size : 0),
    0
  );
  const modelSizeMb = (totalBytes / (1024 * 1024)).toFixed(1);

  const t0 = performance.now();
  const recognizer = sherpa.createOnlineRecognizer({
    modelConfig: {
      transducer: {
        encoder: encPath,
        decoder: decPath,
        joiner: joinPath,
      },
      tokens: tokPath,
      numThreads: 1,
      provider: 'cpu',
      debug: 0,
    },
    featConfig: {
      sampleRate: SR,
      featureDim: 80,
    },
    decodingMethod: 'greedy_search',
    enableEndpoint: 0,
  });
  const loadTimeMs = performance.now() - t0;

  // Warmup stream
  const warmStream = recognizer.createStream();
  warmStream.acceptWaveform(SR, new Float32Array(SR));
  warmStream.inputFinished();
  while (recognizer.isReady(warmStream)) recognizer.decode(warmStream);
  warmStream.free();

  return {
    name,
    modelSizeMb,
    loadTimeMs,
    transcribeStream: (pcm) => {
      const stream = recognizer.createStream();
      const tStart = performance.now();
      let firstTokenMs = null;
      const chunkSize = 1600; // 100ms frames to simulate streaming input

      for (let i = 0; i < pcm.length; i += chunkSize) {
        const chunk = pcm.subarray(i, Math.min(i + chunkSize, pcm.length));
        stream.acceptWaveform(SR, chunk);
        while (recognizer.isReady(stream)) {
          recognizer.decode(stream);
          const txt = recognizer.getResult(stream).text;
          if (txt && firstTokenMs === null) {
            firstTokenMs = performance.now() - tStart;
          }
        }
      }

      // Add 250ms silence hangover to flush causal lookahead
      stream.acceptWaveform(SR, new Float32Array(4000));
      stream.inputFinished();
      while (recognizer.isReady(stream)) {
        recognizer.decode(stream);
      }
      const totalMs = performance.now() - tStart;
      const result = recognizer.getResult(stream).text || '';
      stream.free();

      return {
        hypothesis: result,
        processingMs: totalMs,
        firstTokenMs: firstTokenMs || totalMs,
      };
    },
    destroy: () => {
      recognizer.free();
    },
  };
}

// -----------------------------------------------------------------------------
// Engine 3: Transformers.js / Whisper-tiny.en
// -----------------------------------------------------------------------------

async function createWhisperTinyEngine() {
  const { pipeline, env } = await import('@huggingface/transformers');
  const t0 = performance.now();
  const transcriber = await pipeline('automatic-speech-recognition', 'onnx-community/whisper-tiny.en', {
    dtype: 'fp32',
  });
  const loadTimeMs = performance.now() - t0;

  // Warmup
  await transcriber(new Float32Array(SR));

  return {
    name: 'Whisper-tiny.en (Transformers.js ONNX)',
    modelSizeMb: '39.0',
    loadTimeMs,
    transcribeStream: async (pcm) => {
      const tStart = performance.now();
      const res = await transcriber(pcm);
      const totalMs = performance.now() - tStart;
      return {
        hypothesis: (res && res.text) ? res.text : '',
        processingMs: totalMs,
        firstTokenMs: totalMs, // Non-streaming batch pipeline
      };
    },
    destroy: () => {},
  };
}

// -----------------------------------------------------------------------------
// Main Test Runner
// -----------------------------------------------------------------------------

async function runBenchmarkForEngine(engine, manifest, testClips, nonSpeechTestClips) {
  console.log(`\n================================================================`);
  console.log(`BENCHMARKING: ${engine.name}`);
  console.log(`Model Size: ${engine.modelSizeMb} MB | Load Time: ${engine.loadTimeMs.toFixed(1)} ms | Host RSS: ${getMemoryRssMb()} MB`);
  console.log(`================================================================`);

  const resultsByCategory = {
    clean: { clips: [], totalAudioS: 0, totalProcMs: 0, edits: { subs: 0, dels: 0, ins: 0, refWords: 0 }, cleanCount: 0 },
    'noisy-clean': { clips: [], totalAudioS: 0, totalProcMs: 0, edits: { subs: 0, dels: 0, ins: 0, refWords: 0 }, cleanCount: 0 },
    disfluent: { clips: [], totalAudioS: 0, totalProcMs: 0, edits: { subs: 0, dels: 0, ins: 0, refWords: 0 }, cleanCount: 0 },
    'noisy-disfluent': { clips: [], totalAudioS: 0, totalProcMs: 0, edits: { subs: 0, dels: 0, ins: 0, refWords: 0 }, cleanCount: 0 },
  };

  let totalAllAudioS = 0;
  let totalAllProcMs = 0;
  const clipDetails = [];

  for (const item of testClips) {
    const { id, category, audioPcm, reference } = item;
    const audioS = audioPcm.length / SR;
    const { hypothesis, processingMs, firstTokenMs } = await engine.transcribeStream(audioPcm);

    const editStats = score(reference, hypothesis);
    const catStats = resultsByCategory[category];

    catStats.clips.push(id);
    catStats.totalAudioS += audioS;
    catStats.totalProcMs += processingMs;
    catStats.edits.subs += editStats.substitutions;
    catStats.edits.dels += editStats.deletions;
    catStats.edits.ins += editStats.insertions;
    catStats.edits.refWords += editStats.reference_words;
    if (editStats.total === 0) {
      catStats.cleanCount += 1;
    }

    totalAllAudioS += audioS;
    totalAllProcMs += processingMs;

    clipDetails.push({
      id,
      category,
      audioS: audioS.toFixed(2),
      procMs: processingMs.toFixed(1),
      rtf: (processingMs / 1000 / audioS).toFixed(3),
      firstTokenMs: firstTokenMs.toFixed(1),
      wer: (editStats.wer * 100).toFixed(1) + '%',
      edits: `${editStats.substitutions}s/${editStats.deletions}d/${editStats.insertions}i`,
      refNorm: normalize(reference).join(' '),
      hypNorm: normalize(hypothesis).join(' '),
    });
  }

  // Non-speech test
  const nonSpeechResults = [];
  for (const ns of nonSpeechTestClips) {
    const { hypothesis } = await engine.transcribeStream(ns.pcm);
    const norm = normalize(hypothesis).join(' ');
    nonSpeechResults.push({
      id: ns.id,
      rawOutput: hypothesis.trim(),
      normalizedOutput: norm,
      hallucinated: norm.length > 0,
    });
  }

  // Compile Category Summaries
  const summaries = {};
  for (const [cat, data] of Object.entries(resultsByCategory)) {
    const totalEdits = data.edits.subs + data.edits.dels + data.edits.ins;
    const wer = data.edits.refWords > 0 ? (totalEdits / data.edits.refWords) : 0;
    const editsPerMin = data.totalAudioS > 0 ? (totalEdits / (data.totalAudioS / 60)) : 0;
    const avgRtf = data.totalAudioS > 0 ? (data.totalProcMs / 1000 / data.totalAudioS) : 0;

    summaries[cat] = {
      clipCount: data.clips.length,
      audioS: data.totalAudioS.toFixed(1),
      werPercent: (wer * 100).toFixed(1) + '%',
      editsPerMin: editsPerMin.toFixed(1),
      cleanClips: `${data.cleanCount}/${data.clips.length}`,
      avgRtf: avgRtf.toFixed(3),
      edits: data.edits,
    };
  }

  const overallRtf = totalAllAudioS > 0 ? (totalAllProcMs / 1000 / totalAllAudioS) : 0;

  console.log('\n--- Category Results ---');
  console.table(summaries);

  console.log('\n--- Non-Speech & Silence Handling (Silence / Hum / Noise) ---');
  console.table(nonSpeechResults);

  console.log(`\nOverall RTF: ${overallRtf.toFixed(4)} (processes ${(1 / overallRtf).toFixed(1)}x faster than real-time)`);

  return {
    engineName: engine.name,
    modelSizeMb: engine.modelSizeMb,
    loadTimeMs: engine.loadTimeMs,
    rssMb: getMemoryRssMb(),
    overallRtf: overallRtf.toFixed(4),
    summaries,
    nonSpeechResults,
    clipDetails,
  };
}

async function main() {
  console.log('Loading test manifest from:', path.join(TESTSET_DIR, 'manifest.json'));
  const manifestRaw = JSON.parse(fs.readFileSync(path.join(TESTSET_DIR, 'manifest.json'), 'utf8'));

  // Load all 30 base audio clips (clean and disfluent)
  const baseClips = [];
  for (const item of manifestRaw) {
    const wavPath = path.join(TESTSET_DIR, `${item.id}.wav`);
    const wave = sherpa.readWave(wavPath);
    baseClips.push({
      id: item.id,
      category: item.category,
      reference: item.reference,
      pcm: wave.samples,
    });
  }

  // Generate 60 test audio clips (clean, noisy-clean, disfluent, noisy-disfluent) with seeded noise
  const rng = new Rng(SEED);
  const testClips = [];

  for (const item of baseClips) {
    // 1. Clean / Disfluent base
    testClips.push({
      id: item.id,
      category: item.category,
      reference: item.reference,
      audioPcm: item.pcm,
    });

    // 2. Noisy variant (10 dB SNR)
    const noisyPcm = addNoise(item.pcm, 10.0, rng);
    testClips.push({
      id: `noisy-${item.id}`,
      category: `noisy-${item.category}`,
      reference: item.reference,
      audioPcm: noisyPcm,
    });
  }

  // 4 Non-speech clips (10s each)
  const nsRng = new Rng(SEED);
  const nonSpeechTestClips = nonSpeechClips(nsRng);

  console.log(`Prepared ${testClips.length} speech clips (15 per category) + ${nonSpeechTestClips.length} non-speech clips.`);

  // 1. Sherpa Zipformer Standard (2023-06-26)
  const sherpaStd = createSherpaEngine(
    path.join(MODELS_CACHE, 'sherpa-onnx-streaming-zipformer-en-2023-06-26'),
    'encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx',
    'decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx',
    'joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx',
    'tokens.txt',
    'Sherpa-ONNX Zipformer En (Standard INT8, 68MB)'
  );
  const resSherpaStd = await runBenchmarkForEngine(sherpaStd, manifestRaw, testClips, nonSpeechTestClips);
  sherpaStd.destroy();

  // 2. Sherpa Zipformer 20M (2023-02-17)
  const sherpa20M = createSherpaEngine(
    path.join(MODELS_CACHE, 'sherpa-onnx-streaming-zipformer-en-20M-2023-02-17'),
    'encoder-epoch-99-avg-1.int8.onnx',
    'decoder-epoch-99-avg-1.int8.onnx',
    'joiner-epoch-99-avg-1.int8.onnx',
    'tokens.txt',
    'Sherpa-ONNX Zipformer En-20M (Ultra-light INT8, 42MB)'
  );
  const resSherpa20M = await runBenchmarkForEngine(sherpa20M, manifestRaw, testClips, nonSpeechTestClips);
  sherpa20M.destroy();

  // 3. Whisper-tiny.en (Transformers.js)
  const whisperTiny = await createWhisperTinyEngine();
  const resWhisperTiny = await runBenchmarkForEngine(whisperTiny, manifestRaw, testClips, nonSpeechTestClips);
  whisperTiny.destroy();

  // Save report to JSON
  const reportPath = path.join(MODELS_CACHE, '..', 'ea0-streaming-benchmark-results.json');
  fs.writeFileSync(
    reportPath,
    JSON.stringify({
      timestamp: new Date().toISOString(),
      engines: [resSherpaStd, resSherpa20M, resWhisperTiny],
    }, null, 2)
  );
  console.log(`\nSaved benchmark results to: ${reportPath}`);
}

main().catch(console.error);
