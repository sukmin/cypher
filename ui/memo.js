// 메모 항목: 글자 편집기. mome(메모장 대체 편집기)의 기능을 화면용으로 옮겼다.
// 찾기·바꾸기(대/소문자·단어 단위·정규식), 줄로 이동, 시간/날짜, 자동 줄 바꿈, 확대/축소,
// 상태 표시줄(줄/열·글자 수·원고지 매수·줄 끝·인코딩), JSON 정리, 보이지 않는 문자 찾기·정리,
// 파일 열기·저장(인코딩과 줄 끝 유지), 쓰던 내용 임시 저장.

let api = null; // { invoke, make, ask, setStatus, baseName, showMenu, openMemo }

export function initMemo(given) {
  api = given;
}

const WRAP_KEY = 'cypher.memo.wrap', ZOOM_KEY = 'cypher.memo.zoom';
const DEFAULT_SIZE = 14, MIN_SIZE = 8, MAX_SIZE = 40;
const MAX_MARK_TEXT = 400_000;  // 이보다 긴 글은 본문 표시(강조)를 하지 않는다.
const MAX_MARKS = 5000;
const MAX_MATCHES = 50_000;
const ENCODINGS = ['UTF-8', 'UTF-8 BOM', 'UTF-16 LE', 'UTF-16 BE', 'ANSI (CP949)'];
/** 연결된 파일이 화면이 본 뒤에 다른 곳에서 바뀌어 저장하지 않았을 때 앱(text_save)이 돌려주는 오류 글의 머리. */
const CHANGED_ON_DISK = 'changed-on-disk';
/** 찾기 칸에서 한글 조합이 끝난 뒤 이만큼 지나도 다시 조합하지 않으면 찾은 곳을 보여 준다(큰 글에서만 쓴다). */
const REVEAL_AFTER_COMPOSE_MS = 300;
const EOLS = [['CRLF', 'Windows (CRLF)'], ['LF', 'Unix (LF)'], ['CR', 'Macintosh (CR)']];

// 눈에 안 보이거나 일반 공백과 헷갈리는 문자. 한국어 글에 흔한 전각 공백(U+3000)은 뺀다.
// (소스에 보이지 않는 문자를 직접 적지 않도록 코드 번호로 만든다.)
const INVISIBLE_RANGES = [
  [0x00, 0x08], [0x0B, 0x0C], [0x0E, 0x1F], [0x7F, 0x9F], [0xA0], [0xAD], [0x115F, 0x1160], [0x1680],
  [0x2000, 0x200F], [0x2028, 0x202F], [0x205F, 0x2064], [0x2066, 0x2069], [0x3164], [0xFEFF], [0xFFA0],
  [0xE0000, 0xE007F],
];
const BACKSLASH = String.fromCharCode(92);
const codeEscape = (code) => `${BACKSLASH}u{${code.toString(16)}}`;
const INVISIBLE = new RegExp(`[${INVISIBLE_RANGES.map(([from, to]) => (to ? `${codeEscape(from)}-${codeEscape(to)}` : codeEscape(from))).join('')}]`, 'gu');
const ZWJ = String.fromCodePoint(0x200D);
// 이모지를 잇는 결합자 앞에 올 수 있는 것: 이모지, 모양 선택자, 피부색 수정자(👩🏽‍💻).
const EMOJI_BEFORE = new RegExp(`(${BACKSLASH}p{Extended_Pictographic}|${BACKSLASH}p{Emoji_Modifier}|${codeEscape(0xFE0F)})$`, 'u');
const BLACK_FLAG = String.fromCodePoint(0x1F3F4);
const isTag = (code) => code >= 0xE0000 && code <= 0xE007F;

/** 그 자리의 태그 문자가 지역 깃발(검은 깃발 + 태그 문자들, 예: 잉글랜드 기)의 일부인지. */
function inFlag(text, index) {
  let at = index;
  while (at >= 2 && isTag(text.codePointAt(at - 2))) at -= 2; // 태그 문자는 UTF-16 두 칸짜리다.
  return text.slice(Math.max(0, at - 2), at) === BLACK_FLAG;
}

/** 보이지 않는 문자의 종류 이름과, 정리할 때 바꿔 넣을 글자. */
function classify(ch) {
  const code = ch.codePointAt(0);
  if (code === 0xA0 || code === 0x1680 || (code >= 0x2000 && code <= 0x200A) || code === 0x202F || code === 0x205F) return ['특수 공백', ' '];
  if (code === 0x2028 || code === 0x2029) return ['특수 줄 바꿈', '\n'];
  if (code === 0x3164 || code === 0x115F || code === 0x1160 || code === 0xFFA0) return ['한글 채움 문자', ' '];
  if (code === 0x200E || code === 0x200F || (code >= 0x202A && code <= 0x202E) || (code >= 0x2066 && code <= 0x2069)) return ['방향 제어 문자', ''];
  if (code >= 0xE0000) return ['태그 문자(숨은 글자)', ''];
  if (code < 0x20 || (code >= 0x7F && code <= 0x9F)) return ['제어 문자', ''];
  if (code === 0xAD) return ['소프트 하이픈', ''];
  return ['폭 없는 문자', ''];
}

/** 글에서 보이지 않는 문자를 찾는다. 이모지를 잇는 결합자(👨‍👩‍👧)는 정상 문자로 본다. */
export function findInvisible(text) {
  const found = [];
  for (const m of text.matchAll(INVISIBLE)) {
    if (m[0] === ZWJ && EMOJI_BEFORE.test(text.slice(Math.max(0, m.index - 2), m.index))) continue;
    if (isTag(m[0].codePointAt(0)) && inFlag(text, m.index)) continue;
    const [kind, replacement] = classify(m[0]);
    found.push({ start: m.index, end: m.index + m[0].length, kind, replacement });
    if (found.length >= MAX_MATCHES) break;
  }
  return found;
}

/**
 * 200자 원고지(20칸 × 10줄) 매수. 한 칸에 한 글자(공백 포함), 줄이 바뀌면 남은 칸은 채운 것으로 본다.
 * 빈 줄도 한 줄을 차지하고(문서 끝의 빈 줄은 제외), 덜 찬 장도 한 장으로 센다.
 */
export function manuscriptPages(text) {
  const lines = text.split('\n');
  while (lines.length && !lines[lines.length - 1]) lines.pop();
  let rows = 0;
  for (const line of lines) rows += Math.max(1, Math.ceil([...line].length / 20));
  return Math.ceil(rows / 10);
}

/** JSON을 공백 2칸 들여쓰기로 다시 적는다. 키 순서·숫자 표기·글자는 그대로 둔다. 형식이 틀리면 던진다. */
export function formatJson(text) {
  JSON.parse(text);
  const n = text.length;
  let out = '', depth = 0, i = 0;
  const pad = () => `\n${'  '.repeat(depth)}`;
  while (i < n) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      out += text.slice(i, j + 1);
      i = j + 1;
    } else if (c === '{' || c === '[') {
      let j = i + 1;
      while (/\s/.test(text[j])) j++;
      if (text[j] === (c === '{' ? '}' : ']')) { out += c + text[j]; i = j + 1; } // 빈 것은 한 줄로
      else { depth++; out += c + pad(); i++; }
    } else if (c === '}' || c === ']') { depth--; out += pad() + c; i++; }
    else if (c === ',') { out += `,${pad()}`; i++; }
    else if (c === ':') { out += ': '; i++; }
    else if (/\s/.test(c)) i++;
    else {
      let j = i;
      while (j < n && !/[\s,\]}:]/.test(text[j])) j++;
      out += text.slice(i, j);
      i = j;
    }
  }
  return out;
}

/** 바꿀 내용의 `$1`·`${이름}`·`$0`·`\n`·`\t`를 찾은 항목에 맞춰 푼다(정규식으로 찾을 때만). */
export function expandReplacement(template, match) {
  return template.replace(/\\([nt\\$])|\$(\$|\d{1,2}|&|\{([^}]+)\})/g, (all, escaped, ref, name) => {
    if (escaped) return escaped === 'n' ? '\n' : escaped === 't' ? '\t' : escaped;
    if (ref === '$') return '$';
    if (name) {
      // ${1}처럼 숫자를 적었으면 번호 그룹, 아니면 이름 붙인 그룹.
      if (/^\d+$/.test(name)) return Number(name) < match.length ? match[Number(name)] ?? '' : all;
      return match.groups && name in match.groups ? match.groups[name] ?? '' : all;
    }
    if (ref === '&' || ref === '0') return match[0];
    if (Number(ref) < match.length) return match[Number(ref)] ?? '';
    // 그런 번호의 그룹이 없으면 앞 한 자리만 그룹 번호로 본다($12 → 1번 그룹 + "2").
    if (ref.length === 2 && Number(ref[0]) > 0 && Number(ref[0]) < match.length) return (match[Number(ref[0])] ?? '') + ref[1];
    return all; // 가리키는 그룹이 없으면 적힌 그대로 둔다("가격 $20").
  });
}

const two = (n) => String(n).padStart(2, '0');
const CR = String.fromCharCode(13), LF = String.fromCharCode(10);
/** 줄 끝을 편집기가 쓰는 한 가지(LF)로 맞춘다(Windows 클립보드의 CRLF 등). */
const normalizeEol = (text) => text.split(CR + LF).join(LF).split(CR).join(LF);

/**
 * 메모 항목의 화면을 host 안에 만든다.
 * saved: { memo(임시 저장 번호), path(연결된 파일), dirty, open(처음에 열 파일) }
 * onChange: 제목이 바뀌었을 때 사이드바를 다시 그리게 알린다.
 */
export function createMemo(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, ask, setStatus, baseName, showMenu } = api;
  const newId = () => `m${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
  let id = saved.memo || newId();
  let path = saved.path || null;
  let encoding = 'UTF-8', eol = 'CRLF';
  let stored = null;            // 파일에 마지막으로 저장한(읽은) 내용. 파일이 없으면 null.
  let storedFormat = '';        // 그때의 인코딩·줄 끝 형식
  let lossy = false;            // 읽지 못한 글자가 있던 파일인지
  let stamp = null;             // 연결된 파일을 마지막으로 읽거나 저장했을 때의 고친 시각(밀리초). 본 적이 없으면 null.
  let changed = false;          // 그 뒤로 파일이 다른 곳에서 바뀐 것을 알고 있다(아직 다시 읽지도 덮어쓰지도 않았다).
  let diskSeq = 0;              // 저장하거나 다시 읽을 때마다 올린다(그 전에 시작한 "바뀌었는지 보기"의 결과를 버린다).
  let saving = 0;               // 저장하는 중인 횟수(그동안은 바뀌었는지 보지 않는다)
  let checking = false, started = false, disposed = false, shown = false;
  let composing = false, revealTimer = 0; // 찾기 칸에서 한글을 조합하는 중인지
  let wrap = localStorage.getItem(WRAP_KEY) !== 'off';
  let size = Number(localStorage.getItem(ZOOM_KEY)) || DEFAULT_SIZE;
  let lastTitle = '', lastStamp = null;
  let storeTimer = 0, statTimer = 0;
  let invisible = [];           // 보이지 않는 문자 위치
  const find = { open: false, replace: false, matchCase: false, word: false, regex: false, matches: [], at: -1, error: '' };

  // ── 화면 ──
  const button = (text, title, onClick, className = '') => {
    const b = make('button', className, text);
    b.title = title;
    b.onclick = onClick;
    return b;
  };
  const $title = make('span', 'doc-title');
  const $wrap = button('줄 바꿈', '자동 줄 바꿈 켜기/끄기 (Alt+Z)', () => setWrap(!wrap));
  const $json = button('{ } JSON 정리', 'JSON 들여쓰기 정리 (Ctrl+Z: 되돌리기)', tidyJson);
  const bar = make('div', 'doc-bar');
  bar.append(
    $title,
    button('열기', '파일 열기 (Ctrl+O)', () => openFile()),
    button('저장', '저장 (Ctrl+S)', () => save(false), 'memo-save'),
    button('다른 이름으로', '다른 이름으로 저장 (Ctrl+Shift+S)', () => save(true)),
    button('찾기', '찾기 (Ctrl+F) · 바꾸기 (Ctrl+R)', () => openFind(false)),
    $wrap, $json,
  );
  const $save = bar.querySelector('.memo-save');

  // 찾기·바꾸기 줄
  const $query = make('input', 'find-query');
  $query.placeholder = '찾을 내용';
  const $replace = make('input', 'find-replace');
  $replace.placeholder = '바꿀 내용';
  for (const input of [$query, $replace]) Object.assign(input, { spellcheck: false, autocomplete: 'off' });
  const toggle = (text, title, key) => {
    const b = button(text, title, () => { find[key] = !find[key]; b.classList.toggle('on', find[key]); search(true); $query.focus(); }, 'find-toggle');
    return b;
  };
  const $case = toggle('Cc', '대/소문자 구분 (Alt+C)', 'matchCase');
  const $word = toggle('W', '단어 단위 (Alt+O)', 'word');
  const $regex = toggle('.*', '정규식 (Alt+X) — ^·$는 줄마다 맞습니다. 바꿀 내용에 $1, ${이름}, $0, \\n, \\t', 'regex');
  const $count = make('span', 'find-count');
  const findRow = make('div', 'find-row');
  findRow.append(
    $query, $case, $word, $regex, $count,
    button('↑', '이전 (Shift+Enter, Shift+F3)', () => step(-1)),
    button('↓', '다음 (Enter, F3)', () => step(1)),
    button('×', '닫기 (Esc)', closeFind, 'close'),
  );
  const replaceRow = make('div', 'find-row');
  replaceRow.append(
    $replace,
    button('바꾸기', '바꾸고 다음으로 (Alt+P)', replaceOne),
    button('모두 바꾸기', '모두 바꾸기 (Alt+A)', replaceAll),
  );
  const $find = make('div', 'memo-find');
  $find.append(findRow, replaceRow);
  $find.hidden = true;

  const $back = make('div', 'memo-back');
  const $text = make('textarea', 'memo-text');
  Object.assign($text, { spellcheck: false, autocomplete: 'off' });
  $text.placeholder = '여기에 메모를 씁니다. 쓰는 대로 임시 저장되고, Ctrl+S로 파일에 저장할 수 있습니다.';
  const edit = make('div', 'memo-edit');
  edit.append($back, $text);

  const $pos = make('span', 'stat');
  const $chars = make('span', 'stat');
  const $pages = make('span', 'stat');
  $pages.title = '200자 원고지 매수 (20칸 × 10줄)';
  const $invisible = make('button', 'stat invisible');
  $invisible.title = '보이지 않거나 헷갈리는 문자 있음 · 눌러서 찾기·정리';
  $invisible.onclick = (ev) => invisibleMenu(ev);
  const $changed = make('button', 'stat invisible', '다른 곳에서 바뀜');
  $changed.title = '파일이 다른 곳에서 바뀜 · 눌러서 다시 읽기·덮어쓰기';
  $changed.hidden = true;
  $changed.onclick = () => askChanged($changed);
  const $note = make('span', 'stat note-text');
  const $eol = make('button', 'stat');
  $eol.title = '줄 끝 형식 (저장할 때 적용)';
  $eol.onclick = (ev) => showMenu(ev, EOLS.map(([value, text]) => ({ label: `${value === eol ? '● ' : ''}${text}`, action: () => { eol = value; touch(); } })));
  const $encoding = make('button', 'stat');
  $encoding.title = '인코딩 (저장할 때 적용)';
  $encoding.onclick = (ev) => showMenu(ev, ENCODINGS.map((value) => ({ label: `${value === encoding ? '● ' : ''}${value}`, action: () => { encoding = value; touch(); } })));
  const status = make('div', 'memo-status');
  status.append($pos, $chars, $pages, $invisible, $changed, $note, $eol, $encoding);

  const root = make('div', 'doc memo');
  root.append(bar, $find, edit, status);
  host.append(root);

  // ── 제목·상태 ──
  const format = () => `${encoding}/${eol}`;
  // 연결된 파일과 달라졌는지: 글이 바뀌었거나, 저장할 인코딩·줄 끝 형식을 바꿨을 때.
  const dirty = () => (path ? $text.value !== stored || format() !== storedFormat : false);

  /** 사이드바에 보일 기본 이름: 파일 이름, 없으면 첫 줄. */
  function title() {
    if (path) return baseName(path);
    const first = $text.value.split('\n').find((line) => line.trim());
    // 글자(코드 포인트) 단위로 자른다. 이모지가 반으로 잘리면 파일 이름으로 쓸 수 없다.
    return first ? `메모 · ${[...first.trim()].slice(0, 24).join('')}` : '새 메모';
  }

  function refreshTitle() {
    const now = `${dirty() ? '● ' : ''}${path || title()}`;
    $title.textContent = now;
    $title.title = path ? `${path}${dirty() ? ' (저장하지 않은 변경 있음)' : ''}` : '파일로 저장하지 않은 메모 (임시 저장됨)';
    $save.classList.toggle('on', dirty());
    const looksJson = /\.(json|canvas)$/i.test(path || '') || (!path && /^\s*[{[]/.test($text.value));
    $json.hidden = !looksJson;
    // 본 판의 시각이 바뀌었을 때도 알린다(다시 시작할 때 되살릴 내용에 들어간다).
    if (now !== lastTitle || stamp !== lastStamp) { lastTitle = now; lastStamp = stamp; onChange(); }
  }

  function refreshPosition() {
    const at = $text.selectionStart, end = $text.selectionEnd, value = $text.value;
    let line = 1;
    for (let i = value.indexOf('\n'); i !== -1 && i < at; i = value.indexOf('\n', i + 1)) line++;
    const column = at - (at ? value.lastIndexOf('\n', at - 1) : -1);
    $pos.textContent = `줄 ${line}, 열 ${column}`;
    const selected = end > at ? ` (선택 ${[...value.slice(at, end)].length}자)` : '';
    $chars.dataset.selected = selected;
    $chars.textContent = `${$chars.dataset.count || '0자'}${selected}`;
  }

  /** 글이 바뀐 뒤의 느린 계산(글자 수, 원고지, 보이지 않는 문자)은 잠깐 모았다가 한다. */
  function refreshStats() {
    const value = $text.value;
    $chars.dataset.count = `${[...value].length.toLocaleString('ko-KR')}자`;
    $chars.textContent = `${$chars.dataset.count}${$chars.dataset.selected || ''}`;
    $pages.textContent = `원고지 ${manuscriptPages(value).toLocaleString('ko-KR')}매`;
    invisible = findInvisible(value);
    $invisible.hidden = !invisible.length;
    $invisible.textContent = `보이지 않는 문자 ${invisible.length}`;
    $eol.textContent = eol;
    $encoding.textContent = encoding;
    refreshTitle();
    paint();
  }

  /** 글이나 저장 형식이 바뀌었다: 임시 저장을 예약하고 표시를 고친다. */
  function touch() {
    // 임시 저장은 미루지 않고 일정 간격으로 한다(쉬지 않고 계속 쳐도 0.7초마다 저장된다).
    if (!storeTimer) storeTimer = setTimeout(store, 700);
    clearTimeout(statTimer);
    statTimer = setTimeout(refreshStats, 120);
    refreshPosition();
    if (find.open) search(false);
  }

  /** 쓰던 내용을 임시 저장한다(파일로 저장하지 않은 메모도 다시 시작하면 돌아온다). */
  function store() {
    clearTimeout(storeTimer);
    storeTimer = 0;
    return invoke('memo_write', { id, text: $text.value }).catch((err) => setStatus(`메모 임시 저장 실패: ${err}`));
  }

  // ── 본문 표시(찾은 항목, 보이지 않는 문자) ──

  /** 편집기 뒤에 같은 글을 깔고, 표시할 구간만 색을 입힌다. */
  function paint() {
    const value = $text.value;
    const marks = [];
    if (value.length <= MAX_MARK_TEXT) {
      if (find.open) find.matches.slice(0, MAX_MARKS).forEach((m, index) => marks.push({ ...m, cls: index === find.at ? 'hit cur' : 'hit' }));
      for (const m of invisible.slice(0, MAX_MARKS)) marks.push({ ...m, cls: 'inv' });
    }
    if (!marks.length) { $back.replaceChildren(); return; }
    marks.sort((a, b) => a.start - b.start);
    const nodes = [];
    let at = 0;
    for (const mark of marks) {
      if (mark.start < at) continue; // 겹치는 표시는 앞의 것만
      if (mark.start > at) nodes.push(document.createTextNode(value.slice(at, mark.start)));
      nodes.push(make('mark', mark.cls, value.slice(mark.start, mark.end)));
      at = mark.end;
    }
    nodes.push(document.createTextNode(`${value.slice(at)} `)); // 끝의 빈 줄도 자리를 차지하게
    $back.replaceChildren(...nodes);
    syncBack();
  }

  function syncBack() {
    $back.style.width = `${$text.clientWidth}px`;
    $back.style.height = `${$text.clientHeight}px`;
    $back.scrollTop = $text.scrollTop;
    $back.scrollLeft = $text.scrollLeft;
  }

  /** 지금 항목(표시된 것)이 보이도록 편집기를 스크롤한다. 초점은 옮기지 않는다. */
  function reveal() {
    const mark = $back.querySelector('mark.cur');
    if (!mark) return;
    const top = mark.offsetTop, left = mark.offsetLeft;
    if (top < $text.scrollTop + 20 || top > $text.scrollTop + $text.clientHeight - 40) $text.scrollTop = Math.max(0, top - $text.clientHeight / 2);
    if (left < $text.scrollLeft || left > $text.scrollLeft + $text.clientWidth - 60) $text.scrollLeft = Math.max(0, left - $text.clientWidth / 2);
    syncBack();
  }

  // ── 찾기·바꾸기 ──

  function pattern() {
    const query = $query.value;
    if (!query) return null;
    // 일반 찾기에서는 정규식에서 뜻이 있는 글자만 무력화한다(`-`나 `/`를 건드리면 오히려 오류가 난다).
    let source = find.regex ? query : query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    if (find.word) source = `(?<![\\p{L}\\p{N}_])(?:${source})(?![\\p{L}\\p{N}_])`;
    return new RegExp(source, `gmu${find.matchCase ? '' : 'i'}`);
  }

  /** 찾을 내용에 맞는 곳을 모두 찾는다. `jump`이면 커서 뒤의 첫 항목으로 간다. */
  function search(jump) {
    find.matches = [];
    find.error = '';
    try {
      const regex = pattern();
      if (regex) {
        for (const m of $text.value.matchAll(regex)) {
          if (!m[0]) continue; // 빈 문자열에 맞은 것은 건너뛴다
          find.matches.push({ start: m.index, end: m.index + m[0].length, match: m });
          if (find.matches.length >= MAX_MATCHES) break;
        }
      }
    } catch (err) {
      find.error = String(err.message || err).replace(/^Invalid regular expression: /, '정규식 오류: ');
    }
    const from = $text.selectionStart;
    const next = find.matches.findIndex((m) => m.start >= from);
    find.at = find.matches.length ? (next === -1 ? 0 : next) : -1;
    showCount();
    paint();
    if (jump) select();
  }

  function showCount() {
    $count.textContent = find.error || (!$query.value ? '' : find.matches.length
      ? `${find.at + 1}/${find.matches.length}${find.matches.length >= MAX_MATCHES ? '+' : ''}` : '없음');
    $count.classList.toggle('fail', !!find.error || (!!$query.value && !find.matches.length));
    $count.title = find.error;
  }

  /** 지금 항목을 고르고 보이게 한다. */
  function select() {
    const hit = find.matches[find.at];
    if (!hit) return;
    $text.setSelectionRange(hit.start, hit.end);
    refreshPosition();
    if ($back.querySelector('mark.cur')) return reveal();
    // 글이 너무 길어 본문 표시를 하지 않는 경우: 초점을 잠깐 주어 고른 곳이 보이게 스크롤시킨다.
    // 찾기 칸에서 한글을 조합하는 중에는 하지 않는다(초점이 나갔다 오면 조합이 끊겨 자모가 흩어진다). 조합이 끝나면 그때 보여 준다.
    if (composing) return undefined;
    const focused = document.activeElement;
    $text.blur();
    $text.focus();
    if (focused && focused !== $text && root.contains(focused)) focused.focus();
  }

  function step(delta) {
    if (!find.open) openFind(false);
    if (!find.matches.length) return;
    find.at = (find.at + delta + find.matches.length) % find.matches.length;
    showCount();
    paint();
    select();
  }

  function openFind(withReplace) {
    find.open = true;
    find.replace = withReplace || (find.replace && !$find.hidden);
    $find.hidden = false;
    replaceRow.hidden = !find.replace;
    // 한 줄 안에서 고른 글자가 있으면 그것을 찾을 내용으로 한다.
    const picked = $text.value.slice($text.selectionStart, $text.selectionEnd);
    if (picked && !picked.includes('\n') && document.activeElement === $text) $query.value = picked;
    search(false);
    $query.focus();
    $query.select();
  }

  function closeFind() {
    find.open = false;
    $find.hidden = true;
    paint();
    $text.focus();
  }

  /** 편집기의 고른 부분을 바꾼다(Ctrl+Z로 되돌릴 수 있게 편집 명령으로 넣는다). 초점은 원래 자리로 돌린다. */
  function replaceSelection(start, end, text) {
    const focused = document.activeElement;
    $text.focus();
    $text.setSelectionRange(start, end);
    if (text) document.execCommand('insertText', false, text);
    else if (end > start) document.execCommand('delete');
    if (focused && focused !== $text && root.contains(focused)) focused.focus();
  }

  const replacementFor = (hit) => (find.regex ? expandReplacement($replace.value, hit.match) : $replace.value);

  function replaceOne() {
    const hit = find.matches[find.at];
    if (!hit) return;
    const text = replacementFor(hit);
    replaceSelection(hit.start, hit.end, text);
    $text.setSelectionRange(hit.start + text.length, hit.start + text.length);
    search(true); // 바꾼 자리 뒤의 다음 항목으로
  }

  function replaceAll() {
    if (!find.matches.length) return;
    // 찾은 목록은 개수 제한이 있으므로, 글 전체를 처음부터 다시 훑어 빠짐없이 바꾼다.
    const value = $text.value;
    let out = '', at = 0, count = 0;
    for (const match of value.matchAll(pattern())) {
      if (!match[0]) continue;
      out += value.slice(at, match.index) + replacementFor({ match });
      at = match.index + match[0].length;
      count++;
    }
    out += value.slice(at);
    const scroll = $text.scrollTop;
    replaceSelection(0, value.length, out);
    $text.scrollTop = scroll;
    search(false);
    setStatus(`${count.toLocaleString('ko-KR')}곳을 바꿨습니다. (Ctrl+Z로 되돌릴 수 있습니다)`);
  }

  // ── 보이지 않는 문자 ──

  function invisibleMenu(ev) {
    const counts = new Map();
    for (const m of invisible) counts.set(m.kind, (counts.get(m.kind) || 0) + 1);
    showMenu(ev, [
      { label: '다음 문자로 가기', action: nextInvisible },
      ...[...counts].map(([kind, count]) => ({ label: `${kind} ${count}개만 정리`, action: () => cleanInvisible(kind) })),
      { label: `모두 정리 (${invisible.length}개: 지우거나 일반 공백·줄 바꿈으로)`, action: () => cleanInvisible(null) },
    ]);
  }

  function nextInvisible() {
    if (!invisible.length) return;
    const hit = invisible.find((m) => m.start >= $text.selectionEnd) || invisible[0];
    // 고른 뒤에 초점을 주어야 편집기가 그 자리로 스크롤한다.
    $text.blur();
    $text.setSelectionRange(hit.start, hit.end);
    $text.focus();
    const code = $text.value.codePointAt(hit.start).toString(16).toUpperCase().padStart(4, '0');
    setStatus(`${hit.kind} U+${code}`);
  }

  function cleanInvisible(kind) {
    const targets = invisible.filter((m) => !kind || m.kind === kind);
    if (!targets.length) return;
    const value = $text.value;
    let out = '', at = 0;
    for (const m of targets) {
      out += value.slice(at, m.start) + m.replacement;
      at = m.end;
    }
    out += value.slice(at);
    const scroll = $text.scrollTop;
    replaceSelection(0, value.length, out);
    $text.scrollTop = scroll;
    setStatus(`보이지 않는 문자 ${targets.length}개를 정리했습니다. (Ctrl+Z로 되돌릴 수 있습니다)`);
  }

  // ── 편집 명령 ──

  function insert(text) {
    $text.focus();
    document.execCommand('insertText', false, text);
  }

  function insertDate() {
    const d = new Date();
    insert(`${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`);
  }

  async function goToLine() {
    const lines = $text.value.split('\n');
    const answer = await ask(`이동할 줄 번호 (1 ~ ${lines.length})`, '예: 42');
    const line = Math.min(lines.length, Math.max(1, parseInt(answer, 10)));
    if (!answer || Number.isNaN(line)) return $text.focus();
    const offset = lines.slice(0, line - 1).join('\n').length + (line > 1 ? 1 : 0);
    // 초점을 다시 주면 편집기가 커서 위치로 스크롤한다.
    $text.blur();
    $text.setSelectionRange(offset, offset);
    $text.focus();
    refreshPosition();
  }

  function tidyJson() {
    const value = $text.value;
    try {
      const out = formatJson(value);
      if (out === value) return setStatus('이미 정리된 JSON입니다.');
      replaceSelection(0, value.length, out);
      $text.setSelectionRange(0, 0);
      $text.scrollTop = 0;
      setStatus('JSON을 정리했습니다. (Ctrl+Z로 되돌릴 수 있습니다)');
    } catch (err) {
      // 어디가 틀렸는지 알려 주고 커서를 그 위치로 옮긴다.
      const at = Number(/position (\d+)/.exec(String(err.message))?.[1]);
      let where = '';
      if (Number.isFinite(at)) {
        const before = value.slice(0, at);
        where = ` (${before.split('\n').length}줄 ${at - before.lastIndexOf('\n')}열)`;
        $text.blur();
        $text.setSelectionRange(at, Math.min(value.length, at + 1));
        $text.focus();
      }
      setStatus(`JSON 형식이 틀렸습니다${where}: ${err.message}`);
    }
  }

  function setWrap(on) {
    wrap = on;
    localStorage.setItem(WRAP_KEY, on ? 'on' : 'off');
    root.classList.toggle('nowrap', !on);
    $text.wrap = on ? 'soft' : 'off';
    $wrap.classList.toggle('on', on);
    paint();
  }

  function setSize(next) {
    size = Math.min(MAX_SIZE, Math.max(MIN_SIZE, Math.round(next)));
    localStorage.setItem(ZOOM_KEY, String(size));
    root.style.setProperty('--memo-size', `${size}px`);
    setStatus(`메모 글자 크기 ${Math.round((size / DEFAULT_SIZE) * 100)}%`);
    requestAnimationFrame(paint);
  }

  // ── 파일 ──

  function load(file) {
    path = file.path;
    encoding = file.encoding;
    eol = file.eol;
    stored = file.text;
    storedFormat = format();
    stamp = file.modified ?? null;
    diskSeq++;
    setChanged(false);
    lossy = !!file.lossy;
    $note.textContent = lossy ? '읽지 못한 글자(�)가 있습니다. 이대로 저장하면 원본의 그 부분이 바뀝니다' : '';
  }

  // ── 연결된 파일이 다른 곳에서 바뀌었을 때 ──
  // 메모는 파일을 읽은(저장한) 때의 고친 시각을 기억한다. 저장할 때 그 시각을 함께 보내, 그 뒤로 다른 곳(Claude 세션,
  // 다른 편집기)에서 파일이 바뀌었으면 말없이 덮어쓰지 않고 묻는다. 고치지 않은 메모는 초점을 받거나 다시 보일 때 스스로 다시 읽는다.

  function setChanged(on) {
    changed = on;
    $changed.hidden = !on;
  }

  /** 디스크의 내용으로 다시 읽을지, 이 메모의 글로 덮어쓸지 고르게 한다(anchor: 메뉴를 띄울 자리의 단추). */
  function askChanged(anchor) {
    if (!path) return;
    const rect = anchor.getBoundingClientRect();
    showMenu({ clientX: rect.left, clientY: rect.bottom + 2 }, [
      { caption: `"${baseName(path)}" 파일이 다른 곳에서 바뀌었습니다. 이 메모에는 그 변경이 들어 있지 않습니다.` },
      { label: '다시 읽기 (디스크의 내용으로 바꾸고, 이 메모의 글은 30일 동안 보관)', action: () => reloadFromDisk(true) },
      { label: '덮어쓰기 (이 메모의 글로 저장. 다른 곳에서 고친 내용은 사라짐)', action: () => save(false, true) },
      { label: '그대로 두기', action: () => $text.focus() },
    ]);
  }

  /**
   * 연결된 파일을 다시 읽어 메모에 넣는다. 다시 읽었으면 참.
   * `discard`이면(사용자가 [다시 읽기]를 골랐다) 고치던 글을 버리고 읽는다. 버리는 글은 닫은 메모 보관 폴더에 30일 동안 남긴다.
   * 아니면(스스로 다시 읽는 경우) 그사이 사용자가 글을 고쳤을 때 읽지 않고 표시만 해 둔다.
   */
  async function reloadFromDisk(discard) {
    const watched = path;
    if (!watched) return false;
    try {
      const file = await invoke('text_open', { path: watched, start: null });
      if (disposed || watched !== path) return false;
      if (!discard && dirty()) { setChanged(true); return false; }
      const same = file.text === $text.value;
      let kept = null;
      if (discard && !same && $text.value.trim()) {
        await invoke('memo_write', { id, text: $text.value });
        kept = await invoke('memo_remove', { id, keep: true, before: true });
        if (disposed || watched !== path) return false;
      }
      load(file);
      if (!same) {
        // 보던 자리와 커서를 될 수 있는 대로 지킨다(값을 통째로 넣으면 커서가 글 끝으로 간다).
        const scroll = $text.scrollTop, at = Math.min($text.selectionStart, file.text.length);
        $text.value = file.text;
        $text.setSelectionRange(at, at);
        $text.scrollTop = scroll;
      }
      touch();
      if (kept) setStatus(`디스크의 내용으로 다시 읽었습니다: ${baseName(watched)} · 그 전의 글은 30일 동안 보관합니다: ${kept}`);
      else if (!same) setStatus(`다른 곳에서 바뀐 내용을 다시 읽었습니다: ${baseName(watched)}`);
      return true;
    } catch (err) {
      setStatus(`다시 읽지 못했습니다: ${err}`);
      return false;
    }
  }

  /** 연결된 파일이 다른 곳에서 바뀌었는지 본다. 고치지 않은 메모는 바로 다시 읽고, 고치던 메모는 표시만 해 둔다(저장할 때 묻는다). */
  async function checkDisk() {
    if (!started || disposed || !path || checking || saving) return;
    checking = true;
    const watched = path, seq = diskSeq;
    try {
      // 문서 항목이 쓰는 것과 같은 명령이다(고친 시각의 기준이 text_open이 돌려준 것과 같다).
      const now = await invoke('reader_stamp', { path: watched });
      // 그사이 저장했거나 다시 읽었으면 이 결과는 옛것이다. 없어진 파일은 그대로 둔다(저장하면 다시 만들어진다).
      if (disposed || watched !== path || seq !== diskSeq || saving || now == null || now === stamp) return;
      if (dirty()) setChanged(true);
      else await reloadFromDisk(false);
    } catch { /* 다음에 초점을 받을 때 다시 본다 */ } finally {
      checking = false;
    }
  }

  /** 파일을 연다. 이 메모에 쓰던 것이 있으면 새 메모 항목으로 연다. */
  async function openFile(given) {
    try {
      const pristine = () => !path && !$text.value;
      if (given && !pristine()) return await api.openMemo(given);
      const file = await invoke('text_open', { path: given || null, start: path ? path.slice(0, path.lastIndexOf('\\')) : item.startCwd });
      if (!file) return $text.focus();
      // 파일을 고르는 동안 이 메모에 글을 썼을 수 있으니 다시 확인한다(쓴 글을 덮어쓰지 않게).
      if (!pristine()) return await api.openMemo(file.path);
      load(file);
      $text.value = file.text;
      $text.setSelectionRange(0, 0);
      touch();
      $text.focus();
    } catch (err) {
      setStatus(`열지 못했습니다: ${err}`);
    }
  }

  /**
   * 파일에 저장한다. 연결된 파일이 없거나 `as`이면 저장할 곳을 묻는다.
   * 연결된 파일이 마지막으로 본 뒤에 다른 곳에서 바뀌었으면 저장하지 않고 묻는다. `force`이면(덮어쓰기를 골랐다) 견주지 않고 쓴다.
   */
  let saveQueue = Promise.resolve(); // 저장은 한 번에 하나씩: 앞의 저장이 끝나 본 시각이 갱신된 뒤에 다음 저장을 보낸다.
  function save(as, force = false) {
    const done = saveQueue.then(() => saveNow(as, force));
    saveQueue = done.catch(() => {});
    return done;
  }
  async function saveNow(as, force) {
    const text = $text.value;
    saving += 1;
    try {
      const target = as ? null : path;
      // 읽지 못한 글자가 있던 파일을 원본에 덮어쓰면 그 부분이 영영 바뀐다. 먼저 확인한다.
      if (target && lossy && !confirm(`"${baseName(path)}"에는 읽지 못한 글자(�)가 있습니다.\n이대로 원본에 저장하면 그 부분이 � 글자로 바뀝니다. 저장할까요?\n("취소"를 누르고 [다른 이름으로] 저장하면 원본은 그대로 남습니다)`)) return false;
      const name = path ? baseName(path) : `${title().replace(/^메모 · /, '').replace(/[\\/:*?"<>|]/g, '_') || '메모'}.txt`;
      const written = await invoke('text_save', {
        path: target, name, start: path ? path.slice(0, path.lastIndexOf('\\')) : item.startCwd, text, encoding, eol,
        // 연결된 파일에 쓸 때는 마지막으로 본 판의 시각을 함께 보낸다(본 적이 없으면 0: 파일이 이미 있으면 묻는다).
        expected: target && !force ? (stamp ?? 0) : null,
        stamped: true,
      });
      if (!written) { $text.focus(); return false; }
      path = written.path;
      stored = text;
      storedFormat = format();
      stamp = written.modified ?? null;
      diskSeq++;
      setChanged(false);
      lossy = false;
      $note.textContent = '';
      refreshTitle();
      store();
      setStatus(`저장했습니다: ${written.path}`);
      $text.focus();
      return true;
    } catch (err) {
      if (String(err).startsWith(CHANGED_ON_DISK)) {
        // 다른 곳에서 바뀐 파일이다: 저장하지 않았다. 다시 읽을지 덮어쓸지 고르게 한다.
        setChanged(true);
        setStatus(`저장하지 않았습니다: "${baseName(path)}" 파일이 다른 곳에서 바뀌었습니다. [다시 읽기]나 [덮어쓰기]를 고르세요.`);
        askChanged($save);
        return false;
      }
      setStatus(`저장 실패: ${err}`);
      return false;
    } finally {
      saving -= 1;
    }
  }

  // ── 키와 마우스 ──

  root.addEventListener('keydown', (ev) => {
    const key = ev.key.toLowerCase();
    const ctrl = ev.ctrlKey && !ev.altKey, alt = ev.altKey && !ev.ctrlKey;
    let handled = true;
    if (ctrl && !ev.shiftKey && key === 's') save(false);
    else if (ctrl && ev.shiftKey && key === 's') save(true);
    else if (ctrl && !ev.shiftKey && key === 'o') openFile();
    else if (ctrl && !ev.shiftKey && key === 'f') openFind(false);
    else if (ctrl && !ev.shiftKey && (key === 'r' || key === 'h')) openFind(true);
    else if (ev.ctrlKey && key === 'r') { /* Ctrl+Shift+R(강력 새로 고침)로 화면이 날아가지 않게 막기만 한다 */ }
    else if (ctrl && !ev.shiftKey && key === 'g') goToLine();
    else if (key === 'f3') step(ev.shiftKey ? -1 : 1);
    else if (key === 'f5') insertDate();
    else if (ctrl && (key === '=' || key === '+')) setSize(size + 1);
    else if (ctrl && key === '-') setSize(size - 1);
    else if (ctrl && key === '0') setSize(DEFAULT_SIZE);
    else if (alt && key === 'z') setWrap(!wrap);
    else if (key === 'escape' && find.open) closeFind();
    else if (find.open && alt && key === 'c') $case.click();
    else if (find.open && alt && key === 'o') $word.click();
    else if (find.open && alt && key === 'x') $regex.click();
    // 바꾸기는 바꾸기 줄이 보일 때만(찾기만 열어 둔 채 Alt+A로 찾은 것을 모두 지워 버리지 않게).
    else if (find.open && find.replace && alt && key === 'p') replaceOne();
    else if (find.open && find.replace && alt && key === 'a') replaceAll();
    else if (key === 'enter' && ev.target === $query) step(ev.shiftKey ? -1 : 1);
    else if (key === 'enter' && ev.target === $replace) replaceOne();
    else if (key === 'tab' && ev.target === $text && !ev.ctrlKey && !ev.altKey) insert('\t');
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });
  root.addEventListener('wheel', (ev) => {
    if (!ev.ctrlKey) return;
    ev.preventDefault();
    setSize(size + (ev.deltaY < 0 ? 1 : -1));
  }, { passive: false });

  $query.addEventListener('input', () => search(true));
  $text.addEventListener('input', touch);
  $text.addEventListener('blur', () => { if (storeTimer) store(); }); // 다른 곳으로 가기 전에 임시 저장을 밀어 넣는다.
  $text.addEventListener('scroll', syncBack);
  for (const name of ['keyup', 'mouseup', 'select', 'focus']) $text.addEventListener(name, refreshPosition);
  // 초점을 받을 때(다른 항목이나 다른 프로그램에 갔다 왔을 때) 연결된 파일이 그사이 바뀌었는지 본다.
  $text.addEventListener('focus', () => { checkDisk(); });
  // 찾기 칸에서 한글을 조합하는 동안에는 초점을 옮기지 않는다(select 참고). 한글은 한 글자가 끝나자마자 다음 글자의 조합이
  // 시작되므로, 조합이 끝나고 잠깐 지나도 다시 조합하지 않을 때(입력을 마쳤을 때) 찾은 곳을 보여 준다.
  $query.addEventListener('compositionstart', () => { composing = true; clearTimeout(revealTimer); });
  $query.addEventListener('compositionend', () => {
    composing = false;
    clearTimeout(revealTimer);
    revealTimer = setTimeout(() => {
      if (!composing && !disposed && find.open && document.activeElement === $query) select();
    }, REVEAL_AFTER_COMPOSE_MS);
  });
  $query.addEventListener('blur', () => { composing = false; clearTimeout(revealTimer); });
  const observer = new ResizeObserver(() => {
    syncBack();
    if ($back.firstChild) paint();
    // 가려졌다가 다시 보이면(다른 항목을 보다 돌아왔을 때) 연결된 파일이 그사이 바뀌었는지 본다.
    const now = $text.clientHeight > 0;
    if (now && !shown) checkDisk();
    shown = now;
  });
  observer.observe($text);

  // 오른쪽 클릭: 편집 메뉴
  $text.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    const picked = $text.selectionEnd > $text.selectionStart;
    showMenu(ev, [
      ...(picked ? [
        { label: '잘라내기', action: () => { $text.focus(); document.execCommand('cut'); } },
        { label: '복사', action: () => { $text.focus(); document.execCommand('copy'); } },
      ] : []),
      { label: '붙여넣기', action: async () => { const clip = await invoke('clipboard_read'); if (clip.text) insert(normalizeEol(clip.text)); } },
      { label: '모두 선택', action: () => { $text.focus(); $text.select(); refreshPosition(); } },
      { label: '찾기…', action: () => openFind(false) },
      { label: '바꾸기…', action: () => openFind(true) },
      { label: '줄로 이동…', action: goToLine },
      { label: '시간/날짜 넣기 (F5)', action: insertDate },
    ]);
  });

  // ── 시작: 쓰던 내용과 연결된 파일을 되살린다 ──

  async function start() {
    root.style.setProperty('--memo-size', `${size}px`);
    setWrap(wrap);
    let text = '';
    try {
      if (saved.open) {
        const file = await invoke('text_open', { path: saved.open, start: null });
        load(file);
        text = file.text;
      } else {
        let working = null;
        if (saved.memo) {
          try {
            working = await invoke('memo_read', { id });
          } catch (err) {
            // 읽지 못한 임시 저장본을 빈 내용으로 덮어쓰지 않도록, 이 메모는 새 번호로 시작한다(원본은 그대로 남는다).
            setStatus(`메모의 임시 저장본을 읽지 못했습니다(${id}.txt는 그대로 둡니다): ${err}`);
            id = newId();
          }
        }
        text = working ?? '';
        // 끌어다 놓은 글자로 만든 새 메모는 그 글자로 시작한다.
        if (!saved.memo && saved.text != null) text = String(saved.text);
        // 파일로 저장하지 않은 메모에서 골라 둔 인코딩·줄 끝 형식을 되살린다.
        if (saved.encoding) encoding = saved.encoding;
        if (saved.eol) eol = saved.eol;
        if (path) {
          // 지난번에 본 판의 고친 시각. 파일을 읽지 못하더라도 저장할 때 이것과 견준다.
          const seen = typeof saved.stamp === 'number' ? saved.stamp : null;
          stamp = seen;
          try {
            const file = await invoke('text_open', { path, start: null });
            load(file);
            // 지난번에 저장하지 않은 변경이 없었으면 파일의 지금 내용을 보여 준다.
            if (!saved.dirty || working == null) text = file.text;
            else {
              // 저장하지 않은 변경에는 바꿔 둔 인코딩·줄 끝 형식도 들어 있다.
              encoding = saved.encoding || encoding;
              eol = saved.eol || eol;
              // 쓰던 내용은 지난번에 본 판을 고친 것이다. 꺼져 있는 사이 파일이 다른 곳에서 바뀌었으면
              // 본 판의 시각을 그대로 두고 표시해 둔다(저장할 때 덮어쓸지 묻는다).
              if (seen != null && file.modified !== seen) { stamp = seen; setChanged(true); }
            }
          } catch (err) {
            setStatus(`연결된 파일을 읽지 못했습니다: ${err}`);
          }
        }
      }
    } catch (err) {
      setStatus(`메모를 열지 못했습니다: ${err}`);
    }
    $text.value = text;
    started = true;
    refreshStats();
    refreshPosition();
    if (text) store();
  }
  const ready = start();

  return {
    ready,
    focus: () => $text.focus(),
    insert,
    title,
    openFile,
    find: () => openFind(false),
    text: () => $text.value,
    /**
     * 같은 그룹의 Claude 세션이 글을 넣는다: 끝에 덧붙이거나(`replace`가 거짓) 통째로 바꾼다.
     * 사용자가 다른 곳에 입력하는 중일 수 있으므로 초점은 옮기지 않는다. 통째로 바꿀 때는 바꾸기 전 글을
     * 닫은 메모 보관 폴더에 30일 동안 남기고 그 경로를 돌려준다(남길 글이 없었으면 null).
     */
    write: async (given, replace) => {
      await ready;
      const added = normalizeEol(String(given));
      let kept = null;
      if (replace && $text.value.trim()) {
        await invoke('memo_write', { id, text: $text.value });
        // 닫은 메모와 다른 이름으로 보관한다("닫은 항목 다시 열기"가 이 보관본을 되찾아 가 버리지 않게).
        kept = await invoke('memo_remove', { id, keep: true, before: true });
      }
      const before = $text.value;
      // 사용자가 이 메모를 고치는 중일 수 있다: 커서·선택과 보던 자리를 지킨다(값을 통째로 넣으면 커서가 글 끝으로 간다).
      const scroll = $text.scrollTop, from = $text.selectionStart, to = $text.selectionEnd, direction = $text.selectionDirection;
      if (replace) {
        $text.value = added;
        $text.setSelectionRange(Math.min(from, added.length), Math.min(to, added.length), direction);
      } else {
        $text.setRangeText(`${before && !before.endsWith('\n') ? '\n' : ''}${added}`, before.length, before.length, 'preserve');
      }
      $text.scrollTop = scroll;
      touch();
      return kept;
    },
    /** 연결된 파일의 경로(없으면 null). 같은 파일을 연 메모가 이미 있는지 찾을 때 쓴다(main.js의 openMemo). */
    file: () => path,
    /** 다시 시작할 때 되살릴 내용. `stamp`는 연결된 파일을 마지막으로 본 때의 고친 시각이다(그 뒤로 바뀌었는지 견준다). */
    state: () => ({ memo: id, path, dirty: dirty(), encoding, eol, stamp }),
    /** 앱이 꺼질 때: 기다리던 임시 저장을 바로 한다. 돌려주는 약속은 저장이 끝나면 끝난다(기다리던 것이 없으면 값 없음). */
    flush: () => (storeTimer ? store() : undefined),
    /** 닫기 전에 부른다. 거짓을 돌려주면 닫지 않는다. 내용이 있으면 30일 동안 보관한다. */
    close: async () => {
      // 처음 읽기가 끝난 뒤에 닫는다. 끝나기 전에는 편집기가 비어 있어, 그대로 닫으면 쓰던 내용(임시 저장본)을 빈 글로 덮어쓴다.
      await ready;
      if (dirty() && !confirm(`"${baseName(path)}"에 저장하지 않은 변경이 있습니다. 저장하지 않고 닫을까요?\n(닫은 메모의 내용은 30일 동안 보관됩니다)`)) return false;
      clearTimeout(storeTimer);
      const text = $text.value;
      try {
        await invoke('memo_write', { id, text });
        const kept = await invoke('memo_remove', { id, keep: !!text.trim() && (!path || dirty()) });
        if (kept) setStatus(`닫은 메모는 30일 동안 보관합니다: ${kept}`);
      } catch (err) {
        setStatus(String(err));
      }
      return true;
    },
    dispose: () => { disposed = true; clearTimeout(storeTimer); clearTimeout(statTimer); clearTimeout(revealTimer); observer.disconnect(); },
  };
}
