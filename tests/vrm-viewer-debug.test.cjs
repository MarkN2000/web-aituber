const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/vrm-viewer.js"), "utf8")
  .replace(/^import .*;\r?\n/gm, "")
  .replace("export class VrmViewer", "class VrmViewer");
let currentTime = 0;

class FakeGeometry {
  dispose() {}
}

class FakeMaterial {
  constructor(options) {
    Object.assign(this, options);
  }

  dispose() {}
}

class FakeMesh {
  constructor(geometry, material) {
    this.geometry = geometry;
    this.material = material;
    this.scale = { setScalar: (value) => { this.scaleValue = value; } };
  }
}

class FakeDirectionalLight {
  constructor(color, intensity) {
    this.color = color;
    this.intensity = intensity;
    this.position = { fromArray: (value) => { this.lightPosition = value; } };
  }
}

class FakeAmbientLight {
  constructor(color, intensity) {
    this.color = color;
    this.intensity = intensity;
  }
}

const context = vm.createContext({
  THREE: {
    LoopOnce: "once",
    SRGBColorSpace: "srgb",
    DoubleSide: "double",
    PlaneGeometry: FakeGeometry,
    MeshBasicMaterial: FakeMaterial,
    Mesh: FakeMesh,
    DirectionalLight: FakeDirectionalLight,
    AmbientLight: FakeAmbientLight,
    MathUtils: { clamp: (value, min, max) => Math.min(Math.max(value, min), max) },
  },
  createVRMAnimationClip: () => ({ tracks: [] }),
  isEmotion: (value) => ["neutral", "happy", "sad", "angry", "surprised"].includes(value),
  motionFileName: (url) => new URL(url, "http://localhost/").pathname.split("/").pop(),
  performance: { now: () => currentTime },
  window: { devicePixelRatio: 1, innerWidth: 1280, innerHeight: 720 },
  console: { ...console, error() {} },
});
vm.runInContext(`${source}\nthis.VrmViewer = VrmViewer;`, context);

function createAction() {
  return {
    clampWhenFinished: false,
    running: false,
    transitions: [],
    fadeOut(duration) { this.fadeOutDuration = duration; return this; },
    reset() { return this; },
    setLoop() { return this; },
    setEffectiveWeight() { return this; },
    fadeIn() { return this; },
    crossFadeTo(next, duration, warp) {
      this.transitions.push({ next, duration, warp });
      return this;
    },
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
  viewer.currentExpressionSupport = "base";
  viewer.foodActionId = 0;
  viewer.foodActionState = "none";
  viewer.lastDebugStateKey = undefined;
  viewer.idleClips = [];
  viewer.emotionClips = new Map();
  viewer.debugStates = [];
  viewer.onDebugStateChange = (state) => viewer.debugStates.push({ ...state });
  return viewer;
}

test("明るさ倍率を主光源と環境光の両方へ適用する", () => {
  const viewer = Object.create(context.VrmViewer.prototype);
  const lights = [];
  viewer.camera = {
    position: { fromArray() {} },
    lookAt() {},
    updateProjectionMatrix() {},
  };
  viewer.renderer = { setClearColor() {} };
  viewer.scene = { add: (light) => lights.push(light) };

  viewer.configureScene({
    camera: {},
    light: { color: "#fff", intensity: 1.5, position: [2, 3, 2], ambient_intensity: 0.8, brightness: 1.5 },
  });

  assert.equal(lights[0].intensity, 2.25);
  assert.ok(Math.abs(lights[1].intensity - 1.2) < 1e-10);
});

test("Canvasはリサイズ時の実DPRで描画する", () => {
  const viewer = Object.create(context.VrmViewer.prototype);
  let pixelRatio;
  viewer.canvas = { clientWidth: 800, clientHeight: 600 };
  viewer.camera = { updateProjectionMatrix() {} };
  viewer.renderer = {
    setPixelRatio: (value) => { pixelRatio = value; },
    setSize() {},
  };

  context.window.devicePixelRatio = 1.75;
  viewer.onResize();

  assert.equal(pixelRatio, 1.75);
  assert.equal(viewer.camera.aspect, 4 / 3);
});

test("VRM描画を最大30fpsに制限する", () => {
  const viewer = Object.create(context.VrmViewer.prototype);
  let renders = 0;
  viewer.lastFrameTime = undefined;
  viewer.clock = { getDelta: () => 1 / 30 };
  viewer.mixer = { update() {} };
  viewer.updateBlink = () => {};
  viewer.updateLipSync = () => {};
  viewer.updateFoodAction = () => {};
  viewer.vrm = { update() {} };
  viewer.renderer = { render: () => { renders += 1; } };

  for (const timestamp of [0, 16, 33, 49, 66, 82, 99]) viewer.frame(timestamp);

  assert.equal(renders, 4);
});

test("表示状態に応じて描画ループを開始・停止する", () => {
  const viewer = Object.create(context.VrmViewer.prototype);
  const loops = [];
  let clockResets = 0;
  viewer.renderingEnabled = false;
  viewer.lastFrameTime = 100;
  viewer.frame = () => {};
  viewer.clock = { getDelta: () => { clockResets += 1; } };
  viewer.renderer = { setAnimationLoop: (callback) => loops.push(callback) };

  viewer.setRenderingEnabled(true);
  viewer.setRenderingEnabled(true);
  viewer.setRenderingEnabled(false);

  assert.deepEqual(loops, [viewer.frame, null]);
  assert.equal(clockResets, 2);
  assert.equal(viewer.lastFrameTime, undefined);
});

test("音声停止中は口の表情を毎フレーム更新しない", () => {
  const viewer = Object.create(context.VrmViewer.prototype);
  const applied = [];
  viewer.applyMouthWeights = (weights) => applied.push(weights);
  viewer.lipSync = { update: () => undefined };

  viewer.updateLipSync(1 / 30);
  viewer.lipSync.update = () => ({ aa: 0.5 });
  viewer.updateLipSync(1 / 30);

  assert.deepEqual(applied, [{ aa: 0.5 }]);
});

function debugState(overrides = {}) {
  return {
    motionFileName: undefined,
    motionKind: undefined,
    expression: "neutral",
    expressionSupport: "base",
    foodAction: "none",
    ...overrides,
  };
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
    debugState({ motionFileName: "idle.vrma", motionKind: "idle" }),
    debugState({ motionFileName: "happy.vrma", motionKind: "emotion" }),
    debugState({
      motionFileName: "happy.vrma",
      motionKind: "emotion",
      expression: "happy",
      expressionSupport: "unsupported",
    }),
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

  assert.deepEqual(viewer.debugStates.at(-1), debugState({
    motionFileName: "idle.vrma",
    motionKind: "idle",
  }));
  assert.equal(emotionAction.transitions.length, 1);
  assert.equal(emotionAction.transitions[0].next, viewer.currentAction);
  assert.equal(emotionAction.transitions[0].duration, 0.4);
  assert.equal(emotionAction.transitions[0].warp, false);
});

test("再生できる身体モーションがない場合はなしを通知する", () => {
  const viewer = createViewer();

  viewer.resumeIdle();

  assert.deepEqual(viewer.debugStates, [
    debugState(),
  ]);
});

test("待機モーションがない場合も終了したモーションを滑らかに解除する", () => {
  const viewer = createViewer();
  const emotion = { clip: {}, fileName: "happy.vrma" };
  viewer.emotionClips.set("happy", emotion);
  viewer.playEmotionMotion("happy");
  const emotionAction = viewer.currentAction;

  viewer.onAnimationFinished({ action: emotionAction });

  assert.equal(emotionAction.fadeOutDuration, 0.4);
  assert.equal(viewer.currentAction, undefined);
  assert.deepEqual(viewer.debugStates.at(-1), debugState());
});

test("不正な表情はneutralとして通知する", () => {
  const viewer = createViewer();

  viewer.setEmotion("unknown");

  assert.deepEqual(viewer.debugStates, [
    debugState(),
  ]);
});

test("要求表情に対するVRMの対応状況を通知する", () => {
  const viewer = createViewer();
  viewer.vrm = {
    expressionManager: {
      expressions: [{ expressionName: "Happy" }],
      getExpression: (name) => name === "Happy" ? {} : undefined,
      setValue() {},
    },
  };

  viewer.setEmotion("happy");
  viewer.setEmotion("sad");
  viewer.setEmotion("neutral");

  assert.deepEqual(viewer.debugStates.map((state) => state.expressionSupport), [
    "supported",
    "unsupported",
    "base",
  ]);
});

test("食事動作は画像読込中・表示中・消費中・なしへ遷移する", async () => {
  const viewer = createViewer();
  let resolveTexture;
  viewer.foodAnchor = { add() {}, remove() {} };
  viewer.foodPropSize = 0.2;
  viewer.report = () => {};
  viewer.textureLoader = {
    loadAsync: () => new Promise((resolve) => { resolveTexture = resolve; }),
  };
  currentTime = 0;

  viewer.playFoodAction("/food/test.png", 1000, 3000);
  assert.equal(viewer.debugStates.at(-1).foodAction, "loading");

  resolveTexture({ dispose() {} });
  await Promise.resolve();
  assert.equal(viewer.debugStates.at(-1).foodAction, "displaying");

  currentTime = 1000;
  viewer.updateFoodAction();
  assert.equal(viewer.debugStates.at(-1).foodAction, "consuming");

  currentTime = 3000;
  viewer.updateFoodAction();
  assert.equal(viewer.debugStates.at(-1).foodAction, "none");
});

test("食事画像を読み込めない場合は読込失敗を通知する", async () => {
  const viewer = createViewer();
  viewer.foodAnchor = { add() {}, remove() {} };
  viewer.foodPropSize = 0.2;
  viewer.report = () => {};
  viewer.textureLoader = { loadAsync: async () => { throw new Error("load failed"); } };
  currentTime = 0;

  viewer.playFoodAction("/food/test.png", 1000, 3000);
  await Promise.resolve();
  await Promise.resolve();

  assert.equal(viewer.debugStates.at(-1).foodAction, "failed");
});

test("食事動作終了後に読み込みが完了した画像は表示しない", async () => {
  const viewer = createViewer();
  let resolveTexture;
  let textureDisposed = false;
  viewer.foodAnchor = { add() { throw new Error("終了後の画像を追加しました"); }, remove() {} };
  viewer.foodPropSize = 0.2;
  viewer.report = () => {};
  viewer.textureLoader = {
    loadAsync: () => new Promise((resolve) => { resolveTexture = resolve; }),
  };
  currentTime = 0;

  viewer.playFoodAction("/food/test.png", 1000, 3000);
  currentTime = 3000;
  resolveTexture({ dispose: () => { textureDisposed = true; } });
  await Promise.resolve();

  assert.equal(textureDisposed, true);
  assert.equal(viewer.foodMesh, undefined);
  assert.equal(viewer.debugStates.at(-1).foodAction, "none");
});
