export function isDebugEnabled(search) {
  return new URLSearchParams(search).has('debug');
}

export function motionFileName(url, baseUrl = window.location.href) {
  const name = new URL(url, baseUrl).pathname.split('/').pop();
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}

export function renderDebugState(element, state) {
  const connection = {
    connecting: '接続中',
    connected: '接続済み',
    reconnecting: '再接続中',
  }[state.connection];
  const motionName = state.motionFileName || 'なし';
  const motionKind = state.motionKind === 'idle'
    ? '待機'
    : state.motionKind === 'emotion' ? '感情' : 'なし';
  const expressionSupport = {
    base: '基本状態',
    supported: 'あり',
    unsupported: '未対応',
  }[state.expressionSupport];
  const foodAction = {
    none: 'なし',
    loading: '画像読込中',
    displaying: '表示中',
    consuming: '消費中',
    failed: '読込失敗',
  }[state.foodAction];
  element.textContent = [
    `接続: ${connection}`,
    `モーション: ${motionName}`,
    `種別: ${motionKind}`,
    `要求表情: ${state.expression}`,
    `表情対応: ${expressionSupport}`,
    `食事動作: ${foodAction}`,
  ].join('\n');
  element.hidden = false;
}
