import { UserDictionaryEditor } from "./user-dictionary.js?v=9";
import { canvasToWebp } from "./webp.js?v=1";
import { initMotionSettings } from "./admin-motions.js?v=2";
import { initVenueSettings } from "./admin-venue.js?v=1";

const token = new URLSearchParams(window.location.search).get("token");
const MAX_BACKGROUND_BYTES = 10 * 1024 * 1024;
const MAX_VRM_MODEL_BYTES = 100 * 1024 * 1024;
const MAX_BACKGROUND_MUSIC_BYTES = 100 * 1024 * 1024;
const MAX_BACKGROUND_DIMENSION = 1920;
const BACKGROUND_WEBP_QUALITY = 0.85;
const UPDATE_RECONNECT_REQUEST_TIMEOUT_MS = 3_000;
const UPDATE_RECONNECT_TOTAL_TIMEOUT_MS = 120_000;
const elements = {
  tabs: [...document.querySelectorAll('[role="tab"]')],
  status: document.querySelector("#admin-status"), skip: document.querySelector("#skip"), reload: document.querySelector("#reload-config"),
  operationStatus: document.querySelector("#operation-status"), operationError: document.querySelector("#operation-error"),
  currentAppVersion: document.querySelector("#current-app-version"), checkUpdate: document.querySelector("#check-update"),
  updateStatus: document.querySelector("#update-status"), updateError: document.querySelector("#update-error"),
  eventForm: document.querySelector("#event-access-form"), publicBaseUrl: document.querySelector("#public-base-url"), eventIdentifier: document.querySelector("#event-identifier"),
  randomizeEventIdentifier: document.querySelector("#randomize-event-identifier"), saveEventAccess: document.querySelector("#save-event-access"),
  eventMainUrl: document.querySelector("#event-main-url"), eventDebugUrl: document.querySelector("#event-debug-url"),
  eventInputUrl: document.querySelector("#event-input-url"), eventDrawUrl: document.querySelector("#event-draw-url"),
  eventCopyButtons: [...document.querySelectorAll("[data-copy-event-url]")], eventQrButtons: [...document.querySelectorAll("[data-qr-event-url]")],
  eventAccessStatus: document.querySelector("#event-access-status"), eventAccessError: document.querySelector("#event-access-error"),
  eventQrDialog: document.querySelector("#event-qr-dialog"), eventQrTitle: document.querySelector("#event-qr-title"),
  eventQrImage: document.querySelector("#event-qr-image"), eventQrUrl: document.querySelector("#event-qr-url"), eventQrClose: document.querySelector("#event-qr-close"),
  aiForm: document.querySelector("#ai-config-form"), ttsForm: document.querySelector("#tts-config-form"),
  aiStatus: document.querySelector("#ai-config-status"), aiError: document.querySelector("#ai-config-error"),
  clearHistory: document.querySelector("#clear-conversation-history"),
  ttsStatus: document.querySelector("#tts-config-status"), ttsError: document.querySelector("#tts-config-error"),
  clearTtsCache: document.querySelector("#clear-tts-cache"),
  saveAi: document.querySelector("#save-ai-config"), saveTts: document.querySelector("#save-tts-config"), preview: document.querySelector("#preview-tts"),
  loadSpeakers: document.querySelector("#load-tts-speakers"), speakerList: document.querySelector("#tts-speaker-list"), speakersStatus: document.querySelector("#tts-speakers-status"),
  apiUrl: document.querySelector("#llm-api-url"), model: document.querySelector("#llm-model"), systemPrompt: document.querySelector("#system-prompt"),
  foodPrompt: document.querySelector("#food-reaction-prompt"), fillers: document.querySelector("#search-fillers"),
  idleEnabled: document.querySelector("#idle-speech-enabled"), idleMin: document.querySelector("#idle-speech-min"), idleMax: document.querySelector("#idle-speech-max"),
  idleEntries: document.querySelector("#idle-speech-entries"), idleTemplate: document.querySelector("#idle-speech-entry-template"), addIdleSpeech: document.querySelector("#add-idle-speech"),
  engineUrl: document.querySelector("#tts-engine-url"),
  preparationModeForm: document.querySelector("#preparation-mode-form"), preparationMode: document.querySelector("#preparation-mode"),
  savePreparationMode: document.querySelector("#save-preparation-mode"), preparationStatus: document.querySelector("#preparation-status"), preparationError: document.querySelector("#preparation-error"),
  drawingForm: document.querySelector("#drawing-stabilization-form"), drawingStabilization: document.querySelector("#drawing-stabilization"),
  drawingStabilizationValue: document.querySelector("#drawing-stabilization-value"), saveDrawingStabilization: document.querySelector("#save-drawing-stabilization"),
  drawingStatus: document.querySelector("#drawing-stabilization-status"), drawingError: document.querySelector("#drawing-stabilization-error"),
  vrmForm: document.querySelector("#vrm-model-form"), vrmInput: document.querySelector("#vrm-model"),
  uploadVrm: document.querySelector("#upload-vrm-model"),
  brightnessForm: document.querySelector("#model-brightness-form"), brightness: document.querySelector("#model-brightness"),
  brightnessValue: document.querySelector("#model-brightness-value"), saveBrightness: document.querySelector("#save-model-brightness"),
  antialiasForm: document.querySelector("#model-antialias-form"), antialias: document.querySelector("#model-antialias"),
  saveAntialias: document.querySelector("#save-model-antialias"),
  layoutForm: document.querySelector("#model-layout-form"), saveLayout: document.querySelector("#save-model-layout"),
  cameraPosition: ["x", "y", "z"].map((axis) => document.querySelector(`#camera-position-${axis}`)),
  cameraFov: document.querySelector("#camera-fov"), cameraFovNumber: document.querySelector("#camera-fov-number"),
  foodPosition: ["x", "y", "z"].map((axis) => document.querySelector(`#food-prop-position-${axis}`)),
  foodRotation: ["x", "y", "z"].map((axis) => document.querySelector(`#food-prop-rotation-${axis}`)),
  foodScale: document.querySelector("#food-prop-scale"),
  vrmStatus: document.querySelector("#vrm-status"), vrmError: document.querySelector("#vrm-error"),
  musicForm: document.querySelector("#background-music-form"), musicInput: document.querySelector("#background-music"),
  uploadMusic: document.querySelector("#upload-background-music"), deleteMusic: document.querySelector("#delete-background-music"),
  currentMusic: document.querySelector("#current-background-music"), currentMusicEmpty: document.querySelector("#current-background-music-empty"),
  musicVolumeForm: document.querySelector("#background-music-volume-form"), musicVolume: document.querySelector("#background-music-volume"),
  musicVolumeValue: document.querySelector("#background-music-volume-value"), saveMusicVolume: document.querySelector("#save-background-music-volume"),
  musicDuckRatio: document.querySelector("#background-music-duck-ratio"), musicDuckRatioValue: document.querySelector("#background-music-duck-ratio-value"),
  musicStatus: document.querySelector("#background-music-status"), musicError: document.querySelector("#background-music-error"),
  displayStatus: document.querySelector("#display-config-status"), displayError: document.querySelector("#display-config-error"),
};
const screenOverlays = [...document.querySelectorAll(".screen-overlay-slot")].map((section) => {
  const slot = section.dataset.screenOverlaySlot;
  const scaleForm = section.querySelector(".screen-overlay-scale-form");
  const key = slot.replace("-", "_");
  const corner = { "top-left": "左上", "top-right": "右上", "bottom-left": "左下", "bottom-right": "右下" }[slot];
  return {
    scaleValue: (config) => config.screen_overlays?.[key]?.scale,
    label: `${corner}の画面オーバーレイ`,
    endpoint: `/api/admin/screen-overlays/${slot}`,
    fileName: `screen-overlay-${slot}.webp`,
    imageUrl: (config) => config.screen_overlays?.[key]?.image_url,
    status: elements.displayStatus, error: elements.displayError,
    scaleForm,
    input: section.querySelector("[data-screen-overlay-input]"),
    remove: section.querySelector("[data-screen-overlay-delete]"),
    currentPreview: section.querySelector("[data-screen-overlay-current]"),
    currentEmpty: section.querySelector("[data-screen-overlay-current-empty]"),
    scale: scaleForm.querySelector("[data-screen-overlay-scale]"),
    scaleNumber: scaleForm.querySelector("[data-screen-overlay-scale-number]"),
    saveScale: scaleForm.querySelector("[data-screen-overlay-save-scale]"),
    currentExists: false,
    busy: false,
  };
});
let currentTurn;
let socket;
let reconnectTimer;
let activePreview;
let configLoaded = false;
let selectedSpeakerId;
let selectedVrmFile;
let drawingBusy = false;
let vrmBusy = false;
let brightnessBusy = false;
let antialiasBusy = false;
let layoutBusy = false;
let preparationModeEnabled = false;
let preparationModeBusy = false;
let selectedMusicFile;
let currentMusicExists = false;
let musicBusy = false;
let currentEventIdentifier;
let currentPublicBaseUrl;
let eventQrImageUrl;
let updateInProgress = false;

const backgroundImageAsset = {
  label: "背景画像", fileName: "background.webp", endpoint: "/api/admin/background-image",
  imageUrl: (config) => config.background_image_url,
  input: document.querySelector("#background-image"), remove: document.querySelector("#delete-background"),
  currentPreview: document.querySelector("#current-background-preview"), currentEmpty: document.querySelector("#current-background-empty"),
  status: elements.displayStatus, error: elements.displayError,
  currentExists: false, busy: false,
};
const preparationImageAsset = {
  label: "準備中画像", fileName: "preparation.webp", endpoint: "/api/admin/preparation-image",
  imageUrl: (config) => config.preparation_image_url,
  input: document.querySelector("#preparation-image"), remove: document.querySelector("#delete-preparation-image"),
  currentPreview: document.querySelector("#current-preparation-preview"), currentEmpty: document.querySelector("#current-preparation-empty"),
  status: elements.preparationStatus, error: elements.preparationError,
  currentExists: false, busy: false,
};

const venueMapAsset = {
  label: "会場地図", fileName: "venue-map.webp", endpoint: "/api/admin/venue-map",
  imageUrl: (config) => config.venue_map_url,
  input: document.querySelector("#venue-map-file"),
  remove: document.querySelector("#venue-delete-map"),
  currentPreview: document.querySelector("#venue-current-map"), currentEmpty: document.querySelector("#venue-current-empty"),
  status: document.querySelector("#venue-map-status"), error: document.querySelector("#venue-map-error"),
  scaleValue: (config) => config.venue_map_scale,
  scaleForm: document.querySelector("#venue-map-scale-form"),
  scale: document.querySelector("#venue-map-scale"), scaleNumber: document.querySelector("#venue-map-scale-number"),
  saveScale: document.querySelector("#venue-map-save-scale"),
};
const scaledImages = [venueMapAsset, ...screenOverlays];
const imageAssets = [backgroundImageAsset, preparationImageAsset, ...scaledImages];

function adminUrl(path) { return `${path}?token=${encodeURIComponent(token)}`; }
function setStatus(message) { elements.status.textContent = message; }
function setMessage(status, error, message = "", isError = false) { status.textContent = isError ? "" : message; error.textContent = isError ? message : ""; }
function setCurrentTurn(turn) { currentTurn = turn; elements.skip.disabled = !turn; }
function turnStatusLabel(status) { return status === "idle_speaking" ? "待機発話中" : status === "generating" ? "回答生成中" : status === "eating" ? "食事演出中" : "発話中"; }
function readError(response, fallback) { return response.json().catch(() => ({})).then((body) => body.error || fallback); }
const userDictionary = new UserDictionaryEditor({ token, engineUrl: elements.engineUrl, adminUrl, readError, playPreview, stopPreview });

function eventUrls(identifier = elements.eventIdentifier.value.trim()) {
  const publicBaseUrl = elements.publicBaseUrl.value.trim().replace(/\/+$/, "");
  const base = publicBaseUrl && identifier ? `${publicBaseUrl}/event/${identifier}` : "";
  return { main: base, debug: base ? `${base}?debug` : "", input: base ? `${base}/input` : "", draw: base ? `${base}/draw` : "" };
}

function renderEventUrls() {
  const urls = eventUrls();
  elements.eventMainUrl.value = urls.main;
  elements.eventDebugUrl.value = urls.debug;
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
function updateDrawingControls() {
  elements.drawingStabilization.disabled = drawingBusy || !token;
  elements.saveDrawingStabilization.disabled = drawingBusy || !token;
}
function updateDrawingStabilizationLabel() {
  const label = `${elements.drawingStabilization.value} / 10`;
  elements.drawingStabilizationValue.value = label;
  elements.drawingStabilizationValue.textContent = label;
}
function updateBrightnessControls() {
  elements.brightness.disabled = brightnessBusy || !token;
  elements.saveBrightness.disabled = brightnessBusy || !token;
}
function updateAntialiasControls() {
  elements.antialias.disabled = antialiasBusy || !token;
  elements.saveAntialias.disabled = antialiasBusy || !token;
}
function updateLayoutControls() {
  [...elements.layoutForm.elements].forEach((element) => { element.disabled = layoutBusy || !token; });
}
function syncCameraFovFromNumber() {
  const fov = Number(elements.cameraFovNumber.value);
  if (Number.isFinite(fov) && fov >= 1 && fov <= 179) elements.cameraFov.value = String(fov);
}
function updateBrightnessLabel() {
  elements.brightnessValue.value = `${elements.brightness.value}%`;
  elements.brightnessValue.textContent = `${elements.brightness.value}%`;
}
function updateImageAssetControls(asset) {
  asset.input.disabled = asset.busy || !token;
  asset.remove.disabled = asset.busy || !asset.currentExists || !token;
  if (asset.scale) {
    asset.scale.disabled = asset.busy || !token;
    asset.scaleNumber.disabled = asset.busy || !token;
    asset.saveScale.disabled = asset.busy || !token;
  }
}
function updatePreparationModeControls() {
  elements.preparationMode.disabled = preparationModeBusy || !token;
  elements.savePreparationMode.disabled = preparationModeBusy || !token;
}
function syncImageScaleFromRange(asset) {
  asset.scaleNumber.value = asset.scale.value;
}
function syncImageScaleFromNumber(asset) {
  const scale = Number(asset.scaleNumber.value);
  if (!Number.isInteger(scale) || scale < 1 || scale > 100) return;
  asset.scale.value = String(scale);
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
    setMessage(elements.vrmStatus, elements.vrmError, `${(file.size / 1024 / 1024).toFixed(1)}MiBのモデルを選択しました。`);
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
    setMessage(elements.musicStatus, elements.musicError, "アップロードすると現在のBGMを上書きします。");
  }
  updateMusicControls();
}
async function showCurrentImageAsset(asset, image) {
  const previousUrl = asset.currentPreview.getAttribute("src");
  if (previousUrl?.startsWith("blob:")) URL.revokeObjectURL(previousUrl);
  asset.currentExists = Boolean(image);
  asset.currentPreview.hidden = true;
  asset.currentPreview.removeAttribute("src");
  asset.currentEmpty.hidden = false;
  asset.currentEmpty.textContent = image ? `現在の${asset.label}を読み込み中です…` : `${asset.label}は設定されていません。`;
  updateImageAssetControls(asset);
  if (!image) return;
  const url = image instanceof Blob ? URL.createObjectURL(image) : image;
  asset.currentPreview.src = url;
  try {
    await asset.currentPreview.decode();
  } catch {
    if (asset.currentPreview.getAttribute("src") !== url) return;
    asset.currentEmpty.textContent = `現在の${asset.label}を読み込めませんでした。`;
    throw new Error(asset.currentEmpty.textContent);
  }
  if (asset.currentPreview.getAttribute("src") !== url) return;
  asset.currentPreview.hidden = false;
  asset.currentEmpty.hidden = true;
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
    blob = await canvasToWebp(canvas, BACKGROUND_WEBP_QUALITY);
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
async function fetchDisplayConfig() {
  const response = await fetch(adminUrl("/api/admin/display-config"), { cache: "no-store" });
  if (!response.ok) throw new Error("現在の表示設定を確認できませんでした。");
  return response.json();
}
async function loadDisplayConfig() {
  const previews = imageAssets.map((asset) => asset.currentPreview.getAttribute("src"));
  const config = await fetchDisplayConfig();
  showCurrentMusic(config.background_music_url);
  const stabilization = Number(config.drawing?.stabilization);
  elements.drawingStabilization.value = String(Number.isInteger(stabilization) ? stabilization : 3);
  updateDrawingStabilizationLabel();
  const configuredBrightness = Number(config.light?.brightness);
  elements.brightness.value = String(Math.round((Number.isFinite(configuredBrightness) ? configuredBrightness : 1) * 100));
  updateBrightnessLabel();
  elements.antialias.checked = config.antialias !== false;
  setVectorInputs(elements.cameraPosition, config.camera?.position);
  elements.cameraFovNumber.value = String(config.camera?.fov ?? 30);
  syncCameraFovFromNumber();
  setVectorInputs(elements.foodPosition, config.food_prop?.position);
  setVectorInputs(elements.foodRotation, config.food_prop?.rotation_degrees);
  elements.foodScale.value = String(config.food_prop?.size ?? 0.2);
  const configuredVolume = Number(config.background_music_volume);
  elements.musicVolume.value = String(Math.round((Number.isFinite(configuredVolume) ? configuredVolume : 0.3) * 100));
  const configuredDuckRatio = Number(config.background_music_duck_ratio);
  elements.musicDuckRatio.value = String(Math.round((Number.isFinite(configuredDuckRatio) ? configuredDuckRatio : 0.4) * 100));
  updateMusicVolumeLabels();
  preparationModeEnabled = config.preparation_mode === true;
  elements.preparationMode.checked = preparationModeEnabled;
  updatePreparationModeControls();
  for (const asset of scaledImages) {
    asset.scale.value = String(Math.min(100, Math.max(1, Number(asset.scaleValue(config)) || 100)));
    syncImageScaleFromRange(asset);
  }
  await Promise.all(imageAssets.map((asset, i) => {
    if (asset.currentPreview.getAttribute("src") === previews[i]) {
      return showCurrentImageAsset(asset, asset.imageUrl(config));
    }
  }));
}
async function saveImageScale(event, asset) {
  event.preventDefault();
  if (!token || asset.busy) return;
  const scale = Number(asset.scaleNumber.value);
  if (!Number.isInteger(scale) || scale < 1 || scale > 100) {
    setMessage(asset.status, asset.error, "表示倍率は1〜100%の整数で指定してください。", true);
    return;
  }
  asset.busy = true;
  updateImageAssetControls(asset);
  const original = asset.saveScale.textContent;
  asset.saveScale.textContent = "保存中…";
  setMessage(asset.status, asset.error);
  try {
    const response = await fetch(adminUrl(`${asset.endpoint}/scale`), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ scale }),
    });
    if (!response.ok) throw new Error(await readError(response, "表示倍率を保存できませんでした。"));
    setMessage(asset.status, asset.error, `${asset.label}の表示倍率を保存しました。接続中のメイン画面へ反映されます。`);
  } catch (error) {
    console.error(error);
    setMessage(asset.status, asset.error, error.message || "表示倍率を保存できませんでした。", true);
  } finally {
    asset.busy = false;
    asset.saveScale.textContent = original;
    updateImageAssetControls(asset);
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
async function saveDrawingStabilization(event) {
  event.preventDefault();
  if (!token || drawingBusy) return;
  drawingBusy = true;
  updateDrawingControls();
  const original = elements.saveDrawingStabilization.textContent;
  elements.saveDrawingStabilization.textContent = "保存中…";
  setMessage(elements.drawingStatus, elements.drawingError);
  try {
    const response = await fetch(adminUrl("/api/admin/drawing-stabilization"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ stabilization: Number(elements.drawingStabilization.value) }),
    });
    if (!response.ok) throw new Error(await readError(response, "手ブレ補正を保存できませんでした。"));
    setMessage(elements.drawingStatus, elements.drawingError, "手ブレ補正を保存しました。描画画面を再読み込みすると反映されます。");
  } catch (error) {
    console.error(error);
    setMessage(elements.drawingStatus, elements.drawingError, error.message || "手ブレ補正を保存できませんでした。", true);
  } finally {
    drawingBusy = false;
    elements.saveDrawingStabilization.textContent = original;
    updateDrawingControls();
  }
}
async function saveModelAntialias(event) {
  event.preventDefault();
  if (!token || antialiasBusy) return;
  antialiasBusy = true;
  updateAntialiasControls();
  const original = elements.saveAntialias.textContent;
  elements.saveAntialias.textContent = "保存中…";
  setMessage(elements.vrmStatus, elements.vrmError);
  try {
    const response = await fetch(adminUrl("/api/admin/model-antialias"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ antialias: elements.antialias.checked }),
    });
    if (!response.ok) throw new Error(await readError(response, "アンチエイリアス設定を保存できませんでした。"));
    setMessage(elements.vrmStatus, elements.vrmError, "アンチエイリアス設定を保存しました。接続中のメイン画面は現在の処理後に反映します。");
  } catch (error) {
    console.error(error);
    setMessage(elements.vrmStatus, elements.vrmError, error.message || "アンチエイリアス設定を保存できませんでした。", true);
  } finally {
    antialiasBusy = false;
    elements.saveAntialias.textContent = original;
    updateAntialiasControls();
  }
}
async function saveModelLayout(event) {
  event.preventDefault();
  if (!token || layoutBusy) return;
  const cameraPosition = vectorValues(elements.cameraPosition);
  const cameraFov = Number(elements.cameraFovNumber.value);
  if (!Number.isFinite(cameraFov) || cameraFov < 1 || cameraFov > 179) {
    setMessage(elements.vrmStatus, elements.vrmError, "画角（FOV）は1〜179度の数値で入力してください。", true);
    return;
  }
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
        camera_fov: cameraFov,
        food_prop_position: foodPosition,
        food_prop_rotation_degrees: foodRotation,
        food_prop_scale: foodScale,
      }),
    });
    if (!response.ok) throw new Error(await readError(response, "画角・位置調整を保存できませんでした。"));
    setMessage(elements.vrmStatus, elements.vrmError, "画角・位置調整を保存しました。接続中のメイン画面は待機中ならすぐ、投稿処理中なら終了後に反映します。");
  } catch (error) {
    console.error(error);
    setMessage(elements.vrmStatus, elements.vrmError, error.message || "画角・位置調整を保存できませんでした。", true);
  } finally {
    layoutBusy = false;
    elements.saveLayout.textContent = original;
    updateLayoutControls();
  }
}
async function uploadImageAsset(asset) {
  const [file] = asset.input.files;
  if (!token || !file || asset.busy) return;
  asset.busy = true;
  updateImageAssetControls(asset);
  setMessage(asset.status, asset.error, `${asset.label}を保存中です…`);
  try {
    const image = await convertBackground(file);
    const body = new FormData();
    body.append("image", image, asset.fileName);
    const response = await fetch(adminUrl(asset.endpoint), { method: "POST", body });
    if (!response.ok) throw new Error(await readError(response, `${asset.label}をアップロードできませんでした。`));
    try {
      await showCurrentImageAsset(asset, image);
      setMessage(asset.status, asset.error, `${asset.label}を更新しました。接続中のメイン画面へ反映されます。`);
    } catch (error) {
      console.error(error);
      setMessage(asset.status, asset.error, `${asset.label}は更新されましたが、現在のプレビューを更新できませんでした。`, true);
    }
  } catch (error) {
    console.error(error);
    setMessage(asset.status, asset.error, error.message || `${asset.label}をアップロードできませんでした。`, true);
  } finally {
    asset.busy = false;
    asset.input.value = "";
    updateImageAssetControls(asset);
  }
}
async function deleteImageAsset(asset) {
  if (!token || asset.busy || !window.confirm(`現在の${asset.label}を削除しますか？`)) return;
  asset.busy = true;
  updateImageAssetControls(asset);
  const original = asset.remove.textContent;
  asset.remove.textContent = "削除中…";
  setMessage(asset.status, asset.error);
  try {
    const response = await fetch(adminUrl(asset.endpoint), { method: "DELETE" });
    if (!response.ok) throw new Error(await readError(response, `${asset.label}を削除できませんでした。`));
    await showCurrentImageAsset(asset, null);
    setMessage(asset.status, asset.error, `${asset.label}を削除しました。接続中のメイン画面へ反映されます。`);
  } catch (error) {
    console.error(error);
    setMessage(asset.status, asset.error, error.message || `${asset.label}を削除できませんでした。`, true);
  } finally {
    asset.busy = false;
    asset.remove.textContent = original;
    updateImageAssetControls(asset);
  }
}

async function savePreparationMode(event) {
  event.preventDefault();
  if (!token || preparationModeBusy) return;
  const enabled = elements.preparationMode.checked;
  preparationModeBusy = true;
  updatePreparationModeControls();
  const original = elements.savePreparationMode.textContent;
  elements.savePreparationMode.textContent = "保存中…";
  setMessage(elements.preparationStatus, elements.preparationError);
  try {
    const response = await fetch(adminUrl("/api/admin/preparation-mode"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ enabled }),
    });
    if (!response.ok) throw new Error(await readError(response, "準備中モードを保存できませんでした。"));
    preparationModeEnabled = enabled;
    setMessage(elements.preparationStatus, elements.preparationError, enabled
      ? "準備中モードを有効にしました。進行中の処理は中断されます。"
      : "準備中モードを解除しました。");
  } catch (error) {
    console.error(error);
    elements.preparationMode.checked = preparationModeEnabled;
    setMessage(elements.preparationStatus, elements.preparationError, error.message || "準備中モードを保存できませんでした。", true);
  } finally {
    preparationModeBusy = false;
    elements.savePreparationMode.textContent = original;
    updatePreparationModeControls();
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
    try {
      const config = await fetchDisplayConfig();
      showCurrentMusic(config.background_music_url);
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
  socket.addEventListener("close", () => {
    elements.skip.disabled = true;
    clearTimeout(reconnectTimer);
    if (updateInProgress) { setStatus("アップデート後の再起動中"); return; }
    setStatus("再接続中"); reconnectTimer = setTimeout(connect, 2000);
  });
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
function createIdleSpeechEntry(entry = { emotion: "neutral", text: "" }) {
  const row = elements.idleTemplate.content.firstElementChild.cloneNode(true);
  row.querySelector("select").value = entry.emotion;
  row.querySelector("textarea").value = entry.text;
  row.querySelector("[data-idle-remove]").addEventListener("click", () => {
    if (elements.idleEntries.childElementCount <= 1) return;
    row.remove();
    updateIdleSpeechEntries();
    elements.addIdleSpeech.focus();
  });
  return row;
}
function updateIdleSpeechEntries() {
  [...elements.idleEntries.children].forEach((row, index) => {
    row.setAttribute("aria-label", `セリフ候補 ${index + 1}`);
    row.querySelector("[data-idle-number]").textContent = `${index + 1}`;
    row.querySelector("textarea").setAttribute("aria-label", `セリフ ${index + 1}（300文字以内）`);
    const remove = row.querySelector("[data-idle-remove]");
    remove.disabled = elements.idleEntries.childElementCount <= 1;
    remove.setAttribute("aria-label", `セリフ候補 ${index + 1}を削除`);
  });
}
function llmConfig() {
  return { api_url: elements.apiUrl.value.trim(), model: elements.model.value.trim(), system_prompt: elements.systemPrompt.value.trim(), food_reaction_prompt: elements.foodPrompt.value.trim(), search_fillers: elements.fillers.value.split(/\r?\n/).map((value) => value.trim()).filter(Boolean) };
}
function configForSave(section) {
  if (!configLoaded) throw new Error("設定の読み込みが完了していません。");
  if (section === "tts") return { tts: ttsConfig() };
  return {
    llm: llmConfig(),
    idle_speech: {
      enabled: elements.idleEnabled.checked, min_seconds: Number(elements.idleMin.value), max_seconds: Number(elements.idleMax.value),
      entries: [...elements.idleEntries.children].map((row) => ({
        emotion: row.querySelector("select").value, text: row.querySelector("textarea").value.trim(),
      })),
    },
  };
}
function validate(form) {
  elements.idleMax.setCustomValidity(Number(elements.idleMin.value) <= Number(elements.idleMax.value) ? "" : "最長待機時間は最短待機時間以上にしてください。");
  for (const text of elements.idleEntries.querySelectorAll("textarea")) {
    text.setCustomValidity(text.value.trim() && [...text.value.trim()].length <= 300 ? "" : "セリフを1〜300文字で入力してください。");
  }
  elements.fillers.setCustomValidity(elements.fillers.value.split(/\r?\n/).some((value) => value.trim()) ? "" : "検索中フィラーを1文以上入力してください。");
  return form.reportValidity();
}
function applyConfig(config) {
  elements.idleEnabled.checked = config.idle_speech.enabled;
  elements.idleMin.value = config.idle_speech.min_seconds; elements.idleMax.value = config.idle_speech.max_seconds;
  elements.idleEntries.replaceChildren(...config.idle_speech.entries.map(createIdleSpeechEntry));
  updateIdleSpeechEntries();
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
  try { const response = await fetch(adminUrl("/api/admin/config"), { cache: "no-store" }); if (response.status === 401) throw new Error("管理用トークンを確認してください。"); if (response.status === 404) throw new Error("サーバーが設定画面に対応していません。新しい実行ファイルで再起動してください。"); if (!response.ok) throw new Error(await readError(response, "設定を読み込めませんでした。")); applyConfig(await response.json()); configLoaded = true; await loadSpeakers(); }
  catch (error) { console.error(error); setMessage(elements.aiStatus, elements.aiError, error.message || "設定を読み込めませんでした。", true); setMessage(elements.ttsStatus, elements.ttsError, "音声設定も読み込めませんでした。", true); }
}
async function saveConfig(section, form, button, status, error) {
  if (!token || !validate(form)) return;
  if (section === "tts" && !hasSelectedSpeaker()) { setMessage(status, error, speakerSelectionError(), true); return; }
  button.disabled = true; const original = button.textContent; button.textContent = "保存中…"; setMessage(status, error);
  try { const nextConfig = configForSave(section); const response = await fetch(adminUrl("/api/admin/config"), { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(nextConfig) }); if (!response.ok) throw new Error(await readError(response, "設定を保存できませんでした。")); setMessage(status, error, "保存しました。待機設定はすぐ、回答設定は次の投稿から反映されます。"); }
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
  try { const response = await fetch(adminUrl("/api/admin/reload-config"), { method: "POST" }); const result = await response.json().catch(() => ({})); if (!response.ok) throw new Error(result.error || "設定を再読み込みできませんでした。"); await Promise.all([loadConfig(), loadEventAccess(), loadMotionSettings(), loadVenueSettings(), loadDisplayConfig()]); setMessage(elements.operationStatus, elements.operationError, result.restart_required ? "ファイルから再読み込みしました。表示設定は接続中のメイン画面へ反映されます。待受アドレスとポートは再起動後に反映されます。" : "ファイルから再読み込みしました。表示設定は接続中のメイン画面へ反映され、AI・音声設定は次の投稿から反映されます。"); }
  catch (error) { console.error(error); setMessage(elements.operationStatus, elements.operationError, error.message || "設定を再読み込みできませんでした。", true); }
  finally { elements.reload.disabled = false; elements.reload.textContent = original; }
}
async function fetchVersion(timeoutMs) {
  const signal = timeoutMs ? AbortSignal.timeout(timeoutMs) : undefined;
  const response = await fetch(adminUrl("/api/admin/version"), { cache: "no-store", signal });
  if (!response.ok) throw new Error(await readError(response, "バージョンを確認できませんでした。"));
  return response.json();
}
async function loadVersion() {
  try {
    const result = await fetchVersion();
    elements.currentAppVersion.textContent = `v${result.current_version}`;
  } catch (error) {
    console.error(error);
    elements.currentAppVersion.textContent = "取得できません";
    setMessage(elements.updateStatus, elements.updateError, error.message || "バージョンを確認できませんでした。", true);
  }
}
async function waitForUpdatedServer(targetVersion) {
  let disconnected = false;
  const deadline = Date.now() + UPDATE_RECONNECT_TOTAL_TIMEOUT_MS;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 1000));
    let result;
    try {
      result = await fetchVersion(UPDATE_RECONNECT_REQUEST_TIMEOUT_MS);
    } catch {
      disconnected = true;
      continue;
    }
    if (result.current_version === targetVersion) {
      window.location.reload();
      return;
    }
    if (disconnected) throw new Error("アップデートに失敗したため、以前のバージョンで再起動しました。");
  }
  throw new Error("再起動を確認できませんでした。ページを再読み込みして状態を確認してください。");
}
async function checkAndApplyUpdate() {
  if (!token || updateInProgress) return;
  elements.checkUpdate.disabled = true;
  const original = elements.checkUpdate.textContent;
  elements.checkUpdate.textContent = "確認中…";
  setMessage(elements.updateStatus, elements.updateError);
  try {
    const checkResponse = await fetch(adminUrl("/api/admin/update"), { cache: "no-store" });
    if (!checkResponse.ok) throw new Error(await readError(checkResponse, "アップデートを確認できませんでした。"));
    const result = await checkResponse.json();
    elements.currentAppVersion.textContent = `v${result.current_version}`;
    if (!result.update_available) {
      setMessage(elements.updateStatus, elements.updateError, `v${result.current_version}が最新です。`);
      return;
    }
    if (!result.self_update_supported) {
      setMessage(elements.updateStatus, elements.updateError, `v${result.latest_version}を利用できます。${result.unsupported_reason}`);
      return;
    }
    if (!window.confirm(`v${result.latest_version}へアップデートして再起動しますか？\n処理中の投稿は中断されます。`)) {
      setMessage(elements.updateStatus, elements.updateError, `v${result.latest_version}を利用できます。`);
      return;
    }

    elements.checkUpdate.textContent = "準備中…";
    const updateResponse = await fetch(adminUrl("/api/admin/update"), { method: "POST" });
    if (!updateResponse.ok) throw new Error(await readError(updateResponse, "アップデートを開始できませんでした。"));
    const prepared = await updateResponse.json();
    updateInProgress = true;
    clearTimeout(reconnectTimer);
    socket?.close();
    elements.checkUpdate.textContent = "再起動中…";
    setMessage(elements.updateStatus, elements.updateError, `v${prepared.version}へアップデートして再起動しています。`);
    await waitForUpdatedServer(prepared.version);
  } catch (error) {
    console.error(error);
    updateInProgress = false;
    setMessage(elements.updateStatus, elements.updateError, error.message || "アップデートできませんでした。", true);
    if (!socket || socket.readyState === WebSocket.CLOSED) connect();
  } finally {
    if (!updateInProgress) {
      elements.checkUpdate.disabled = false;
      elements.checkUpdate.textContent = original;
    }
  }
}
async function clearTtsCache() {
  if (!token || elements.clearTtsCache.disabled) return;
  elements.clearTtsCache.disabled = true;
  setMessage(elements.ttsStatus, elements.ttsError);
  try {
    const response = await fetch(adminUrl("/api/admin/tts-cache"), { method: "DELETE" });
    if (!response.ok) throw new Error(await readError(response, "音声キャッシュを削除できませんでした。"));
    setMessage(elements.ttsStatus, elements.ttsError, "音声キャッシュを削除しました。次の発話時に生成し直します。");
  } catch (error) {
    setMessage(elements.ttsStatus, elements.ttsError, error.message || "音声キャッシュを削除できませんでした。", true);
  } finally { elements.clearTtsCache.disabled = false; }
}
function stopPreview(button) {
  if (activePreview && (!button || activePreview.button === button)) activePreview.stop();
}
async function playPreview({ button, path, body, errorMessage, playingMessage, report }) {
  stopPreview();
  const controller = new AbortController();
  const { signal } = controller;
  let audio;
  let objectUrl;
  function stop() {
    if (signal.aborted) return;
    activePreview = undefined;
    controller.abort();
    audio?.pause();
    if (objectUrl) URL.revokeObjectURL(objectUrl);
  }
  activePreview = { button, stop };
  try {
    const response = await fetch(adminUrl(path), {
      method: "POST", headers: { "Content-Type": "application/json" }, signal, body: JSON.stringify(body),
    });
    if (!response.ok) throw new Error(await readError(response, errorMessage));
    const blob = await response.blob();
    if (signal.aborted) return;
    objectUrl = URL.createObjectURL(blob);
    audio = new Audio(objectUrl);
    audio.addEventListener("ended", stop, { signal });
    audio.addEventListener("error", () => {
      stop();
      report("試聴音声を再生できませんでした。", true);
    }, { signal });
    await audio.play();
    if (!signal.aborted) report(playingMessage);
  } catch (error) {
    if (signal.aborted) return;
    console.error(error);
    stop();
    report(error.message || errorMessage, true);
  }
}
async function previewTts() {
  if (!token || !validate(elements.ttsForm)) return;
  const report = (message, error) => setMessage(elements.ttsStatus, elements.ttsError, message, error);
  if (!hasSelectedSpeaker()) { report(speakerSelectionError(), true); return; }
  const button = elements.preview;
  const original = button.textContent;
  button.disabled = true;
  button.textContent = "試聴を準備中…";
  report();
  try {
    await playPreview({
      button, path: "/api/admin/tts-preview", body: { tts: ttsConfig() },
      errorMessage: "TTSへ接続できませんでした。", playingMessage: "試聴を再生しています。", report,
    });
  } finally {
    button.disabled = false;
    button.textContent = original;
  }
}

const loadMotionSettings = initMotionSettings({ token, adminUrl, readError, setMessage });
const loadVenueSettings = initVenueSettings({ token, adminUrl, readError, setMessage });
elements.tabs.forEach((tab) => { tab.addEventListener("click", () => activateTab(tab)); tab.addEventListener("keydown", tabKeydown); });
for (const field of [...elements.aiForm.elements, ...elements.ttsForm.elements]) {
  field.addEventListener("invalid", () => field.setAttribute("aria-invalid", "true"));
  field.addEventListener("input", () => {
    if (field === elements.fillers) elements.fillers.setCustomValidity(elements.fillers.value.split(/\r?\n/).some((value) => value.trim()) ? "" : "検索中フィラーを1文以上入力してください。");
    field.setAttribute("aria-invalid", String(!field.validity.valid));
  });
}
elements.skip.addEventListener("click", skip); elements.reload.addEventListener("click", reload); elements.preview.addEventListener("click", previewTts);
elements.checkUpdate.addEventListener("click", checkAndApplyUpdate);
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
elements.clearTtsCache.addEventListener("click", clearTtsCache);
elements.vrmInput.addEventListener("change", selectVrmModel);
elements.vrmForm.addEventListener("submit", uploadVrmModel);
elements.drawingStabilization.addEventListener("input", updateDrawingStabilizationLabel);
elements.drawingForm.addEventListener("submit", saveDrawingStabilization);
updateDrawingControls();
elements.brightness.addEventListener("input", updateBrightnessLabel);
elements.brightnessForm.addEventListener("submit", saveModelBrightness);
elements.antialiasForm.addEventListener("submit", saveModelAntialias);
elements.layoutForm.addEventListener("submit", saveModelLayout);
elements.cameraFov.addEventListener("input", () => { elements.cameraFovNumber.value = elements.cameraFov.value; });
elements.cameraFovNumber.addEventListener("input", syncCameraFovFromNumber);
updateLayoutControls();
elements.preparationModeForm.addEventListener("submit", savePreparationMode);
elements.preparationMode.addEventListener("change", updatePreparationModeControls);
updatePreparationModeControls();
for (const asset of imageAssets) {
  asset.input.addEventListener("change", () => { void uploadImageAsset(asset); });
  asset.remove.addEventListener("click", () => { void deleteImageAsset(asset); });
  updateImageAssetControls(asset);
}
for (const asset of scaledImages) {
  asset.scale.addEventListener("input", () => syncImageScaleFromRange(asset));
  asset.scaleNumber.addEventListener("input", () => syncImageScaleFromNumber(asset));
  asset.scaleForm.addEventListener("submit", (event) => { void saveImageScale(event, asset); });
  syncImageScaleFromRange(asset);
}
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
elements.addIdleSpeech.addEventListener("click", () => {
  const row = createIdleSpeechEntry();
  elements.idleEntries.append(row);
  updateIdleSpeechEntries();
  row.querySelector("textarea").focus();
});
elements.aiForm.addEventListener("submit", (event) => { event.preventDefault(); saveConfig("ai", elements.aiForm, elements.saveAi, elements.aiStatus, elements.aiError); });
elements.ttsForm.addEventListener("submit", (event) => { event.preventDefault(); saveConfig("tts", elements.ttsForm, elements.saveTts, elements.ttsStatus, elements.ttsError); });
if (!token) {
  setMessage(elements.operationStatus, elements.operationError, "管理用トークンが指定されていません。", true);
  setMessage(elements.drawingStatus, elements.drawingError, "手ブレ補正を変更するには管理用トークンが必要です。", true);
  setMessage(elements.vrmStatus, elements.vrmError, "VRMモデルを変更するには管理用トークンが必要です。", true);
  setMessage(elements.displayStatus, elements.displayError, "表示設定を変更するには管理用トークンが必要です。", true);
  setMessage(elements.musicStatus, elements.musicError, "BGMを変更するには管理用トークンが必要です。", true);
  [...elements.eventForm.elements, ...elements.aiForm.elements, ...elements.ttsForm.elements, ...elements.preparationModeForm.elements, ...elements.drawingForm.elements, ...elements.vrmForm.elements, ...elements.brightnessForm.elements, ...elements.antialiasForm.elements, ...elements.musicForm.elements, ...elements.musicVolumeForm.elements, elements.reload, elements.checkUpdate, elements.clearTtsCache].forEach((element) => { element.disabled = true; });
} else {
  connect();
  loadVersion();
  loadConfig();
  loadMotionSettings();
  loadVenueSettings();
  loadEventAccess().catch((error) => { console.error(error); setMessage(elements.eventAccessStatus, elements.eventAccessError, error.message || "公開URLを読み込めませんでした。", true); });
  loadDisplayConfig().catch((error) => { console.error(error); setMessage(elements.displayStatus, elements.displayError, error.message || "現在の表示設定を確認できませんでした。", true); });
}
window.addEventListener("beforeunload", () => { clearTimeout(reconnectTimer); socket?.close(); stopPreview(); releaseEventQrImage(); elements.currentMusic.pause(); });
