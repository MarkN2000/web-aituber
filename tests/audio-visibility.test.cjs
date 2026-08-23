const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

function loadClass(fileName, className, { fetch } = {}) {
  const instances = [];
  const contexts = [];

  class FakeAudio {
    constructor() {
      this.src = "";
      this.currentTime = 0;
      this.muted = false;
      this.paused = true;
      this.playCalls = 0;
      this.pauseCalls = 0;
      this.playHandler = undefined;
      this.listeners = new Map();
      instances.push(this);
    }

    addEventListener(type, listener) {
      this.listeners.set(type, listener);
    }

    play() {
      this.playCalls += 1;
      this.paused = false;
      return this.playHandler ? this.playHandler() : Promise.resolve();
    }

    pause() {
      this.pauseCalls += 1;
      this.paused = true;
    }

    removeAttribute(name) {
      if (name === "src") this.src = "";
    }

    load() {}
  }

  class FakeAudioContext {
    constructor() {
      this.currentTime = 0;
      this.state = "suspended";
      this.resumeCalls = 0;
      this.suspendCalls = 0;
      this.resumeHandler = undefined;
      contexts.push(this);
    }

    resume() {
      this.resumeCalls += 1;
      this.state = "running";
      return this.resumeHandler ? this.resumeHandler() : Promise.resolve();
    }

    suspend() {
      this.suspendCalls += 1;
      this.state = "suspended";
      return Promise.resolve();
    }

    close() {
      this.state = "closed";
      return Promise.resolve();
    }

    createMediaElementSource() {
      return { connect() {}, disconnect() {} };
    }

    createAnalyser() {
      return { fftSize: 0, connect() {}, disconnect() {} };
    }

    createGain() {
      const gain = {
        value: 1,
        cancelScheduledValues() {},
        setValueAtTime(value) { this.value = value; },
        linearRampToValueAtTime(value) { this.value = value; },
      };
      return { gain, connect() {}, disconnect() {} };
    }
  }

  const source = fs.readFileSync(path.join(__dirname, `../web/js/${fileName}`), "utf8")
    .replace(`export class ${className}`, `class ${className}`);
  const context = vm.createContext({
    Audio: FakeAudio,
    URL: {
      createObjectURL: (() => {
        let sequence = 0;
        return () => `blob:test-${sequence += 1}`;
      })(),
      revokeObjectURL() {},
    },
    console,
    fetch,
    window: { AudioContext: FakeAudioContext },
  });
  vm.runInContext(`${source}\nthis.LoadedClass = ${className};`, context);
  return { LoadedClass: context.LoadedClass, instances, contexts };
}

async function flushAsyncWork() {
  await new Promise((resolve) => setImmediate(resolve));
  await Promise.resolve();
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

test("BGMは一時停止位置を保持して再開する", async () => {
  const { LoadedClass: BackgroundMusic, instances } = loadClass("background-music.js", "BackgroundMusic");
  const music = new BackgroundMusic();
  const audio = instances[0];

  await music.resume();
  await music.play("/assets/background-music.webm", 0.3, 0.4);
  audio.currentTime = 12.5;

  await music.pause();

  assert.equal(audio.paused, true);
  assert.equal(audio.currentTime, 12.5);
  assert.equal(audio.src, "/assets/background-music.webm");

  await music.resumePlayback();

  assert.equal(audio.paused, false);
  assert.equal(audio.currentTime, 12.5);
  assert.equal(audio.src, "/assets/background-music.webm");
});

test("BGM設定から音源がなくなった場合は現在の再生を停止する", async () => {
  const { LoadedClass: BackgroundMusic, instances } = loadClass("background-music.js", "BackgroundMusic");
  const music = new BackgroundMusic();
  const audio = instances[0];

  await music.play("/assets/background-music.webm", 0.3, 0.4);
  const playCalls = audio.playCalls;
  await music.play(null, 0.5, 0.2);

  assert.equal(audio.paused, true);
  assert.equal(audio.src, "");
  assert.equal(music.hasTrack, false);
  await music.resumePlayback();
  assert.equal(audio.playCalls, playCalls);
});

test("TTSは再生位置と待機列を保持して再開する", async () => {
  const starts = [];
  const fetch = async () => ({ ok: true, blob: async () => ({}) });
  const { LoadedClass: AudioQueue, instances } = loadClass("audio-queue.js", "AudioQueue", { fetch });
  const queue = new AudioQueue({ onStart: (item) => starts.push(item.sequence) });
  const audio = instances[0];

  await queue.unlock();
  queue.enqueue({ url: "/audio/0", turnId: "turn-1", sequence: 0, durationMs: 1000 });
  queue.enqueue({ url: "/audio/1", turnId: "turn-1", sequence: 1, durationMs: 1000 });
  await flushAsyncWork();
  audio.currentTime = 0.75;

  await queue.suspend();

  assert.equal(audio.paused, true);
  assert.equal(audio.currentTime, 0.75);
  assert.equal(queue.current.sequence, 0);
  assert.equal(queue.pending.size, 1);

  await queue.resume();

  assert.equal(audio.paused, false);
  assert.equal(audio.currentTime, 0.75);
  assert.equal(queue.current.sequence, 0);
  assert.deepEqual(starts, [0]);
});

test("非表示中に別の投稿へ切り替わっても復帰まで再生しない", async () => {
  const fetch = async () => ({ ok: true, blob: async () => ({}) });
  const { LoadedClass: AudioQueue, instances } = loadClass("audio-queue.js", "AudioQueue", { fetch });
  const queue = new AudioQueue();
  const audio = instances[0];

  await queue.unlock();
  queue.enqueue({ url: "/audio/old", turnId: "turn-1", sequence: 0, durationMs: 1000 });
  await flushAsyncWork();
  await queue.suspend();
  const playCallsWhileSuspended = audio.playCalls;

  queue.enqueue({ url: "/audio/new", turnId: "turn-2", sequence: 0, durationMs: 1000 });
  await flushAsyncWork();

  assert.equal(queue.current, null);
  assert.equal(queue.pending.size, 1);
  assert.equal(audio.playCalls, playCallsWhileSuspended);

  await queue.resume();
  await flushAsyncWork();

  assert.equal(queue.current.turnId, "turn-2");
  assert.equal(audio.paused, false);
  assert.equal(audio.playCalls, playCallsWhileSuspended + 1);
});

test("BGMの古い再開処理が非表示へ戻った後に再生しない", async () => {
  const { LoadedClass: BackgroundMusic, instances, contexts } = loadClass("background-music.js", "BackgroundMusic");
  const music = new BackgroundMusic();
  const audio = instances[0];
  const context = contexts[0];
  music.hasTrack = true;
  audio.src = "/assets/background-music.webm";
  music.suspended = true;
  const resume = deferred();
  context.resumeHandler = () => resume.promise;

  const staleResume = music.resumePlayback();
  await music.pause();
  resume.resolve();
  await staleResume;

  assert.equal(music.suspended, true);
  assert.equal(audio.paused, true);
  assert.equal(audio.playCalls, 0);
});

test("非表示へ切り替わった後に完了したplayはBGMを停止状態に戻す", async () => {
  const { LoadedClass: BackgroundMusic, instances } = loadClass("background-music.js", "BackgroundMusic");
  const music = new BackgroundMusic();
  const audio = instances[0];
  music.hasTrack = true;
  music.suspended = true;
  audio.src = "/assets/background-music.webm";
  const play = deferred();
  audio.playHandler = () => play.promise;

  const staleResume = music.resumePlayback();
  await flushAsyncWork();
  assert.equal(audio.playCalls, 1);
  await music.pause();
  play.resolve();
  await staleResume;

  assert.equal(music.suspended, true);
  assert.equal(audio.paused, true);
  assert.equal(audio.playCalls, 1);
});

test("再表示後に古いBGMのplayが完了しても新しい再生を止めない", async () => {
  const { LoadedClass: BackgroundMusic, instances } = loadClass("background-music.js", "BackgroundMusic");
  const music = new BackgroundMusic();
  const audio = instances[0];
  music.hasTrack = true;
  music.suspended = true;
  audio.src = "/assets/background-music.webm";
  const oldPlay = deferred();
  audio.playHandler = () => oldPlay.promise;

  const staleResume = music.resumePlayback();
  await flushAsyncWork();
  await music.pause();
  audio.playHandler = undefined;
  await music.resumePlayback();

  assert.equal(audio.paused, false);

  oldPlay.resolve();
  await staleResume;

  assert.equal(music.suspended, false);
  assert.equal(audio.paused, false);
});

test("TTSの古い再開処理が非表示へ戻った後に再生しない", async () => {
  const fetch = async () => ({ ok: true, blob: async () => ({}) });
  const { LoadedClass: AudioQueue, instances, contexts } = loadClass("audio-queue.js", "AudioQueue", { fetch });
  const queue = new AudioQueue();
  const audio = instances[0];

  await queue.unlock();
  const context = contexts[0];
  queue.enqueue({ url: "/audio/0", turnId: "turn-1", sequence: 0, durationMs: 1000 });
  await flushAsyncWork();
  await queue.suspend();
  const playCallsBeforeResume = audio.playCalls;
  const resume = deferred();
  context.resumeHandler = () => resume.promise;

  const staleResume = queue.resume();
  await queue.suspend();
  resume.resolve();
  await staleResume;

  assert.equal(queue.suspended, true);
  assert.equal(audio.paused, true);
  assert.equal(audio.playCalls, playCallsBeforeResume);
});

test("非表示へ切り替わった後に完了したplayはTTS開始を通知しない", async () => {
  const starts = [];
  const fetch = async () => ({ ok: true, blob: async () => ({}) });
  const { LoadedClass: AudioQueue, instances } = loadClass("audio-queue.js", "AudioQueue", { fetch });
  const queue = new AudioQueue({ onStart: (item) => starts.push(item.sequence) });
  const audio = instances[0];

  await queue.unlock();
  const play = deferred();
  audio.playHandler = () => play.promise;
  queue.enqueue({ url: "/audio/0", turnId: "turn-1", sequence: 0, durationMs: 1000 });
  await flushAsyncWork();
  const suspend = queue.suspend();
  play.resolve();
  await Promise.all([suspend, flushAsyncWork()]);

  assert.deepEqual(starts, []);
  assert.equal(audio.paused, true);

  audio.playHandler = undefined;
  await queue.resume();

  assert.deepEqual(starts, [0]);
  assert.equal(audio.paused, false);
});

test("再表示後に古いTTSのplayが完了しても新しい再生を止めない", async () => {
  const starts = [];
  const fetch = async () => ({ ok: true, blob: async () => ({}) });
  const { LoadedClass: AudioQueue, instances } = loadClass("audio-queue.js", "AudioQueue", { fetch });
  const queue = new AudioQueue({ onStart: (item) => starts.push(item.sequence) });
  const audio = instances[0];

  await queue.unlock();
  const oldPlay = deferred();
  audio.playHandler = () => oldPlay.promise;
  queue.enqueue({ url: "/audio/0", turnId: "turn-1", sequence: 0, durationMs: 1000 });
  await flushAsyncWork();
  await queue.suspend();
  audio.playHandler = undefined;
  await queue.resume();

  assert.deepEqual(starts, [0]);
  assert.equal(audio.paused, false);

  oldPlay.resolve();
  await flushAsyncWork();

  assert.deepEqual(starts, [0]);
  assert.equal(queue.suspended, false);
  assert.equal(audio.paused, false);
});
