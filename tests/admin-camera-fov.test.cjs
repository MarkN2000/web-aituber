const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/admin.js"), "utf8");
function section(start, end) { return source.slice(source.indexOf(start), source.indexOf(end)); }
function setup() {
  const input = (value) => ({ value, listeners: {}, addEventListener(type, fn) { this.listeners[type] = fn; } });
  const elements = {
    cameraFov: input("30"), cameraFovNumber: input("30"),
    cameraPosition: [input("0"), input("1.4"), input("2.5")],
    foodPosition: [input("0"), input("0"), input("0")],
    foodRotation: [input("0"), input("0"), input("0")], foodScale: input("0.2"),
    saveLayout: { textContent: "画角・位置調整を保存" }, vrmStatus: {}, vrmError: {},
    drawingStabilization: {}, drawingStabilizationValue: {}, brightness: {}, brightnessValue: {}, antialias: {},
    musicInput: {}, selectedMusic: {}, uploadMusic: {}, deleteMusic: {}, currentMusicEmpty: {},
    currentMusic: { pause() {}, removeAttribute() { delete this.src; }, load() {} },
    musicVolume: {}, musicDuckRatio: {}, musicVolumeValue: {}, musicDuckRatioValue: {}, saveMusicVolume: {},
    preparationMode: {}, savePreparationMode: {},
  };
  const imageAsset = () => ({
    upload: {}, remove: {}, input: {}, selectedEmpty: {}, selectedPreview: { removeAttribute() {} },
    currentEmpty: {}, currentPreview: {
      removeAttribute() { delete this.url; }, set src(url) { this.url = url; queueMicrotask(() => this.onload()); },
    },
  });
  const backgroundImageAsset = { ...imageAsset(), endpoint: "/api/admin/background-image", fileName: "background.webp" };
  const preparationImageAsset = { ...imageAsset(), preparation: true, endpoint: "/api/admin/preparation-image", fileName: "preparation.webp" };
  const screenOverlays = new Map(["top-left", "top-right"].map((slot) => [slot, {
    ...imageAsset(), slot, key: slot.replace("-", "_"), scale: {}, scaleNumber: {}, saveScale: {},
  }]));
  elements.layoutForm = { elements: [elements.cameraFov, elements.cameraFovNumber, elements.saveLayout] };
  const requests = [];
  const messages = [];
  const context = vm.createContext({
    elements, token: "test", layoutBusy: false, console, FormData, URL, window: { confirm: () => true },
    backgroundImageAsset, preparationImageAsset, screenOverlays, musicBusy: false, selectedMusicFile: null,
    preparationModeBusy: false, preparationModeEnabled: false, currentMusicExists: false,
    config: { camera: { fov: 22.5, position: [1, 2, 3] } },
    adminUrl: (url) => url, readError: async () => "保存失敗",
    setMessage: (_status, _error, text, error) => messages.push({ text, error }),
  });
  context.fetch = async (url, options) => {
    if (options.method === "PUT") requests.push(JSON.parse(options.body));
    return { ok: true, json: async () => context.config };
  };
  vm.runInContext([
    section("function updateDrawingControls", "function handleServerEvent"),
    source.match(/^elements\.cameraFov(?:Number)?\.addEventListener.*$/gm).join("\n"),
  ].join("\n"), context);
  return { context, elements, requests, messages };
}

test("FOVのスライダーと数値を双方向に同期し、不正な数値でスライダーを変更しない", () => {
  const { elements } = setup();
  elements.cameraFov.value = "20.5";
  elements.cameraFov.listeners.input();
  assert.equal(elements.cameraFovNumber.value, "20.5");
  elements.cameraFovNumber.value = "25.5";
  elements.cameraFovNumber.listeners.input();
  assert.equal(elements.cameraFov.value, "25.5");
  for (const invalid of ["", "0", "180", "NaN"]) {
    elements.cameraFovNumber.value = invalid;
    elements.cameraFovNumber.listeners.input();
    assert.equal(elements.cameraFov.value, "25.5");
  }
});

test("表示設定のFOVを読み込み、位置と一緒に保存する", async () => {
  const { context, elements, requests } = setup();
  await context.loadDisplayConfig();
  assert.equal(elements.cameraFov.value, "22.5");
  assert.equal(elements.cameraFovNumber.value, "22.5");
  await context.saveModelLayout({ preventDefault() {} });
  assert.equal(requests[0].camera_fov, 22.5);
  assert.deepEqual(requests[0].camera_position, [1, 2, 3]);
  assert.equal(elements.cameraFov.disabled, false);
  assert.equal(elements.cameraFovNumber.disabled, false);
});

test("画像・BGMの更新と削除は操作したプレビューだけに反映し、編集中の値を保つ", async () => {
  const { context, elements } = setup();
  const left = context.screenOverlays.get("top-left");
  const right = context.screenOverlays.get("top-right");
  Object.assign(context.config, {
    background_image_url: "/bg?v=1", preparation_image_url: "/prep?v=1", background_music_url: "/music?v=1",
    screen_overlays: { top_left: { image_url: "/left?v=1", scale: 50 }, top_right: { image_url: "/right?v=1", scale: 60 } },
  });
  await context.loadDisplayConfig();
  assert.equal(left.scaleNumber.value, "50");
  elements.cameraFovNumber.value = "55";
  elements.drawingStabilization.value = "9";
  elements.preparationMode.checked = true;
  elements.musicVolume.value = "15";
  left.scaleNumber.value = "";
  right.scaleNumber.value = "91";
  const assertDrafts = () => {
    assert.equal(elements.cameraFovNumber.value, "55");
    assert.equal(elements.drawingStabilization.value, "9");
    assert.equal(elements.preparationMode.checked, true);
    assert.equal(elements.musicVolume.value, "15");
    assert.equal(left.scaleNumber.value, "");
    assert.equal(right.scaleNumber.value, "91");
  };
  Object.assign(context.config, {
    background_image_url: "/bg?v=2", preparation_image_url: "/prep?v=2", background_music_url: "/music?v=2",
    screen_overlays: { top_left: { image_url: "/left?v=2", scale: 100 }, top_right: { image_url: "/right?v=2", scale: 100 } },
  });
  const event = { preventDefault() {} };
  for (const [asset, url] of [[context.backgroundImageAsset, "/bg?v=2"], [context.preparationImageAsset, "/prep?v=2"]]) {
    asset.selectedBlob = new Blob(["image"]);
    await context.uploadImageAsset(event, asset);
    assert.equal(asset.currentPreview.url, url);
    assertDrafts();
    await context.deleteImageAsset(asset);
    assert.equal(asset.currentExists, false);
    assertDrafts();
  }
  left.selectedBlob = new Blob(["overlay"]);
  await context.uploadScreenOverlay(event, left);
  assert.equal(left.currentPreview.url, "/left?v=2");
  assert.equal(right.currentPreview.url, "/right?v=1");
  assertDrafts();
  await context.deleteScreenOverlay(left);
  assert.equal(left.currentExists, false);
  assertDrafts();
  assert.equal(elements.currentMusic.src, "/music?v=1");
  context.selectedMusicFile = new File(["audio"], "music.mp3");
  await context.uploadMusic(event);
  assert.equal(elements.currentMusic.src, "/music?v=2");
  assertDrafts();
  await context.deleteMusic();
  assert.equal(elements.currentMusic.hidden, true);
  assertDrafts();
});

test("範囲外や空欄のFOVは送信せず、上下限は保存できる", async () => {
  const { context, elements, requests, messages } = setup();
  for (const value of ["", "0", "0.9", "179.1", "180", "Infinity", "NaN"]) {
    elements.cameraFovNumber.value = value;
    await context.saveModelLayout({ preventDefault() {} });
    assert.equal(messages.at(-1).error, true);
  }
  assert.equal(requests.length, 0);
  for (const value of ["1", "179"]) {
    elements.cameraFovNumber.value = value;
    await context.saveModelLayout({ preventDefault() {} });
    assert.equal(requests.at(-1).camera_fov, Number(value));
  }
});

test("認証なし・保存中はFOVも操作できず、保存失敗でも入力を維持する", async () => {
  const { context, elements, requests, messages } = setup();
  for (const state of [{ token: "", layoutBusy: false }, { token: "test", layoutBusy: true }]) {
    Object.assign(context, state);
    context.updateLayoutControls();
    assert.equal(elements.cameraFov.disabled, true);
    assert.equal(elements.cameraFovNumber.disabled, true);
    await context.saveModelLayout({ preventDefault() {} });
  }
  assert.equal(requests.length, 0);
  Object.assign(context, { token: "test", layoutBusy: false, console: { error() {} } });
  context.fetch = async () => ({ ok: false });
  elements.cameraFovNumber.value = "20";
  await context.saveModelLayout({ preventDefault() {} });
  assert.equal(messages.at(-1).error, true);
  assert.equal(elements.cameraFovNumber.value, "20");
  assert.equal(elements.cameraFovNumber.disabled, false);
});
