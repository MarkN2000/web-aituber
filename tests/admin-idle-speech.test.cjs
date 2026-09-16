const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/admin.js"), "utf8");
const functions = source.slice(source.indexOf("function createIdleSpeechEntry"), source.indexOf("async function loadSpeakers"));
const addListener = source.slice(source.indexOf("elements.addIdleSpeech.addEventListener"), source.indexOf("elements.aiForm.addEventListener"));

test("待機候補を追加・削除・再読込でき各候補を検証し保存する", () => {
  const field = (value = "") => ({
    value, setCustomValidity(message) { this.error = message; },
    setAttribute(name, value) { this[name] = value; },
    addEventListener(type, callback) { this[type] = callback; },
    focus() { this.focused = true; },
  });
  const elements = Object.fromEntries(["apiUrl", "model", "systemPrompt", "foodPrompt", "fillers", "engineUrl", "idleMin", "idleMax", "addIdleSpeech"].map((key) => [key, field()]));
  elements.idleEnabled = { checked: false };
  elements.idleEntries = {
    children: [],
    get childElementCount() { return this.children.length; },
    replaceChildren(...rows) { this.children = rows; },
    append(row) { this.children.push(row); },
    querySelectorAll(selector) { return this.children.map((row) => row.querySelector(selector)); },
  };
  elements.idleTemplate = { content: { firstElementChild: { cloneNode() {
    const fields = Object.fromEntries(["select", "textarea", "[data-idle-number]", "[data-idle-remove]"].map((key) => [key, field()]));
    return { ...field(), querySelector: (key) => fields[key], remove() { elements.idleEntries.children.splice(elements.idleEntries.children.indexOf(this), 1); } };
  } } } };
  const loadedConfig = {
    llm: { api_url: "https://example.com", model: "model", system_prompt: "設定", food_reaction_prompt: "食事", search_fillers: ["確認します"] },
    tts: { engine_url: "http://localhost", speaker_id: 1 },
    idle_speech: { enabled: false, min_seconds: 30, max_seconds: 90, entries: [{ emotion: "neutral", text: "ひと休み" }] },
  };
  const context = vm.createContext({ elements, loadedConfig, resetSpeakerList() {}, ttsConfig: () => loadedConfig.tts });
  vm.runInContext(`let selectedSpeakerId; ${functions}\n${addListener}`, context);
  context.applyConfig(loadedConfig);
  const first = elements.idleEntries.children[0];
  assert.equal(first.querySelector("[data-idle-remove]").disabled, true);
  elements.addIdleSpeech.click();
  const second = elements.idleEntries.children[1];
  assert.equal(second.querySelector("textarea").focused, true);
  assert.equal(first.querySelector("[data-idle-remove]").disabled, false);
  assert.equal(second.querySelector("[data-idle-number]").textContent, "2");
  assert.equal(second.querySelector("textarea")["aria-label"], "セリフ 2（300文字以内）");
  elements.idleEnabled.checked = true;
  elements.idleMin.value = "40";
  elements.idleMax.value = "40";
  second.querySelector("select").value = "happy";
  second.querySelector("textarea").value = "  こんにちは\nお元気ですか  ";
  const saved = JSON.parse(JSON.stringify(context.configForSave("ai").idle_speech));
  assert.deepEqual(saved, {
    enabled: true, min_seconds: 40, max_seconds: 40,
    entries: [{ emotion: "neutral", text: "ひと休み" }, { emotion: "happy", text: "こんにちは\nお元気ですか" }],
  });
  assert.deepEqual(JSON.parse(JSON.stringify(context.configForSave("tts").idle_speech)), loadedConfig.idle_speech);
  const form = { reportValidity: () => !elements.idleMax.error && elements.idleEntries.querySelectorAll("textarea").every((text) => !text.error) };
  assert.equal(context.validate(form), true);
  elements.idleMin.value = "41";
  assert.equal(context.validate(form), false);
  elements.idleMin.value = "40";
  second.querySelector("textarea").value = "😀".repeat(300);
  assert.equal(context.validate(form), true);
  second.querySelector("textarea").value += "😀";
  assert.equal(context.validate(form), false);
  second.querySelector("textarea").value = "   ";
  assert.equal(context.validate(form), false);
  first.querySelector("[data-idle-remove]").click();
  assert.equal(elements.idleEntries.childElementCount, 1);
  assert.equal(second.querySelector("[data-idle-number]").textContent, "1");
  assert.equal(second.querySelector("textarea")["aria-label"], "セリフ 1（300文字以内）");
  assert.equal(second.querySelector("[data-idle-remove]").disabled, true);
  second.querySelector("[data-idle-remove]").click();
  assert.equal(elements.idleEntries.childElementCount, 1);
  context.applyConfig({ ...loadedConfig, idle_speech: saved });
  assert.equal(elements.idleEntries.childElementCount, 2);
  assert.equal(elements.idleEntries.children[1].querySelector("textarea").value, "こんにちは\nお元気ですか");
});
