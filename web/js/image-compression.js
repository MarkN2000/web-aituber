const MAX_IMAGE_EDGE = 512;
const JPEG_QUALITY = 0.8;

export async function compressImage(file) {
  const sourceUrl = URL.createObjectURL(file);
  const source = new Image();

  try {
    source.src = sourceUrl;
    await source.decode();

    const longestEdge = Math.max(source.naturalWidth, source.naturalHeight);
    if (longestEdge === 0) {
      throw new Error("画像の大きさを取得できませんでした。");
    }

    const scale = Math.min(1, MAX_IMAGE_EDGE / longestEdge);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(source.naturalWidth * scale));
    canvas.height = Math.max(1, Math.round(source.naturalHeight * scale));

    const context = canvas.getContext("2d");
    if (!context) {
      throw new Error("画像を処理できませんでした。");
    }
    context.fillStyle = "#fff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(source, 0, 0, canvas.width, canvas.height);

    const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/jpeg", JPEG_QUALITY));
    if (!blob) {
      throw new Error("画像をJPEGへ変換できませんでした。");
    }
    return blob;
  } catch (error) {
    console.error("画像を圧縮できませんでした", error);
    throw new Error("選択した画像を処理できませんでした。別の画像を選んでください。");
  } finally {
    URL.revokeObjectURL(sourceUrl);
  }
}
