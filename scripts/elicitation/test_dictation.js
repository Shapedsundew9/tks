#!/usr/bin/env node
// Deterministic dictation regressions: no microphone, model, server, or database.
const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { chromium } = require('./gate/node_modules/playwright');

const uiDir = path.resolve(__dirname, '../../src/gateway/elicitation_ui');
const phrase = 'We need reliable dictation';
let browser;

before(async () => { browser = await chromium.launch({ headless: true }); });
after(async () => { await browser?.close(); });

async function openUI(t, options = {}) {
  const page = await browser.newPage();
  page.setDefaultTimeout(3000);
  t.after(() => page.close());
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  t.after(() => assert.deepEqual(errors, []));
  const turns = [];
  await page.addInitScript(options => {
    const state = window.audioTest = { messages: [], tracks: [], contexts: [], worklets: [] };
    window.EventSource = class {
      constructor() { state.events = this; this.listeners = {}; }
      addEventListener(name, listener) { this.listeners[name] = listener; }
      close() {}
    };
    window.Worker = class {
      postMessage(message) {
        state.messages.push(message.type);
        if (message.type === 'init') {
          queueMicrotask(() => this.onmessage({ data: { type: 'ready' } }));
        } else if (message.type === 'stop') {
          queueMicrotask(() => this.onmessage({
            data: { type: 'final', text: 'WE NEED RELIABLE DICTATION' },
          }));
        }
      }
    };
    Object.defineProperty(navigator, 'mediaDevices', { value: {
      getUserMedia() {
        if (options.permissionDenied) return Promise.reject(new Error('Microphone denied'));
        const track = { readyState: 'live', stop() { this.readyState = 'ended'; } };
        state.tracks.push(track);
        const stream = { getTracks: () => [track] };
        if (options.pendingPermission) {
          return new Promise(resolve => { state.grantPermission = () => resolve(stream); });
        }
        return Promise.resolve(stream);
      },
    } });
    window.AudioContext = class {
      constructor() {
        this.state = options.suspended ? 'suspended' : 'running';
        this.audioWorklet = { addModule: () => {
          if (options.moduleFails) return Promise.reject(new Error('Worklet unavailable'));
          return options.pendingModule
            ? new Promise(resolve => { state.loadModule = resolve; })
            : Promise.resolve();
        } };
        this.destination = {};
        state.contexts.push(this);
      }
      async resume() { this.state = 'running'; }
      createMediaStreamSource() { return { connect() {} }; }
      async close() { this.state = 'closed'; }
    };
    window.AudioWorkletNode = class {
      constructor() {
        state.worklets.push(this);
        this.port = {
          postMessage: message => {
            if (message.type === 'flush' && !options.missingFlushAck) {
              queueMicrotask(() => {
                this.port.onmessage({ data: { type: 'pcm', data: new Float32Array([0.5]) } });
                this.port.onmessage({ data: { type: 'flushed' } });
              });
            }
          },
        };
      }
      connect() {}
      disconnect() {}
    };
  }, options);
  await page.route('http://elicitation.test/**', async route => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith('/capabilities')) {
      return route.fulfill({ json: {
        asr: { available: true, version: 'test', assets_base: '/asr' },
        facilitator: { provider: 'none', available: true },
      } });
    }
    if (url.pathname === '/api/v1/elicitation/sessions') {
      return route.fulfill({ status: 201, json: {
        elicitation_id: 'dictation-test', session_id: 'dictation-test', title: 'Dictation test',
        state: 'open', turns: [], participants: [], explored_items: [],
      } });
    }
    if (url.pathname.endsWith('/turns')) {
      turns.push(route.request().postDataJSON());
      return route.fulfill(options.submitFails
        ? { status: 503, json: { message: 'Test submission failure' } }
        : { status: 201, json: { number: turns.length, ...turns.at(-1) } });
    }
    const name = path.basename(url.pathname);
    const file = ['elicitation.js', 'elicitation.css'].includes(name) ? name : 'index.html';
    return route.fulfill({
      contentType: file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html',
      body: fs.readFileSync(path.join(uiDir, file)),
    });
  });
  await page.goto('http://elicitation.test/explorer/elicitation');
  await page.waitForFunction(() => document.getElementById('mic-status-label').textContent.startsWith('Ready'));
  if (options.session) {
    await page.locator('#setup-title').fill('Dictation test');
    await page.locator('#setup-slug').fill('dictation-test');
    await page.locator('#setup-stakeholder').fill('Member');
    await page.locator('#setup-facilitator').fill('Facilitator');
    await page.locator('#btn-create-session').click();
    await page.waitForFunction(() => window.audioTest.events);
  }
  return { page, turns };
}

async function pressMic(page, selector = '#mic-btn') {
  await page.locator(selector).dispatchEvent('mousedown', { button: 0 });
  await page.waitForFunction(() => window.audioTest.messages.includes('start'));
}

async function waitForFinal(page) {
  await page.waitForFunction(() =>
    window.audioTest.messages.includes('stop') &&
    document.getElementById('mic-status-label').textContent.startsWith('Ready'));
  assert.deepEqual(await page.evaluate(() => window.audioTest.tracks.map(t => t.readyState)), ['ended']);
}

test('release outside the mic button finalizes and retains the phrase', async t => {
  const { page, turns } = await openUI(t);
  await pressMic(page);
  await page.locator('body').dispatchEvent('mouseup');
  await waitForFinal(page);
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
  assert.equal(turns.length, 0);
});

test('auto-send without a session retains dictated text in the composer', async t => {
  const { page, turns } = await openUI(t);
  await pressMic(page);
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await waitForFinal(page);
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
  assert.equal(await page.locator('#draft-mode-badge').textContent(), 'Mode: Spoken (Dictated)');
  assert.equal(turns.length, 0);
});

test('review mode retains dictated text without submitting', async t => {
  const { page, turns } = await openUI(t, { session: true });
  await page.locator('#auto-submit-toggle').uncheck();
  await pressMic(page);
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await waitForFinal(page);
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
  assert.equal(turns.length, 0);
  assert.deepEqual(await page.evaluate(() => window.audioTest.messages.filter(m => ['pcm', 'stop'].includes(m))), ['pcm', 'stop']);
});

test('auto-send submits exactly one dictated turn and renders the committed phrase', async t => {
  const { page, turns } = await openUI(t, { session: true });
  await pressMic(page);
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await page.locator('body').dispatchEvent('mouseup');
  await waitForFinal(page);
  await page.waitForFunction(() => !document.getElementById('draft-box').value);
  assert.deepEqual(turns, [{ role: 'stakeholder', text: phrase, input_mode: 'dictated' }]);
  await page.evaluate(turn => window.audioTest.events.listeners.turn_committed({
    data: JSON.stringify({ data: { number: 1, ...turn } }),
  }), turns[0]);
  assert.equal(await page.locator('.turn-text').textContent(), phrase);
});

test('failed auto-send keeps the recognized phrase for retry', async t => {
  const { page } = await openUI(t, { session: true, submitFails: true });
  await pressMic(page);
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await waitForFinal(page);
  await page.waitForFunction(() => document.getElementById('toast-msg').textContent.includes('Test submission failure'));
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
  assert.equal(await page.locator('#btn-submit-turn').isEnabled(), true);
});

for (const pending of ['pendingPermission', 'pendingModule']) {
  test(`release during ${pending} is honored after initialization`, async t => {
    const { page } = await openUI(t, { [pending]: true });
    await page.locator('#mic-btn').dispatchEvent('mousedown', { button: 0 });
    const ready = pending === 'pendingPermission' ? 'grantPermission' : 'loadModule';
    await page.waitForFunction(ready => Boolean(window.audioTest[ready]), ready);
    await page.locator('#mic-btn').dispatchEvent('mouseup');
    await page.evaluate(ready => window.audioTest[ready](), ready);
    await waitForFinal(page);
    assert.equal(await page.locator('#draft-box').inputValue(), phrase);
  });
}

test('a suspended audio context is resumed before recording', async t => {
  const { page } = await openUI(t, { suspended: true });
  await pressMic(page);
  assert.equal(await page.evaluate(() => window.audioTest.contexts[0].state), 'running');
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await waitForFinal(page);
});

test('topic mic release outside the button still delivers the topic', async t => {
  const { page } = await openUI(t);
  await pressMic(page, '#mic-topic-btn');
  await page.locator('body').dispatchEvent('mouseup');
  await waitForFinal(page);
  assert.equal(await page.locator('#topic-input').inputValue(), phrase);
});

test('keyboard push-to-talk finalizes when the shortcut is released', async t => {
  const { page } = await openUI(t);
  await page.keyboard.down('Control');
  await page.keyboard.down('Alt');
  await page.keyboard.down('v');
  await page.waitForFunction(() => window.audioTest.messages.includes('start'));
  await page.keyboard.up('v');
  await page.keyboard.up('Alt');
  await page.keyboard.up('Control');
  await waitForFinal(page);
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
});

test('flush acknowledgement timeout still stops the worker once', async t => {
  const { page } = await openUI(t, { missingFlushAck: true });
  await page.locator('#auto-submit-toggle').uncheck();
  await pressMic(page);
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await waitForFinal(page);
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
  assert.equal(await page.evaluate(() => window.audioTest.messages.filter(m => m === 'stop').length), 1);
});

test('losing window focus finalizes recording rather than leaving the mic active', async t => {
  const { page } = await openUI(t);
  await pressMic(page);
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await waitForFinal(page);
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
});

test('repeated presses during startup create only one capture stream', async t => {
  const { page } = await openUI(t, { pendingPermission: true });
  await page.locator('#mic-btn').dispatchEvent('mousedown', { button: 0 });
  await page.waitForFunction(() => window.audioTest.grantPermission);
  await page.locator('#mic-topic-btn').dispatchEvent('mousedown', { button: 0 });
  await page.evaluate(() => window.audioTest.grantPermission());
  await page.waitForFunction(() => window.audioTest.messages.includes('start'));
  await page.locator('body').dispatchEvent('mouseup');
  await waitForFinal(page);
  assert.equal(await page.locator('#draft-box').inputValue(), phrase);
  assert.equal(await page.locator('#topic-input').inputValue(), '');
});

test('worklet initialization failure stops the acquired microphone and closes its context', async t => {
  const { page } = await openUI(t, { moduleFails: true });
  await page.locator('#mic-btn').dispatchEvent('mousedown', { button: 0 });
  await page.waitForFunction(() => document.getElementById('toast-msg').textContent.includes('Worklet unavailable'));
  assert.deepEqual(await page.evaluate(() => window.audioTest.tracks.map(t => t.readyState)), ['ended']);
  assert.deepEqual(await page.evaluate(() => window.audioTest.contexts.map(c => c.state)), ['closed']);
  assert.equal(await page.evaluate(() => window.audioTest.messages.includes('start')), false);
});

test('microphone permission failure is visible and does not start the worker', async t => {
  const { page } = await openUI(t, { permissionDenied: true });
  await page.locator('#mic-btn').dispatchEvent('mousedown', { button: 0 });
  await page.waitForFunction(() => document.getElementById('toast-msg').textContent.includes('Microphone denied'));
  assert.equal(await page.evaluate(() => window.audioTest.messages.includes('start')), false);
});

test('a second utterance can start after the first is finalized', async t => {
  const { page } = await openUI(t);
  await pressMic(page);
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await waitForFinal(page);
  await pressMic(page);
  await page.waitForFunction(() => window.audioTest.messages.filter(m => m === 'start').length === 2);
  await page.locator('#mic-btn').dispatchEvent('mouseup');
  await page.waitForFunction(() => window.audioTest.messages.filter(m => m === 'stop').length === 2);
  assert.deepEqual(await page.evaluate(() => window.audioTest.tracks.map(t => t.readyState)), ['ended', 'ended']);
});

test('audio worklet flush emits the buffered final syllable before its acknowledgement', () => {
  let Processor;
  const messages = [];
  vm.runInNewContext(fs.readFileSync(path.join(uiDir, 'audio-worklet.js'), 'utf8'), {
    AudioWorkletProcessor: class { constructor() { this.port = { postMessage: message => messages.push(message) }; } },
    registerProcessor: (name, implementation) => { Processor = implementation; },
    Float32Array,
  });
  const processor = new Processor();
  processor.process([[new Float32Array([0.25, 0.5, 0.75])]]);
  assert.equal(messages.length, 0);
  processor.port.onmessage({ data: { type: 'flush' } });
  assert.deepEqual(messages.map(m => m.type), ['pcm', 'flushed']);
  assert.deepEqual(Array.from(messages[0].data), [0.25, 0.5, 0.75]);
  processor.port.onmessage({ data: { type: 'flush' } });
  assert.deepEqual(messages.map(m => m.type), ['pcm', 'flushed', 'flushed']);
});

test('ASR worker drains final audio before publishing the recognized phrase', () => {
  const messages = [];
  const calls = [];
  const stream = {
    acceptWaveform: (rate, samples) => calls.push(['audio', rate, samples.length]),
    inputFinished: () => calls.push(['inputFinished']),
    free: () => calls.push(['free']),
  };
  let ready = false;
  const context = {
    self: { postMessage: message => messages.push(message) },
    performance,
    Float32Array,
    createOnlineRecognizer: () => ({
      createStream: () => stream,
      isReady: () => ready,
      decode: () => { calls.push(['decode']); ready = false; },
      getResult: () => ({ text: phrase }),
    }),
  };
  vm.runInNewContext(fs.readFileSync(path.join(uiDir, 'asr-worker.js'), 'utf8'), context);
  context.Module.onRuntimeInitialized();
  context.self.onmessage({ data: { type: 'start' } });
  context.self.onmessage({ data: { type: 'pcm', data: new Float32Array(1600) } });
  ready = true;
  context.self.onmessage({ data: { type: 'stop' } });
  assert.deepEqual(calls, [
    ['audio', 16000, 1600], ['audio', 16000, 9600], ['inputFinished'], ['decode'], ['free'],
  ]);
  assert.equal(messages.at(-1).type, 'final');
  assert.equal(messages.at(-1).text, phrase);
});
