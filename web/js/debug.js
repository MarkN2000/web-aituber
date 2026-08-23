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
  const motionName = state.motionFileName || 'なし';
  const motionKind = state.motionKind === 'idle'
    ? '待機'
    : state.motionKind === 'emotion' ? '感情' : 'なし';
  element.textContent = [
    `モーション: ${motionName}`,
    `種別: ${motionKind}`,
    `要求表情: ${state.expression}`,
  ].join('\n');
  element.hidden = false;
}
