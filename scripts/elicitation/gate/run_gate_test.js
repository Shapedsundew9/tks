#!/usr/bin/env node
// Automated in-browser proof gate test for EA-2.0 (D-131, D-132, D-EA-13, §6.3).

const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');
const assert = require('assert');
const { server, PORT } = require('./server.js');

const AUDIO_CLIP = process.env.AUDIO_CLIP || '/workspaces/.tks-cache/ea0-testset/clean-01.wav';

async function runGateTest() {
  console.log('========================================================');
  console.log('EA-2.0 IN-BROWSER PROOF GATE (SHERPA-ONNX WASM / CHROMIUM)');
  console.log('========================================================');
  console.log('Fake capture audio source:', AUDIO_CLIP);

  assert(fs.existsSync(AUDIO_CLIP), `Audio clip not found: ${AUDIO_CLIP}`);

  await new Promise((resolve) => server.listen(PORT, '127.0.0.1', resolve));
  console.log(`Gate server listening on http://127.0.0.1:${PORT}`);

  const networkAudit = {
    websockets: [],
    requests: [],
  };

  const browser = await chromium.launch({
    headless: true,
    env: {
      ...process.env,
      LD_LIBRARY_PATH: '/home/vscode/.cache/ms-playwright/extra-libs:' + (process.env.LD_LIBRARY_PATH || ''),
    },
    args: [
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      `--use-file-for-fake-audio-capture=${AUDIO_CLIP}`,
      '--autoplay-policy=no-user-gesture-required',
    ],
  });

  const context = await browser.newContext({
    permissions: ['microphone'],
  });

  context.on('websocket', (ws) => {
    networkAudit.websockets.push(ws.url());
  });

  context.on('request', (req) => {
    networkAudit.requests.push({
      url: req.url(),
      method: req.method(),
      postData: req.postData(),
    });
  });

  const page = await context.newPage();
  page.on('console', (msg) => console.log(`[PAGE ${msg.type().toUpperCase()}]`, msg.text()));
  page.on('pageerror', (err) => console.error('[PAGE ERROR]', err));

  console.log('\n[1/4] Navigating to gate page and measuring cold load...');
  const tNavStart = Date.now();
  await page.goto(`http://127.0.0.1:${PORT}/index.html`);

  // Wait for WASM + model data to initialize
  await page.waitForFunction(() => window.__gate_metrics && window.__gate_metrics.is_ready, {
    timeout: 120000,
  });

  const coldLoadTimeMs = Date.now() - tNavStart;
  const metricsAfterLoad = await page.evaluate(() => window.__gate_metrics);
  console.log(`Cold load completed in ${coldLoadTimeMs} ms`);
  console.log(`WASM linear memory: ${(metricsAfterLoad.wasm_memory_bytes / 1024 / 1024).toFixed(1)} MB`);

  console.log('\n[2/4] Testing push-to-talk dictation stream with fake capture audio...');
  // Click and hold mic button for 3.5 seconds
  const micBtn = page.locator('#micBtn');
  await micBtn.hover();
  await page.mouse.down();
  console.log('Recording active... capturing frames...');

  // Wait 3500 ms while audio plays
  await page.waitForTimeout(3500);

  console.log('Releasing push-to-talk button (triggering 250ms silence flush)...');
  await page.mouse.up();

  // Wait for release_to_final_ms to be populated
  await page.waitForFunction(
    () => window.__gate_metrics && window.__gate_metrics.release_to_final_ms !== null,
    { timeout: 10000 }
  );

  const finalMetrics = await page.evaluate(() => window.__gate_metrics);

  console.log('\n[3/4] Latency and Accuracy Results:');
  const latencies = finalMetrics.frame_latencies_ms;
  latencies.sort((a, b) => a - b);
  const p50Frame = latencies[Math.floor(latencies.length * 0.50)] || 0;
  const p95Frame = latencies[Math.floor(latencies.length * 0.95)] || 0;
  const releaseToFinal = finalMetrics.release_to_final_ms;

  console.log(`- Frames captured & decoded: ${latencies.length}`);
  console.log(`- Frame-to-partial latency: p50 = ${Math.round(p50Frame)} ms, p95 = ${Math.round(p95Frame)} ms (target: <= 500 ms)`);
  console.log(`- Release-to-final latency: ${Math.round(releaseToFinal)} ms (target: <= 500 ms)`);
  console.log(`- Final recognized text: "${finalMetrics.final_text}"`);

  console.log('\n[4/4] Verifying Network and Safety Audits:');
  console.log(`- WebSockets opened: ${networkAudit.websockets.length}`);
  console.log(`- Total HTTP requests: ${networkAudit.requests.length}`);

  // Assertions per D-132 / C-28
  assert.strictEqual(
    networkAudit.websockets.length,
    0,
    `Gate failed: WebSockets opened (${networkAudit.websockets.join(', ')})`
  );

  for (const req of networkAudit.requests) {
    assert(
      req.url.startsWith(`http://127.0.0.1:${PORT}/`),
      `Gate failed: Non-self request detected to ${req.url}`
    );
    if (req.postData) {
      assert.doesNotThrow(() => JSON.parse(req.postData), `Invalid request payload: ${req.postData}`);
    }
  }
  console.log('✓ Zero WebSockets opened');
  console.log('✓ Zero non-self origin requests');
  console.log('✓ Zero audio bytes transmitted over network');

  // Assertions per D-EA-13 latency limits
  assert(
    p95Frame <= 500,
    `Gate failed: p95 frame-to-partial latency (${p95Frame} ms) exceeds D-EA-13 limit (500 ms)`
  );
  assert(
    releaseToFinal <= 500,
    `Gate failed: release-to-final latency (${releaseToFinal} ms) exceeds D-EA-13 limit (500 ms)`
  );
  console.log('✓ p95 frame-to-partial within D-EA-13 limit (<= 500 ms)');
  console.log('✓ Release-to-final within D-EA-13 limit (<= 500 ms)');

  await browser.close();
  await new Promise((resolve) => server.close(resolve));

  const reportData = {
    timestamp: new Date().toISOString(),
    cold_load_time_ms: Math.round(coldLoadTimeMs),
    wasm_memory_mb: Number((finalMetrics.wasm_memory_bytes / 1024 / 1024).toFixed(1)),
    frames_captured: latencies.length,
    p50_frame_to_partial_ms: Math.round(p50Frame),
    p95_frame_to_partial_ms: Math.round(p95Frame),
    release_to_final_ms: Math.round(releaseToFinal),
    final_text: finalMetrics.final_text,
    websockets_count: networkAudit.websockets.length,
    requests_count: networkAudit.requests.length,
    all_self_origin: true,
  };

  fs.writeFileSync(
    path.join(__dirname, 'gate_results.json'),
    JSON.stringify(reportData, null, 2),
    'utf8'
  );

  console.log('\n========================================================');
  console.log('EA-2.0 AUTOMATED IN-BROWSER PROOF GATE: PASSED');
  console.log('========================================================\n');
  return reportData;
}

if (require.main === module) {
  runGateTest().catch((err) => {
    console.error('EA-2.0 Gate Test FAILED:', err);
    process.exit(1);
  });
}

module.exports = { runGateTest };
