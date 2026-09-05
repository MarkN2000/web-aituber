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
  createVRMAnimationClip: (animation) => ({ tracks: animation.tracks || [] }),
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
    reset() { this.resetCount = (this.resetCount || 0) + 1; return this; },
    setLoop(mode, repetitions) { this.loop = [mode, repetitions]; return this; },
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
  viewer.foodMotion = { clip: { duration: 14.44 }, fileName: "eat2.vrma" };
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

test("WebGL Rendererへアンチエイリアス設定を渡す", () => {
  assert.match(source, /this\.antialias = antialias;/);
  assert.match(source, /new THREE\.WebGLRenderer\(\{ canvas, antialias, alpha: true \}\)/);
});

test("アンチエイリアスONではカットアウトマテリアルへalphaToCoverageを適用する", () => {
  const viewer = createViewer();
  const cutout = { alphaTest: 0.5, alphaToCoverage: false };
  const opaque = { alphaTest: 0, alphaToCoverage: false };
  viewer.antialias = true;
  viewer.vrm = {
    scene: {
      traverse(callback) {
        callback({ material: [cutout, opaque] });
        callback({});
      },
    },
  };

  viewer.configureMaterialAntialiasing();

  assert.equal(cutout.alphaToCoverage, true);
  assert.equal(opaque.alphaToCoverage, false);
});

test("アンチエイリアスOFFではカットアウトマテリアルを変更しない", () => {
  const viewer = createViewer();
  const cutout = { alphaTest: 0.5, alphaToCoverage: false };
  viewer.antialias = false;
  viewer.vrm = {
    scene: {
      traverse(callback) {
        callback({ material: cutout });
      },
    },
  };

  viewer.configureMaterialAntialiasing();

  assert.equal(cutout.alphaToCoverage, false);
});

test("モデルをシーンへ追加する前にカットアウト用アンチエイリアスを設定する", () => {
  assert.match(source, /this\.configureMaterialAntialiasing\(\);[\s\S]*?this\.scene\.add\(this\.vrm\.scene\);/);
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

test("描画ループは画面の表示状態では解除せず破棄時だけ停止する", () => {
  assert.match(source, /this\.renderer\.setAnimationLoop\(this\.frame\)/);
  assert.match(source, /dispose\(\) \{[\s\S]*this\.renderer\.setAnimationLoop\(null\)/);
  assert.doesNotMatch(source, /document\.visibilityState/);
  assert.doesNotMatch(source, /setRenderingEnabled/);
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

test("neutral以外の表情中は進行中の瞬きを解除して停止する", () => {
  const viewer = createViewer();
  const applied = [];
  viewer.setExpressionValue = (name, value) => applied.push([name, value]);
  viewer.currentExpression = "happy";
  viewer.blinkTimer = 0;
  viewer.blinkTime = 0.08;

  viewer.updateBlink(1 / 30);
  viewer.updateBlink(1 / 30);

  assert.deepEqual(applied, [["blink", 0]]);
  assert.equal(viewer.blinkTime, 0);
  assert.equal(viewer.blinkTimer, 0);

  viewer.currentExpression = "neutral";
  viewer.updateBlink(0.08);

  assert.deepEqual(applied, [["blink", 0], ["blink", 1]]);
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

test("食事VRMAは身体の回転を残し、移動・表情・視線を適用しない", async () => {
  const viewer = createViewer();
  const tracks = [
    { name: "hips.position" }, { name: "hips.quaternion" },
    { name: "rightHand.quaternion" }, { name: "expression_aa.weight" },
    { name: "lookAtQuaternionProxy.quaternion" },
  ];
  viewer.loader = { loadAsync: async () => ({ userData: { vrmAnimations: [{ tracks }] } }) };
  viewer.vrm = { humanoid: {
    normalizedHumanBones: { hips: { node: { name: "hips" } }, rightHand: { node: { name: "rightHand" } } },
    getNormalizedBoneNode: () => ({ name: "hips" }),
  } };

  const motion = await viewer.loadMotion("/assets/motions/eat2.vrma", true);

  assert.deepEqual(motion.clip.tracks.map((track) => track.name), ["hips.quaternion", "rightHand.quaternion"]);
});

test("食事用URLのモーションを読み込み、失敗した場合は設定エラーを表示する", async () => {
  const viewer = createViewer();
  viewer.foodMotion = undefined;
  const loaded = [];
  const warnings = [];
  viewer.report = (message) => warnings.push(message);
  const motion = { clip: {}, fileName: "eat2.vrma" };
  viewer.loadMotion = async (url, bodyOnly) => { loaded.push([url, bodyOnly]); return motion; };
  const config = { food_motion: { url: "/assets/motions/eat2.vrma" } };

  await viewer.loadMotions(config);

  assert.deepEqual(loaded, [["/assets/motions/eat2.vrma", true]]);
  assert.equal(viewer.foodMotion, motion);
  assert.deepEqual(warnings, []);

  viewer.foodMotion = undefined;
  viewer.loadMotion = async () => { throw new Error("missing"); };
  await viewer.loadMotions(config);
  assert.equal(viewer.foodMotion, undefined);
  assert.match(warnings[0], /食事モーションを読み込めませんでした/);
});

test("食事設定のない更新前の設定でも待機モーションを読み込める", async () => {
  for (const food_motion of [undefined, null]) {
    const viewer = createViewer();
    viewer.foodMotion = undefined;
    const loaded = [];
    viewer.loadMotion = async (url) => { loaded.push(url); return { clip: {}, fileName: "idle.vrma" }; };
    viewer.report = (message) => assert.fail(message);

    await viewer.loadMotions({ idle_motions: ["/idle.vrma"], food_motion });
    viewer.resumeIdle();

    assert.deepEqual(loaded, ["/idle.vrma"]);
    assert.equal(viewer.foodMotion, undefined);
    assert.equal(viewer.currentMotion.kind, "idle");
  }
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

test("neutralの要求はモデルのneutral表情を100%適用する", () => {
  const viewer = createViewer();
  const applied = [];
  viewer.vrm = {
    expressionManager: {
      expressions: [{ expressionName: "neutral" }],
      getExpression: (name) => name === "neutral" ? {} : undefined,
      setValue: (name, value) => applied.push([name, value]),
    },
  };

  viewer.setEmotion("neutral");

  assert.deepEqual(applied, [["neutral", 0], ["neutral", 1]]);
  assert.deepEqual(viewer.debugStates.at(-1), debugState({
    expressionSupport: "supported",
  }));
});

test("待機中は直前の表情を解除してneutral表情を100%適用する", () => {
  const viewer = createViewer();
  const applied = [];
  viewer.vrm = {
    expressionManager: {
      expressions: [{ expressionName: "Happy" }, { expressionName: "Neutral" }],
      getExpression: (name) => ["Happy", "Neutral"].includes(name) ? {} : undefined,
      setValue: (name, value) => applied.push([name, value]),
    },
  };

  viewer.setEmotion("happy");
  viewer.setIdleExpression();

  assert.deepEqual(applied, [["Neutral", 0], ["Happy", 1], ["Happy", 0], ["Neutral", 1]]);
  assert.deepEqual(viewer.debugStates.at(-1), debugState({
    expressionSupport: "supported",
  }));
});

test("neutral表情がないモデルの待機中は全表情を解除する", () => {
  const viewer = createViewer();
  const applied = [];
  viewer.vrm = {
    expressionManager: {
      expressions: [{ expressionName: "Happy" }],
      getExpression: (name) => name === "Happy" ? {} : undefined,
      setValue: (name, value) => applied.push([name, value]),
    },
  };

  viewer.setEmotion("happy");
  viewer.setIdleExpression();

  assert.deepEqual(applied, [["Happy", 1], ["Happy", 0]]);
  assert.deepEqual(viewer.debugStates.at(-1), debugState());
});

test("モデル読込後は待機用表情を適用する", () => {
  assert.match(source, /this\.setIdleExpression\(\);\s+this\.resumeIdle\(\);/);
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

function configureFoodViewer() {
  const viewer = createViewer();
  viewer.idleClips = [{ clip: {}, fileName: "idle.vrma" }];
  viewer.emotionClips.set("happy", { clip: {}, fileName: "happy.vrma" });
  viewer.foodAnchor = { add() {}, remove() {} };
  viewer.foodPropSize = 0.2;
  viewer.report = () => {};
  viewer.textureLoader = { loadAsync: async () => ({ dispose() {} }) };
  currentTime = 0;
  viewer.resumeIdle();
  return viewer;
}

test("食事は1回再生し、画像消去後も演出終了まで食事姿勢を維持する", async () => {
  const viewer = configureFoodViewer();
  const idle = viewer.currentAction;
  viewer.playFoodAction("/food/test.webp", 3505, 14440);
  await Promise.resolve();
  const food = viewer.currentAction;
  const mesh = viewer.foodMesh;
  assert.equal(viewer.currentMotion.kind, "food");
  assert.equal(viewer.currentMotion.fileName, "eat2.vrma");
  assert.deepEqual(Array.from(food.loop), ["once", 1]);
  assert.equal(idle.transitions.at(-1).next, food);

  currentTime = 3504;
  viewer.updateFoodAction();
  assert.equal(viewer.foodActionState, "displaying");
  currentTime = 3705;
  viewer.updateFoodAction();
  assert.equal(mesh.scaleValue, 0.5);
  currentTime = 3905;
  viewer.updateFoodAction();
  assert.equal(mesh.scaleValue, 0);
  assert.equal(viewer.foodMesh, undefined);
  assert.equal(viewer.currentAction, food);

  viewer.resumeIdle();
  viewer.playEmotionMotion("happy");
  viewer.onAnimationFinished({ action: food });
  assert.equal(viewer.currentAction, food);

  currentTime = 14440;
  viewer.updateFoodAction();
  assert.equal(viewer.foodAction, undefined);
  assert.equal(viewer.currentMotion.kind, "idle");
  assert.equal(food.transitions.at(-1).next, viewer.currentAction);
});

test("食事中断は待機へ戻し、次の食事は先頭から再生して遅延画像を破棄する", async () => {
  const viewer = configureFoodViewer();
  const pending = [];
  viewer.textureLoader = { loadAsync: () => new Promise((resolve) => pending.push(resolve)) };
  viewer.playFoodAction("/food/first.webp", 3505, 14440);
  const food = viewer.currentAction;
  viewer.clearFoodProp();
  assert.equal(viewer.currentMotion.kind, "idle");
  assert.equal(viewer.foodAction, undefined);

  viewer.playFoodAction("/food/second.webp", 3505, 14440);
  assert.equal(viewer.currentAction, food);
  assert.equal(food.resetCount, 2);
  let disposed = false;
  pending[0]({ dispose() { disposed = true; } });
  await Promise.resolve();
  assert.equal(disposed, true);
  assert.equal(viewer.foodMesh, undefined);
  pending[1]({ dispose() {} });
  await Promise.resolve();
  assert.ok(viewer.foodMesh);
});

test("画像消去時刻を過ぎた遅延画像は再表示せず食事モーションを継続する", async () => {
  const viewer = configureFoodViewer();
  let resolveTexture;
  let disposed = false;
  viewer.textureLoader = { loadAsync: () => new Promise((resolve) => { resolveTexture = resolve; }) };
  viewer.playFoodAction("/food/late.webp", 3505, 14440);
  currentTime = 5000;
  resolveTexture({ dispose() { disposed = true; } });
  await Promise.resolve();
  assert.equal(disposed, true);
  assert.equal(viewer.foodMesh, undefined);
  assert.equal(viewer.currentMotion.kind, "food");
});

test("食事VRMAが未読込なら画像だけの食事演出を開始しない", () => {
  const viewer = configureFoodViewer();
  viewer.foodMotion = undefined;
  const warnings = [];
  viewer.report = (message) => warnings.push(message);
  viewer.textureLoader = { loadAsync() { throw new Error("画像だけの演出を開始しました"); } };
  viewer.playFoodAction("/food/test.webp", 3505, 14440);
  assert.equal(viewer.foodAction, undefined);
  assert.match(warnings[0], /character.food_motion.url/);
});
