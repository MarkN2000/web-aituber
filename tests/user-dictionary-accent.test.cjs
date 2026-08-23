const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/user-dictionary.js"), "utf8")
  .replaceAll("export function ", "function ")
  .replace("export class ", "class ");
const context = vm.createContext({});
vm.runInContext(
  `${source}\nthis.toKatakana = hiraganaToKatakana; this.splitMoras = splitPronunciationMoras; this.pitchLevels = accentPitchLevels; this.toSlider = accentTypeToSliderValue; this.toAccent = sliderValueToAccentType;`,
  context,
);

test("ひらがなをカタカナへ変換する", () => {
  assert.equal(context.toKatakana("かいづか"), "カイヅカ");
  assert.equal(context.toKatakana("ゔぉーかる"), "ヴォーカル");
  assert.equal(context.toKatakana("カタカナ・漢字123"), "カタカナ・漢字123");
});

test("読みを小書きカタカナだけ直前へ連結してモーラに分ける", () => {
  const cases = [
    ["カイヅカ", ["カ", "イ", "ヅ", "カ"]],
    ["キャラクター", ["キャ", "ラ", "ク", "タ", "ー"]],
    ["ティッシュ", ["ティ", "ッ", "シュ"]],
    ["ヴォーカル", ["ヴォ", "ー", "カ", "ル"]],
    ["ァイ", ["ァ", "イ"]],
    ["ヵヶ", ["ヵ", "ヶ"]],
  ];

  for (const [pronunciation, expected] of cases) {
    assert.deepEqual(Array.from(context.splitMoras(pronunciation)), expected);
  }
});

test("平板と各アクセント位置から単語の後ろまでの簡易高低を求める", () => {
  assert.deepEqual(Array.from(context.pitchLevels(4, 0)), [0, 1, 1, 1, 1]);
  assert.deepEqual(Array.from(context.pitchLevels(4, 1)), [1, 0, 0, 0, 0]);
  assert.deepEqual(Array.from(context.pitchLevels(4, 2)), [0, 1, 0, 0, 0]);
  assert.deepEqual(Array.from(context.pitchLevels(4, 3)), [0, 1, 1, 0, 0]);
  assert.deepEqual(Array.from(context.pitchLevels(4, 4)), [0, 1, 1, 1, 0]);
});

test("平板と語末アクセントは単語の後ろの高さで区別する", () => {
  const flat = Array.from(context.pitchLevels(4, 0));
  const finalAccent = Array.from(context.pitchLevels(4, 4));

  assert.deepEqual(flat.slice(0, 4), finalAccent.slice(0, 4));
  assert.equal(flat[4], 1);
  assert.equal(finalAccent[4], 0);
});

test("スライダーの右端だけを平板へ変換する", () => {
  assert.equal(context.toSlider(4, 1), 1);
  assert.equal(context.toSlider(4, 4), 4);
  assert.equal(context.toSlider(4, 0), 5);
  assert.equal(context.toAccent(4, 1), 1);
  assert.equal(context.toAccent(4, 4), 4);
  assert.equal(context.toAccent(4, 5), 0);
});

test("存在しないアクセント位置では高低線を生成しない", () => {
  assert.deepEqual(Array.from(context.pitchLevels(0, 0)), []);
  assert.deepEqual(Array.from(context.pitchLevels(3, 4)), []);
  assert.deepEqual(Array.from(context.pitchLevels(3, -1)), []);
});
