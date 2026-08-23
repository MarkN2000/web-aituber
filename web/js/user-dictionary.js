const WORD_TYPE_LABELS = {
  PROPER_NOUN: "固有名詞",
  COMMON_NOUN: "普通名詞",
  VERB: "動詞",
  ADJECTIVE: "形容詞",
  SUFFIX: "接尾辞",
};

const COMBINING_SMALL_KATAKANA = new Set(["ァ", "ィ", "ゥ", "ェ", "ォ", "ャ", "ュ", "ョ", "ヮ"]);
const KATAKANA_PRONUNCIATION_PATTERN = /^[ァ-ヴー]+$/u;
const ACCENT_MORA_WIDTH = 52;
const SVG_NAMESPACE = "http://www.w3.org/2000/svg";

export function hiraganaToKatakana(value) {
  return value.replace(/[ぁ-ゖ]/gu, (character) => (
    String.fromCodePoint(character.codePointAt(0) + 0x60)
  ));
}

export function splitPronunciationMoras(pronunciation) {
  const moras = [];
  for (const character of pronunciation) {
    if (COMBINING_SMALL_KATAKANA.has(character) && moras.length > 0) {
      moras[moras.length - 1] += character;
    } else {
      moras.push(character);
    }
  }
  return moras;
}

export function accentPitchLevels(moraCount, accentType) {
  if (!Number.isInteger(moraCount) || moraCount < 1
    || !Number.isInteger(accentType) || accentType < 0 || accentType > moraCount) return [];
  return Array.from({ length: moraCount + 1 }, (_, index) => {
    if (accentType === 1) return index === 0 ? 1 : 0;
    if (index === 0) return 0;
    if (accentType === 0) return 1;
    return index < accentType ? 1 : 0;
  });
}

export function accentTypeToSliderValue(moraCount, accentType) {
  return accentType === 0 ? moraCount + 1 : accentType;
}

export function sliderValueToAccentType(moraCount, sliderValue) {
  return sliderValue === moraCount + 1 ? 0 : sliderValue;
}

export class UserDictionaryEditor {
  constructor({ token, engineUrl, adminUrl, readError, stopOtherPreview }) {
    this.token = token;
    this.engineUrl = engineUrl;
    this.adminUrl = adminUrl;
    this.readError = readError;
    this.stopOtherPreview = stopOtherPreview;
    this.loadedEngineUrl = undefined;
    this.words = new Map();
    this.editingUuid = undefined;
    this.busy = false;
    this.previewAudio = undefined;
    this.previewAudioUrl = undefined;
    this.previewAbortController = undefined;
    this.elements = {
      load: document.querySelector("#load-tts-user-dictionary"),
      add: document.querySelector("#add-tts-user-dictionary-word"),
      status: document.querySelector("#tts-user-dictionary-status"),
      error: document.querySelector("#tts-user-dictionary-error"),
      excluded: document.querySelector("#tts-user-dictionary-excluded"),
      form: document.querySelector("#tts-user-dictionary-form"),
      formHeading: document.querySelector("#tts-user-dictionary-form-heading"),
      surface: document.querySelector("#tts-user-dictionary-surface"),
      pronunciation: document.querySelector("#tts-user-dictionary-pronunciation"),
      accentType: document.querySelector("#tts-user-dictionary-accent-type"),
      accentEmpty: document.querySelector("#tts-user-dictionary-accent-empty"),
      accentPicker: document.querySelector("#tts-user-dictionary-accent-picker"),
      accentSlider: document.querySelector("#tts-user-dictionary-accent-slider"),
      accentTrack: document.querySelector("#tts-user-dictionary-accent-track"),
      accentDiagram: document.querySelector("#tts-user-dictionary-accent-diagram"),
      accentLabels: document.querySelector("#tts-user-dictionary-accent-labels"),
      wordType: document.querySelector("#tts-user-dictionary-word-type"),
      priority: document.querySelector("#tts-user-dictionary-priority"),
      priorityValue: document.querySelector("#tts-user-dictionary-priority-value"),
      cancel: document.querySelector("#cancel-tts-user-dictionary-word"),
      preview: document.querySelector("#preview-tts-user-dictionary-word"),
      save: document.querySelector("#save-tts-user-dictionary-word"),
      speakerList: document.querySelector("#tts-speaker-list"),
      empty: document.querySelector("#tts-user-dictionary-empty"),
      list: document.querySelector("#tts-user-dictionary-list"),
    };
    this.elements.load.addEventListener("click", () => this.load());
    this.elements.add.addEventListener("click", () => this.openEditor());
    this.elements.cancel.addEventListener("click", () => this.closeEditor());
    this.elements.preview.addEventListener("click", () => this.preview());
    this.elements.form.addEventListener("submit", (event) => this.save(event));
    this.elements.pronunciation.addEventListener("input", (event) => {
      if (!event.isComposing) {
        this.elements.pronunciation.value = hiraganaToKatakana(this.elements.pronunciation.value);
      }
      this.renderAccentPicker();
    });
    this.elements.accentSlider.addEventListener("input", () => this.updateAccentFromSlider());
    this.elements.priority.addEventListener("input", () => this.updatePriorityLabel());
    for (const field of this.elements.form.elements) {
      field.addEventListener("invalid", () => field.setAttribute("aria-invalid", "true"));
      field.addEventListener("input", () => field.setAttribute("aria-invalid", String(!field.validity.valid)));
    }
    if (!token) {
      this.setMessage("ユーザー辞書を編集するには管理用トークンが必要です。", true);
    }
    this.updateControls();
  }

  setMessage(message = "", isError = false) {
    this.elements.status.textContent = isError ? "" : message;
    this.elements.error.textContent = isError ? message : "";
  }

  updatePriorityLabel() {
    this.elements.priorityValue.value = this.elements.priority.value;
    this.elements.priorityValue.textContent = this.elements.priority.value;
  }

  updateAccentFromSlider() {
    const moraCount = Number(this.elements.accentSlider.max) - 1;
    const sliderValue = Number(this.elements.accentSlider.value);
    this.elements.accentType.value = String(sliderValueToAccentType(moraCount, sliderValue));
    this.renderAccentPicker();
  }

  renderAccentPicker() {
    const pronunciation = this.elements.pronunciation.value;
    const isValid = KATAKANA_PRONUNCIATION_PATTERN.test(pronunciation);
    this.elements.accentDiagram.replaceChildren();
    this.elements.accentLabels.replaceChildren();
    this.elements.accentEmpty.hidden = isValid;
    this.elements.accentPicker.hidden = !isValid;
    if (!isValid) {
      this.elements.accentEmpty.textContent = pronunciation
        ? "全角カタカナで入力すると選択できます。"
        : "読みを入力すると選択できます。";
      return;
    }

    const moras = splitPronunciationMoras(pronunciation);
    let accentType = Number(this.elements.accentType.value);
    if (!Number.isInteger(accentType) || accentType < 0 || accentType > moras.length) {
      accentType = 0;
      this.elements.accentType.value = "0";
    }
    const sliderMax = moras.length + 1;
    this.elements.accentSlider.max = String(sliderMax);
    this.elements.accentSlider.value = String(accentTypeToSliderValue(moras.length, accentType));

    const levels = accentPitchLevels(moras.length, accentType);
    const trackWidth = levels.length * ACCENT_MORA_WIDTH;
    const svg = document.createElementNS(SVG_NAMESPACE, "svg");
    svg.classList.add("tts-user-dictionary-accent-line");
    svg.setAttribute("viewBox", `0 0 ${trackWidth} 44`);
    svg.setAttribute("width", String(trackWidth));
    svg.setAttribute("height", "44");
    svg.setAttribute("aria-hidden", "true");
    const pointCoordinates = levels.map((level, index) => ({
      x: (index * ACCENT_MORA_WIDTH) + (ACCENT_MORA_WIDTH / 2),
      y: level === 1 ? 10 : 34,
    }));
    const line = document.createElementNS(SVG_NAMESPACE, "polyline");
    line.setAttribute("points", pointCoordinates.map(({ x, y }) => `${x},${y}`).join(" "));
    svg.append(line);
    for (const [index, { x, y }] of pointCoordinates.entries()) {
      const point = document.createElementNS(SVG_NAMESPACE, "circle");
      point.setAttribute("cx", String(x));
      point.setAttribute("cy", String(y));
      point.setAttribute("r", index === pointCoordinates.length - 1 ? "4" : "5");
      if (index === pointCoordinates.length - 1) point.classList.add("is-after-word");
      svg.append(point);
    }

    this.elements.accentLabels.style.gridTemplateColumns = `repeat(${levels.length}, ${ACCENT_MORA_WIDTH}px)`;
    for (const mora of moras) {
      const label = document.createElement("span");
      label.textContent = mora;
      this.elements.accentLabels.append(label);
    }
    const flatLabel = document.createElement("span");
    flatLabel.className = "is-flat";
    flatLabel.textContent = "平板";
    this.elements.accentLabels.append(flatLabel);
    this.elements.accentTrack.style.width = `${trackWidth}px`;
    this.elements.accentDiagram.append(svg);
  }

  updateControls() {
    const loaded = Boolean(this.loadedEngineUrl) && this.loadedEngineUrl === this.engineUrl.value.trim();
    this.elements.load.disabled = this.busy || !this.token;
    this.elements.add.disabled = this.busy || !loaded;
    for (const button of this.elements.list.querySelectorAll("button")) button.disabled = this.busy || !loaded;
    for (const field of this.elements.form.elements) field.disabled = this.busy || !loaded;
  }

  invalidate(message = "エンジンURLを変更しました。辞書を読み直してください。") {
    this.loadedEngineUrl = undefined;
    this.words.clear();
    this.releasePreview();
    this.closeEditor();
    this.elements.list.replaceChildren();
    this.elements.empty.hidden = false;
    this.elements.empty.textContent = "辞書を読み込むと、編集できる単語が表示されます。";
    this.elements.excluded.hidden = true;
    if (this.token) this.setMessage(message);
    this.updateControls();
  }

  async load(successMessage) {
    if (!this.token || !this.engineUrl.reportValidity()) return;
    const engineUrl = this.engineUrl.value.trim();
    this.busy = true;
    this.elements.load.textContent = "読み込み中…";
    this.setMessage();
    this.updateControls();
    try {
      const response = await fetch(this.adminUrl("/api/admin/tts-user-dict"), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ engine_url: engineUrl }),
      });
      if (!response.ok) throw new Error(await this.readError(response, "ユーザー辞書を取得できませんでした。"));
      const result = await response.json();
      if (!Array.isArray(result.words)) throw new Error("ユーザー辞書の応答形式が不正です。");
      if (this.engineUrl.value.trim() !== engineUrl) return;
      this.loadedEngineUrl = engineUrl;
      this.words = new Map(result.words.map((word) => [word.uuid, word]));
      this.closeEditor();
      this.render();
      this.elements.excluded.hidden = !result.has_excluded_words;
      this.setMessage(successMessage || `${result.words.length}件の単語を読み込みました。`);
    } catch (error) {
      console.error(error);
      if (this.engineUrl.value.trim() === engineUrl) this.invalidate();
      this.setMessage(error.message || "ユーザー辞書を取得できませんでした。", true);
    } finally {
      this.busy = false;
      this.elements.load.textContent = "辞書を読み込む";
      this.updateControls();
    }
  }

  render() {
    this.elements.list.replaceChildren();
    for (const word of this.words.values()) this.elements.list.append(this.wordCard(word));
    this.elements.empty.hidden = this.words.size > 0;
    this.elements.empty.textContent = "編集できる単語は登録されていません。";
    this.updateControls();
  }

  wordCard(word) {
    const card = document.createElement("article");
    card.className = "tts-user-dictionary-word";
    const content = document.createElement("div");
    content.className = "tts-user-dictionary-word-content";
    const surface = document.createElement("h4");
    surface.textContent = word.surface;
    const pronunciation = document.createElement("p");
    pronunciation.textContent = word.pronunciation;
    const details = document.createElement("p");
    details.className = "tts-user-dictionary-word-details";
    const accent = word.accent_type === 0 ? "平板" : `アクセント ${word.accent_type}`;
    details.textContent = `${WORD_TYPE_LABELS[word.word_type] || word.word_type} ・ ${accent} ・ 優先度 ${word.priority}`;
    content.append(surface, pronunciation, details);

    const actions = document.createElement("div");
    actions.className = "tts-user-dictionary-word-actions";
    const edit = document.createElement("button");
    edit.type = "button";
    edit.className = "secondary-button";
    edit.textContent = "編集";
    edit.addEventListener("click", () => this.openEditor(word));
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "secondary-button danger-button";
    remove.textContent = "削除";
    remove.addEventListener("click", () => this.remove(word));
    actions.append(edit, remove);
    card.append(content, actions);
    return card;
  }

  openEditor(word) {
    if (!this.loadedEngineUrl || this.loadedEngineUrl !== this.engineUrl.value.trim()) return;
    this.editingUuid = word?.uuid;
    this.elements.formHeading.textContent = word ? "単語を編集" : "単語を追加";
    this.elements.surface.value = word?.surface || "";
    this.elements.pronunciation.value = word?.pronunciation || "";
    this.elements.accentType.value = String(word?.accent_type ?? 0);
    this.elements.wordType.value = word?.word_type || "PROPER_NOUN";
    this.elements.priority.value = String(word?.priority ?? 5);
    this.elements.save.textContent = word ? "変更を保存" : "辞書に追加";
    for (const field of this.elements.form.elements) field.removeAttribute("aria-invalid");
    this.updatePriorityLabel();
    this.renderAccentPicker();
    this.elements.form.hidden = false;
    this.updateControls();
    this.elements.surface.focus();
  }

  closeEditor() {
    this.editingUuid = undefined;
    this.releasePreview();
    this.elements.form.hidden = true;
  }

  releasePreview() {
    this.previewAbortController?.abort();
    this.previewAbortController = undefined;
    this.previewAudio?.pause();
    this.previewAudio = undefined;
    if (this.previewAudioUrl) URL.revokeObjectURL(this.previewAudioUrl);
    this.previewAudioUrl = undefined;
  }

  async preview() {
    if (!this.elements.form.reportValidity() || !this.isCurrentEngine()) return;
    const speakerId = Number(this.elements.speakerList.value);
    if (!this.elements.speakerList.value || !Number.isInteger(speakerId) || speakerId < 0) {
      this.setMessage("話者一覧を取得して、試聴する話者を選択してください。", true);
      return;
    }
    this.busy = true;
    this.elements.preview.textContent = "試聴を準備中…";
    this.setMessage();
    this.updateControls();
    this.stopOtherPreview();
    this.releasePreview();
    const controller = new AbortController();
    this.previewAbortController = controller;
    try {
      const response = await fetch(this.adminUrl("/api/admin/tts-user-dict-preview"), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        signal: controller.signal,
        body: JSON.stringify({
          tts: { engine_url: this.loadedEngineUrl, speaker_id: speakerId },
          pronunciation: this.elements.pronunciation.value.trim(),
          accent_type: Number(this.elements.accentType.value),
        }),
      });
      if (!response.ok) throw new Error(await this.readError(response, "ユーザー辞書を試聴できませんでした。"));
      const blob = await response.blob();
      if (this.previewAbortController !== controller) return;
      this.previewAudioUrl = URL.createObjectURL(blob);
      this.previewAudio = new Audio(this.previewAudioUrl);
      this.previewAudio.addEventListener("ended", () => this.releasePreview(), { once: true });
      this.previewAudio.addEventListener("error", () => {
        this.releasePreview();
        this.setMessage("試聴音声を再生できませんでした。", true);
      }, { once: true });
      await this.previewAudio.play();
      if (this.previewAbortController !== controller) return;
      this.setMessage("入力中の読みとアクセントで試聴しています。辞書には保存されていません。");
    } catch (error) {
      if (error.name === "AbortError") return;
      console.error(error);
      this.releasePreview();
      this.setMessage(error.message || "ユーザー辞書を試聴できませんでした。", true);
    } finally {
      this.busy = false;
      this.elements.preview.textContent = "この読みで試聴";
      this.updateControls();
    }
  }

  requestWord() {
    return {
      engine_url: this.loadedEngineUrl,
      surface: this.elements.surface.value.trim(),
      pronunciation: this.elements.pronunciation.value.trim(),
      accent_type: Number(this.elements.accentType.value),
      word_type: this.elements.wordType.value,
      priority: Number(this.elements.priority.value),
    };
  }

  async save(event) {
    event.preventDefault();
    if (!this.elements.form.reportValidity() || !this.isCurrentEngine()) return;
    const editingUuid = this.editingUuid;
    const path = editingUuid
      ? `/api/admin/tts-user-dict-word/${encodeURIComponent(editingUuid)}`
      : "/api/admin/tts-user-dict-word";
    this.busy = true;
    this.elements.save.textContent = "保存中…";
    this.setMessage();
    this.updateControls();
    let succeeded = false;
    try {
      const response = await fetch(this.adminUrl(path), {
        method: editingUuid ? "PUT" : "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(this.requestWord()),
      });
      if (!response.ok) throw new Error(await this.readError(response, "ユーザー辞書へ保存できませんでした。"));
      succeeded = true;
    } catch (error) {
      console.error(error);
      this.setMessage(error.message || "ユーザー辞書へ保存できませんでした。", true);
    } finally {
      this.busy = false;
      this.elements.save.textContent = editingUuid ? "変更を保存" : "辞書に追加";
      this.updateControls();
    }
    if (succeeded) await this.load(editingUuid ? "単語を更新しました。" : "単語を追加しました。");
  }

  async remove(word) {
    if (!this.isCurrentEngine() || !window.confirm(`「${word.surface}」をユーザー辞書から削除しますか？`)) return;
    this.busy = true;
    this.setMessage();
    this.updateControls();
    let succeeded = false;
    try {
      const response = await fetch(this.adminUrl(`/api/admin/tts-user-dict-word/${encodeURIComponent(word.uuid)}`), {
        method: "DELETE",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ engine_url: this.loadedEngineUrl }),
      });
      if (!response.ok) throw new Error(await this.readError(response, "ユーザー辞書から削除できませんでした。"));
      succeeded = true;
    } catch (error) {
      console.error(error);
      this.setMessage(error.message || "ユーザー辞書から削除できませんでした。", true);
    } finally {
      this.busy = false;
      this.updateControls();
    }
    if (succeeded) await this.load("単語を削除しました。");
  }

  isCurrentEngine() {
    if (this.loadedEngineUrl && this.loadedEngineUrl === this.engineUrl.value.trim()) return true;
    this.invalidate();
    return false;
  }
}
