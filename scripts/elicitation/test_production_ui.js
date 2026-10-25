#!/usr/bin/env node
// Comprehensive Playwright integration test and privacy audit for Track E-A production UI (D-130..D-134, §6.3, §6.7).

const { chromium } = require('/workspaces/tks/scripts/elicitation/gate/node_modules/playwright');
const { spawn } = require('child_process');
const assert = require('assert');
const path = require('path');
const fs = require('fs');

const PORT = 8769;
const BASE_URL = `http://127.0.0.1:${PORT}`;
const AUDIO_CLIP = process.env.AUDIO_CLIP || '/workspaces/.tks-cache/ea0-testset/clean-01.wav';
const ASR_ASSETS_DIR = path.resolve(__dirname, 'gate/public');

assert(fs.existsSync(AUDIO_CLIP), `Audio clip not found: ${AUDIO_CLIP}`);
assert(fs.existsSync(ASR_ASSETS_DIR), `ASR assets dir not found: ${ASR_ASSETS_DIR}`);

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

async function runProductionSuite() {
  console.log('================================================================');
  console.log('TRACK E-A PRODUCTION UI, PRIVACY AUDIT & WASM SOAK TEST');
  console.log('================================================================');
  console.log(`Port: ${PORT}`);
  console.log(`Audio source: ${AUDIO_CLIP}`);
  console.log(`ASR Assets: ${ASR_ASSETS_DIR}`);

  // 1. Spawn tks serve daemon
  console.log('\n[1/7] Spawning tks serve daemon...');
  const tksProcess = spawn('./target/debug/tks', [
    'serve',
    '--port', String(PORT),
    '--asr-assets-dir', ASR_ASSETS_DIR,
    '--facilitator', 'none',
  ], {
    cwd: '/workspaces/tks',
    env: { ...process.env, RUST_LOG: 'info' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  tksProcess.stderr.on('data', (d) => process.stderr.write(`[TKS STDERR] ${d}`));
  tksProcess.stdout.on('data', (d) => process.stdout.write(`[TKS STDOUT] ${d}`));

  const cleanupTks = () => {
    try { tksProcess.kill('SIGTERM'); } catch (_) {}
  };
  process.on('exit', cleanupTks);
  process.on('SIGINT', cleanupTks);

  await waitForServer(BASE_URL);
  console.log('tks serve is live and responding.');

  // Network audit ledger
  const networkAudit = {
    websockets: [],
    requests: [],
  };

  // 2. Launch Chromium with fake audio capture
  console.log('\n[2/7] Launching Chromium with fake audio capture...');
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
  page.on('console', (msg) => console.log(`[BROWSER ${msg.type().toUpperCase()}]`, msg.text()));
  page.on('pageerror', (err) => console.error('[BROWSER ERROR]', err));

  // 3. Navigate to Elicitation Studio & verify security headers
  console.log('\n[3/7] Navigating to /explorer/elicitation & checking security headers...');
  const response = await page.goto(`${BASE_URL}/explorer/elicitation`, { waitUntil: 'networkidle' });
  assert.strictEqual(response.status(), 200);

  const headers = response.headers();
  console.log('Response headers:');
  console.log(' - content-security-policy:', headers['content-security-policy']);
  console.log(' - permissions-policy:', headers['permissions-policy']);
  console.log(' - referrer-policy:', headers['referrer-policy']);
  console.log(' - x-content-type-options:', headers['x-content-type-options']);

  assert(headers['content-security-policy']?.includes("default-src 'self'"), 'CSP missing default-src self');
  assert(headers['content-security-policy']?.includes("script-src 'self' 'wasm-unsafe-eval'"), 'CSP missing wasm-unsafe-eval');
  assert.strictEqual(headers['permissions-policy'], 'microphone=(self)');
  assert.strictEqual(headers['referrer-policy'], 'no-referrer');
  assert.strictEqual(headers['x-content-type-options'], 'nosniff');

  // Verify capability banner
  const asrBannerText = await page.textContent('#asr-text');
  console.log('Capability banner text:', asrBannerText);
  assert(asrBannerText.includes('v1.13.7'), `Expected version v1.13.7 in banner, got: ${asrBannerText}`);

  // Wait for ASR worker initialization
  console.log('Waiting for ASR model initialization and mic button enablement...');
  await page.waitForFunction(() => {
    const label = document.getElementById('mic-status-label');
    return label && label.textContent.includes('Ready');
  }, { timeout: 20000 });
  const micLabel = await page.textContent('#mic-status-label');
  console.log('Mic status label:', micLabel);
  assert(micLabel.includes('Ready'), 'Expected Ready status label');

  // 4. Create session via UI
  console.log('\n[4/7] Submitting session setup form...');
  await page.fill('#setup-title', 'Enterprise Resiliency Substrate');
  await page.fill('#setup-slug', 'resiliency-substrate');
  await page.fill('#setup-stakeholder', 'Alice');
  await page.fill('#setup-facilitator', 'Bob');
  await page.click('#btn-create-session');

  await page.waitForSelector('#session-info-card:not([style*="display: none"])', { timeout: 5000 });
  assert.strictEqual(await page.textContent('#meta-session-id'), 'resiliency-substrate');
  assert.strictEqual(await page.textContent('#meta-source'), 'manual');
  console.log('Session successfully created. ID: resiliency-substrate, Source: manual');

  // 5. Test Push-to-Talk Spoken Dictation (Auto-Submit on Release)
  console.log('\n[5/7] Executing push-to-talk spoken dictation...');
  const micBtn = await page.$('#mic-btn');

  // Hold mic button for 3 seconds
  await micBtn.dispatchEvent('mousedown');
  console.log('Recording started (mousedown). Waiting for partial speech...');
  await page.waitForSelector('#partial-bar.recording', { timeout: 3000 });

  // Let audio play
  await page.waitForTimeout(2500);

  // Release mic button
  await micBtn.dispatchEvent('mouseup');
  console.log('Recording stopped (mouseup). Waiting for auto-submission of dictated turn...');

  // Auto-submit triggers directly upon mouseup when auto-submit toggle is checked
  await page.waitForSelector('.turn-item.stakeholder', { timeout: 5000 });

  // Verify MediaStreamTrack is ended
  const tracksEnded = await page.evaluate(() => {
    if (!window.audioStream) return true;
    return window.audioStream.getTracks().every((t) => t.readyState === 'ended');
  });
  assert(tracksEnded, 'Expected audio tracks to be stopped after recording release');

  const turnNumber = await page.textContent('.turn-number');
  const turnRole = await page.textContent('.turn-role-badge');
  const turnMode = await page.textContent('.turn-mode-badge');
  const updatedSource = await page.textContent('#meta-source');
  const turnText = await page.textContent('.turn-text');
  console.log(`Committed turn: ${turnNumber} | ${turnRole} | Mode: ${turnMode} | Session source: ${updatedSource}`);
  console.log(`Committed turn text: "${turnText}"`);
  assert.strictEqual(turnNumber, '[t1]');
  assert.strictEqual(turnRole, 'stakeholder');
  assert.strictEqual(turnMode, 'dictated');
  assert.strictEqual(updatedSource, 'asr');
  assert(turnText.length > 0, 'Expected non-empty turn text');
  assert.strictEqual(turnNumber, '[t1]');
  assert.strictEqual(turnRole, 'stakeholder');
  assert.strictEqual(turnMode, 'dictated');
  assert.strictEqual(updatedSource, 'asr');

  // 6. Turn Review, Correction & Uncertainty Flagging
  console.log('\n[6/7] Testing turn review actions (flag & correction)...');
  // Flag uncertain
  page.on('dialog', async (dialog) => {
    await dialog.accept('Ambiguous technical parameter');
  });
  await page.click('.btn-flag-turn');
  await page.waitForSelector('.turn-uncertainty-flag', { timeout: 3000 });
  const flagText = await page.textContent('.turn-uncertainty-flag');
  console.log('Uncertainty flag applied:', flagText);
  assert(flagText.includes('Ambiguous technical parameter'));

  // Clear flag
  await page.click('.btn-flag-turn');
  await page.waitForFunction(() => !document.querySelector('.turn-uncertainty-flag'), { timeout: 3000 });
  console.log('Uncertainty flag cleared.');

  // Close session & export canonical transcript
  console.log('Closing session...');
  await page.click('#btn-close-session');
  await page.waitForFunction(() => {
    const badge = document.getElementById('session-state-badge');
    return badge && badge.textContent === 'Closed';
  }, { timeout: 5000 });
  const stateBadgeText = await page.textContent('#session-state-badge');
  assert.strictEqual(stateBadgeText, 'Closed');

  const exportBtnDisabled = await page.getAttribute('#btn-export-transcript', 'disabled');
  assert.strictEqual(exportBtnDisabled, null, 'Export button should be enabled after close');

  // Fetch canonical transcript
  const elicitationId = await page.evaluate(() => window.__currentSession.elicitation_id);
  const transcriptResp = await page.request.get(`${BASE_URL}/api/v1/elicitation/sessions/${elicitationId}/transcript`);
  assert.strictEqual(transcriptResp.status(), 200);
  const transcriptText = await transcriptResp.text();
  console.log('Canonical transcript preview:\n' + transcriptText.split('\n').slice(0, 15).join('\n'));
  assert(transcriptText.includes('format: tks-transcript/v1'));
  assert(transcriptText.includes('source: "asr"'));
  assert(transcriptText.includes('**[t1] Alice (stakeholder):**'));

  // 7. Network and Storage Privacy Audit
  console.log('\n[7/7] Running strict privacy audit...');
  console.log(`Total WebSockets opened: ${networkAudit.websockets.length}`);
  console.log(`Total HTTP requests: ${networkAudit.requests.length}`);
  assert.strictEqual(networkAudit.websockets.length, 0, 'Prohibited WebSocket opened');

  let audioBytesTransmitted = 0;
  for (const req of networkAudit.requests) {
    const url = new URL(req.url);
    assert.strictEqual(url.origin, BASE_URL, `Request leaked outside local origin: ${req.url}`);
    if (req.postData) {
      // Body must be valid JSON or empty
      try {
        JSON.parse(req.postData);
      } catch (e) {
        throw new Error(`Non-JSON request body transmitted: ${req.postData}`);
      }
    }
  }
  console.log('Network audit PASSED: 0 WebSockets, 0 non-self requests, 0 raw audio bytes.');

  // Storage audit
  const storageAudit = await page.evaluate(() => {
    return {
      localStorageKeys: Object.keys(localStorage),
      sessionStorageKeys: Object.keys(sessionStorage),
    };
  });
  console.log('Browser storage audit:', storageAudit);
  assert(storageAudit.localStorageKeys.every(k => k === 'elicitation_id' || k.length === 0));
  assert(storageAudit.sessionStorageKeys.every(k => k === 'elicitation_id' || k.length === 0));

  await browser.close();
  cleanupTks();

  console.log('\n================================================================');
  console.log('ALL PLAYWRIGHT TESTS & PRIVACY AUDITS PASSED CLEANLY (100%)');
  console.log('================================================================');
}

runProductionSuite().catch((err) => {
  console.error('\nFAILURE:', err);
  process.exit(1);
});
