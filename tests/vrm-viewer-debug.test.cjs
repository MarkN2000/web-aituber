const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/vrm-viewer.js"), "utf8")
  .replace(/^import .*;\r?\n/gm, "")
  .replace("export class VrmViewer", "class VrmViewer");
const context = vm.createContext({
  THREE: { LoopOnce: "once" },
  createVRMAnimationClip: () => ({ tracks: [] }),
  isEmotion: (value) => ["neutral", "happy", "sad", "angry", "surprised"].includes(value),
  motionFileName: (url) => new URL(url, "http://localhost/").pathname.split("/").pop(),
  console,
});
vm.runInContext(`${source}\nthis.VrmViewer = VrmViewer;`, context);

function createAction() {
  return {
    clampWhenFinished: false,
    running: false,
    fadeOut() { return this; },
    reset() { return this; },
    setLoop() { return this; },
    setEffectiveWeight() { return this; },
    fadeIn() { return this; },
    play() { this.running = true; return this; },
    isRunning() { return this.running; },
  };
}

function createViewer() {
  const viewer = Object.create(context.VrmViewer.prototype);
  const actions = new Map();
  viewer.mixer = {
    clipAction(clip) {
      if (!actions.has(clip)) actions.set(clip, createAction());
      return actions.get(clip);
    },
  };
  viewer.currentAction = undefined;
  viewer.currentMotion = undefined;
  viewer.currentExpression = "neutral";
  viewer.lastDebugStateKey = undefined;
  viewer.idleClips = [];
  viewer.emotionClips = new Map();
  viewer.debugStates = [];
  viewer.onDebugStateChange = (state) => viewer.debugStates.push({ ...state });
  return viewer;
}

test("読み込んだモーションはクリップとファイル名を保持する", async () => {
  const viewer = createViewer();
  viewer.loader = {
    loadAsync: async () => ({ userData: { vrmAnimations: [{}] } }),
  };
  viewer.vrm = {};

  const motion = await viewer.loadMotion("/assets/motions/idle.vrma?v=1");

  assert.equal(motion.fileName, "idle.vrma");
  assert.deepEqual(motion.clip.tracks, []);
});

test("実際に選択した待機・感情モーションを状態変更時だけ通知する", () => {
  const viewer = createViewer();
  const idle = { clip: {}, fileName: "idle.vrma" };
  const emotion = { clip: {}, fileName: "happy.vrma" };
  viewer.idleClips = [idle];
  viewer.emotionClips.set("happy", emotion);

  viewer.resumeIdle();
  viewer.resumeIdle();
  viewer.playEmotionMotion("happy");
  viewer.setEmotion("happy");
  viewer.setEmotion("happy");

  assert.deepEqual(viewer.debugStates, [
    { motionFileName: "idle.vrma", motionKind: "idle", expression: "neutral" },
    { motionFileName: "happy.vrma", motionKind: "emotion", expression: "neutral" },
    { motionFileName: "happy.vrma", motionKind: "emotion", expression: "happy" },
  ]);
});

test("感情モーション終了後は実際に選択した待機モーションへ表示を戻す", () => {
  const viewer = createViewer();
  const idle = { clip: {}, fileName: "idle.vrma" };
  const emotion = { clip: {}, fileName: "happy.vrma" };
  viewer.idleClips = [idle];
  viewer.emotionClips.set("happy", emotion);
  viewer.playEmotionMotion("happy");
  const emotionAction = viewer.currentAction;

  viewer.onAnimationFinished({ action: emotionAction });

  assert.deepEqual(viewer.debugStates.at(-1), {
    motionFileName: "idle.vrma",
    motionKind: "idle",
    expression: "neutral",
  });
});

test("再生できる身体モーションがない場合はなしを通知する", () => {
  const viewer = createViewer();

  viewer.resumeIdle();

  assert.deepEqual(viewer.debugStates, [
    { motionFileName: undefined, motionKind: undefined, expression: "neutral" },
  ]);
});

test("不正な表情はneutralとして通知する", () => {
  const viewer = createViewer();

  viewer.setEmotion("unknown");

  assert.deepEqual(viewer.debugStates, [
    { motionFileName: undefined, motionKind: undefined, expression: "neutral" },
  ]);
});
