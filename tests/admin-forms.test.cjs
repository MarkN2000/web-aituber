const assert = require("node:assert/strict");
const { resolveObjectURL } = require("node:buffer");
const { randomUUID } = require("node:crypto");
const { setImmediate: settle } = require("node:timers/promises");
const test = require("node:test");

class Element extends EventTarget {
  value = "";
  textContent = "";
  elements = [];
  children = [];
  fields = new Map();
  dataset = {};
  style = { setProperty() {} };
  classList = { add() {} };
  querySelector(selector) {
    if (!this.fields.has(selector)) this.fields.set(selector, new Element());
    return this.fields.get(selector);
  }
  querySelectorAll() { return []; }
  createElement() { return new Element(); }
  createElementNS() { return new Element(); }
  setAttribute() {}
  getAttribute(name) { return name === "src" ? this.currentUrl ?? null : null; }
  async decode() {}
  setCustomValidity() {}
  reportValidity() { return true; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  add(child) { this.append(child); }
  pause() {}
  load() {}
  getContext() { return { drawImage() {} }; }
  toBlob(resolve, type) { resolve(new Blob(["image"], { type })); }
  set src(url) {
    this.currentUrl = url;
    if (url) queueMicrotask(() => this.onload?.());
  }
  get src() { return this.currentUrl; }
  removeAttribute(name) { if (name === "src") this.currentUrl = undefined; }
}

async function setup(t, token = "test") {
  const document = new Element();
  const field = (id) => document.querySelector("#" + id);
  const overlays = ["top-left", "top-right"].map((slot) => {
    const section = new Element();
    const scaleForm = section.querySelector(".screen-overlay-scale-form");
    section.dataset.screenOverlaySlot = slot;
    return { section, scaleForm, field: (key) => section.querySelector(`[data-screen-overlay-${key}]`),
      scale: scaleForm.querySelector("[data-screen-overlay-scale-number]") };
  });
  document.querySelectorAll = (selector) => selector === ".screen-overlay-slot" ? overlays.map((slot) => slot.section) : [];
  field("model-layout-form").elements = ["camera-fov", "camera-fov-number", "save-model-layout"].map(field);
  const config = {
    camera: { fov: 22.5, position: [1, 2, 3] }, preparation_image_url: null,
    background_image_url: "/bg?v=1", background_music_url: "/music?v=1", venue_map_scale: 75,
    screen_overlays: { top_left: { image_url: "/left?v=1", scale: 50 }, top_right: { image_url: "/right?v=1", scale: 60 } },
  };
  const location = new URL("http://localhost/admin?token=" + token);
  const window = Object.assign(new EventTarget(), { location, confirm: () => true });
  const requests = [];
  const audio = [];
  const globals = {
    document, window, location, WebSocket: EventTarget, Option: Element,
    Image: class extends Element { naturalWidth = 16; naturalHeight = 16; },
    Audio: class extends EventTarget {
      constructor(url) { super(); this.url = url; audio.push(this); }
      play() { return Promise.resolve(); }
      pause() { this.paused = true; }
    },
    fetch: async (url, options = {}) => {
      const path = new URL(url, location).pathname;
      requests.push({ path, ...options });
      const payloads = {
        "/api/admin/display-config": config, "/api/admin/version": { current_version: "1" },
        "/api/admin/config": { llm: { search_fillers: ["test"] }, tts: { engine_url: "http://engine", speaker_id: 1 },
          idle_speech: { entries: [], min_seconds: 1, max_seconds: 2 } },
        "/api/admin/event-access": { public_base_url: "http://localhost", event_identifier: "test" },
        "/api/admin/motions": { files: [], idle_motions: [], emotion_motions: {} },
        "/api/admin/tts-speakers": { speakers: [{ id: 1 }] }, "/api/admin/tts-user-dict": { words: [] },
      };
      return { ok: true, json: async () => payloads[path], blob: async () => new Blob(["wav"]) };
    },
  };
  const previous = Object.getOwnPropertyDescriptors(globalThis);
  for (const [key, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
  }
  t.after(() => {
    for (const key of Object.keys(globals)) {
      if (previous[key]) Object.defineProperty(globalThis, key, previous[key]);
      else delete globalThis[key];
    }
  });
  await import(`../web/js/admin.js?test=${randomUUID()}`);
  await settle();
  const send = async (element, type) => {
    element.dispatchEvent(new Event(type, { cancelable: true }));
    await settle();
  };
  return { field, overlays, config, requests, audio, send };
}

test("画角を初期表示し、スライダーと数値を同期して有効な値を位置と一緒に保存する", async (t) => {
  const { field, requests, send } = await setup(t);
  const slider = field("camera-fov");
  const number = field("camera-fov-number");
  const form = field("model-layout-form");
  assert.equal(slider.value, "22.5");
  slider.value = "20.5";
  await send(slider, "input");
  assert.equal(number.value, "20.5");
  for (const value of ["1", "25.5", "179"]) {
    number.value = value;
    await send(number, "input");
    assert.equal(slider.value, value);
    await send(form, "submit");
    assert.equal(requests.at(-1).path, "/api/admin/model-layout");
    const payload = JSON.parse(requests.at(-1).body);
    assert.equal(payload.camera_fov, Number(value));
    assert.deepEqual(payload.camera_position, [1, 2, 3]);
  }
  const count = requests.length;
  for (const value of ["", "0", "0.9", "179.1", "180", "NaN", "Infinity"]) {
    number.value = value;
    await send(number, "input");
    assert.equal(slider.value, "179");
    await send(form, "submit");
    assert.match(field("vrm-error").textContent, /画角/);
  }
  assert.equal(requests.length, count);
});

test("認証なしでは画角の保存やキャッシュの削除を送信しない", async (t) => {
  const { field, requests, send } = await setup(t, "");
  await send(field("model-layout-form"), "submit");
  await send(field("clear-tts-cache"), "click");
  assert.equal(field("camera-fov").disabled, true);
  assert.equal(field("camera-fov-number").disabled, true);
  assert.equal(requests.length, 0);
});

test("音声キャッシュ削除は二重送信を防ぎ成功と失敗を表示する", async (t) => {
  const { field, send } = await setup(t);
  const button = field("clear-tts-cache");
  const blocked = Promise.withResolvers();
  const fetch = t.mock.method(globalThis, "fetch", () => blocked.promise);
  await send(button, "click");
  assert.equal(button.disabled, true);
  await send(button, "click");
  assert.equal(fetch.mock.callCount(), 1);
  const [url, options] = fetch.mock.calls[0].arguments;
  assert.equal(url, "/api/admin/tts-cache?token=test");
  assert.equal(options.method, "DELETE");
  blocked.resolve({ ok: true });
  await settle();
  assert.match(field("tts-config-status").textContent, /削除しました/);
  assert.equal(button.disabled, false);

  fetch.mock.mockImplementation(async () => ({ ok: false, json: async () => ({ error: "削除失敗" }) }));
  await send(button, "click");
  assert.equal(field("tts-config-error").textContent, "削除失敗");
  assert.equal(button.disabled, false);
});

test("画角の保存中は二重送信を防ぎ、保存に失敗しても編集値を維持する", async (t) => {
  const { field, send } = await setup(t);
  const blocked = Promise.withResolvers();
  const fetch = t.mock.method(globalThis, "fetch", () => blocked.promise);
  t.mock.method(console, "error", () => {});
  field("camera-fov-number").value = "20";
  await send(field("model-layout-form"), "submit");
  assert.equal(field("camera-fov-number").disabled, true);
  await send(field("model-layout-form"), "submit");
  assert.equal(fetch.mock.callCount(), 1);
  blocked.resolve({ ok: false, json: async () => ({ error: "保存失敗" }) });
  await settle();
  assert.equal(field("camera-fov-number").value, "20");
  assert.equal(field("camera-fov-number").disabled, false);
  assert.equal(field("vrm-error").textContent, "保存失敗");
});

test("画像未設定でも準備中にでき、アンチエイリアスの切り替えも個別に保存できる", async (t) => {
  const { field, send, requests } = await setup(t);
  for (const [id, key, value] of [
    ["preparation-mode", "enabled", true],
    ["model-antialias", "antialias", false],
    ["model-antialias", "antialias", true],
  ]) {
    field(id).checked = value;
    await send(field(id + "-form"), "submit");
    const request = requests.at(-1);
    assert.equal(request.path, "/api/admin/" + id);
    assert.equal(request.method, "PUT");
    assert.deepEqual(JSON.parse(request.body), { [key]: value });
  }
});

test("画像・BGMの更新と削除は操作したプレビューだけに反映し、編集中の値を保つ", async (t) => {
  const { field, overlays: [left, right], config, send, requests } = await setup(t);
  assert.equal(left.scale.value, "50");
  assert.equal(field("venue-map-scale-number").value, "75");
  assert.equal(field("venue-pin-size").value, "72");
  const drafts = [
    [field("camera-fov-number"), "55"], [field("drawing-stabilization"), "9"],
    [field("background-music-volume"), "15"], [field("venue-map-scale-number"), "42"], [field("venue-pin-size"), "60"], [left.scale, ""], [right.scale, "91"],
  ];
  for (const [element, value] of drafts) element.value = value;
  field("preparation-mode").checked = true;
  const checkDrafts = () => {
    for (const [element, value] of drafts) assert.equal(element.value, value);
    assert.equal(field("preparation-mode").checked, true);
  };
  config.background_music_url = "/music?v=2";
  config.screen_overlays.top_right = { image_url: "/right?v=2", scale: 100 };
  for (const [input, preview, remove, endpoint] of [
    [field("background-image"), field("current-background-preview"), field("delete-background"), "/api/admin/background-image"],
    [field("venue-map-file"), field("venue-current-map"), field("venue-delete-map"), "/api/admin/venue-map"],
    [field("preparation-image"), field("current-preparation-preview"), field("delete-preparation-image"), "/api/admin/preparation-image"],
    [left.field("input"), left.field("current"), left.field("delete"), "/api/admin/screen-overlays/top-left"],
  ]) {
    const before = requests.length;
    input.files = [new File(["image"], "test.png", { type: "image/png" })];
    await send(input, "change");
    assert.equal(requests.length, before + 1);
    const upload = requests.at(-1);
    assert.equal(upload.path, endpoint);
    assert.equal(upload.method, "POST");
    assert.equal(upload.body.get("image").type, "image/webp");
    const url = preview.src;
    assert.deepEqual(await resolveObjectURL(url).arrayBuffer(), await upload.body.get("image").arrayBuffer());
    assert.equal(preview.hidden, false);
    checkDrafts();
    await send(remove, "click");
    assert.equal(requests.at(-1).path, endpoint);
    assert.equal(requests.at(-1).method, "DELETE");
    assert.equal(preview.hidden, true);
    assert.equal(resolveObjectURL(url), undefined);
    checkDrafts();
  }
  assert.equal(right.field("current").src, "/right?v=1");
  assert.equal(field("current-background-music").src, "/music?v=1");
  field("background-music").files = [new File(["audio"], "music.mp3")];
  await send(field("background-music"), "change");
  await send(field("background-music-form"), "submit");
  assert.equal(field("current-background-music").src, "/music?v=2");
  await send(field("delete-background-music"), "click");
  assert.equal(field("current-background-music").hidden, true);
  checkDrafts();
  await send(right.scaleForm, "submit");
  assert.equal(requests.at(-1).path, "/api/admin/screen-overlays/top-right/scale");
  assert.deepEqual(JSON.parse(requests.at(-1).body), { scale: 91 });
  await send(field("venue-map-scale-form"), "submit");
  assert.equal(requests.at(-1).path, "/api/admin/venue-map/scale");
  assert.deepEqual(JSON.parse(requests.at(-1).body), { scale: 42, pin_size: 60 });
});

test("画像の自動保存に失敗しても元の画像を表示し、同じファイルを再選択できる", async (t) => {
  const { field, send, requests } = await setup(t);
  const input = field("background-image");
  const preview = field("current-background-preview");
  const original = preview.src;
  t.mock.method(console, "error", () => {});
  const before = requests.length;
  input.files = [new File(["text"], "bad.txt", { type: "text/plain" })];
  await send(input, "change");
  assert.equal(requests.length, before);
  assert.match(field("display-config-error").textContent, /WebP/);
  assert.equal(preview.src, original);

  const pending = Promise.withResolvers();
  const fetch = t.mock.method(globalThis, "fetch", () => pending.promise);
  const file = new File(["image"], "retry.png", { type: "image/png" });
  const selectFile = () => {
    input.files = [file];
    input.value = file.name;
    return send(input, "change");
  };
  await selectFile();
  assert.equal(input.disabled, true);
  await send(input, "change");
  assert.equal(fetch.mock.callCount(), 1);
  pending.resolve({ ok: false, json: async () => ({ error: "保存失敗" }) });
  await settle();
  assert.equal(field("display-config-error").textContent, "保存失敗");
  assert.equal(preview.src, original);
  assert.equal(preview.hidden, false);
  assert.equal(input.value, "");
  assert.equal(input.disabled, false);

  fetch.mock.mockImplementation(async () => ({ ok: true }));
  await selectFile();
  assert.notEqual(preview.src, original);
  assert.equal(field("display-config-error").textContent, "");
});

test("試聴を切り替えると古い音声と通信を解放し、前の終了通知は次の試聴を止めない", async (t) => {
  const { field, send, requests, audio } = await setup(t);
  const revoke = t.mock.method(URL, "revokeObjectURL");
  await send(field("load-tts-user-dictionary"), "click");
  field("tts-user-dictionary-pronunciation").value = "テスト";
  field("tts-user-dictionary-accent-type").value = "1";
  await send(field("preview-tts-user-dictionary-word"), "click");
  assert.deepEqual(JSON.parse(requests.at(-1).body), {
    tts: { engine_url: "http://engine", speaker_id: 1 }, pronunciation: "テスト", accent_type: 1,
  });
  const dictionary = audio[0];
  const signal = requests.at(-1).signal;
  await send(field("preview-tts"), "click");
  assert.equal(signal.aborted, true);
  assert.equal(dictionary.paused, true);
  assert.equal(revoke.mock.calls[0].arguments[0], dictionary.url);
  assert.equal(requests.at(-1).path, "/api/admin/tts-preview");
  dictionary.dispatchEvent(new Event("ended"));
  dictionary.dispatchEvent(new Event("error"));
  await send(field("cancel-tts-user-dictionary-word"), "click");
  assert.equal(audio[1].paused, undefined);
  assert.equal(field("tts-config-error").textContent, "");
  audio[1].dispatchEvent(new Event("error"));
  assert.equal(audio[1].paused, true);
  assert.equal(revoke.mock.calls[1].arguments[0], audio[1].url);
  assert.match(field("tts-config-error").textContent, /再生できません/);

  const blocked = Promise.withResolvers();
  const originalFetch = globalThis.fetch;
  let pendingSignal;
  t.mock.method(globalThis, "fetch", (url, options) => {
    if (url.startsWith("/api/admin/tts-user-dict-preview?")) {
      pendingSignal = options.signal;
      return blocked.promise;
    }
    return originalFetch(url, options);
  });
  await send(field("preview-tts-user-dictionary-word"), "click");
  await send(field("preview-tts"), "click");
  assert.equal(pendingSignal.aborted, true);
  blocked.resolve({ ok: true, blob: async () => new Blob(["old audio"]) });
  await settle();
  assert.equal(audio.length, 3);
  assert.equal(audio[2].paused, undefined);
});
