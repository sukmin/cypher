// 빈 항목의 작업 영역: 볼 파일이나 실행할 명령을 아직 정하지 않은 항목(표·문서·명령·할 일)이 보여 주는 자리.
//
// 항목은 묻지 않고 먼저 만들고, 대상은 이 자리에서 정한다: 파일을 끌어다 놓거나, 단추로 고르거나, 경로를 적는다.
// 끌어다 놓은 파일은 drop.js가 받아 그 항목의 `takeFile(path)`로 넘긴다(빈 항목만 받는다: `wantsFile()`).

/** 적은 경로에서 앞뒤 빈칸과 감싼 따옴표를 뗀다(탐색기의 "경로로 복사"는 큰따옴표로 감싼다). */
export const cleanPath = (text) => String(text ?? '').trim().replace(/^"(.*)"$/, '$1').replace(/^'(.*)'$/, '$1').trim();

/**
 * 빈 영역을 만든다.
 *   title: 굵은 한 줄 · hint: 설명
 *   pick: { label, tip, action() } — 고르는 단추
 *   path: { placeholder, action(path) } — 경로를 적는 칸(없으면 두지 않는다)
 * → { node, focus() }
 */
export function emptyZone(make, { title, hint, pick, path }) {
  const node = make('div', 'empty-zone');
  const box = make('div', 'empty-box');
  box.append(make('div', 'empty-title', title), make('div', 'empty-hint', hint));
  let first = null;
  if (pick) {
    const row = make('div', 'empty-actions');
    const button = make('button', '', pick.label);
    button.title = pick.tip || '';
    button.onclick = () => pick.action();
    row.append(button);
    box.append(row);
    first = button;
  }
  if (path) {
    const row = make('div', 'empty-actions');
    const input = make('input', 'empty-path');
    Object.assign(input, { placeholder: path.placeholder, spellcheck: false, autocomplete: 'off' });
    const go = make('button', '', '열기');
    go.title = '적은 경로의 파일 열기 (Enter)';
    const submit = () => {
      const given = cleanPath(input.value);
      if (given) path.action(given); else input.focus();
    };
    go.onclick = submit;
    input.addEventListener('keydown', (ev) => {
      if (ev.key !== 'Enter') return;
      ev.preventDefault();
      ev.stopPropagation();
      submit();
    });
    row.append(input, go);
    box.append(row);
    first ??= input;
  }
  node.append(box);
  return { node, focus: () => first?.focus() };
}
