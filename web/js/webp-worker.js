import createEncoder from "../vendor/jsquash-webp-1.5.0/webp_enc.js";
import { defaultOptions } from "../vendor/jsquash-webp-1.5.0/meta.js";

let encoder;
let queue = Promise.resolve();

async function getEncoder() {
  if (!encoder) {
    const response = await fetch(new URL("../vendor/jsquash-webp-1.5.0/webp_enc.wasm", import.meta.url));
    if (!response.ok) throw new Error("WebPエンコーダーを読み込めませんでした。");
    encoder = await createEncoder({ wasmBinary: await response.arrayBuffer(), noInitialRun: true });
  }
  return encoder;
}

self.onmessage = ({ data: { id, image, quality } }) => {
  // 初期化と変換を直列化し、複数画像でもWASMのメモリーを共用する。
  queue = queue.then(async () => {
    try {
      const module = await getEncoder();
      const result = module.encode(image.data, image.width, image.height, {
        ...defaultOptions,
        quality: quality * 100,
      });
      if (!result) throw new Error("WebP変換に失敗しました。");
      const buffer = result.buffer.slice(result.byteOffset, result.byteOffset + result.byteLength);
      self.postMessage({ id, buffer }, [buffer]);
    } catch {
      encoder = undefined;
      self.postMessage({ id, error: true });
    }
  });
};
