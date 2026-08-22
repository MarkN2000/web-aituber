/**
 * 表示端末ごとに音声を順番に再生するキュー。
 * `unlock()` はユーザー操作から呼び出す。
 */
const SILENT_WAV = "data:audio/wav;base64,UklGRigAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQQAAACAgICA";

export class AudioQueue {
  constructor({ onStart = () => {}, onEnd = () => {}, onError = () => {} } = {}) {
    this.onStart = onStart;
    this.onEnd = onEnd;
    this.onError = onError;
    this.audio = new Audio();
    this.audio.preload = "auto";
    this.audio.addEventListener("ended", () => this.finishCurrent());
    this.audio.addEventListener("error", () => {
      if (this.current) {
        this.reportError(this.current, new Error("音声を再生できませんでした。"));
        this.finishCurrent();
      }
    });
    this.context = null;
    this.analyser = null;
    this.source = null;
    this.unlocked = false;
    this.pending = new Map();
    this.activeTurnId = null;
    this.nextSequence = null;
    this.current = null;
    this.playbackId = 0;
    this.suspended = false;
    this.operationId = 0;
    this.disposed = false;
  }

  async unlock() {
    if (this.disposed) throw new Error("AudioQueue は破棄されています。");
    const operationId = this.operationId;

    if (!this.context) {
      const AudioContextClass = window.AudioContext || window.webkitAudioContext;
      if (!AudioContextClass) throw new Error("このブラウザは音声再生に対応していません。");
      this.context = new AudioContextClass();
      this.source = this.context.createMediaElementSource(this.audio);
      this.analyser = this.context.createAnalyser();
      this.analyser.fftSize = 512;
      this.source.connect(this.analyser);
      this.analyser.connect(this.context.destination);
    }

    await this.context.resume();
    this.audio.muted = true;
    this.audio.src = SILENT_WAV;
    try {
      await this.audio.play();
    } catch (error) {
      if (this.operationId === operationId && !this.suspended) throw error;
    } finally {
      this.audio.pause();
      this.audio.removeAttribute("src");
      this.audio.load();
      this.audio.muted = false;
    }
    this.unlocked = true;
    if (this.suspended) {
      await this.context.suspend();
    } else {
      this.playNext();
    }
  }

  async suspend() {
    if (this.disposed) return;
    const operationId = ++this.operationId;
    this.suspended = true;
    this.audio.pause();
    if (this.context) await this.context.suspend();
    if (this.context && this.operationId !== operationId && !this.suspended) await this.context.resume();
  }

  async resume() {
    if (this.disposed) return;
    const operationId = ++this.operationId;
    this.suspended = false;
    if (!this.unlocked || !this.context) return;
    await this.context.resume();
    if (this.operationId !== operationId || this.suspended) return;
    const item = this.current;
    if (!item) {
      this.playNext();
      return;
    }
    const playbackId = this.playbackId;
    try {
      await this.audio.play();
      if (this.operationId !== operationId || this.suspended) {
        if (this.suspended) this.audio.pause();
        return;
      }
      this.notifyStart(item, playbackId);
    } catch (error) {
      if (this.current !== item || this.playbackId !== playbackId) return;
      if (this.operationId !== operationId || this.suspended) return;
      this.reportError(item, error);
      this.finishCurrent();
    }
  }

  enqueue({ url, turnId, sequence, durationMs, meta = {} }) {
    if (this.disposed || !url || !turnId || !Number.isInteger(sequence)) return;
    this.activateTurn(turnId);
    const key = this.itemKey(turnId, sequence);
    if (this.pending.has(key) || this.current?.key === key) return;

    const item = { key, url, turnId, sequence, durationMs, meta, objectUrl: null, ready: false, failed: false, started: false };
    this.pending.set(key, item);
    if (this.nextSequence === null) this.nextSequence = sequence;
    this.fetchAudio(item);
  }

  async fetchAudio(item) {
    try {
      const response = await fetch(item.url);
      if (!response.ok) throw new Error(`音声の取得に失敗しました (${response.status})。`);
      const blob = await response.blob();
      if (this.pending.get(item.key) !== item) return;
      item.objectUrl = URL.createObjectURL(blob);
      item.ready = true;
      this.playNext();
    } catch (error) {
      if (this.pending.get(item.key) !== item) return;
      item.failed = true;
      this.reportError(item, error);
      this.playNext();
    }
  }

  cancelTurn(turnId) {
    if (!turnId) return;
    for (const [key, item] of this.pending) {
      if (item.turnId === turnId) this.removePending(key);
    }
    if (this.current?.turnId === turnId) this.stopCurrent();
    if (this.activeTurnId === turnId) {
      this.activeTurnId = null;
      this.resetNextSequence();
    }
    this.playNext();
  }

  clear() {
    for (const key of this.pending.keys()) this.removePending(key);
    this.stopCurrent();
    this.activeTurnId = null;
    this.nextSequence = null;
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.suspended = true;
    this.operationId += 1;
    this.clear();
    this.source?.disconnect();
    this.analyser?.disconnect();
    this.context?.close();
  }

  playNext() {
    if (!this.unlocked || this.suspended || this.current || this.nextSequence === null) return;
    const item = this.pending.get(this.itemKey(this.activeTurnId, this.nextSequence));
    if (!item) return;
    if (item.failed) {
      this.removePending(item.key);
      this.nextSequence += 1;
      this.onEnd(item);
      this.playNext();
      return;
    }
    if (!item.ready) return;

    this.pending.delete(item.key);
    this.current = item;
    const playbackId = ++this.playbackId;
    const operationId = this.operationId;
    this.audio.src = item.objectUrl;
    this.audio.currentTime = 0;
    this.audio.play()
      .then(() => {
        if (this.operationId !== operationId || this.suspended) {
          if (this.suspended) this.audio.pause();
          return;
        }
        this.notifyStart(item, playbackId);
      })
      .catch((error) => {
        if (this.current !== item || this.playbackId !== playbackId) return;
        if (this.operationId !== operationId || this.suspended) return;
        this.reportError(item, error);
        this.finishCurrent();
      });
  }

  notifyStart(item, playbackId) {
    if (this.current !== item || this.playbackId !== playbackId || item.started) return;
    item.started = true;
    this.onStart(item, this.analyser);
  }

  finishCurrent() {
    const item = this.current;
    if (!item) return;
    this.current = null;
    this.audio.removeAttribute("src");
    this.audio.load();
    this.revoke(item);
    this.nextSequence = item.sequence + 1;
    this.onEnd(item);
    this.playNext();
  }

  stopCurrent() {
    const item = this.current;
    if (!item) return;
    this.playbackId += 1;
    this.current = null;
    this.audio.pause();
    this.audio.removeAttribute("src");
    this.audio.load();
    this.revoke(item);
  }

  removePending(key) {
    const item = this.pending.get(key);
    if (!item) return;
    this.pending.delete(key);
    this.revoke(item);
  }

  resetNextSequence() {
    const sequences = [...this.pending.values()]
      .filter((item) => item.turnId === this.activeTurnId)
      .map((item) => item.sequence);
    this.nextSequence = sequences.length ? Math.min(...sequences) : null;
  }

  activateTurn(turnId) {
    if (this.activeTurnId === turnId) return;
    for (const [key, item] of this.pending) {
      if (item.turnId !== turnId) this.removePending(key);
    }
    if (this.current?.turnId !== turnId) this.stopCurrent();
    this.activeTurnId = turnId;
    this.nextSequence = null;
  }

  itemKey(turnId, sequence) {
    return `${turnId}:${sequence}`;
  }

  revoke(item) {
    if (item.objectUrl) URL.revokeObjectURL(item.objectUrl);
  }

  reportError(item, error) {
    console.error(error);
    this.onError(item, error);
  }
}
