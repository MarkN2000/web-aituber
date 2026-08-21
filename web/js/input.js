import { compressImage } from "./image-compression.js";

const form = document.querySelector("#submission-form");
const text = document.querySelector("#text");
const image = document.querySelector("#image");
const submitButton = document.querySelector("#submit-button");
const status = document.querySelector("#submission-status");
const textCount = document.querySelector("#text-count");
const imageButton = document.querySelector("#image-button");
const imagePreview = document.querySelector("#image-preview");
const imageThumbnail = document.querySelector("#image-thumbnail");
const imageName = document.querySelector("#image-name");
const imageRemove = document.querySelector("#image-remove");

let isSubmitting = false;
let previewUrl;

function setStatus(message, kind = "") {
  status.textContent = message;
  status.dataset.kind = kind;
}

function updateSubmitState() {
  submitButton.disabled = isSubmitting || !text.value.trim();
  submitButton.dataset.loading = String(isSubmitting);
  if (submitButton.classList.contains("composer-send-button")) {
    const label = isSubmitting ? "質問を送信中" : "質問を送る";
    submitButton.setAttribute("aria-label", label);
    submitButton.title = label;
  }
}

function updateTextCount() {
  textCount.textContent = String(text.value.length);
  updateSubmitState();
}

function updateImagePreview() {
  if (previewUrl) {
    URL.revokeObjectURL(previewUrl);
    previewUrl = undefined;
  }

  const file = image.files[0];
  if (imageButton) {
    imageButton.dataset.selected = String(Boolean(file));
  }
  if (!imagePreview || !imageThumbnail || !imageName) return;

  imagePreview.hidden = !file;
  if (!file) {
    imageThumbnail.removeAttribute("src");
    imageName.textContent = "";
    return;
  }

  previewUrl = URL.createObjectURL(file);
  imageThumbnail.src = previewUrl;
  imageName.textContent = file.name;
}

text.addEventListener("input", updateTextCount);
image.addEventListener("change", updateImagePreview);
imageButton?.addEventListener("click", () => image.click());
imageRemove?.addEventListener("click", () => {
  image.value = "";
  updateImagePreview();
});

form.addEventListener("submit", async (event) => {
  event.preventDefault();

  if (!text.value.trim()) {
    setStatus("質問を入力してください。", "error");
    text.focus();
    return;
  }

  if (image.files.length > 1) {
    setStatus("画像は1枚だけ選択してください。", "error");
    return;
  }
  if (image.files[0]?.size > 10 * 1024 * 1024) {
    setStatus("画像は10MB以下にしてください。", "error");
    return;
  }

  isSubmitting = true;
  updateSubmitState();
  setStatus("");

  try {
    const formData = new FormData(form);
    const selectedImage = image.files[0];
    if (selectedImage) {
      const compressedImage = await compressImage(selectedImage);
      formData.set("image", compressedImage, "image.jpg");
    } else {
      formData.delete("image");
    }

    const response = await fetch("/api/submissions", {
      method: "POST",
      body: formData,
    });

    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw new Error(body.error || "送信を受け付けられませんでした。");
    }

    form.reset();
    updateTextCount();
    updateImagePreview();
    setStatus("");
  } catch (error) {
    console.error(error);
    setStatus(error.message || "送信に失敗しました。通信を確認して、もう一度お試しください。", "error");
  } finally {
    isSubmitting = false;
    updateSubmitState();
  }
});

window.addEventListener("beforeunload", () => {
  if (previewUrl) URL.revokeObjectURL(previewUrl);
});

updateTextCount();
updateImagePreview();
