const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/main.js"), "utf8");
const keySource = source.slice(
  source.indexOf("function viewerConfigKey"),
  source.indexOf("async function fetchDisplayConfig"),
);
const applySource = source.slice(
  source.indexOf("function applyUpdatedDisplayConfig"),
  source.indexOf("async function refreshDisplayConfig"),
);

function config(overrides = {}) {
  return {
    preparation_mode: false,
    preparation_image_url: null,
    vrm_url: "/assets/model.vrm?v=1",
    antialias: true,
    idle_motions: ["/assets/idle.vrma?v=1"],
    emotion_motions: {},
    food_prop: { position: [0, 0, 0], rotation_degrees: [0, 0, 0], size: 0.2 },
    camera: { fov: 30, position: [0, 1, 3], target: [0, 1, 0] },
    light: { color: "#fff", intensity: 1, position: [1, 2, 3], ambient_intensity: 1, brightness: 1 },
    background_color: "#000",
    background_image_url: null,
    screen_overlays: {
      top_left: { image_url: null, scale: 100 },
      top_right: { image_url: null, scale: 100 },
      bottom_left: { image_url: null, scale: 100 },
      bottom_right: { image_url: null, scale: 100 },
    },
    background_music_url: "/assets/background-music.webm?v=1",
    background_music_volume: 0.3,
    background_music_duck_ratio: 0.4,
    ...overrides,
  };
}

function loadContext(initialConfig) {
  const calls = { backgrounds: [], overlays: [], levels: [], tracks: [], viewerReloads: 0, preparation: [] };
  const context = vm.createContext({ JSON, calls });
  vm.runInContext(`
    ${keySource}
    let displayConfig = ${JSON.stringify(initialConfig)};
    let appliedViewerConfigKey = viewerConfigKey(displayConfig);
    let pendingViewerConfig;
    const calls = this.calls;
    const backgroundMusic = {
      setLevels(volume, ratio) { calls.levels.push([volume, ratio]); },
      play(url, volume, ratio) { calls.tracks.push([url, volume, ratio]); },
    };
    function applyBackground(value) { calls.backgrounds.push(value); }
    function applyScreenOverlays(value) { calls.overlays.push(value); }
    function applyPendingViewerConfig() { calls.viewerReloads += 1; }
    function enterPreparationMode(value) {
      calls.preparation.push(["enter", value.preparation_image_url]);
      appliedViewerConfigKey = undefined;
    }
    function leavePreparationMode() { calls.preparation.push(["leave"]); }
    ${applySource}
    this.apply = applyUpdatedDisplayConfig;
    this.pending = () => pendingViewerConfig;
  `, context);
  return { context, calls };
}

test("BGM音量と背景だけの変更ではモデルを再読み込みしない", () => {
  const initial = config();
  const { context, calls } = loadContext(initial);

  context.apply(config({ background_color: "#123", background_music_volume: 0.6 }));

  assert.deepEqual(JSON.parse(JSON.stringify(calls.levels)), [[0.6, 0.4]]);
  assert.deepEqual(calls.tracks, []);
  assert.equal(calls.overlays.length, 1);
  assert.equal(calls.viewerReloads, 0);
  assert.equal(context.pending(), undefined);
});

test("感情モーションのJSON順序だけが変わってもモデルを再読み込みしない", () => {
  const initial = config({ emotion_motions: { happy: "/happy.vrma", sad: "/sad.vrma" } });
  const { context, calls } = loadContext(initial);

  context.apply(config({ emotion_motions: { sad: "/sad.vrma", happy: "/happy.vrma" } }));

  assert.equal(calls.viewerReloads, 0);
});

test("VRMのURLが変わった場合だけビューアーの再読み込みを予約する", () => {
  const initial = config();
  const { context, calls } = loadContext(initial);
  const changed = config({ vrm_url: "/assets/model.vrm?v=2" });

  context.apply(changed);

  assert.equal(calls.viewerReloads, 1);
  assert.equal(context.pending().vrm_url, changed.vrm_url);
});

test("モデルの明るさが変わるとビューアーの再読み込みを予約する", () => {
  const initial = config();
  const { context, calls } = loadContext(initial);
  const changed = config({ light: { ...initial.light, brightness: 1.5 } });

  context.apply(changed);

  assert.equal(calls.viewerReloads, 1);
  assert.equal(context.pending().light.brightness, 1.5);
});

test("アンチエイリアスが変わるとビューアーの再読み込みを予約する", () => {
  const initial = config();
  const { context, calls } = loadContext(initial);

  context.apply(config({ antialias: false }));

  assert.equal(calls.viewerReloads, 1);
  assert.equal(context.pending().antialias, false);
});

test("CameraとFood Propの配置が変わるとビューアーの再読み込みを予約する", () => {
  const initial = config();
  const { context, calls } = loadContext(initial);
  const changed = config({
    camera: { ...initial.camera, position: [0.1, 1.5, 2.8] },
    food_prop: { ...initial.food_prop, rotation_degrees: [10, 20, 30], size: 0.25 },
  });

  context.apply(changed);

  assert.equal(calls.viewerReloads, 1);
  assert.deepEqual(context.pending().camera.position, [0.1, 1.5, 2.8]);
  assert.deepEqual(context.pending().food_prop.rotation_degrees, [10, 20, 30]);
});

test("準備中モードでは準備中画像へ切り替えてモデルを再読み込みしない", () => {
  const initial = config();
  const { context, calls } = loadContext(initial);

  context.apply(config({
    preparation_mode: true,
    preparation_image_url: "/assets/preparation.webp?v=1",
  }));

  assert.deepEqual(JSON.parse(JSON.stringify(calls.preparation)), [["enter", "/assets/preparation.webp?v=1"]]);
  assert.equal(calls.backgrounds.length, 0);
  assert.equal(calls.overlays.length, 0);
  assert.equal(calls.viewerReloads, 0);
});

test("準備中モードを解除すると通常表示とモデルを復元する", () => {
  const initial = config({
    preparation_mode: true,
    preparation_image_url: "/assets/preparation.webp?v=1",
  });
  const { context, calls } = loadContext(initial);
  context.apply(initial);

  context.apply(config());

  assert.deepEqual(JSON.parse(JSON.stringify(calls.preparation)), [
    ["enter", "/assets/preparation.webp?v=1"],
    ["leave"],
  ]);
  assert.equal(calls.backgrounds.length, 1);
  assert.equal(calls.overlays.length, 1);
  assert.equal(calls.viewerReloads, 1);
});
