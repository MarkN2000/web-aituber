const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/main.js"), "utf8");
const updateSource = source.slice(
  source.indexOf("function updateDebugState"),
  source.indexOf("function resetDebugState"),
);
const connectSource = source.slice(
  source.indexOf("function connect("),
  source.indexOf("function handleServerEvent"),
);

function loadDebugContext() {
  const sockets = [];
  const timers = [];

  class FakeWebSocket {
    constructor(url) {
      this.url = url;
      this.listeners = new Map();
      sockets.push(this);
    }

    addEventListener(type, listener) {
      this.listeners.set(type, listener);
    }

    emit(type, event = {}) {
      this.listeners.get(type)?.(event);
    }

    close() {
      this.emit("close");
    }
  }

  const rendered = [];
  const context = vm.createContext({
    JSON,
    WebSocket: FakeWebSocket,
    console,
    rendered,
    window: {
      location: { protocol: "http:", host: "localhost:3000" },
      clearTimeout() {},
      setTimeout(callback, delay) {
        timers.push({ callback, delay });
        return timers.length;
      },
    },
  });
  vm.runInContext(`
    let debugState = { connection: "connecting" };
    let debugStateKey;
    let socket;
    let reconnectTimer;
    let displayConfigRefreshes = 0;
    let started = true;
    const eventBasePath = "/event/test-event-2026";
    const elements = { debugOverlay: {} };
    const rendered = this.rendered;
    function renderDebugState(_element, state) { rendered.push({ ...state }); }
    function updateDebugMotionControls() {}
    function handleServerEvent() {}
    function showError() {}
    function refreshDisplayConfig() { displayConfigRefreshes += 1; }
    ${updateSource}
    ${connectSource}
    this.connect = connect;
    this.update = updateDebugState;
    this.disableDebug = () => { debugState = undefined; };
    this.displayConfigRefreshes = () => displayConfigRefreshes;
  `, context);
  return { context, rendered, sockets, timers };
}

test("接続状態は接続中・接続済み・再接続中へ遷移する", () => {
  const { context, rendered, sockets, timers } = loadDebugContext();

  context.connect();
  assert.equal(sockets[0].url, "ws://localhost:3000/event/test-event-2026/ws");
  sockets[0].emit("open");
  assert.equal(context.displayConfigRefreshes(), 1);
  sockets[0].emit("close");

  assert.deepEqual(rendered.map((state) => state.connection), [
    "connecting",
    "connected",
    "reconnecting",
  ]);
  assert.equal(timers[0].delay, 2000);

  timers[0].callback();
  assert.equal(sockets.length, 2);
  assert.equal(rendered.at(-1).connection, "reconnecting");
});

test("接続状態とViewer状態を互いに失わず合成する", () => {
  const { context, rendered } = loadDebugContext();

  context.update({ motionFileName: "idle.vrma", expression: "neutral" });
  context.update({ connection: "connected" });

  assert.deepEqual(JSON.parse(JSON.stringify(rendered.at(-1))), {
    connection: "connected",
    motionFileName: "idle.vrma",
    expression: "neutral",
  });
});

test("通常モードではデバッグ状態を描画しない", () => {
  const { context, rendered } = loadDebugContext();
  context.disableDebug();

  context.update({ connection: "connected" });

  assert.deepEqual(rendered, []);
});

test("画面の表示状態ではVRM描画ループを解除しない", () => {
  assert.doesNotMatch(source, /viewer\?\.setRenderingEnabled\(visible\)/);
});

function loadMotionTestContext() {
  const calls = [];
  const elements = Object.fromEntries([
    "debugPanel", "debugEmotion", "debugMotion", "debugMotionPlay", "debugMotionStop", "debugMotionStatus",
    "answer", "loader", "panel", "answerText",
  ].map((key) => [key, { value: "", textContent: "" }]));
  elements.debugEmotion.value = "happy";
  elements.debugMotion.replaceChildren = (...options) => { elements.debugMotion.options = options; };
  const context = vm.createContext({
    elements, calls,
    Option: function (text, value) { return { text, value }; },
    debugEnabled: true, started: true, eventEnded: false, viewerReloading: false,
    currentTurn: undefined,
    displayConfig: { emotion_motions: { happy: ["/a.vrma", "/b.vrma"], sad: ["/missing.vrma"] } },
    viewer: {
      emotionClips: new Map([["happy", [
        { fileName: "a.vrma", url: "/a.vrma" }, { fileName: "b.vrma", url: "/b.vrma" },
      ]]]),
      playEmotionMotion(emotion, options) { calls.push([emotion, options.url, options.preview]); },
      setIdleExpression() { calls.push("neutral"); },
      resumeIdle() { this.motionPreview = false; calls.push("idle"); },
    },
    clearAnswer() {}, applyPendingViewerConfig() {},
  });
  vm.runInContext(source.slice(source.indexOf("function refreshDebugMotionOptions"),
    source.indexOf("const SCREEN_OVERLAY_SLOTS")) + source.slice(source.indexOf("function setTurn"),
    source.indexOf("function setEmotion")), context);
  return { context, elements, calls };
}

test("読み込み済み候補だけを表示し、選択を維持して再生・停止できる", () => {
  const { context, elements, calls } = loadMotionTestContext();
  elements.debugMotion.value = "/b.vrma";
  context.refreshDebugMotionOptions();
  assert.deepEqual(elements.debugMotion.options.map((option) => option.value), ["", "/a.vrma", "/b.vrma"]);
  assert.equal(elements.debugMotion.value, "/b.vrma");
  context.playDebugMotion();
  context.stopDebugMotion();
  assert.deepEqual(calls, [["happy", "/b.vrma", true], "neutral", "idle"]);
  context.viewer.emotionClips.set("happy", [{ fileName: "a.vrma", url: "/a.vrma" }]);
  context.refreshDebugMotionOptions();
  assert.equal(elements.debugMotion.value, "");
});

test("未登録と読み込み失敗を区別し、候補なしでは再生できない", () => {
  const { context, elements, calls } = loadMotionTestContext();
  for (const [emotion, message] of [["neutral", /未登録/], ["sad", /読み込み失敗/]]) {
    elements.debugEmotion.value = emotion;
    context.refreshDebugMotionOptions();
    context.playDebugMotion();
    assert.match(elements.debugMotionStatus.textContent, message);
    assert.equal(elements.debugMotionPlay.disabled, true);
    assert.equal(elements.debugEmotion.disabled, false);
  }
  assert.deepEqual(calls, []);
});

test("通常画面・読み込み中・モデルなし・準備中・投稿処理中・食事中・終了後は操作しない", () => {
  for (const change of [
    (c) => { c.debugEnabled = false; }, (c) => { c.started = false; },
    (c) => { c.viewerReloading = true; }, (c) => { c.viewer = undefined; },
    (c) => { c.displayConfig.preparation_mode = true; },
    (c) => { c.currentTurn = { turn_id: "turn-1" }; },
    (c) => { c.viewer.foodAction = {}; }, (c) => { c.eventEnded = true; },
  ]) {
    const { context, calls } = loadMotionTestContext();
    change(context);
    context.playDebugMotion();
    context.stopDebugMotion();
    assert.deepEqual(calls, []);
  }
});

test("投稿開始でテストを中断し、処理完了後に操作を再び有効にする", () => {
  const { context, elements, calls } = loadMotionTestContext();
  context.viewer.motionPreview = true;
  context.setTurn({ turn_id: "turn-1" });
  assert.equal(context.viewer.motionPreview, false);
  assert.deepEqual(calls, ["idle"]);
  assert.equal(elements.debugMotionPlay.disabled, true);
  context.setTurn(undefined);
  assert.equal(elements.debugMotionPlay.disabled, false);
});
