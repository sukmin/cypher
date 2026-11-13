// 빠른 이동(Ctrl+Shift+K): 사이드바의 항목과 앱의 명령을 글자로 찾아 바로 간다.
//
// 항목이 많아지면 사이드바에서 눈으로 찾기 어렵다. 이름·별칭·그룹·폴더의 일부나 초성("ㄱㅈ" → "결제")을 치고
// ↑/↓로 고른 뒤 Enter를 누른다. 아무것도 치지 않으면 최근에 본 항목이 위에 오므로, 열자마자 Enter를 누르면
// 바로 전에 보던 항목으로 돌아간다.

let api = null; // { make, openPopup, entries() → [{ icon, title, note, keywords, action, tone }] }

const MAX_ROWS = 60;
const CHO = 'ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ';

/** 한글 글자를 초성으로 바꾼 글("결제 개편" → "ㄱㅈ ㄱㅍ"). 한글이 아닌 글자는 그대로 둔다. */
export function chosung(text) {
  let out = '';
  for (const ch of text) {
    const code = ch.charCodeAt(0) - 0xac00;
    out += code >= 0 && code < 11172 ? CHO[Math.floor(code / 588)] : ch;
  }
  return out;
}

const onlyChosung = (word) => [...word].every((ch) => CHO.includes(ch));

/** word의 글자들이 text에 순서대로 들어 있는지(띄엄띄엄 쳐도 찾는다: "cyp" → "c:\workspace\cypher"). */
function inOrder(word, text) {
  let at = 0;
  for (const ch of word) {
    at = text.indexOf(ch, at);
    if (at < 0) return false;
    at += ch.length;
  }
  return true;
}

/** 낱말 하나가 이름·설명에 얼마나 잘 맞는지. 0이면 맞지 않는다. */
function wordScore(word, title, rest) {
  const at = title.indexOf(word);
  if (at === 0) return 100;
  if (at > 0) return /[\s·\-_/\\.:(]/.test(title[at - 1]) ? 80 : 60;
  if (onlyChosung(word)) {
    // 초성은 띄어쓰기를 건너뛰고 맞춘다("ㅅㅁ" → "새 메모").
    const tight = (text) => chosung(text).replace(/\s+/g, '');
    const head = tight(title).indexOf(word);
    if (head === 0) return 70;
    if (head > 0) return 50;
    return tight(rest).includes(word) ? 10 : 0;
  }
  if (rest.includes(word)) return 25;
  // 띄엄띄엄 맞추기는 영문·숫자 낱말에만 쓴다(한글은 엉뚱한 것이 너무 많이 걸린다).
  if (word.length >= 2 && /^[\x21-\x7e]+$/.test(word) && inOrder(word, title)) return 30;
  return 0;
}

/**
 * 찾는 글(query)이 항목에 얼마나 잘 맞는지. 띄어 쓴 낱말은 모두 맞아야 한다. 0이면 맞지 않는다.
 * entry: { title, note, keywords }
 */
export function score(query, entry) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return 1;
  const title = (entry.title || '').toLowerCase();
  const rest = `${entry.note || ''} ${entry.keywords || ''}`.toLowerCase();
  let total = 0;
  for (const word of words) {
    const got = wordScore(word, title, rest);
    if (!got) return 0;
    total += got;
  }
  return total;
}

/** 찾는 글에 맞는 항목을 잘 맞는 순서로. 점수가 같으면 원래 순서(최근에 본 순서)를 지킨다. */
export function search(query, entries) {
  return entries
    .map((entry, index) => ({ entry, index, score: score(query, entry) }))
    .filter((hit) => hit.score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((hit) => hit.entry);
}

export function initPalette(given) {
  api = given;
}

/** 빠른 이동 창을 연다. */
export function showPalette() {
  const { make, openPopup } = api;
  const entries = api.entries();
  const input = make('input', 'palette-input');
  Object.assign(input, { spellcheck: false, autocomplete: 'off', placeholder: '항목 이름·그룹·폴더, 또는 할 일을 치세요 (초성도 됩니다: ㄱㅈ → 결제)' });
  const list = make('div', 'palette-list');
  const hint = make('p', 'note palette-hint', '↑/↓: 고르기 · Enter: 가기 · Esc: 닫기');
  let hits = [];
  let chosen = 0;

  const run = (entry) => {
    if (!entry) return;
    // 창이 완전히 닫힌 뒤에 실행한다. 창이 닫히면서 초점이 보던 항목으로 돌아가므로,
    // 그 뒤에 실행해야 명령이 옮긴 초점(찾기 줄, 프롬프트 편집기, 다른 항목)이 그대로 남는다.
    input.closest('dialog').addEventListener('close', () => entry.action(), { once: true });
    close();
  };

  const mark = () => {
    [...list.children].forEach((row, index) => row.classList.toggle('chosen', index === chosen));
    list.children[chosen]?.scrollIntoView({ block: 'nearest' });
  };

  const refresh = () => {
    hits = search(input.value.trim(), entries).slice(0, MAX_ROWS);
    chosen = 0;
    list.replaceChildren(...hits.map((entry, index) => {
      const row = make('button', `palette-row${entry.tone ? ` ${entry.tone}` : ''}`);
      row.append(make('span', 'palette-icon', entry.icon || ''), make('span', 'palette-title', entry.title), make('span', 'palette-note', entry.note || ''));
      row.onclick = () => run(entry);
      row.onmousemove = () => { if (chosen !== index) { chosen = index; mark(); } };
      return row;
    }));
    if (!hits.length) list.append(make('p', 'note', '맞는 항목이 없습니다.'));
    mark();
  };

  input.addEventListener('input', refresh);
  input.addEventListener('keydown', (ev) => {
    if (ev.isComposing) return; // 한글을 조합하는 중의 Enter·방향키는 글자 입력에 쓰인다.
    const step = { ArrowDown: 1, ArrowUp: -1, PageDown: 8, PageUp: -8 }[ev.key];
    if (step) {
      ev.preventDefault();
      if (hits.length) { chosen = Math.min(hits.length - 1, Math.max(0, chosen + step)); mark(); }
    } else if (ev.key === 'Enter') {
      ev.preventDefault();
      run(hits[chosen]);
    }
  });

  const close = openPopup('빠른 이동', [input, list, hint]);
  refresh();
  input.focus();
}
