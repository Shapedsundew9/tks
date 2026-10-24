// EA-0.2 audio utilities: deterministic xorshift64* noise and streaming simulation.

const SR = 16000;
const SEED = 20261023n;

class Rng {
  constructor(seed = SEED) {
    this.state = BigInt(seed);
  }

  nextF64() {
    this.state ^= this.state >> 12n;
    this.state ^= (this.state << 25n) & 0xffffffffffffffffn;
    this.state ^= this.state >> 27n;
    const mult = (this.state * 0x2545f4914f6cdd1dn) & 0xffffffffffffffffn;
    const val = mult >> 11n;
    return Number(val) / 9007199254740992.0; // 1 << 53
  }

  gauss() {
    const u1 = Math.max(this.nextF64(), 1e-15);
    const u2 = this.nextF64();
    return Math.sqrt(-2.0 * Math.log(u1)) * Math.cos(2.0 * Math.PI * u2);
  }
}

function addNoise(pcm, snrDb = 10.0, rng) {
  let sumSq = 0;
  for (let i = 0; i < pcm.length; i++) {
    sumSq += pcm[i] * pcm[i];
  }
  const power = sumSq / Math.max(1, pcm.length);
  const sigma = Math.sqrt(power / Math.pow(10.0, snrDb / 10.0));
  const noisy = new Float32Array(pcm.length);
  for (let i = 0; i < pcm.length; i++) {
    noisy[i] = pcm[i] + rng.gauss() * sigma;
  }
  return noisy;
}

function nonSpeechClips(rng) {
  const n = SR * 10; // 10 seconds each
  const silenceDigital = new Float32Array(n);

  const silenceRoom = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    silenceRoom[i] = rng.gauss() * 3e-4;
  }

  const noiseLoud = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    noiseLoud[i] = rng.gauss() * 0.03;
  }

  const noiseHum = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    noiseHum[i] = 0.025 * Math.sin(2.0 * Math.PI * 50.0 * i / SR);
  }

  return [
    { id: "silence-digital", pcm: silenceDigital },
    { id: "silence-room-noise", pcm: silenceRoom },
    { id: "noise-loud-white", pcm: noiseLoud },
    { id: "noise-mains-hum", pcm: noiseHum },
  ];
}

module.exports = {
  SR,
  SEED,
  Rng,
  addNoise,
  nonSpeechClips,
};
