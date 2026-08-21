const VOWELS = ["aa", "ih", "ou", "ee", "oh"];

const VOWEL_PROFILES = {
  aa: [0.25, 0.6, 1, 0.8, 0.25, 0.1],
  ih: [0.85, 0.25, 0.15, 0.25, 0.7, 1],
  ou: [1, 0.35, 0.25, 0.9, 0.4, 0.1],
  ee: [0.55, 0.8, 0.25, 0.2, 1, 0.55],
  oh: [0.45, 0.9, 1, 0.45, 0.15, 0.05],
};

const FREQUENCY_BANDS = [
  [200, 400],
  [400, 700],
  [700, 1100],
  [1100, 1700],
  [1700, 2500],
  [2500, 3500],
];

const SILENCE_RMS = 0.012;
const FULL_OPEN_RMS = 0.16;
const MAX_OPEN = 0.82;
const VOWEL_CONFIRM_FRAMES = 3;

/** 再生音声から口の開きと母音を近似する。 */
export class LipSyncAnalyzer {
  constructor() {
    this.weights = Object.fromEntries(VOWELS.map((vowel) => [vowel, 0]));
    this.currentVowel = "aa";
    this.candidateVowel = "aa";
    this.candidateFrames = 0;
    this.openValue = 0;
  }

  start(analyser) {
    this.analyser = analyser;
    analyser.fftSize = 2048;
    analyser.smoothingTimeConstant = 0.25;
    this.timeData = new Float32Array(analyser.fftSize);
    this.frequencyData = new Uint8Array(analyser.frequencyBinCount);
    this.sampleRate = analyser.context.sampleRate;
  }

  stop() {
    this.analyser = undefined;
    this.timeData = undefined;
    this.frequencyData = undefined;
    this.openValue = 0;
    this.candidateFrames = 0;
    for (const vowel of VOWELS) this.weights[vowel] = 0;
    return this.weights;
  }

  update(delta) {
    if (!this.analyser || !this.timeData || !this.frequencyData) return this.weights;

    this.analyser.getFloatTimeDomainData(this.timeData);
    const rms = calculateRms(this.timeData);
    const targetOpen = mouthOpening(rms);
    const openRate = targetOpen > this.openValue ? 15 : 26;
    this.openValue = smooth(this.openValue, targetOpen, openRate, delta);

    if (targetOpen > 0.04) {
      this.analyser.getByteFrequencyData(this.frequencyData);
      this.updateVowel(classifyVowel(
        this.frequencyData,
        this.sampleRate,
        this.analyser.fftSize,
      ));
    }

    for (const vowel of VOWELS) {
      const target = vowel === this.currentVowel ? this.openValue : 0;
      const rate = target > this.weights[vowel] ? 18 : 28;
      this.weights[vowel] = smooth(this.weights[vowel], target, rate, delta);
      if (this.weights[vowel] < 0.001) this.weights[vowel] = 0;
    }
    return this.weights;
  }

  updateVowel(vowel) {
    if (vowel === this.candidateVowel) {
      this.candidateFrames += 1;
    } else {
      this.candidateVowel = vowel;
      this.candidateFrames = 1;
    }
    if (this.candidateFrames >= VOWEL_CONFIRM_FRAMES) this.currentVowel = vowel;
  }
}

function calculateRms(samples) {
  let total = 0;
  for (const sample of samples) total += sample * sample;
  return Math.sqrt(total / samples.length);
}

function mouthOpening(rms) {
  const value = clamp((rms - SILENCE_RMS) / (FULL_OPEN_RMS - SILENCE_RMS), 0, 1);
  return value * value * (3 - 2 * value) * MAX_OPEN;
}

function classifyVowel(data, sampleRate, fftSize) {
  const features = FREQUENCY_BANDS.map(([from, to], index) => {
    const tiltCompensation = 1 + index * 0.12;
    return bandEnergy(data, sampleRate, fftSize, from, to) * tiltCompensation;
  });
  const magnitude = Math.hypot(...features) || 1;
  const normalized = features.map((value) => value / magnitude);

  let selected = "aa";
  let bestSimilarity = -Infinity;
  for (const vowel of VOWELS) {
    const profile = VOWEL_PROFILES[vowel];
    const profileMagnitude = Math.hypot(...profile);
    const similarity = normalized.reduce(
      (total, value, index) => total + value * profile[index] / profileMagnitude,
      0,
    );
    if (similarity > bestSimilarity) {
      selected = vowel;
      bestSimilarity = similarity;
    }
  }
  return selected;
}

function bandEnergy(data, sampleRate, fftSize, fromHz, toHz) {
  const binHz = sampleRate / fftSize;
  const start = Math.max(1, Math.floor(fromHz / binHz));
  const end = Math.min(data.length, Math.ceil(toHz / binHz));
  if (end <= start) return 0;

  let total = 0;
  for (let index = start; index < end; index += 1) {
    const amplitude = data[index] / 255;
    total += amplitude * amplitude;
  }
  return Math.sqrt(total / (end - start));
}

function smooth(current, target, rate, delta) {
  const amount = 1 - Math.exp(-rate * Math.max(delta, 0));
  return current + (target - current) * amount;
}

function clamp(value, minimum, maximum) {
  return Math.min(maximum, Math.max(minimum, value));
}
