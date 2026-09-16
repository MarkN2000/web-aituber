import { motionFileName } from './debug.js?v=3';

const MOTION_GROUPS = [
  ['idle', '待機', '会話をしていない間に再生します。'],
  ['neutral', '通常（neutral）', '通常の感情で発話するときの動きです。待機とは別に設定します。'],
  ['happy', '喜び', ''], ['sad', '悲しみ', ''], ['angry', '怒り', ''], ['surprised', '驚き', ''],
  ['food', '食事', '食事専用の動きを1つ設定します。未設定の場合は食事投稿を受け付けません。'],
];

export function secondsToMilliseconds(value) {
  if (!/^\d+(?:\.\d{1,3})?$/.test(value)) throw new Error('時間は0以上、秒単位で小数点以下3桁まで入力してください。');
  const milliseconds = Math.round(Number(value) * 1000);
  if (!Number.isSafeInteger(milliseconds)) throw new Error('時間が大きすぎます。');
  return milliseconds;
}

export function initMotionSettings({ token, adminUrl, readError, setMessage }) {
  const form = document.querySelector('#motion-settings-form');
  const fields = document.querySelector('#motion-settings-fields');
  const container = document.querySelector('#motion-groups');
  const reload = document.querySelector('#reload-motion-settings');
  const status = document.querySelector('#motion-settings-status');
  const error = document.querySelector('#motion-settings-error');
  const consume = document.querySelector('#food-consume-seconds');
  const duration = document.querySelector('#food-duration-seconds');
  const groups = new Map();
  let files = new Map();
  let loaded = false;
  let busy = false;

  function setBusy(value) {
    busy = value;
    fields.disabled = busy || !loaded || !token;
    reload.disabled = busy || !token;
  }

  function changed() { setMessage(status, error, '未保存の変更があります。「モーション設定を保存」で反映します。'); }

  function fillOptions(select, selected = '') {
    select.replaceChildren(new Option('ファイルを選択してください', ''));
    for (const [url, name] of files) select.add(new Option(name, url));
    select.value = selected;
    select.title = selected;
  }

  function addCandidate(group, url) {
    if (group.key === 'food') group.list.replaceChildren();
    if ([...group.list.querySelectorAll('select')].some((select) => select.value === url)) return;
    const row = document.createElement('div');
    row.className = 'motion-candidate';
    const select = document.createElement('select');
    select.setAttribute('aria-label', `${group.label}のモーション`);
    select.required = true;
    fillOptions(select, url);
    select.addEventListener('change', () => { select.title = select.value; changed(); });
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'secondary-button';
    remove.textContent = '候補から外す';
    remove.setAttribute('aria-label', `${group.label}の候補を外す`);
    remove.addEventListener('click', () => {
      row.remove();
      group.empty.hidden = group.list.childElementCount > 0;
      group.add.focus();
      changed();
    });
    row.append(select, remove);
    group.list.append(row);
    group.empty.hidden = true;
  }

  async function upload(group, input) {
    const file = input.files[0];
    if (!file || busy) return;
    setBusy(true);
    setMessage(status, error, `${group.label}のモーションをアップロード中…`);
    try {
      if (!/\.vrma$/i.test(file.name) || file.size > 100 * 1024 * 1024) {
        throw new Error('100MiB以下の.vrmaファイルを選択してください。');
      }
      const body = new FormData();
      body.append('motion', file);
      const response = await fetch(adminUrl('/api/admin/motions'), { method: 'POST', body });
      if (!response.ok) throw new Error(await readError(response, 'モーションをアップロードできませんでした。'));
      const uploaded = await response.json();
      files.set(uploaded.url, uploaded.name);
      for (const select of container.querySelectorAll('select')) fillOptions(select, select.value);
      addCandidate(group, uploaded.url);
      changed();
    } catch (failure) {
      setMessage(status, error, failure.message, true);
    } finally {
      input.value = '';
      setBusy(false);
    }
  }

  for (const [key, label, help] of MOTION_GROUPS) {
    const section = document.createElement('section');
    section.className = 'display-setting-section motion-group';
    const heading = document.createElement('h3');
    heading.id = `motion-${key}-title`;
    heading.textContent = label;
    section.setAttribute('aria-labelledby', heading.id);
    section.append(heading);
    if (help) {
      const note = document.createElement('p');
      note.className = 'admin-help';
      note.textContent = help;
      section.append(note);
    }
    const list = document.createElement('div');
    const empty = document.createElement('p');
    empty.className = 'admin-help';
    empty.textContent = '未設定';
    const toolbar = document.createElement('div');
    toolbar.className = 'motion-candidate';
    const picker = document.createElement('select');
    picker.setAttribute('aria-label', `${label}へ登録するファイル`);
    const add = document.createElement('button');
    add.type = 'button';
    add.className = 'secondary-button';
    add.textContent = key === 'food' ? '選択した動きに変更' : '候補を追加';
    const uploadLabel = document.createElement('label');
    uploadLabel.htmlFor = `upload-motion-${key}`;
    uploadLabel.textContent = 'VRMAをアップロードして登録（100MiB以下）';
    const input = document.createElement('input');
    input.type = 'file';
    input.id = uploadLabel.htmlFor;
    input.accept = '.vrma';
    const group = { key, label, list, empty, picker, add };
    groups.set(key, group);
    add.addEventListener('click', () => {
      if (!picker.value) {
        setMessage(status, error, '追加するファイルを選択してください。', true);
        picker.focus();
        return;
      }
      addCandidate(group, picker.value);
      changed();
    });
    input.addEventListener('change', () => upload(group, input));
    toolbar.append(picker, add);
    section.append(list, empty, toolbar, uploadLabel, input);
    container.append(section);
  }

  async function load() {
    if (!token || busy) return;
    setBusy(true);
    setMessage(status, error, 'モーション設定を読み込み中…');
    try {
      const response = await fetch(adminUrl('/api/admin/motions'), { cache: 'no-store' });
      if (!response.ok) throw new Error(await readError(response, 'モーション設定を読み込めませんでした。'));
      const config = await response.json();
      files = new Map(config.files.map((file) => [file.url, file.name]));
      const urls = [...config.idle_motions, ...Object.values(config.emotion_motions).flat(), config.food_motion?.url];
      for (const url of urls.filter((url) => url?.trim())) {
        if (!files.has(url)) files.set(url, `${motionFileName(url) || url}（現在の設定）`);
      }
      for (const [key, group] of groups) {
        group.list.replaceChildren();
        group.empty.hidden = false;
        fillOptions(group.picker);
        const candidates = key === 'idle' ? config.idle_motions
          : key === 'food' ? [config.food_motion?.url].filter((url) => url?.trim()) : config.emotion_motions[key] || [];
        for (const url of candidates) addCandidate(group, url);
      }
      consume.value = String((config.food_motion?.consume_at_ms ?? 3505) / 1000);
      duration.value = String((config.food_motion?.duration_ms ?? 14440) / 1000);
      loaded = true;
      setMessage(status, error, '保存済みのモーション設定を読み込みました。');
    } catch (failure) {
      setMessage(status, error, failure.message, true);
    } finally {
      setBusy(false);
    }
  }

  form.addEventListener('input', changed);
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    if (busy || !loaded || !token || !form.reportValidity()) return;
    setBusy(true);
    try {
      const candidates = (key) => [...new Set([...groups.get(key).list.querySelectorAll('select')].map((select) => select.value))];
      const consumeAt = secondsToMilliseconds(consume.value);
      const durationMs = secondsToMilliseconds(duration.value);
      if (durationMs - consumeAt < 400) throw new Error('演出終了は食べ物の消去開始から0.4秒後以降にしてください。');
      const settings = {
        idle_motions: candidates('idle'),
        emotion_motions: Object.fromEntries(MOTION_GROUPS.filter(([key]) => key !== 'idle' && key !== 'food').map(([key]) => [key, candidates(key)])),
        food_motion: { url: candidates('food')[0] || '', consume_at_ms: consumeAt, duration_ms: durationMs },
      };
      const response = await fetch(adminUrl('/api/admin/motions'), {
        method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(settings),
      });
      if (!response.ok) throw new Error(await readError(response, 'モーション設定を保存できませんでした。'));
      setMessage(status, error, '保存しました。メイン画面が発話・食事中の場合は、終了後に反映します。');
    } catch (failure) {
      setMessage(status, error, failure.message, true);
    } finally {
      setBusy(false);
    }
  });
  reload.addEventListener('click', load);
  setBusy(false);
  if (!token) setMessage(status, error, 'モーション設定には管理用トークンが必要です。', true);
  return load;
}
