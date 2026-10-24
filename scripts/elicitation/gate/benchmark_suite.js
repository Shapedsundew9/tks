#!/usr/bin/env node
// Benchmark runner across multiple clean and disfluent audio clips for EA-2.0.

const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');
const assert = require('assert');
const { server, PORT } = require('./server.js');

const TESTSET_DIR = '/workspaces/.tks-cache/ea0-testset';

const CLIPS = [
  { name: 'clean-01', file: 'clean-01.wav', category: 'clean', duration_ms: 3500 },
  { name: 'clean-02', file: 'clean-02.wav', category: 'clean', duration_ms: 3500 },
  { name: 'clean-03', file: 'clean-03.wav', category: 'clean', duration_ms: 4500 },
  { name: 'clean-04', file: 'clean-04.wav', category: 'clean', duration_ms: 4000 },
  { name: 'clean-05', file: 'clean-05.wav', category: 'clean', duration_ms: 5000 },
  { name: 'disfluent-01', file: 'disfluent-01.wav', category: 'disfluent', duration_ms: 3500 },
  { name: 'disfluent-02', file: 'disfluent-02.wav', category: 'disfluent', duration_ms: 4500 },
  { name: 'disfluent-03', file: 'disfluent-03.wav', category: 'disfluent', duration_ms: 4500 },
  { name: 'disfluent-04', file: 'disfluent-04.wav', category: 'disfluent', duration_ms: 4500 },
  { name: 'disfluent-05', file: 'disfluent-05.wav', category: 'disfluent', duration_ms: 5000 },
];

async function runClip(clip) {
  const audioPath = path.join(TESTSET_DIR, clip.file);
  assert(fs.existsSync(audioPath), `Clip not found: ${audioPath}`);

  const networkAudit = { websockets: 0, nonSelfRequests: 0 };

  const browser = await chromium.launch({
    headless: true,
    env: {
      ...process.env,
      LD_LIBRARY_PATH: '/home/vscode/.cache/ms-playwright/extra-libs:' + (process.env.LD_LIBRARY_PATH || ''),
    },
    args: [
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      `--use-file-for-fake-audio-capture=${audioPath}`,
      '--autoplay-policy=no-user-gesture-required',
    ],
  });

  const context = await browser.newContext({ permissions: ['microphone'] });
  context.on('websocket', () => { networkAudit.websockets++; });
  context.on('request', (req) => {
    if (!req.url().startsWith(`http://127.0.0.1:${PORT}/`)) {
      networkAudit.nonSelfRequests++;
    }
  });

  const page = await context.newPage();
  const tNavStart = Date.now();
  await page.goto(`http://127.0.0.1:${PORT}/index.html`);

  await page.waitForFunction(() => window.__gate_metrics && window.__gate_metrics.is_ready, {
    timeout: 60000,
  });
  const coldLoadTimeMs = Date.now() - tNavStart;

  const micBtn = page.locator('#micBtn');
  await micBtn.hover();
  await page.mouse.down();

  await page.waitForTimeout(clip.duration_ms);

  await page.mouse.up();

  await page.waitForFunction(
    () => window.__gate_metrics && window.__gate_metrics.release_to_final_ms !== null,
    { timeout: 10000 }
  );

  const metrics = await page.evaluate(() => window.__gate_metrics);
  const latencies = metrics.frame_latencies_ms.sort((a, b) => a - b);
  const p50 = latencies[Math.floor(latencies.length * 0.5)] || 0;
  const p95 = latencies[Math.floor(latencies.length * 0.95)] || 0;

  await browser.close();

  return {
    clip: clip.name,
    category: clip.category,
    cold_load_ms: Math.round(coldLoadTimeMs),
    frames: latencies.length,
    p50_frame_ms: Math.round(p50),
    p95_frame_ms: Math.round(p95),
    release_to_final_ms: Math.round(metrics.release_to_final_ms),
    final_text: metrics.final_text,
    websockets: networkAudit.websockets,
    non_self_requests: networkAudit.nonSelfRequests,
  };
}

async function main() {
  await new Promise((resolve) => server.listen(PORT, '127.0.0.1', resolve));
  console.log(`Gate server listening on http://127.0.0.1:${PORT}`);

  const results = [];
  for (const clip of CLIPS) {
    process.stdout.write(`Benchmarking ${clip.name} (${clip.category})... `);
    const res = await runClip(clip);
    console.log(`p95=${res.p95_frame_ms}ms, release=${res.release_to_final_ms}ms, text="${res.final_text.slice(0, 35)}..."`);
    results.push(res);
  }

  await new Promise((resolve) => server.close(resolve));

  fs.writeFileSync(
    path.join(__dirname, 'benchmark_results.json'),
    JSON.stringify(results, null, 2),
    'utf8'
  );
  console.log('Results written to benchmark_results.json');
}

main().catch((err) => {
  console.error('Benchmark failed:', err);
  process.exit(1);
});
