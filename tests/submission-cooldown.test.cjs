const assert = require("node:assert/strict");
const test = require("node:test");
const { setImmediate: nextTurn } = require("node:timers/promises");

function pageElement() {
  return Object.assign(new EventTarget(), {
    value: "", dataset: {}, style: { setProperty() {} }, classList: { contains: () => false },
    setAttribute() {}, setPointerCapture() {}, hasPointerCapture: () => false,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 512, height: 512 }),
  });
}

function canvas() {
  const image = (width, height) => ({ data: new Uint8ClampedArray(width * height * 4) });
  const context = {
    createImageData: image, getImageData: (_x, _y, width, height) => image(width, height),
    arc() {}, beginPath() {}, clearRect() {}, drawImage() {}, fillRect() {}, lineTo() {},
    moveTo() {}, putImageData() {}, restore() {}, save() {}, stroke() {}, strokeRect() {}, translate() {},
  };
  return Object.assign(pageElement(), {
    width: 512, height: 512, getContext: () => context,
    toBlob: (callback, type) => callback(new Blob(["image"], { type })),
  });
}

for (const question of [true, false]) {
  test(`${question ? "質問" : "描画"}は送信中の編集を保持し、1秒間の再送抑止後に次の投稿を受け付ける`, async (t) => {
    const elements = new Map([["#food-canvas", canvas()], ["#color-picker", canvas()]]);
    const document = Object.assign(new EventTarget(), {
      querySelector(id) {
        if (!elements.has(id)) elements.set(id, pageElement());
        return elements.get(id);
      },
      querySelectorAll: () => [], createElement: canvas,
    });
    const timers = [];
    globalThis.document = document;
    globalThis.window = {
      location: { pathname: "/event/test/draw" },
      setTimeout(callback, delay) { timers.push({ callback, delay }); },
    };
    t.after(() => { delete globalThis.document; delete globalThis.window; });

    const posts = [];
    let accept;
    t.mock.method(globalThis, "fetch", async (url, options) => {
      if (options.method === "POST") {
        posts.push(url);
        return new Promise((resolve) => { accept = () => resolve(new Response(null, { status: 202 })); });
      }
      return Response.json({ drawing: { stabilization: 3 } });
    });
    const form = document.querySelector("#submission-form");
    const text = document.querySelector("#text");
    form.reset = () => { text.value = ""; };
    if (question) {
      // NodeのFormDataにはHTMLFormElementからの生成がない。
      const NativeFormData = globalThis.FormData;
      globalThis.FormData = class extends NativeFormData { constructor() { super(); } };
      t.after(() => { globalThis.FormData = NativeFormData; });
      await import("../web/js/input.js");
    } else {
      await import("../web/js/draw.js");
    }
    const button = document.querySelector(question ? "#submit-button" : "#submit-food");
    const fill = (value = "質問") => {
      if (question) {
        text.value = value;
        text.dispatchEvent(new Event("input"));
      } else {
        const drawing = document.querySelector("#food-canvas");
        for (const type of ["pointerdown", "pointerup"]) {
          drawing.dispatchEvent(Object.assign(new Event(type), { pointerId: 1, clientX: 20, clientY: 20, button: 0 }));
        }
      }
    };
    const submit = () => question ? form.dispatchEvent(new Event("submit")) : button.dispatchEvent(new Event("click"));
    const endpoint = question ? "/event/test/api/submissions" : "/event/test/api/food-submissions";

    fill();
    submit();
    submit();
    await nextTurn();
    assert.deepEqual(posts, [endpoint]);
    fill("次の質問");
    accept();
    await nextTurn();
    assert.equal(button.disabled, true);
    const undo = document.querySelector("#undo-canvas");
    if (question) {
      assert.equal(text.value, "次の質問");
      assert.equal(document.querySelector("#text-count").textContent, "4");
    } else {
      assert.equal(undo.disabled, false);
    }
    submit();
    await nextTurn();
    assert.deepEqual(posts, [endpoint]);
    assert.equal(timers.length, 1);
    assert.equal(timers[0].delay, 1000);
    timers[0].callback();
    assert.equal(button.disabled, false);
    if (!question) {
      undo.dispatchEvent(new Event("click"));
      assert.equal(button.disabled, false);
      assert.equal(undo.disabled, false);
      undo.dispatchEvent(new Event("click"));
      assert.equal(button.disabled, true);
      assert.equal(undo.disabled, true);
      fill();
    }
    submit();
    await nextTurn();
    assert.deepEqual(posts, [endpoint, endpoint]);
    accept();
    await nextTurn();
    timers[1].callback();
    assert.equal(button.disabled, true);
    if (question) assert.equal(text.value, "");
    else assert.equal(undo.disabled, true);
  });
}
