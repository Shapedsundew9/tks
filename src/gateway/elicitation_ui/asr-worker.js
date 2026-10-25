// WebWorker running Sherpa-ONNX WebAssembly streaming transducer (D-131, §6.3).

var Module = self.Module || {};
var recognizer = null;
var recognizerStream = null;
var isInitialized = false;

Module.setStatus = function (text) {
  self.postMessage({ type: 'status', text });
};

Module.onRuntimeInitialized = function () {
  try {
    recognizer = createOnlineRecognizer(Module);
    const memBytes = (Module.HEAPU8 && Module.HEAPU8.buffer)
      ? Module.HEAPU8.buffer.byteLength
      : (Module.wasmMemory ? Module.wasmMemory.buffer.byteLength : 0);
    isInitialized = true;
    self.postMessage({
      type: 'ready',
      memoryBytes: memBytes,
    });
  } catch (err) {
    self.postMessage({ type: 'error', error: err.toString() });
  }
};

self.onmessage = function (e) {
  const { type } = e.data;

  if (type === 'init') {
    if (isInitialized) {
      return;
    }
    const assetsBase = e.data.assetsBase || '/explorer/asr/v1.13.7/';
    Module.locateFile = function (path, scriptDirectory = '') {
      return assetsBase + path;
    };
    try {
      importScripts(assetsBase + 'sherpa-onnx-asr.js', assetsBase + 'sherpa-onnx-wasm-main-asr.js');
    } catch (err) {
      self.postMessage({ type: 'error', error: 'Failed to import ASR scripts: ' + err.toString() });
    }
  } else if (type === 'start') {
    if (!recognizer) {
      self.postMessage({ type: 'error', error: 'Recognizer not ready' });
      return;
    }
    if (recognizerStream && typeof recognizerStream.free === 'function') {
      recognizerStream.free();
    }
    recognizerStream = recognizer.createStream();
    self.postMessage({ type: 'stream_started' });
  } else if (type === 'pcm') {
    if (!recognizer || !recognizerStream) return;

    const pcmData = e.data.data;
    const sentTime = e.data.sentTime || performance.now();
    const t0 = performance.now();

    recognizerStream.acceptWaveform(16000, pcmData);
    while (recognizer.isReady(recognizerStream)) {
      recognizer.decode(recognizerStream);
    }

    const decodeDuration = performance.now() - t0;
    const text = recognizer.getResult(recognizerStream).text;

    self.postMessage({
      type: 'partial',
      text,
      decodeDuration,
      sentTime,
      receivedTime: performance.now(),
    });
  } else if (type === 'stop') {
    if (!recognizer || !recognizerStream) {
      self.postMessage({ type: 'final', text: '', memoryBytes: 0 });
      return;
    }

    const t0 = performance.now();
    // 600 ms trailing silence flush (9600 samples at 16 kHz) + inputFinished()
    // so streaming transducer right-context lookahead emits all final tokens.
    const silence = new Float32Array(9600);
    recognizerStream.acceptWaveform(16000, silence);
    if (typeof recognizerStream.inputFinished === 'function') {
      recognizerStream.inputFinished();
    }
    while (recognizer.isReady(recognizerStream)) {
      recognizer.decode(recognizerStream);
    }

    const finalText = recognizer.getResult(recognizerStream).text;
    const flushDuration = performance.now() - t0;
    if (typeof recognizerStream.free === 'function') {
      recognizerStream.free();
    }
    recognizerStream = null;

    const memBytes = (Module.HEAPU8 && Module.HEAPU8.buffer)
      ? Module.HEAPU8.buffer.byteLength
      : (Module.wasmMemory ? Module.wasmMemory.buffer.byteLength : 0);

    self.postMessage({
      type: 'final',
      text: finalText,
      flushDuration,
      memoryBytes: memBytes,
    });
  }
};
