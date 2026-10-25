#!/usr/bin/env node
// Offline browser regression for provider capabilities and completion metadata.
const { chromium } = require('./gate/node_modules/playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function main() {
  const browser = await chromium.launch({ headless: true });
  try {
    for (const [provider, available] of [['tierfall', true], ['ollama', true], ['none', true], ['tierfall', false]]) {
      const page = await browser.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.addInitScript(() => {
        window.EventSource = class {
          constructor() { window.testEvents = this; this.listeners = {}; }
          addEventListener(name, listener) { this.listeners[name] = listener; }
          close() {}
        };
      });
      await page.route('http://elicitation.test/**', async route => {
        const url = new URL(route.request().url());
        if (url.pathname.endsWith('/capabilities')) {
          return route.fulfill({ json: {
            asr: { available: false },
            facilitator: { provider, route: 'auto', model: 'qwen3:8b', available },
          } });
        }
        if (url.pathname === '/api/v1/elicitation/sessions') {
          return route.fulfill({ status: 201, json: {
            elicitation_id: 'synthetic-id', session_id: 'synthetic-test', title: 'Synthetic test',
            state: 'draft', turns: [], explored_items: [], participants: [],
          } });
        }
        const name = path.basename(url.pathname);
        const file = ['elicitation.js', 'elicitation.css'].includes(name) ? name : 'index.html';
        return route.fulfill({
          contentType: file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html',
          body: fs.readFileSync(path.resolve(__dirname, '../../src/gateway/elicitation_ui', file)),
        });
      });
      await page.goto('http://elicitation.test/explorer/elicitation');
      await page.waitForFunction(() => !document.getElementById('facilitator-text').textContent.includes('Checking'));
      const label = await page.locator('#facilitator-text').textContent();
      assert(label.includes(provider === 'tierfall' ? available ? 'Tierfall (hosted routing)' : 'Tierfall (Unavailable)' : provider === 'ollama' ? 'Ollama' : 'None'));
      assert.equal(await page.locator('#auto-propose-toggle').isChecked(), provider !== 'none' && available);
      if (provider === 'tierfall' && available) {
        await page.locator('#setup-title').fill('Synthetic test');
        await page.locator('#setup-slug').fill('synthetic-test');
        await page.locator('#setup-stakeholder').fill('Member');
        await page.locator('#setup-facilitator').fill('Volunteer');
        await page.locator('#btn-create-session').click();
        await page.waitForFunction(() => window.testEvents);
        await page.evaluate(() => window.testEvents.listeners.facilitator_proposal({
          data: JSON.stringify({ type: 'facilitator_proposal', data: {
            proposal_id: 'synthetic-proposal', mode: 'socratic', text: 'A or B?', flags: [],
            generation: { provider: 'ollama', account: 'local', model: 'qwen3:8b', attempts: 2, duration_ms: 1250 },
          } }),
        }));
        assert.equal(await page.locator('#sidecar-model-tag').textContent(), 'ollama / qwen3:8b · 1.3s');
        assert((await page.locator('#sidecar-model-tag').getAttribute('title')).includes('2 upstream'));
      }
      assert.deepEqual(errors, []);
      await page.close();
    }
    console.log('Facilitator UI: Tierfall, Ollama, manual, and local-fallback metadata passed.');
  } finally {
    await browser.close();
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
