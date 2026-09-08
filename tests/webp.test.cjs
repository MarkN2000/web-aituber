const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const test = require("node:test");
const { pathToFileURL } = require("node:url");

const root = path.join(__dirname, "../web");
const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
const source = read("js/webp.js")
  .replace("export async function", "async function")
  .replaceAll("import.meta.url", '"https://example.test/static/js/webp.js"');

function setup(type) {
  const workers = [];
  let probes = 0;
  class Worker {
    constructor(url, options) {
      assert.equal(url.pathname, "/static/js/webp-worker.js");
      assert.equal(options.type, "module");
      this.messages = [];
      workers.push(this);
    }
    postMessage(data, transfers) {
      assert.equal(transfers[0], data.image.data.buffer);
      this.messages.push(data);
    }
    terminate() { this.terminated = true; }
    reply(index, error = false) {
      this.onmessage({ data: { id: this.messages[index].id, buffer: new Uint8Array([1]).buffer, error } });
    }
  }
  const context = vm.createContext({
    Blob, URL, Worker,
    document: { createElement: () => ({ toBlob: (callback) => { probes++; callback(new Blob([], { type })); } }) },
  });
  vm.runInContext(`${source}; this.convert = canvasToWebp;`, context);
  return { convert: context.convert, workers, probes: () => probes };
}

function canvas() {
  return { width: 2, height: 1, getContext: () => ({ getImageData: () => ({
    width: 2, height: 1, data: new Uint8ClampedArray([255, 0, 0, 255, 0, 0, 0, 0]),
  }) }) };
}
const tick = () => new Promise(setImmediate);

test("WebP対応端末は判定を共有し、WASMを読み込まず品質を渡す", async () => {
  const env = setup("image/webp");
  const qualities = [];
  const native = { toBlob(callback, type, quality) { qualities.push(quality); callback(new Blob(["webp"], { type })); } };
  const results = await Promise.all([env.convert(native, 0.75), env.convert(native, 0.85)]);
  assert.deepEqual(qualities, [0.75, 0.85]);
  assert.ok(results.every((blob) => blob.type === "image/webp"));
  assert.equal(env.probes(), 1);
  assert.equal(env.workers.length, 0);
});

test("PNGを返す端末はWorkerを共有し、並行要求の結果を対応付ける", async () => {
  const env = setup("image/png");
  const first = env.convert(canvas(), 0.75);
  const second = env.convert(canvas(), 0.85);
  await tick();
  assert.equal(env.workers.length, 1);
  const worker = env.workers[0];
  assert.deepEqual(worker.messages.map((m) => m.quality), [0.75, 0.85]);
  assert.equal(worker.messages[0].image.data[7], 0);
  worker.reply(1);
  worker.reply(0);
  assert.equal((await first).type, "image/webp");
  assert.equal((await second).type, "image/webp");
});

test("Worker読み込み失敗で待機中の要求を解放し、次回は作り直す", async () => {
  const env = setup("image/png");
  const first = assert.rejects(env.convert(canvas(), 0.75), /もう一度/);
  const second = assert.rejects(env.convert(canvas(), 0.85), /もう一度/);
  await tick();
  env.workers[0].onerror({ preventDefault() {} });
  await Promise.all([first, second]);
  assert.equal(env.workers[0].terminated, true);
  const retry = env.convert(canvas(), 0.75);
  await tick();
  assert.equal(env.workers.length, 2);
  env.workers[1].reply(0);
  await retry;
});

test("変換失敗は日本語エラーとなり、次の変換を妨げない", async () => {
  const env = setup("image/png");
  const failure = assert.rejects(env.convert(canvas(), 0.75), /画像をWebPへ変換できませんでした/);
  await tick();
  env.workers[0].reply(0, true);
  await failure;
  const retry = env.convert(canvas(), 0.85);
  await tick();
  env.workers[0].reply(1);
  await retry;
});

test("Workerが実WASMで透過WebPを生成し、初期化失敗後も再試行できる", async () => {
  const vendor = "vendor/jsquash-webp-1.5.0/";
  const module = await import(pathToFileURL(path.join(root, vendor, "webp_enc.js")));
  const meta = await import(pathToFileURL(path.join(root, vendor, "meta.js")));
  const wasm = fs.readFileSync(path.join(root, vendor, "webp_enc.wasm"));
  const replies = [];
  let fetches = 0;
  const context = vm.createContext({
    URL, createEncoder: module.default, defaultOptions: meta.defaultOptions,
    fetch: async () => ({ ok: ++fetches > 1, arrayBuffer: async () => wasm }),
    self: { postMessage: (data) => replies.push(data) },
  });
  const workerSource = read("js/webp-worker.js")
    .replace(/^import .*;\n/gm, "")
    .replaceAll("import.meta.url", '"https://example.test/static/js/webp-worker.js"');
  vm.runInContext(workerSource, context);
  const image = { width: 256, height: 256, data: new Uint8ClampedArray(256 * 256 * 4) };
  image.data.set([255, 0, 0, 255]);
  context.self.onmessage({ data: { id: 0, image, quality: 0.75 } });
  await vm.runInContext("queue", context);
  assert.equal(replies[0].error, true);
  for (let id = 1; id <= 2; id++) context.self.onmessage({ data: { id, image, quality: 0.85 } });
  await vm.runInContext("queue", context);
  assert.equal(fetches, 2);
  assert.deepEqual(replies.map((r) => r.id), [0, 1, 2]);
  for (const reply of replies.slice(1)) {
    const bytes = Buffer.from(reply.buffer);
    assert.equal(bytes.toString("ascii", 0, 4), "RIFF");
    assert.equal(bytes.toString("ascii", 8, 12), "WEBP");
    assert.equal(bytes.readUInt32LE(4), bytes.length - 8);
    assert.equal(bytes.toString("ascii", 12, 16), "VP8X");
    assert.ok(bytes[20] & 0x10, "透過フラグが立つ");
    assert.equal(bytes.readUIntLE(24, 3) + 1, image.width);
    assert.equal(bytes.readUIntLE(27, 3) + 1, image.height);
    assert.ok(bytes.includes(Buffer.from("ALPH")), "アルファチャンネルを含む");
  }
});
