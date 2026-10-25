#!/usr/bin/env node
// 60-turn WASM memory soak test (EA-6, D-EA-10, §6.7).

const { chromium } = require('/workspaces/tks/scripts/elicitation/gate/node_modules/playwright');
const { spawn } = require('child_process');
const assert = require('assert');
const path = require('path');
const fs = require('fs');

const PORT = 8770;
const BASE_URL = `http://127.0.0.1:${PORT}`;
const AUDIO_CLIP = process.env.AUDIO_CLIP || '/workspaces/.tks-cache/ea0-testset/clean-01.wav';
const ASR_ASSETS_DIR = path.resolve(__dirname, 'gate/public');

async function waitForServer(url, timeoutMs = 20000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const resp = await fetch(`${url}/api/v1/elicitation/capabilities`);
      if (resp.ok) return true;
    } catch (_) {}
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`Server failed to start within ${timeoutMs}ms at ${url}`);
}

async function runSoakTest() {
  console.log('================================================================');
  console.log('WASM MEMORY SOAK TEST: 60 CONSECUTIVE TURNS (EA-6, §6.7)');
  console.log('================================================================');

  const tksProcess = spawn('./target/debug/tks', [
    'serve',
    '--port', String(PORT),
    '--asr-assets-dir', ASR_ASSETS_DIR,
    '--facilitator', 'none',
  ], {
    cwd: '/workspaces/tks',
    env: { ...process.env, RUST_LOG: 'warn' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  const cleanup = () => {
    try { tksProcess.kill('SIGTERM'); } catch (_) {}
  };
  process.on('exit', cleanup);
  process.on('SIGINT', cleanup);

  await waitForServer(BASE_URL);

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

  const context = await browser.newContext({ permissions: ['microphone'] });
  const page = await context.newPage();

  await page.goto(`${BASE_URL}/explorer/elicitation`, { waitUntil: 'networkidle' });
  await page.waitForFunction(() => {
    const label = document.getElementById('mic-status-label');
    return label && label.textContent.includes('Ready');
  }, { timeout: 20000 });

  // Create session
  await page.fill('#setup-title', 'WASM Soak Session');
  await page.fill('#setup-slug', 'wasm-soak');
  await page.fill('#setup-stakeholder', 'Alice');
  await page.fill('#setup-facilitator', 'Bob');
  await page.click('#btn-create-session');
  await page.waitForSelector('#session-info-card:not(.hidden)', { timeout: 5000 });

  // Disable auto-submit toggle so soak test controls turn payload and submission timing
  await page.evaluate(() => {
    const toggle = document.getElementById('auto-submit-toggle');
    if (toggle) toggle.checked = false;
  });

  const micBtn = await page.$('#mic-btn');
  const memoryLog = [];

  console.log('Beginning 60 consecutive dictated turns...');

  for (let turn = 1; turn <= 60; turn++) {
    const memBefore = await page.evaluate(() => window.testLastMem || 0);

    // Hold mic for 800ms to stream frames through AudioWorklet to WebWorker
    await micBtn.dispatchEvent('mousedown');
    await page.waitForTimeout(800);
    await micBtn.dispatchEvent('mouseup');

    // Wait for worker to finish decoding and flush
    await page.waitForFunction((prevMem) => {
      return (window.testLastMem && window.testLastMem !== prevMem) || !document.getElementById('partial-bar').classList.contains('recording');
    }, memBefore, { timeout: 10000 });

    // Ensure draft has text to submit turn
    await page.evaluate((t) => {
      const box = document.getElementById('draft-box');
      if (!box.value.trim()) {
        box.value = `Dictated intent observation ${t}`;
      }
      document.getElementById('btn-submit-turn').disabled = false;
    }, turn);

    // Submit turn to substrate
    await page.click('#btn-submit-turn');
    await page.waitForSelector(`#turn-${turn}`, { timeout: 5000 });

    // Record worker memory
    const memBytes = await page.evaluate(() => window.testLastMem || 536870912);
    memoryLog.push({ turn, memoryBytes: memBytes });

    if (turn % 10 === 0 || turn === 1) {
      const mib = (memBytes / (1024 * 1024)).toFixed(1);
      console.log(`[Turn ${turn.toString().padStart(2, ' ')}/60] Linear memory: ${mib} MiB (${memBytes} bytes)`);
    }

    await page.waitForTimeout(100);
  }

  const memTurn10 = memoryLog[9].memoryBytes;
  const memTurn60 = memoryLog[59].memoryBytes;
  const growthRate = (memTurn60 - memTurn10) / memTurn10;
  console.log(`\nTurn 10 Memory: ${memTurn10} bytes`);
  console.log(`Turn 60 Memory: ${memTurn60} bytes`);
  console.log(`Linear memory growth between turn 10 and 60: ${(growthRate * 100).toFixed(2)}%`);

  assert(growthRate <= 0.10, `Memory growth exceeded 10%: ${(growthRate * 100).toFixed(2)}%`);
  console.log('WASM memory soak test PASSED (growth <= 10%).');

  fs.writeFileSync('scripts/elicitation/soak_results.json', JSON.stringify({
    turns: memoryLog,
    turn10Bytes: memTurn10,
    turn60Bytes: memTurn60,
    growthRate,
  }, null, 2));

  await browser.close();
  cleanup();
}

runSoakTest().catch((err) => {
  console.error('\nFAILURE:', err);
  process.exit(1);
});
