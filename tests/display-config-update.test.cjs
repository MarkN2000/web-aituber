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
    vrm_url: "/assets/model.vrm?v=1",
    idle_motions: ["/assets/idle.vrma?v=1"],
    emotion_motions: {},
    food_prop: { position: [0, 0, 0], rotation_degrees: [0, 0, 0], size: 0.2 },
    camera: { fov: 30, position: [0, 1, 3], target: [0, 1, 0] },
    light: { color: "#fff", intensity: 1, position: [1, 2, 3], ambient_intensity: 1 },
    background_color: "#000",
    background_image_url: null,
    background_music_url: "/assets/background-music.webm?v=1",
    background_music_volume: 0.3,
    background_music_duck_ratio: 0.4,
    ...overrides,
  };
}

function loadContext(initialConfig) {
  const calls = { backgrounds: [], levels: [], tracks: [], viewerReloads: 0 };
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
    function applyPendingViewerConfig() { calls.viewerReloads += 1; }
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
