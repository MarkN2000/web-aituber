const TRANSITION_SECONDS = 0.25;
const SILENT_WAV = "data:audio/wav;base64,UklGRigAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQQAAACAgICA";

/** 表示端末ごとにBGMをループ再生し、TTS中の音量を制御する。 */
export class BackgroundMusic {
  constructor({ onError = () => {} } = {}) {
    const AudioContextClass = window.AudioContext || window.webkitAudioContext;
    if (!AudioContextClass) throw new Error("このブラウザはBGM再生に対応していません。");

    this.onError = onError;
    this.audio = new Audio();
    this.audio.loop = true;
    this.audio.preload = "auto";
    this.context = new AudioContextClass();
    this.source = this.context.createMediaElementSource(this.audio);
    this.gain = this.context.createGain();
    this.source.connect(this.gain);
    this.gain.connect(this.context.destination);
    this.volume = 0.3;
    this.duckRatio = 0.4;
    this.ducked = false;
    this.transition = undefined;
    this.suspended = false;
    this.hasTrack = false;
    this.operationId = 0;
    this.disposed = false;
    this.gain.gain.value = this.volume;
    this.audio.addEventListener("error", () => this.reportError(new Error("BGMを再生できませんでした。")));
  }

  async resume() {
    if (this.disposed) return Promise.resolve();
    const operationId = this.operationId;
    const contextResume = this.context.resume();
    this.audio.muted = true;
    this.audio.src = SILENT_WAV;
    const mediaResume = this.audio.play();
    try {
      await Promise.all([contextResume, mediaResume]);
    } catch (error) {
      if (this.operationId === operationId && !this.suspended) throw error;
    } finally {
      this.audio.pause();
      this.audio.removeAttribute("src");
      this.audio.load();
      this.audio.muted = false;
    }
    if (this.suspended) await this.context.suspend();
  }

  async play(url, volume, duckRatio) {
    if (this.disposed) return;
    this.setLevels(volume, duckRatio, false);
    if (!url) return;
    this.audio.src = url;
    this.hasTrack = true;
    if (this.suspended) return;
    const operationId = ++this.operationId;
    try {
      await this.context.resume();
      if (this.operationId !== operationId || this.suspended) return;
      await this.audio.play();
      if (this.suspended) this.audio.pause();
    } catch (error) {
      if (this.operationId === operationId && !this.suspended) this.reportError(error);
    }
  }

  async pause() {
    if (this.disposed) return Promise.resolve();
    const operationId = ++this.operationId;
    this.suspended = true;
    this.audio.pause();
    await this.context.suspend();
    if (this.operationId !== operationId && !this.suspended) await this.context.resume();
  }

  async resumePlayback() {
    if (this.disposed) return;
    const operationId = ++this.operationId;
    this.suspended = false;
    if (!this.hasTrack) return;
    try {
      await this.context.resume();
      if (this.operationId !== operationId || this.suspended) return;
      await this.audio.play();
      if (this.suspended) this.audio.pause();
    } catch (error) {
      if (this.operationId === operationId && !this.suspended) this.reportError(error);
    }
  }

  setLevels(volume, duckRatio, transition = true) {
    const parsed = Number(volume);
    this.volume = Number.isFinite(parsed) ? Math.min(1, Math.max(0, parsed)) : 0.3;
    const parsedDuckRatio = Number(duckRatio);
    this.duckRatio = Number.isFinite(parsedDuckRatio) ? Math.min(1, Math.max(0, parsedDuckRatio)) : 0.4;
    this.moveTo(this.volume * (this.ducked ? this.duckRatio : 1), transition);
  }

  setDucked(ducked) {
    if (this.disposed) return;
    this.ducked = Boolean(ducked);
    this.moveTo(this.volume * (this.ducked ? this.duckRatio : 1), true);
  }

  moveTo(target, transition) {
    const now = this.context.currentTime;
    const current = this.currentGain(now);
    this.gain.gain.cancelScheduledValues(now);
    this.gain.gain.setValueAtTime(current, now);
    if (transition) {
      this.gain.gain.linearRampToValueAtTime(target, now + TRANSITION_SECONDS);
      this.transition = { from: current, to: target, startedAt: now, endsAt: now + TRANSITION_SECONDS };
    } else {
      this.gain.gain.setValueAtTime(target, now);
      this.transition = undefined;
    }
  }

  currentGain(now) {
    if (!this.transition) return this.gain.gain.value;
    if (now >= this.transition.endsAt) {
      const value = this.transition.to;
      this.transition = undefined;
      return value;
    }
    const progress = Math.max(0, (now - this.transition.startedAt) / (this.transition.endsAt - this.transition.startedAt));
    return this.transition.from + (this.transition.to - this.transition.from) * progress;
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.suspended = true;
    this.operationId += 1;
    this.audio.pause();
    this.audio.removeAttribute("src");
    this.audio.load();
    this.hasTrack = false;
    this.source.disconnect();
    this.gain.disconnect();
    this.context.close();
  }

  reportError(error) {
    console.error("BGMの再生に失敗しました", error);
    this.onError(error);
  }
}
