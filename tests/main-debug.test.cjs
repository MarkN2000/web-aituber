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

test("画面の表示状態をVRM描画ループへ反映する", () => {
  assert.match(source, /viewer\?\.setRenderingEnabled\(visible\)/);
});
