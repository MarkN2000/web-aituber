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
  };
  elements.layoutForm = { elements: [elements.cameraFov, elements.cameraFovNumber, elements.saveLayout] };
  const requests = [];
  const messages = [];
  const context = vm.createContext({
    elements, token: "test", layoutBusy: false, console,
    config: { camera: { fov: 22.5, position: [1, 2, 3] } },
    adminUrl: (url) => url, readError: async () => "保存失敗",
    setMessage: (_status, _error, text, error) => messages.push({ text, error }),
  });
  context.fetch = async (url, options) => {
    if (options.method === "PUT") requests.push(JSON.parse(options.body));
    return { ok: true, json: async () => context.config };
  };
  vm.runInContext([
    section("function updateLayoutControls", "function updateBrightnessLabel"),
    section("function setVectorInputs", "async function selectImageAsset"),
    section("async function saveModelLayout", "async function uploadImageAsset"),
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
  await context.loadDisplayConfig({ preparation: false, background: false, screenOverlays: false,
    music: false, volume: false, drawing: false, brightness: false, antialias: false });
  assert.equal(elements.cameraFov.value, "22.5");
  assert.equal(elements.cameraFovNumber.value, "22.5");
  await context.saveModelLayout({ preventDefault() {} });
  assert.equal(requests[0].camera_fov, 22.5);
  assert.deepEqual(requests[0].camera_position, [1, 2, 3]);
  assert.equal(elements.cameraFov.disabled, false);
  assert.equal(elements.cameraFovNumber.disabled, false);
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
