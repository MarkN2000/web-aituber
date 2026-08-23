const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

function readWebFile(relativePath) {
  return fs.readFileSync(path.join(__dirname, "../web", relativePath), "utf8");
}

test("無効URL画面はメッセージだけを表示する", () => {
  const html = readWebFile("invalid-event.html");
  assert.match(html, /このURLは使用できません。/);
  assert.doesNotMatch(html, /<button/);
  assert.doesNotMatch(html, /このイベントリンクは終了しました。/);
});

test("表示中にURLが無効になった場合は操作UIを置き換える", () => {
  const invalidEvent = readWebFile("js/invalid-event.js");
  const main = readWebFile("js/main.js");
  const endEventAccess = main.slice(
    main.indexOf("function endEventAccess"),
    main.indexOf("function receiveSegment"),
  );

  assert.match(invalidEvent, /document\.body\.replaceChildren\(message\)/);
  assert.match(endEventAccess, /showInvalidEventScreen\(\)/);
  assert.doesNotMatch(endEventAccess, /elements\.start/);
});

for (const fileName of ["input.js", "draw.js"]) {
  test(`${fileName}は無効URLで操作UIを置き換える`, () => {
    const source = readWebFile(`js/${fileName}`);
    assert.match(source, /response\.status === 404[\s\S]*showInvalidEventScreen\(\)/);
  });
}
