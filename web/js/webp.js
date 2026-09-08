let nativeSupport;
let worker;
let nextId = 0;
const pending = new Map();
const conversionError = () => new Error("画像をWebPへ変換できませんでした。もう一度お試しください。");

function nativeBlob(canvas, quality) {
  return new Promise((resolve, reject) => {
    try {
      canvas.toBlob(resolve, "image/webp", quality);
    } catch (error) {
      reject(error);
    }
  });
}

function supportsNativeWebp() {
  if (!nativeSupport) {
    const probe = document.createElement("canvas");
    probe.width = probe.height = 1;
    nativeSupport = nativeBlob(probe, 0.75)
      .then((blob) => blob?.type === "image/webp")
      .catch(() => false);
  }
  return nativeSupport;
}

function resetWorker() {
  worker?.terminate();
  worker = undefined;
  for (const request of pending.values()) request.reject(conversionError());
  pending.clear();
}

function getWorker() {
  if (!worker) {
    worker = new Worker(new URL("./webp-worker.js?v=1", import.meta.url), { type: "module" });
    worker.onmessage = ({ data }) => {
      const request = pending.get(data.id);
      if (!request) return;
      pending.delete(data.id);
      if (data.error) request.reject(conversionError());
      else request.resolve(new Blob([data.buffer], { type: "image/webp" }));
    };
    worker.onerror = (event) => {
      event.preventDefault();
      resetWorker();
    };
    worker.onmessageerror = resetWorker;
  }
  return worker;
}

export async function canvasToWebp(canvas, quality) {
  try {
    if (await supportsNativeWebp()) {
      const blob = await nativeBlob(canvas, quality);
      if (!blob || blob.type !== "image/webp") throw conversionError();
      return blob;
    }
    const image = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height);
    const encoder = getWorker();
    const id = nextId++;
    return await new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      try {
        encoder.postMessage({ id, image, quality }, [image.data.buffer]);
      } catch (error) {
        pending.delete(id);
        reject(error);
      }
    });
  } catch {
    throw conversionError();
  }
}
