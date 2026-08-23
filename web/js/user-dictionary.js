const WORD_TYPE_LABELS = {
  PROPER_NOUN: "固有名詞",
  COMMON_NOUN: "普通名詞",
  VERB: "動詞",
  ADJECTIVE: "形容詞",
  SUFFIX: "接尾辞",
};

export class UserDictionaryEditor {
  constructor({ token, engineUrl, adminUrl, readError }) {
    this.token = token;
    this.engineUrl = engineUrl;
    this.adminUrl = adminUrl;
    this.readError = readError;
    this.loadedEngineUrl = undefined;
    this.words = new Map();
    this.editingUuid = undefined;
    this.busy = false;
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
      wordType: document.querySelector("#tts-user-dictionary-word-type"),
      priority: document.querySelector("#tts-user-dictionary-priority"),
      priorityValue: document.querySelector("#tts-user-dictionary-priority-value"),
      cancel: document.querySelector("#cancel-tts-user-dictionary-word"),
      save: document.querySelector("#save-tts-user-dictionary-word"),
      empty: document.querySelector("#tts-user-dictionary-empty"),
      list: document.querySelector("#tts-user-dictionary-list"),
    };
    this.elements.load.addEventListener("click", () => this.load());
    this.elements.add.addEventListener("click", () => this.openEditor());
    this.elements.cancel.addEventListener("click", () => this.closeEditor());
    this.elements.form.addEventListener("submit", (event) => this.save(event));
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
    this.elements.form.hidden = false;
    this.updateControls();
    this.elements.surface.focus();
  }

  closeEditor() {
    this.editingUuid = undefined;
    this.elements.form.hidden = true;
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
