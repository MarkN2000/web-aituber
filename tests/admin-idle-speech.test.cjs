const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/admin.js"), "utf8");
const functions = source.slice(source.indexOf("function llmConfig"), source.indexOf("async function loadSpeakers"));

test("AI設定で待機発話を保存しTTSだけの保存では未保存の待機入力を送らない", () => {
  const field = (value) => ({ value, setCustomValidity(message) { this.error = message; } });
  const elements = Object.fromEntries(["apiUrl", "model", "systemPrompt", "foodPrompt", "fillers", "engineUrl", "idleMin", "idleMax", "idleEmotion", "idleText"].map((key) => [key, field("")]));
  elements.idleEnabled = { checked: false };
  const loadedConfig = {
    llm: { api_url: "https://example.com", model: "model", system_prompt: "設定", food_reaction_prompt: "食事", search_fillers: ["確認します"] },
    tts: { engine_url: "http://localhost", speaker_id: 1 },
    idle_speech: { enabled: false, min_seconds: 30, max_seconds: 90, emotion: "neutral", text: "ひと休み" },
  };
  const context = vm.createContext({ elements, loadedConfig, resetSpeakerList() {}, ttsConfig: () => loadedConfig.tts });
  vm.runInContext(`let selectedSpeakerId; ${functions}`, context);
  context.applyConfig(loadedConfig);
  assert.equal(elements.idleMin.value, 30);
  elements.idleEnabled.checked = true;
  elements.idleMin.value = "40";
  elements.idleMax.value = "40";
  elements.idleEmotion.value = "happy";
  elements.idleText.value = "  こんにちは  ";
  assert.deepEqual(JSON.parse(JSON.stringify(context.configForSave("ai").idle_speech)), {
    enabled: true, min_seconds: 40, max_seconds: 40, emotion: "happy", text: "こんにちは",
  });
  assert.deepEqual(JSON.parse(JSON.stringify(context.configForSave("tts").idle_speech)), loadedConfig.idle_speech);
  const form = { reportValidity: () => !elements.idleMax.error && !elements.idleText.error };
  assert.equal(context.validate(form), true);
  elements.idleMin.value = "41";
  assert.equal(context.validate(form), false);
  elements.idleMin.value = "40";
  elements.idleText.value = "😀".repeat(300);
  assert.equal(context.validate(form), true);
  elements.idleText.value += "😀";
  assert.equal(context.validate(form), false);
  elements.idleText.value = "   ";
  assert.equal(context.validate(form), false);
});
