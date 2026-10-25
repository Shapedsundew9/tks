// AudioWorkletProcessor for mono 16 kHz audio frame streaming (D-131, D-132, §6.3).

class AudioRecorderProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    // Fixed buffer size: 1600 samples = 100 ms at 16 kHz
    this.bufferSize = 1600;
    this.buffer = new Float32Array(this.bufferSize);
    this.bufferIndex = 0;

    this.port.onmessage = (e) => {
      if (e.data && e.data.type === 'flush') {
        if (this.bufferIndex > 0) {
          const tailFrame = this.buffer.slice(0, this.bufferIndex);
          this.port.postMessage({ type: 'pcm', data: tailFrame }, [tailFrame.buffer]);
          this.buffer = new Float32Array(this.bufferSize);
          this.bufferIndex = 0;
        }
        this.port.postMessage({ type: 'flushed' });
      }
    };
  }

  process(inputs, outputs, parameters) {
    const input = inputs[0];
    if (!input || !input[0]) {
      return true;
    }

    const channelData = input[0];
    for (let i = 0; i < channelData.length; i++) {
      this.buffer[this.bufferIndex++] = channelData[i];
      if (this.bufferIndex >= this.bufferSize) {
        // Send a copy and reset index
        const frame = new Float32Array(this.buffer);
        this.port.postMessage({ type: 'pcm', data: frame }, [frame.buffer]);
        this.buffer = new Float32Array(this.bufferSize);
        this.bufferIndex = 0;
      }
    }

    return true;
  }
}

registerProcessor('audio-recorder-processor', AudioRecorderProcessor);
