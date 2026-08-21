const token = new URLSearchParams(window.location.search).get("token");
const elements = {
  tabs: [...document.querySelectorAll('[role="tab"]')],
  status: document.querySelector("#admin-status"), skip: document.querySelector("#skip"), reload: document.querySelector("#reload-config"),
  operationStatus: document.querySelector("#operation-status"), operationError: document.querySelector("#operation-error"),
  aiForm: document.querySelector("#ai-config-form"), ttsForm: document.querySelector("#tts-config-form"),
  aiStatus: document.querySelector("#ai-config-status"), aiError: document.querySelector("#ai-config-error"),
  ttsStatus: document.querySelector("#tts-config-status"), ttsError: document.querySelector("#tts-config-error"),
  saveAi: document.querySelector("#save-ai-config"), saveTts: document.querySelector("#save-tts-config"), preview: document.querySelector("#preview-tts"),
  loadSpeakers: document.querySelector("#load-tts-speakers"), speakerList: document.querySelector("#tts-speaker-list"), speakersStatus: document.querySelector("#tts-speakers-status"),
  apiUrl: document.querySelector("#llm-api-url"), model: document.querySelector("#llm-model"), systemPrompt: document.querySelector("#system-prompt"),
  foodPrompt: document.querySelector("#food-reaction-prompt"), fillers: document.querySelector("#search-fillers"),
  engineUrl: document.querySelector("#tts-engine-url"),
};
let currentTurn;
let socket;
let reconnectTimer;
let previewAudio;
let previewAudioUrl;
let loadedConfig;
let selectedSpeakerId;

function adminUrl(path) { return `${path}?token=${encodeURIComponent(token)}`; }
function setStatus(message) { elements.status.textContent = message; }
function setMessage(status, error, message = "", isError = false) { status.textContent = isError ? "" : message; error.textContent = isError ? message : ""; }
function setCurrentTurn(turn) { currentTurn = turn; elements.skip.disabled = !turn; }
function turnStatusLabel(status) { return status === "generating" ? "回答生成中" : status === "eating" ? "食事演出中" : "発話中"; }
function readError(response, fallback) { return response.json().catch(() => ({})).then((body) => body.error || fallback); }

function activateTab(tab, focus = false) {
  elements.tabs.forEach((candidate) => {
    const selected = candidate === tab;
    candidate.setAttribute("aria-selected", String(selected));
    candidate.tabIndex = selected ? 0 : -1;
    document.querySelector(`#${candidate.getAttribute("aria-controls")}`).hidden = !selected;
  });
  if (focus) tab.focus();
}
function tabKeydown(event) {
  const index = elements.tabs.indexOf(event.currentTarget);
  const byKey = { ArrowRight: (index + 1) % 3, ArrowLeft: (index + 2) % 3, Home: 0, End: 2 };
  if (!(event.key in byKey)) return;
  event.preventDefault(); activateTab(elements.tabs[byKey[event.key]], true);
}

function handleServerEvent(event) {
  switch (event.type) {
    case "snapshot":
      setCurrentTurn(event.current);
      setStatus(event.current ? turnStatusLabel(event.current.status) : "待機中");
      break;
    case "state": setCurrentTurn(event.turn); setStatus(turnStatusLabel(event.turn.status)); break;
    case "complete": if (currentTurn?.turn_id === event.turn_id) { setStatus("回答完了"); elements.skip.disabled = true; } break;
    case "cancelled": if (currentTurn?.turn_id === event.turn_id) { setStatus("中断しました"); elements.skip.disabled = true; } break;
    case "error": if (currentTurn?.turn_id === event.turn_id) { setStatus("エラー"); setMessage(elements.operationStatus, elements.operationError, event.message, true); elements.skip.disabled = true; } break;
    case "idle": setCurrentTurn(undefined); setStatus("待機中"); break;
    default: break;
  }
}
function connect() {
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${scheme}//${location.host}/ws`); setStatus("サーバーへ接続中です");
  socket.addEventListener("open", () => setStatus(currentTurn ? "処理中" : "待機中"));
  socket.addEventListener("message", ({ data }) => { try { handleServerEvent(JSON.parse(data)); } catch (error) { console.error(error); setMessage(elements.operationStatus, elements.operationError, "状態を更新できませんでした。", true); } });
  socket.addEventListener("close", () => { setStatus("再接続中"); elements.skip.disabled = true; clearTimeout(reconnectTimer); reconnectTimer = setTimeout(connect, 2000); });
  socket.addEventListener("error", () => socket.close());
}

function hasSelectedSpeaker() { return elements.speakerList.value !== ""; }
function speakerSelectionError() { return "話者一覧を取得して話者を選択してください。"; }
function ttsConfig() { return { engine_url: elements.engineUrl.value.trim(), speaker_id: Number(elements.speakerList.value) }; }
function resetSpeakerList(message = "話者一覧を取得すると、使用する話者を選択できます。") {
  elements.speakerList.replaceChildren(new Option(message, ""));
  elements.speakerList.disabled = true;
  elements.speakersStatus.textContent = message;
}
function speakerLabel(speaker) { return `${speaker.speaker_name} — ${speaker.style_name}`; }
function applySpeakers(speakers, preferredId) {
  elements.speakerList.replaceChildren(new Option("話者を選択してください", ""));
  for (const speaker of speakers) elements.speakerList.add(new Option(speakerLabel(speaker), String(speaker.id)));
  elements.speakerList.disabled = false;
  if (preferredId && speakers.some((speaker) => String(speaker.id) === preferredId)) {
    elements.speakerList.value = preferredId;
    selectedSpeakerId = preferredId;
    elements.speakersStatus.textContent = "話者一覧を取得しました。現在の設定の話者を選択しています。";
  } else {
    selectedSpeakerId = undefined;
    elements.speakersStatus.textContent = "話者一覧を取得しました。使用する話者を選択してください。";
  }
}
function llmConfig() {
  return { api_url: elements.apiUrl.value.trim(), model: elements.model.value.trim(), system_prompt: elements.systemPrompt.value.trim(), food_reaction_prompt: elements.foodPrompt.value.trim(), search_fillers: elements.fillers.value.split(/\r?\n/).map((value) => value.trim()).filter(Boolean) };
}
function configForSave(section) {
  if (!loadedConfig) throw new Error("設定の読み込みが完了していません。");
  return {
    llm: section === "ai" ? llmConfig() : { ...loadedConfig.llm, search_fillers: [...loadedConfig.llm.search_fillers] },
    tts: section === "tts" ? ttsConfig() : { ...loadedConfig.tts },
  };
}
function validate(form) {
  elements.fillers.setCustomValidity(elements.fillers.value.split(/\r?\n/).some((value) => value.trim()) ? "" : "検索中フィラーを1文以上入力してください。");
  return form.reportValidity();
}
function applyConfig(config) {
  elements.apiUrl.value = config.llm.api_url; elements.model.value = config.llm.model; elements.systemPrompt.value = config.llm.system_prompt;
  elements.foodPrompt.value = config.llm.food_reaction_prompt; elements.fillers.value = config.llm.search_fillers.join("\n");
  elements.engineUrl.value = config.tts.engine_url; selectedSpeakerId = String(config.tts.speaker_id);
  resetSpeakerList();
}
async function loadSpeakers() {
  if (!token || !elements.engineUrl.reportValidity()) return;
  const engineUrl = elements.engineUrl.value.trim();
  elements.loadSpeakers.disabled = true; const original = elements.loadSpeakers.textContent; elements.loadSpeakers.textContent = "取得中…";
  setMessage(elements.ttsStatus, elements.ttsError); resetSpeakerList("話者一覧を取得中です。");
  try {
    const response = await fetch(adminUrl("/api/admin/tts-speakers"), { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ engine_url: engineUrl }) });
    if (!response.ok) throw new Error(await readError(response, "話者一覧を取得できませんでした。"));
    const result = await response.json();
    if (!Array.isArray(result.speakers)) throw new Error("話者一覧の応答形式が不正です。");
    if (elements.engineUrl.value.trim() !== engineUrl) return;
    if (result.speakers.length === 0) { resetSpeakerList("利用可能な話者が見つかりませんでした。"); return; }
    applySpeakers(result.speakers, selectedSpeakerId);
  } catch (error) {
    console.error(error);
    if (elements.engineUrl.value.trim() === engineUrl) {
      resetSpeakerList("話者一覧を取得できませんでした。エンジンURLを確認して再取得してください。");
      setMessage(elements.ttsStatus, elements.ttsError, error.message || "話者一覧を取得できませんでした。", true);
    }
  } finally { elements.loadSpeakers.disabled = false; elements.loadSpeakers.textContent = original; }
}
async function loadConfig() {
  try { const response = await fetch(adminUrl("/api/admin/config"), { cache: "no-store" }); if (response.status === 401) throw new Error("管理用トークンを確認してください。"); if (response.status === 404) throw new Error("サーバーが設定画面に対応していません。新しい実行ファイルで再起動してください。"); if (!response.ok) throw new Error(await readError(response, "設定を読み込めませんでした。")); loadedConfig = await response.json(); applyConfig(loadedConfig); await loadSpeakers(); }
  catch (error) { console.error(error); setMessage(elements.aiStatus, elements.aiError, error.message || "設定を読み込めませんでした。", true); setMessage(elements.ttsStatus, elements.ttsError, "音声設定も読み込めませんでした。", true); }
}
async function saveConfig(section, form, button, status, error) {
  if (!token || !validate(form)) return;
  if (section === "tts" && !hasSelectedSpeaker()) { setMessage(status, error, speakerSelectionError(), true); return; }
  button.disabled = true; const original = button.textContent; button.textContent = "保存中…"; setMessage(status, error);
  try { const nextConfig = configForSave(section); const response = await fetch(adminUrl("/api/admin/config"), { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(nextConfig) }); if (!response.ok) throw new Error(await readError(response, "設定を保存できませんでした。")); loadedConfig = nextConfig; setMessage(status, error, "保存しました。次の投稿から反映されます。"); }
  catch (reason) { console.error(reason); setMessage(status, error, reason.message || "設定を保存できませんでした。", true); }
  finally { button.disabled = false; button.textContent = original; }
}
async function skip() {
  if (!currentTurn || !token) return;
  elements.skip.disabled = true; setMessage(elements.operationStatus, elements.operationError, "中断処理中");
  try { const response = await fetch(adminUrl("/api/admin/skip"), { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ turn_id: currentTurn.turn_id }) }); if (!response.ok) throw new Error(await readError(response, "中断操作に失敗しました。")); }
  catch (error) { console.error(error); setMessage(elements.operationStatus, elements.operationError, error.message || "中断操作に失敗しました。", true); elements.skip.disabled = false; }
}
async function reload() {
  if (!token) return;
  elements.reload.disabled = true; const original = elements.reload.textContent; elements.reload.textContent = "再読み込み中…"; setMessage(elements.operationStatus, elements.operationError);
  try { const response = await fetch(adminUrl("/api/admin/reload-config"), { method: "POST" }); const result = await response.json().catch(() => ({})); if (!response.ok) throw new Error(result.error || "設定を再読み込みできませんでした。"); await loadConfig(); setMessage(elements.operationStatus, elements.operationError, result.restart_required ? "ファイルから再読み込みしました。待受アドレスとポートは再起動後に反映されます。" : "ファイルから再読み込みしました。次の投稿から反映されます。"); }
  catch (error) { console.error(error); setMessage(elements.operationStatus, elements.operationError, error.message || "設定を再読み込みできませんでした。", true); }
  finally { elements.reload.disabled = false; elements.reload.textContent = original; }
}
function releasePreview() { previewAudio?.pause(); previewAudio = undefined; if (previewAudioUrl) URL.revokeObjectURL(previewAudioUrl); previewAudioUrl = undefined; }
async function previewTts() {
  if (!token || !validate(elements.ttsForm)) return;
  if (!hasSelectedSpeaker()) { setMessage(elements.ttsStatus, elements.ttsError, speakerSelectionError(), true); return; }
  elements.preview.disabled = true; const original = elements.preview.textContent; elements.preview.textContent = "試聴を準備中…"; setMessage(elements.ttsStatus, elements.ttsError); releasePreview();
  try { const response = await fetch(adminUrl("/api/admin/tts-preview"), { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ tts: ttsConfig() }) }); if (!response.ok) throw new Error(await readError(response, "TTSへ接続できませんでした。")); previewAudioUrl = URL.createObjectURL(await response.blob()); previewAudio = new Audio(previewAudioUrl); previewAudio.addEventListener("ended", releasePreview, { once: true }); previewAudio.addEventListener("error", () => { releasePreview(); setMessage(elements.ttsStatus, elements.ttsError, "試聴音声を再生できませんでした。", true); }, { once: true }); await previewAudio.play(); setMessage(elements.ttsStatus, elements.ttsError, "試聴を再生しています。"); }
  catch (error) { console.error(error); releasePreview(); setMessage(elements.ttsStatus, elements.ttsError, error.message || "TTSへ接続できませんでした。", true); }
  finally { elements.preview.disabled = false; elements.preview.textContent = original; }
}

elements.tabs.forEach((tab) => { tab.addEventListener("click", () => activateTab(tab)); tab.addEventListener("keydown", tabKeydown); });
for (const field of [...elements.aiForm.elements, ...elements.ttsForm.elements]) {
  field.addEventListener("invalid", () => field.setAttribute("aria-invalid", "true"));
  field.addEventListener("input", () => {
    if (field === elements.fillers) elements.fillers.setCustomValidity(elements.fillers.value.split(/\r?\n/).some((value) => value.trim()) ? "" : "検索中フィラーを1文以上入力してください。");
    field.setAttribute("aria-invalid", String(!field.validity.valid));
  });
}
elements.skip.addEventListener("click", skip); elements.reload.addEventListener("click", reload); elements.preview.addEventListener("click", previewTts);
elements.loadSpeakers.addEventListener("click", loadSpeakers);
elements.engineUrl.addEventListener("input", () => {
  selectedSpeakerId = undefined;
  resetSpeakerList("エンジンURLを変更しました。話者一覧を再取得してください。");
});
elements.speakerList.addEventListener("change", () => {
  if (!elements.speakerList.value) {
    selectedSpeakerId = undefined;
    elements.speakersStatus.textContent = "話者を選択してください。";
    return;
  }
  selectedSpeakerId = elements.speakerList.value;
  elements.speakersStatus.textContent = "選択した話者を設定に使用します。";
});
elements.aiForm.addEventListener("submit", (event) => { event.preventDefault(); saveConfig("ai", elements.aiForm, elements.saveAi, elements.aiStatus, elements.aiError); });
elements.ttsForm.addEventListener("submit", (event) => { event.preventDefault(); saveConfig("tts", elements.ttsForm, elements.saveTts, elements.ttsStatus, elements.ttsError); });
if (!token) { setMessage(elements.operationStatus, elements.operationError, "管理用トークンが指定されていません。", true); [...elements.aiForm.elements, ...elements.ttsForm.elements, elements.reload].forEach((element) => { element.disabled = true; }); }
else { connect(); loadConfig(); }
window.addEventListener("beforeunload", () => { clearTimeout(reconnectTimer); socket?.close(); releasePreview(); });
