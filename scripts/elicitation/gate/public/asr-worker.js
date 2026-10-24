// WebWorker running Sherpa-ONNX WebAssembly streaming transducer (D-131, §6.3).

var Module = self.Module || {};
var recognizer = null;
var recognizerStream = null;

Module.locateFile = function (path, scriptDirectory = '') {
  return scriptDirectory + path;
};

Module.setStatus = function (text) {
  self.postMessage({ type: 'status', text });
};

Module.onRuntimeInitialized = function () {
  try {
    recognizer = createOnlineRecognizer(Module);
    const memBytes = (Module.HEAPU8 && Module.HEAPU8.buffer)
      ? Module.HEAPU8.buffer.byteLength
      : (Module.wasmMemory ? Module.wasmMemory.buffer.byteLength : 0);
    self.postMessage({
      type: 'ready',
      memoryBytes: memBytes,
    });
  } catch (err) {
    self.postMessage({ type: 'error', error: err.toString() });
  }
};

// Import Sherpa-ONNX helper and Emscripten main loader
importScripts('sherpa-onnx-asr.js', 'sherpa-onnx-wasm-main-asr.js');

self.onmessage = function (e) {
  const { type } = e.data;

  if (type === 'start') {
    if (!recognizer) {
      self.postMessage({ type: 'error', error: 'Recognizer not ready' });
      return;
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
      self.postMessage({ type: 'final', text: '' });
      return;
    }

    const t0 = performance.now();
    // 250 ms trailing silence flush (4000 samples at 16 kHz)
    const silence = new Float32Array(4000);
    recognizerStream.acceptWaveform(16000, silence);
    while (recognizer.isReady(recognizerStream)) {
      recognizer.decode(recognizerStream);
    }

    const finalText = recognizer.getResult(recognizerStream).text;
    const flushDuration = performance.now() - t0;
    recognizerStream = null;

    self.postMessage({
      type: 'final',
      text: finalText,
      flushDuration,
    });
  }
};
