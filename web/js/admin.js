import { UserDictionaryEditor } from "./user-dictionary.js?v=8";

const token = new URLSearchParams(window.location.search).get("token");
const MAX_BACKGROUND_BYTES = 10 * 1024 * 1024;
const MAX_VRM_MODEL_BYTES = 100 * 1024 * 1024;
const MAX_BACKGROUND_MUSIC_BYTES = 100 * 1024 * 1024;
const MAX_BACKGROUND_DIMENSION = 1920;
const BACKGROUND_WEBP_QUALITY = 0.85;
const elements = {
  tabs: [...document.querySelectorAll('[role="tab"]')],
  status: document.querySelector("#admin-status"), skip: document.querySelector("#skip"), reload: document.querySelector("#reload-config"),
  operationStatus: document.querySelector("#operation-status"), operationError: document.querySelector("#operation-error"),
  eventForm: document.querySelector("#event-access-form"), publicBaseUrl: document.querySelector("#public-base-url"), eventIdentifier: document.querySelector("#event-identifier"),
  randomizeEventIdentifier: document.querySelector("#randomize-event-identifier"), saveEventAccess: document.querySelector("#save-event-access"),
  eventMainUrl: document.querySelector("#event-main-url"), eventInputUrl: document.querySelector("#event-input-url"), eventDrawUrl: document.querySelector("#event-draw-url"),
  eventCopyButtons: [...document.querySelectorAll("[data-copy-event-url]")], eventQrButtons: [...document.querySelectorAll("[data-qr-event-url]")],
  eventAccessStatus: document.querySelector("#event-access-status"), eventAccessError: document.querySelector("#event-access-error"),
  eventQrDialog: document.querySelector("#event-qr-dialog"), eventQrTitle: document.querySelector("#event-qr-title"),
  eventQrImage: document.querySelector("#event-qr-image"), eventQrUrl: document.querySelector("#event-qr-url"), eventQrClose: document.querySelector("#event-qr-close"),
  aiForm: document.querySelector("#ai-config-form"), ttsForm: document.querySelector("#tts-config-form"),
  aiStatus: document.querySelector("#ai-config-status"), aiError: document.querySelector("#ai-config-error"),
  clearHistory: document.querySelector("#clear-conversation-history"),
  ttsStatus: document.querySelector("#tts-config-status"), ttsError: document.querySelector("#tts-config-error"),
  saveAi: document.querySelector("#save-ai-config"), saveTts: document.querySelector("#save-tts-config"), preview: document.querySelector("#preview-tts"),
  loadSpeakers: document.querySelector("#load-tts-speakers"), speakerList: document.querySelector("#tts-speaker-list"), speakersStatus: document.querySelector("#tts-speakers-status"),
  apiUrl: document.querySelector("#llm-api-url"), model: document.querySelector("#llm-model"), systemPrompt: document.querySelector("#system-prompt"),
  foodPrompt: document.querySelector("#food-reaction-prompt"), fillers: document.querySelector("#search-fillers"),
  engineUrl: document.querySelector("#tts-engine-url"),
  vrmForm: document.querySelector("#vrm-model-form"), vrmInput: document.querySelector("#vrm-model"),
  selectedVrm: document.querySelector("#selected-vrm-model"), uploadVrm: document.querySelector("#upload-vrm-model"),
  brightnessForm: document.querySelector("#model-brightness-form"), brightness: document.querySelector("#model-brightness"),
  brightnessValue: document.querySelector("#model-brightness-value"), saveBrightness: document.querySelector("#save-model-brightness"),
  layoutForm: document.querySelector("#model-layout-form"), saveLayout: document.querySelector("#save-model-layout"),
  cameraPosition: ["x", "y", "z"].map((axis) => document.querySelector(`#camera-position-${axis}`)),
  foodPosition: ["x", "y", "z"].map((axis) => document.querySelector(`#food-prop-position-${axis}`)),
  foodRotation: ["x", "y", "z"].map((axis) => document.querySelector(`#food-prop-rotation-${axis}`)),
  foodScale: document.querySelector("#food-prop-scale"),
  vrmStatus: document.querySelector("#vrm-status"), vrmError: document.querySelector("#vrm-error"),
  backgroundForm: document.querySelector("#background-form"), backgroundInput: document.querySelector("#background-image"),
  uploadBackground: document.querySelector("#upload-background"), deleteBackground: document.querySelector("#delete-background"),
  currentBackgroundPreview: document.querySelector("#current-background-preview"), currentBackgroundEmpty: document.querySelector("#current-background-empty"),
  selectedBackgroundPreview: document.querySelector("#selected-background-preview"), selectedBackgroundEmpty: document.querySelector("#selected-background-empty"),
  musicForm: document.querySelector("#background-music-form"), musicInput: document.querySelector("#background-music"),
  uploadMusic: document.querySelector("#upload-background-music"), deleteMusic: document.querySelector("#delete-background-music"),
  currentMusic: document.querySelector("#current-background-music"), currentMusicEmpty: document.querySelector("#current-background-music-empty"), selectedMusic: document.querySelector("#selected-background-music"),
  musicVolumeForm: document.querySelector("#background-music-volume-form"), musicVolume: document.querySelector("#background-music-volume"),
  musicVolumeValue: document.querySelector("#background-music-volume-value"), saveMusicVolume: document.querySelector("#save-background-music-volume"),
  musicDuckRatio: document.querySelector("#background-music-duck-ratio"), musicDuckRatioValue: document.querySelector("#background-music-duck-ratio-value"),
  musicStatus: document.querySelector("#background-music-status"), musicError: document.querySelector("#background-music-error"),
  displayStatus: document.querySelector("#display-config-status"), displayError: document.querySelector("#display-config-error"),
};
let currentTurn;
let socket;
let reconnectTimer;
let previewAudio;
let previewAudioUrl;
let previewAbortController;
let loadedConfig;
let selectedSpeakerId;
let selectedVrmFile;
let vrmBusy = false;
let brightnessBusy = false;
let layoutBusy = false;
let selectedBackgroundBlob;
let selectedBackgroundUrl;
let currentBackgroundExists = false;
let backgroundBusy = false;
let selectedMusicFile;
let currentMusicExists = false;
let musicBusy = false;
let currentEventIdentifier;
let currentPublicBaseUrl;
let eventQrImageUrl;

function adminUrl(path) { return `${path}?token=${encodeURIComponent(token)}`; }
function setStatus(message) { elements.status.textContent = message; }
function setMessage(status, error, message = "", isError = false) { status.textContent = isError ? "" : message; error.textContent = isError ? message : ""; }
function setCurrentTurn(turn) { currentTurn = turn; elements.skip.disabled = !turn; }
function turnStatusLabel(status) { return status === "generating" ? "回答生成中" : status === "eating" ? "食事演出中" : "発話中"; }
function readError(response, fallback) { return response.json().catch(() => ({})).then((body) => body.error || fallback); }
const userDictionary = new UserDictionaryEditor({ token, engineUrl: elements.engineUrl, adminUrl, readError, stopOtherPreview: releasePreview });

function eventUrls(identifier = elements.eventIdentifier.value.trim()) {
  const publicBaseUrl = elements.publicBaseUrl.value.trim().replace(/\/+$/, "");
  const base = publicBaseUrl && identifier ? `${publicBaseUrl}/event/${identifier}` : "";
  return { main: base, input: base ? `${base}/input` : "", draw: base ? `${base}/draw` : "" };
}

function renderEventUrls() {
  const urls = eventUrls();
  elements.eventMainUrl.value = urls.main;
  elements.eventInputUrl.value = urls.input;
  elements.eventDrawUrl.value = urls.draw;
  for (const button of elements.eventCopyButtons) button.disabled = !urls[button.dataset.copyEventUrl];
  for (const button of elements.eventQrButtons) button.disabled = !urls[button.dataset.qrEventUrl];
}

function randomEventIdentifier() {
  const alphabet = "abcdefghjkmnpqrstuvwxyz23456789";
  const values = crypto.getRandomValues(new Uint8Array(16));
  return [...values].map((value) => alphabet[value % alphabet.length]).join("");
}

async function loadEventAccess() {
  const response = await fetch(adminUrl("/api/admin/event-access"), { cache: "no-store" });
  if (!response.ok) throw new Error(await readError(response, "公開URLを読み込めませんでした。"));
  const result = await response.json();
  currentPublicBaseUrl = result.public_base_url;
  currentEventIdentifier = result.event_identifier;
  elements.publicBaseUrl.value = currentPublicBaseUrl;
  elements.eventIdentifier.value = currentEventIdentifier;
  renderEventUrls();
}

async function saveEventAccess(event) {
  event.preventDefault();
  if (!token || !elements.eventForm.reportValidity()) return;
  const publicBaseUrl = elements.publicBaseUrl.value.trim().replace(/\/+$/, "");
  const eventIdentifier = elements.eventIdentifier.value.trim();
  const identifierChanged = eventIdentifier !== currentEventIdentifier;
  if (!identifierChanged && publicBaseUrl === currentPublicBaseUrl) {
    setMessage(elements.eventAccessStatus, elements.eventAccessError, "公開リンク設定は変更されていません。");
    return;
  }
  if (identifierChanged && !window.confirm("以前の公開URLは直ちに使えなくなります。公開URLを変更しますか？")) return;
  elements.saveEventAccess.disabled = true;
  const original = elements.saveEventAccess.textContent;
  elements.saveEventAccess.textContent = "変更中…";
  setMessage(elements.eventAccessStatus, elements.eventAccessError);
  try {
    const response = await fetch(adminUrl("/api/admin/event-access"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ public_base_url: publicBaseUrl, event_identifier: eventIdentifier }),
    });
    if (!response.ok) throw new Error(await readError(response, "公開URLを変更できませんでした。"));
    const result = await response.json();
    currentPublicBaseUrl = result.public_base_url;
    currentEventIdentifier = result.event_identifier;
    elements.publicBaseUrl.value = currentPublicBaseUrl;
    elements.eventIdentifier.value = currentEventIdentifier;
    renderEventUrls();
    setMessage(elements.eventAccessStatus, elements.eventAccessError, identifierChanged ? "公開URLを変更しました。以前の識別子を含むリンクは使用できません。" : "公開先ベースURLを保存しました。");
  } catch (error) {
    console.error(error);
    setMessage(elements.eventAccessStatus, elements.eventAccessError, error.message || "公開URLを変更できませんでした。", true);
  } finally {
    elements.saveEventAccess.disabled = false;
    elements.saveEventAccess.textContent = original;
  }
}

function releaseEventQrImage() {
  if (eventQrImageUrl) URL.revokeObjectURL(eventQrImageUrl);
  eventQrImageUrl = undefined;
  elements.eventQrImage.removeAttribute("src");
}

async function showEventQr(event) {
  const button = event.currentTarget;
  const kind = button.dataset.qrEventUrl;
  const url = eventUrls()[kind];
  if (!url) return;
  const titles = { main: "メイン画面", input: "質問画面", draw: "描画画面" };
  button.disabled = true;
  setMessage(elements.eventAccessStatus, elements.eventAccessError);
  try {
    const response = await fetch(adminUrl("/api/admin/qr-code"), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url }),
    });
    if (!response.ok) throw new Error(await readError(response, "QRコードを生成できませんでした。"));
    releaseEventQrImage();
    eventQrImageUrl = URL.createObjectURL(await response.blob());
    elements.eventQrTitle.textContent = `${titles[kind]}のQRコード`;
    elements.eventQrImage.alt = `${titles[kind]}のQRコード`;
    elements.eventQrImage.src = eventQrImageUrl;
    elements.eventQrUrl.textContent = url;
    elements.eventQrDialog.showModal();
  } catch (error) {
    console.error(error);
    setMessage(elements.eventAccessStatus, elements.eventAccessError, error.message || "QRコードを生成できませんでした。", true);
  } finally {
    button.disabled = false;
  }
}

async function copyEventUrl(event) {
  const url = eventUrls()[event.currentTarget.dataset.copyEventUrl];
  if (!url) return;
  try {
    await navigator.clipboard.writeText(url);
    setMessage(elements.eventAccessStatus, elements.eventAccessError, "URLをコピーしました。");
  } catch (error) {
    console.error(error);
    setMessage(elements.eventAccessStatus, elements.eventAccessError, "URLをコピーできませんでした。欄から選択してコピーしてください。", true);
  }
}

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
  const lastIndex = elements.tabs.length - 1;
  const byKey = { ArrowRight: (index + 1) % elements.tabs.length, ArrowLeft: (index + lastIndex) % elements.tabs.length, Home: 0, End: lastIndex };
  if (!(event.key in byKey)) return;
  event.preventDefault(); activateTab(elements.tabs[byKey[event.key]], true);
}

function updateVrmControls() {
  elements.vrmInput.disabled = vrmBusy || !token;
  elements.uploadVrm.disabled = vrmBusy || !selectedVrmFile || !token;
}
function updateBrightnessControls() {
  elements.brightness.disabled = brightnessBusy || !token;
  elements.saveBrightness.disabled = brightnessBusy || !token;
}
function updateLayoutControls() {
  [...elements.layoutForm.elements].forEach((element) => { element.disabled = layoutBusy || !token; });
}
function updateBrightnessLabel() {
  elements.brightnessValue.value = `${elements.brightness.value}%`;
  elements.brightnessValue.textContent = `${elements.brightness.value}%`;
}
function updateBackgroundControls() {
  elements.backgroundInput.disabled = backgroundBusy || !token;
  elements.uploadBackground.disabled = backgroundBusy || !selectedBackgroundBlob || !token;
  elements.deleteBackground.disabled = backgroundBusy || !currentBackgroundExists || !token;
}
function updateMusicControls() {
  elements.musicInput.disabled = musicBusy || !token;
  elements.uploadMusic.disabled = musicBusy || !selectedMusicFile || !token;
  elements.deleteMusic.disabled = musicBusy || !currentMusicExists || !token;
  elements.musicVolume.disabled = musicBusy || !token;
  elements.musicDuckRatio.disabled = musicBusy || !token;
  elements.saveMusicVolume.disabled = musicBusy || !token;
}
function selectVrmModel() {
  if (vrmBusy) return;
  const [file] = elements.vrmInput.files;
  selectedVrmFile = undefined;
  elements.selectedVrm.textContent = "なし";
  setMessage(elements.vrmStatus, elements.vrmError);
  if (!file) {
    updateVrmControls();
    return;
  }
  if (!file.name.toLowerCase().endsWith(".vrm")) {
    elements.vrmInput.value = "";
    setMessage(elements.vrmStatus, elements.vrmError, ".vrmファイルを選択してください。", true);
  } else if (file.size > MAX_VRM_MODEL_BYTES) {
    elements.vrmInput.value = "";
    setMessage(elements.vrmStatus, elements.vrmError, "VRMモデルは100MiB以下にしてください。", true);
  } else {
    selectedVrmFile = file;
    elements.selectedVrm.textContent = `${file.name}（${(file.size / 1024 / 1024).toFixed(1)}MiB）`;
    setMessage(elements.vrmStatus, elements.vrmError, "置き換えるVRMモデルを選択しました。");
  }
  updateVrmControls();
}
function updateMusicVolumeLabels() {
  elements.musicVolumeValue.value = `${elements.musicVolume.value}%`;
  elements.musicVolumeValue.textContent = `${elements.musicVolume.value}%`;
  elements.musicDuckRatioValue.value = `${elements.musicDuckRatio.value}%`;
  elements.musicDuckRatioValue.textContent = `${elements.musicDuckRatio.value}%`;
  elements.currentMusic.volume = Number(elements.musicVolume.value) / 100;
}
function showCurrentMusic(url) {
  currentMusicExists = Boolean(url);
  elements.currentMusic.pause();
  elements.currentMusic.removeAttribute("src");
  elements.currentMusic.load();
  elements.currentMusic.hidden = !url;
  elements.currentMusicEmpty.hidden = Boolean(url);
  elements.currentMusicEmpty.textContent = url ? "" : "BGMは設定されていません。";
  if (url) elements.currentMusic.src = url;
  updateMusicControls();
}
function selectMusic() {
  if (musicBusy) return;
  const [file] = elements.musicInput.files;
  selectedMusicFile = undefined;
  elements.selectedMusic.textContent = "なし";
  setMessage(elements.musicStatus, elements.musicError);
  if (!file) {
    updateMusicControls();
    return;
  }
  const extension = file.name.toLowerCase().split(".").pop();
  if (!["mp3", "ogg", "wav"].includes(extension)) {
    elements.musicInput.value = "";
    setMessage(elements.musicStatus, elements.musicError, "MP3、OGG、WAV音源を選択してください。", true);
  } else if (file.size > MAX_BACKGROUND_MUSIC_BYTES) {
    elements.musicInput.value = "";
    setMessage(elements.musicStatus, elements.musicError, "BGM音源は100MiB以下にしてください。", true);
  } else {
    selectedMusicFile = file;
    elements.selectedMusic.textContent = file.name;
    setMessage(elements.musicStatus, elements.musicError, "アップロードすると現在のBGMを上書きします。");
  }
  updateMusicControls();
}
async function showCurrentBackground(url) {
  currentBackgroundExists = Boolean(url);
  elements.currentBackgroundPreview.hidden = true;
  elements.currentBackgroundPreview.removeAttribute("src");
  elements.currentBackgroundEmpty.hidden = false;
  elements.currentBackgroundEmpty.textContent = url ? "現在の背景画像を読み込み中です…" : "背景画像は設定されていません。";
  updateBackgroundControls();
  if (!url) return;
  try {
    await new Promise((resolve, reject) => {
      elements.currentBackgroundPreview.onload = resolve;
      elements.currentBackgroundPreview.onerror = () => reject(new Error("現在の背景画像を読み込めませんでした。"));
      elements.currentBackgroundPreview.src = url;
    });
  } catch (error) {
    elements.currentBackgroundPreview.removeAttribute("src");
    elements.currentBackgroundEmpty.textContent = "現在の背景画像を読み込めませんでした。";
    throw error;
  } finally {
    elements.currentBackgroundPreview.onload = null;
    elements.currentBackgroundPreview.onerror = null;
  }
  elements.currentBackgroundPreview.hidden = false;
  elements.currentBackgroundEmpty.hidden = true;
  updateBackgroundControls();
}
function releaseSelectedBackground() {
  if (selectedBackgroundUrl) URL.revokeObjectURL(selectedBackgroundUrl);
  selectedBackgroundUrl = undefined;
  selectedBackgroundBlob = undefined;
  elements.selectedBackgroundPreview.removeAttribute("src");
  elements.selectedBackgroundPreview.hidden = true;
  elements.selectedBackgroundEmpty.hidden = false;
  updateBackgroundControls();
}
function showSelectedBackground(blob) {
  releaseSelectedBackground();
  selectedBackgroundBlob = blob;
  selectedBackgroundUrl = URL.createObjectURL(blob);
  elements.selectedBackgroundPreview.src = selectedBackgroundUrl;
  elements.selectedBackgroundPreview.hidden = false;
  elements.selectedBackgroundEmpty.hidden = true;
  updateBackgroundControls();
}
function loadImage(file) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const image = new Image();
    image.onload = () => { URL.revokeObjectURL(url); resolve(image); };
    image.onerror = () => { URL.revokeObjectURL(url); reject(new Error("画像を読み込めませんでした。")); };
    image.src = url;
  });
}
function canvasToWebp(canvas) {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (!blob || blob.type !== "image/webp") {
        reject(new Error("このブラウザではWebPへ変換できません。"));
        return;
      }
      resolve(blob);
    }, "image/webp", BACKGROUND_WEBP_QUALITY);
  });
}
async function convertBackground(file) {
  if (!file || !["image/jpeg", "image/png", "image/webp"].includes(file.type)) throw new Error("JPEG、PNG、WebP画像を選択してください。");
  if (file.size > MAX_BACKGROUND_BYTES) throw new Error("元画像は10MiB以下にしてください。");
  const image = await loadImage(file);
  if (!image.naturalWidth || !image.naturalHeight) throw new Error("画像のサイズを確認できませんでした。");
  const scale = Math.min(1, MAX_BACKGROUND_DIMENSION / Math.max(image.naturalWidth, image.naturalHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("画像を変換する機能を利用できません。");
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  image.src = "";
  let blob;
  try {
    blob = await canvasToWebp(canvas);
  } finally {
    canvas.width = 0;
    canvas.height = 0;
  }
  if (blob.size > MAX_BACKGROUND_BYTES) throw new Error("WebP変換後の画像が10MiBを超えています。別の画像を選択してください。");
  return blob;
}
function setVectorInputs(inputs, values) {
  inputs.forEach((input, index) => { input.value = String(values?.[index] ?? 0); });
}
function vectorValues(inputs) {
  return inputs.map((input) => Number(input.value));
}
async function loadDisplayConfig({ background = true, music = true, volume = true, brightness = true, layout = true } = {}) {
  const response = await fetch(adminUrl("/api/admin/display-config"), { cache: "no-store" });
  if (!response.ok) throw new Error("現在の表示設定を確認できませんでした。");
  const config = await response.json();
  if (music) showCurrentMusic(config.background_music_url);
  if (brightness) {
    const configuredBrightness = Number(config.light?.brightness);
    elements.brightness.value = String(Math.round((Number.isFinite(configuredBrightness) ? configuredBrightness : 1) * 100));
    updateBrightnessLabel();
  }
  if (layout) {
    setVectorInputs(elements.cameraPosition, config.camera?.position);
    setVectorInputs(elements.foodPosition, config.food_prop?.position);
    setVectorInputs(elements.foodRotation, config.food_prop?.rotation_degrees);
    elements.foodScale.value = String(config.food_prop?.size ?? 0.2);
  }
  if (volume) {
    const configuredVolume = Number(config.background_music_volume);
    elements.musicVolume.value = String(Math.round((Number.isFinite(configuredVolume) ? configuredVolume : 0.3) * 100));
    const configuredDuckRatio = Number(config.background_music_duck_ratio);
    elements.musicDuckRatio.value = String(Math.round((Number.isFinite(configuredDuckRatio) ? configuredDuckRatio : 0.4) * 100));
    updateMusicVolumeLabels();
  }
  if (background) await showCurrentBackground(config.background_image_url);
}
async function selectBackground() {
  if (backgroundBusy) return;
  backgroundBusy = true;
  releaseSelectedBackground();
  updateBackgroundControls();
  setMessage(elements.displayStatus, elements.displayError);
  const [file] = elements.backgroundInput.files;
  if (!file) {
    backgroundBusy = false;
    updateBackgroundControls();
    return;
  }
  elements.selectedBackgroundEmpty.textContent = "画像を変換中です…";
  try {
    showSelectedBackground(await convertBackground(file));
    setMessage(elements.displayStatus, elements.displayError, "画像をWebPへ変換しました。アップロードすると現在の背景画像を上書きします。");
  } catch (error) {
    console.error(error);
    elements.backgroundInput.value = "";
    setMessage(elements.displayStatus, elements.displayError, error.message || "画像を変換できませんでした。", true);
  } finally {
    backgroundBusy = false;
    elements.selectedBackgroundEmpty.textContent = "画像を選択してください。";
    updateBackgroundControls();
  }
}
async function uploadVrmModel(event) {
  event.preventDefault();
  if (!token || !selectedVrmFile || vrmBusy) return;
  vrmBusy = true;
  updateVrmControls();
  const original = elements.uploadVrm.textContent;
  elements.uploadVrm.textContent = "アップロード中…";
  setMessage(elements.vrmStatus, elements.vrmError);
  try {
    const body = new FormData();
    body.append("model", selectedVrmFile, selectedVrmFile.name);
    const response = await fetch(adminUrl("/api/admin/vrm-model"), { method: "POST", body });
    if (!response.ok) throw new Error(await readError(response, "VRMモデルを置き換えられませんでした。"));
    selectedVrmFile = undefined;
    elements.vrmInput.value = "";
    elements.selectedVrm.textContent = "なし";
    setMessage(elements.vrmStatus, elements.vrmError, "VRMモデルを更新しました。接続中のメイン画面は現在の処理後に読み込み直します。");
  } catch (error) {
    console.error(error);
    setMessage(elements.vrmStatus, elements.vrmError, error.message || "VRMモデルを置き換えられませんでした。", true);
  } finally {
    vrmBusy = false;
    elements.uploadVrm.textContent = original;
    updateVrmControls();
  }
}
async function saveModelBrightness(event) {
  event.preventDefault();
  if (!token || brightnessBusy) return;
  brightnessBusy = true;
  updateBrightnessControls();
  const original = elements.saveBrightness.textContent;
  elements.saveBrightness.textContent = "保存中…";
  setMessage(elements.vrmStatus, elements.vrmError);
  try {
    const response = await fetch(adminUrl("/api/admin/model-brightness"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ brightness: Number(elements.brightness.value) / 100 }),
    });
    if (!response.ok) throw new Error(await readError(response, "モデルの明るさを保存できませんでした。"));
    setMessage(elements.vrmStatus, elements.vrmError, "モデルの明るさを保存しました。接続中のメイン画面は現在の処理後に反映します。");
  } catch (error) {
    console.error(error);
    setMessage(elements.vrmStatus, elements.vrmError, error.message || "モデルの明るさを保存できませんでした。", true);
  } finally {
    brightnessBusy = false;
    elements.saveBrightness.textContent = original;
    updateBrightnessControls();
  }
}
async function saveModelLayout(event) {
  event.preventDefault();
  if (!token || layoutBusy) return;
  const cameraPosition = vectorValues(elements.cameraPosition);
  const foodPosition = vectorValues(elements.foodPosition);
  const foodRotation = vectorValues(elements.foodRotation);
  const foodScale = Number(elements.foodScale.value);
  if (![...cameraPosition, ...foodPosition, ...foodRotation, foodScale].every(Number.isFinite) || foodScale <= 0) {
    setMessage(elements.vrmStatus, elements.vrmError, "PositionとRotationは数値、Scaleは0より大きい数値で入力してください。", true);
    return;
  }
  layoutBusy = true;
  updateLayoutControls();
  const original = elements.saveLayout.textContent;
  elements.saveLayout.textContent = "保存中…";
  setMessage(elements.vrmStatus, elements.vrmError);
  try {
    const response = await fetch(adminUrl("/api/admin/model-layout"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        camera_position: cameraPosition,
        food_prop_position: foodPosition,
        food_prop_rotation_degrees: foodRotation,
        food_prop_scale: foodScale,
      }),
    });
    if (!response.ok) throw new Error(await readError(response, "位置調整を保存できませんでした。"));
    setMessage(elements.vrmStatus, elements.vrmError, "位置調整を保存しました。接続中のメイン画面は現在の処理後に反映します。");
  } catch (error) {
    console.error(error);
    setMessage(elements.vrmStatus, elements.vrmError, error.message || "位置調整を保存できませんでした。", true);
  } finally {
    layoutBusy = false;
    elements.saveLayout.textContent = original;
    updateLayoutControls();
  }
}
async function uploadBackground(event) {
  event.preventDefault();
  if (!token || !selectedBackgroundBlob || backgroundBusy) return;
  backgroundBusy = true;
  updateBackgroundControls();
  const original = elements.uploadBackground.textContent;
  elements.uploadBackground.textContent = "アップロード中…";
  setMessage(elements.displayStatus, elements.displayError);
  try {
    const body = new FormData();
    body.append("image", selectedBackgroundBlob, "background.webp");
    const response = await fetch(adminUrl("/api/admin/background-image"), { method: "POST", body });
    if (!response.ok) throw new Error(await readError(response, "背景画像をアップロードできませんでした。"));
    releaseSelectedBackground();
    elements.backgroundInput.value = "";
    try {
      await loadDisplayConfig({ music: false, volume: false });
      setMessage(elements.displayStatus, elements.displayError, "背景画像を更新しました。接続中のメイン画面へ反映されます。");
    } catch (error) {
      console.error(error);
      setMessage(elements.displayStatus, elements.displayError, "背景画像は更新されましたが、現在のプレビューを更新できませんでした。", true);
    }
  } catch (error) {
    console.error(error);
    setMessage(elements.displayStatus, elements.displayError, error.message || "背景画像をアップロードできませんでした。", true);
  } finally {
    backgroundBusy = false;
    elements.uploadBackground.textContent = original;
    updateBackgroundControls();
  }
}
async function deleteBackground() {
  if (!token || backgroundBusy || !window.confirm("現在の背景画像を削除しますか？")) return;
  backgroundBusy = true;
  updateBackgroundControls();
  const original = elements.deleteBackground.textContent;
  elements.deleteBackground.textContent = "削除中…";
  setMessage(elements.displayStatus, elements.displayError);
  try {
    const response = await fetch(adminUrl("/api/admin/background-image"), { method: "DELETE" });
    if (!response.ok) throw new Error(await readError(response, "背景画像を削除できませんでした。"));
    await showCurrentBackground(null);
    try {
      await loadDisplayConfig({ music: false, volume: false });
      setMessage(elements.displayStatus, elements.displayError, "背景画像を削除しました。接続中のメイン画面は背景色へ戻ります。");
    } catch (error) {
      console.error(error);
      setMessage(elements.displayStatus, elements.displayError, "背景画像は削除されましたが、現在のプレビューを更新できませんでした。", true);
    }
  } catch (error) {
    console.error(error);
    setMessage(elements.displayStatus, elements.displayError, error.message || "背景画像を削除できませんでした。", true);
  } finally {
    backgroundBusy = false;
    elements.deleteBackground.textContent = original;
    updateBackgroundControls();
  }
}

async function uploadMusic(event) {
  event.preventDefault();
  if (!token || !selectedMusicFile || musicBusy) return;
  musicBusy = true;
  updateMusicControls();
  const original = elements.uploadMusic.textContent;
  elements.uploadMusic.textContent = "変換・アップロード中…";
  setMessage(elements.musicStatus, elements.musicError);
  try {
    const body = new FormData();
    body.append("audio", selectedMusicFile, selectedMusicFile.name);
    const response = await fetch(adminUrl("/api/admin/background-music"), { method: "POST", body });
    if (!response.ok) throw new Error(await readError(response, "BGMをアップロードできませんでした。"));
    selectedMusicFile = undefined;
    elements.musicInput.value = "";
    elements.selectedMusic.textContent = "なし";
    try {
      await loadDisplayConfig({ background: false, volume: false });
      setMessage(elements.musicStatus, elements.musicError, "BGMを更新しました。接続中のメイン画面へ反映されます。");
    } catch (error) {
      console.error(error);
      setMessage(elements.musicStatus, elements.musicError, "BGMは更新されましたが、現在の表示を更新できませんでした。", true);
    }
  } catch (error) {
    console.error(error);
    setMessage(elements.musicStatus, elements.musicError, error.message || "BGMをアップロードできませんでした。", true);
  } finally {
    musicBusy = false;
    elements.uploadMusic.textContent = original;
    updateMusicControls();
  }
}

async function deleteMusic() {
  if (!token || musicBusy || !window.confirm("現在のBGMを削除しますか？")) return;
  musicBusy = true;
  updateMusicControls();
  const original = elements.deleteMusic.textContent;
  elements.deleteMusic.textContent = "削除中…";
  setMessage(elements.musicStatus, elements.musicError);
  try {
    const response = await fetch(adminUrl("/api/admin/background-music"), { method: "DELETE" });
    if (!response.ok) throw new Error(await readError(response, "BGMを削除できませんでした。"));
    showCurrentMusic(null);
    setMessage(elements.musicStatus, elements.musicError, "BGMを削除しました。接続中のメイン画面へ反映されます。");
  } catch (error) {
    console.error(error);
    setMessage(elements.musicStatus, elements.musicError, error.message || "BGMを削除できませんでした。", true);
  } finally {
    musicBusy = false;
    elements.deleteMusic.textContent = original;
    updateMusicControls();
  }
}

async function saveMusicVolume(event) {
  event.preventDefault();
  if (!token || musicBusy) return;
  musicBusy = true;
  updateMusicControls();
  const original = elements.saveMusicVolume.textContent;
  elements.saveMusicVolume.textContent = "保存中…";
  setMessage(elements.musicStatus, elements.musicError);
  try {
    const volume = Number(elements.musicVolume.value) / 100;
    const duckRatio = Number(elements.musicDuckRatio.value) / 100;
    const response = await fetch(adminUrl("/api/admin/background-music-volume"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ volume, duck_ratio: duckRatio }),
    });
    if (!response.ok) throw new Error(await readError(response, "BGM音量を保存できませんでした。"));
    setMessage(elements.musicStatus, elements.musicError, "BGM音量を保存しました。接続中のメイン画面へ反映されます。");
  } catch (error) {
    console.error(error);
    setMessage(elements.musicStatus, elements.musicError, error.message || "BGM音量を保存できませんでした。", true);
  } finally {
    musicBusy = false;
    elements.saveMusicVolume.textContent = original;
    updateMusicControls();
  }
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
  socket = new WebSocket(`${scheme}//${location.host}/ws?token=${encodeURIComponent(token)}`); setStatus("サーバーへ接続中です");
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
async function clearConversationHistory() {
  if (!token || !window.confirm("共有会話履歴をすべて削除しますか？この操作は元に戻せません。")) return;
  elements.clearHistory.disabled = true;
  const original = elements.clearHistory.textContent;
  elements.clearHistory.textContent = "削除中…";
  setMessage(elements.aiStatus, elements.aiError);
  try {
    const response = await fetch(adminUrl("/api/admin/conversation-history"), { method: "DELETE" });
    if (!response.ok) throw new Error(await readError(response, "会話履歴を削除できませんでした。"));
    setMessage(elements.aiStatus, elements.aiError, "会話履歴を削除しました。");
  } catch (error) {
    console.error(error);
    setMessage(elements.aiStatus, elements.aiError, error.message || "会話履歴を削除できませんでした。", true);
  } finally {
    elements.clearHistory.disabled = false;
    elements.clearHistory.textContent = original;
  }
}
async function reload() {
  if (!token) return;
  elements.reload.disabled = true; const original = elements.reload.textContent; elements.reload.textContent = "再読み込み中…"; setMessage(elements.operationStatus, elements.operationError);
  try { const response = await fetch(adminUrl("/api/admin/reload-config"), { method: "POST" }); const result = await response.json().catch(() => ({})); if (!response.ok) throw new Error(result.error || "設定を再読み込みできませんでした。"); await Promise.all([loadConfig(), loadEventAccess()]); setMessage(elements.operationStatus, elements.operationError, result.restart_required ? "ファイルから再読み込みしました。表示設定は接続中のメイン画面へ反映されます。待受アドレスとポートは再起動後に反映されます。" : "ファイルから再読み込みしました。表示設定は接続中のメイン画面へ反映され、AI・音声設定は次の投稿から反映されます。"); }
  catch (error) { console.error(error); setMessage(elements.operationStatus, elements.operationError, error.message || "設定を再読み込みできませんでした。", true); }
  finally { elements.reload.disabled = false; elements.reload.textContent = original; }
}
function releasePreview() { previewAbortController?.abort(); previewAbortController = undefined; previewAudio?.pause(); previewAudio = undefined; if (previewAudioUrl) URL.revokeObjectURL(previewAudioUrl); previewAudioUrl = undefined; }
async function previewTts() {
  if (!token || !validate(elements.ttsForm)) return;
  if (!hasSelectedSpeaker()) { setMessage(elements.ttsStatus, elements.ttsError, speakerSelectionError(), true); return; }
  elements.preview.disabled = true; const original = elements.preview.textContent; elements.preview.textContent = "試聴を準備中…"; setMessage(elements.ttsStatus, elements.ttsError); userDictionary.releasePreview(); releasePreview();
  const controller = new AbortController(); previewAbortController = controller;
  try { const response = await fetch(adminUrl("/api/admin/tts-preview"), { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ tts: ttsConfig() }), signal: controller.signal }); if (!response.ok) throw new Error(await readError(response, "TTSへ接続できませんでした。")); const blob = await response.blob(); if (previewAbortController !== controller) return; previewAudioUrl = URL.createObjectURL(blob); previewAudio = new Audio(previewAudioUrl); previewAudio.addEventListener("ended", releasePreview, { once: true }); previewAudio.addEventListener("error", () => { releasePreview(); setMessage(elements.ttsStatus, elements.ttsError, "試聴音声を再生できませんでした。", true); }, { once: true }); await previewAudio.play(); if (previewAbortController !== controller) return; setMessage(elements.ttsStatus, elements.ttsError, "試聴を再生しています。"); }
  catch (error) { if (error.name === "AbortError") return; console.error(error); releasePreview(); setMessage(elements.ttsStatus, elements.ttsError, error.message || "TTSへ接続できませんでした。", true); }
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
elements.clearHistory.addEventListener("click", clearConversationHistory);
elements.eventForm.addEventListener("submit", saveEventAccess);
elements.publicBaseUrl.addEventListener("input", renderEventUrls);
elements.eventIdentifier.addEventListener("input", () => { elements.eventIdentifier.value = elements.eventIdentifier.value.toLowerCase(); renderEventUrls(); });
elements.randomizeEventIdentifier.addEventListener("click", () => { elements.eventIdentifier.value = randomEventIdentifier(); renderEventUrls(); elements.eventIdentifier.focus(); });
elements.eventCopyButtons.forEach((button) => button.addEventListener("click", copyEventUrl));
elements.eventQrButtons.forEach((button) => button.addEventListener("click", showEventQr));
elements.eventQrClose.addEventListener("click", () => elements.eventQrDialog.close());
elements.eventQrDialog.addEventListener("click", (event) => { if (event.target === elements.eventQrDialog) elements.eventQrDialog.close(); });
elements.eventQrDialog.addEventListener("close", releaseEventQrImage);
elements.loadSpeakers.addEventListener("click", loadSpeakers);
elements.vrmInput.addEventListener("change", selectVrmModel);
elements.vrmForm.addEventListener("submit", uploadVrmModel);
elements.brightness.addEventListener("input", updateBrightnessLabel);
elements.brightnessForm.addEventListener("submit", saveModelBrightness);
elements.layoutForm.addEventListener("submit", saveModelLayout);
updateLayoutControls();
elements.backgroundInput.addEventListener("change", selectBackground);
elements.backgroundForm.addEventListener("submit", uploadBackground);
elements.deleteBackground.addEventListener("click", deleteBackground);
elements.musicInput.addEventListener("change", selectMusic);
elements.musicForm.addEventListener("submit", uploadMusic);
elements.deleteMusic.addEventListener("click", deleteMusic);
elements.musicVolume.addEventListener("input", updateMusicVolumeLabels);
elements.musicDuckRatio.addEventListener("input", updateMusicVolumeLabels);
elements.musicVolumeForm.addEventListener("submit", saveMusicVolume);
elements.engineUrl.addEventListener("input", () => {
  selectedSpeakerId = undefined;
  resetSpeakerList("エンジンURLを変更しました。話者一覧を再取得してください。");
  userDictionary.invalidate();
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
if (!token) {
  setMessage(elements.operationStatus, elements.operationError, "管理用トークンが指定されていません。", true);
  setMessage(elements.vrmStatus, elements.vrmError, "VRMモデルを変更するには管理用トークンが必要です。", true);
  setMessage(elements.displayStatus, elements.displayError, "表示設定を変更するには管理用トークンが必要です。", true);
  setMessage(elements.musicStatus, elements.musicError, "BGMを変更するには管理用トークンが必要です。", true);
  [...elements.eventForm.elements, ...elements.aiForm.elements, ...elements.ttsForm.elements, ...elements.vrmForm.elements, ...elements.brightnessForm.elements, ...elements.backgroundForm.elements, ...elements.musicForm.elements, ...elements.musicVolumeForm.elements, elements.reload].forEach((element) => { element.disabled = true; });
} else {
  connect();
  loadConfig();
  loadEventAccess().catch((error) => { console.error(error); setMessage(elements.eventAccessStatus, elements.eventAccessError, error.message || "公開URLを読み込めませんでした。", true); });
  loadDisplayConfig().catch((error) => { console.error(error); setMessage(elements.displayStatus, elements.displayError, error.message || "現在の背景画像を確認できませんでした。", true); });
}
window.addEventListener("beforeunload", () => { clearTimeout(reconnectTimer); socket?.close(); releasePreview(); userDictionary.releasePreview(); releaseSelectedBackground(); releaseEventQrImage(); elements.currentMusic.pause(); });
