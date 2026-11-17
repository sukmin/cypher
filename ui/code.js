// 코드 보기 항목: 프로젝트의 코드를 읽는다(고치지는 않는다). 왼쪽에 프로젝트 나무, 오른쪽에 코드.
//
// Claude 세션이 작업한 내용을 편집기를 따로 띄우지 않고 그 자리에서 살펴보려는 것이다:
// - 문법 강조(code-lex.js), 줄 번호, 바뀐 줄 표시(git 기준: 더한 줄·고친 줄·지운 자리)
// - 정의로 가기(Ctrl+클릭, F12), 쓰는 곳 찾기(Alt+F7), 뒤로·앞으로(Alt+←/→)
// - 파일 찾기(Ctrl+P), 기호 찾기(Ctrl+T), 이 파일의 구조(Ctrl+F12), 줄로 가기(Ctrl+G)
// - 이 파일에서 찾기(Ctrl+F), 프로젝트 전체에서 찾기(Ctrl+Shift+F)
// - 파일이 바뀌면 스스로 다시 읽는다(보던 자리는 지킨다).
// - 위쪽의 글꼴 칸에서 코드의 글꼴과 글자 크기를 앱의 글꼴과 따로 정한다(Ctrl+휠, Ctrl+더하기/빼기/0).
//
// 정의와 쓰는 곳은 언어 서버 없이 글자의 모양으로 찾는다(앱의 code.rs). 그래서 이름이 같은 정의가 여럿이면
// 가까운 것(같은 파일, 같은 폴더, `자료형.이름`의 자료형이 감싼 것)을 앞에 세워 고르게 한다.
// 파일 내용은 신뢰할 수 없는 입력이다: 글자는 모두 textContent로만 넣는다.

import { tokenize, langOf, fileBadge, LANG_NAMES, wordPattern } from './code-lex.js';

let api = null; // { invoke, make, setStatus, showMenu, baseName, toClaude(item, text, ev) }

export function initCode(given) {
  api = given;
}

const CHUNK = 150;              // 한 덩이의 줄 수(덩이는 화면 가까이 왔을 때 강조해 그린다)
const MAX_HIGHLIGHT_LINES = 40000; // 이보다 긴 파일은 강조하지 않는다
const MAX_TABS = 14;            // 열어 두는 파일 수(넘으면 가장 오래 안 본 것을 닫는다)
const MAX_HISTORY = 60;         // 뒤로 가기 기록 수
const MAX_PICK = 80;            // 고르기 창에 보이는 줄 수
const MAX_FIND = 5000;          // 이 파일에서 찾기가 세는 자리 수
const MAX_MARK_LINE = 5000;     // 이보다 긴 줄에는 찾은 곳·같은 이름 표시를 걸지 않는다(지금 자리 하나만 표시)
const WATCH_MS = 2000;          // 보고 있는 파일이 바뀌었는지 살피는 간격
const GIT_MS = 6000;            // 바뀐 파일(git)과 나무를 다시 읽는 간격
const KIND_ICONS = {
  function: 'ƒ', method: 'ƒ', constructor: 'ƒ', class: 'C', interface: 'I', enum: 'E', struct: 'S', trait: 'T', type: 'T', module: 'M',
  constant: 'c', variable: 'v', field: 'f', variant: 'e', macro: '!', id: '#', selector: '.', heading: '§', section: '§',
};
const KIND_NAMES = {
  function: '함수', method: '메서드', constructor: '생성자', class: '클래스', interface: '인터페이스', enum: '열거형', struct: '구조체', trait: '트레이트',
  type: '자료형', module: '모듈', constant: '상수', variable: '변수', field: '필드', variant: '열거 값', macro: '매크로', id: 'id', selector: '선택자',
  heading: '제목', section: '구역',
};
const ROLE_NAMES = { def: '정의', use: '쓰는 곳', comment: '주석 안', string: '문자열 안', text: '찾은 곳' };

// 코드의 글꼴: 앱의 글꼴(설정 → 모양)과 따로 정한다. 이 컴퓨터에 기억하고 모든 코드 보기 항목이 함께 쓴다.
const FONT_KEY = 'cypher.code.font';
const FONT_SIZE = 13, FONT_MIN = 9, FONT_MAX = 28;
const CODE_FONTS = ['Cascadia Mono', 'Cascadia Code', 'D2Coding', 'Consolas', 'JetBrains Mono', 'Fira Code', '나눔고딕코딩', '굴림체', 'Lucida Console', 'Courier New'];
let codeFont = { family: '', size: FONT_SIZE };
try {
  const stored = JSON.parse(localStorage.getItem(FONT_KEY) || '{}');
  if (typeof stored?.family === 'string') codeFont.family = stored.family.slice(0, 80);
  if (Number.isFinite(stored?.size)) codeFont.size = Math.min(FONT_MAX, Math.max(FONT_MIN, Math.round(stored.size)));
} catch { /* 깨진 기록은 버린다 */ }
const fontViews = new Set(); // 글꼴이 바뀌면 다시 맞출 코드 보기 화면들(그 화면의 적용 함수)
// CSS에 적을 글꼴 이름. 식별자 꼴일 때만 그대로 적고 나머지는 따옴표로 감싼다(숫자로 시작하는 이름은 그대로 적으면 무효가 된다).
const quotedFont = (name) => (/^[A-Za-z_][\w-]*$/.test(name) ? name : `"${name.replace(/["\\]/g, '')}"`);
/**
 * 이 컴퓨터에 그 글꼴이 있는지. 같은 글을 "그 글꼴, 기본 글꼴"과 "기본 글꼴"로 재서 너비가 다르면 그 글꼴이 쓰인 것이다
 * (브라우저의 글꼴 확인 기능은 없는 글꼴에도 참을 돌려주는 경우가 있다). 알아볼 수 없으면 있는 것으로 친다.
 */
let fontProbe = null;
function hasFont(name) {
  try {
    fontProbe ??= document.createElement('canvas').getContext('2d');
    const sample = 'mmmmmmmmmmlliWW@#0123456789 한글 가나다';
    const width = (family) => { fontProbe.font = `40px ${family}`; return fontProbe.measureText(sample).width; };
    return ['monospace', 'serif', 'sans-serif'].some((base) => width(`${quotedFont(name)}, ${base}`) !== width(base));
  } catch {
    return true;
  }
}
/** 코드의 글꼴을 바꾸고 기억한다. 열려 있는 모든 코드 보기에 바로 적용한다. */
function setCodeFont(change) {
  codeFont = { ...codeFont, ...change };
  codeFont.family = String(codeFont.family || '').trim().slice(0, 80);
  codeFont.size = Math.min(FONT_MAX, Math.max(FONT_MIN, Math.round(Number(codeFont.size) || FONT_SIZE)));
  try { localStorage.setItem(FONT_KEY, JSON.stringify(codeFont)); } catch { /* 저장 공간 문제는 무시 */ }
  for (const apply of fontViews) apply();
}

// 강조 표시(찾은 곳, 같은 이름, 정의로 갈 수 있는 이름)는 화면의 글자를 고치지 않고 브라우저의 강조 기능으로 칠한다.
// 코드 항목이 여럿이면 모두의 것을 모아 한 번에 등록한다.
const painted = new Map(); // 화면(view) → { hit: Range[], cur: Range[], same: Range[], link: Range[] }
function paint() {
  const registry = globalThis.CSS?.highlights;
  if (!registry || typeof Highlight === 'undefined') return;
  for (const kind of ['hit', 'cur', 'same', 'link']) {
    const all = [...painted.values()].flatMap((marks) => marks[kind] || []);
    if (all.length) registry.set(`code-${kind}`, new Highlight(...all)); else registry.delete(`code-${kind}`);
  }
}

/** 경로를 견줄 수 있는 한 가지 모양으로: 구분자는 역슬래시, 겹친 구분자는 하나로(맨 앞의 `\\서버`는 그대로). */
const norm = (path) => String(path).replace(/\//g, '\\').replace(/(?<!^)\\{2,}/g, '\\');
const slash = (path) => String(path).replace(/\\/g, '/');
const lower = (path) => norm(path).toLowerCase();
const dirOf = (path) => { const at = norm(path).lastIndexOf('\\'); return at < 0 ? '' : norm(path).slice(0, at); };
const nameOf = (path) => norm(path).split('\\').pop();

/** 적은 글이 이름과 얼마나 잘 맞는지(작을수록 잘 맞는다. 맞지 않으면 -1): 똑같음 → 앞부분 → 낱말 머리글자 → 안에 들어 있음 → 글자가 차례로. */
function matchRank(query, name) {
  const q = query.toLowerCase(), n = name.toLowerCase();
  if (!q) return 9;
  if (q === n) return 0;
  if (n.startsWith(q)) return 1;
  const heads = [...name].filter((ch, i) => i === 0 || /[A-Z]/.test(ch) || /[_\-. ]/.test(name[i - 1])).join('').toLowerCase();
  if (heads.startsWith(q)) return 2;
  if (n.includes(q)) return 3;
  let at = 0;
  for (const ch of n) if (ch === q[at]) at += 1;
  return at === q.length ? 4 : -1;
}

export function createCode(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  let root = norm(saved.root || item.cwd || '');
  let disposed = false, shown = false;
  /** 열어 둔 파일들: { path, lines, tokens, lang, modified, top(보던 맨 윗줄), outline, marks(바뀐 줄), usedAt, missing } */
  let tabs = [];
  let current = null;
  let caret = null;               // 마지막으로 누른 자리 { line, col, word, from, to }
  const expanded = new Set((Array.isArray(saved.expanded) ? saved.expanded : []).map(lower));
  const listed = new Map();       // 폴더(소문자 경로) → 그 안의 항목들
  let git = { root: '', branch: '', files: new Map() }; // 바뀐 파일: 소문자 전체 경로 → { status, untracked, rel }
  const touched = new Map();      // Claude 세션이 이번에 쓴 파일: 소문자 전체 경로 → 쓴 때
  let changedOnly = !!saved.changedOnly;
  let back = [], forward = [];
  let fileList = null;            // 프로젝트의 파일 목록(파일 찾기용). 필요할 때 읽는다.
  let panel = null;               // 아래 결과 창에 보이는 것 { title, hits, truncated, query? }
  let sideWidth = Math.min(600, Math.max(140, Number(saved.side) || 250));
  let findOpen = false, findHits = [], findAt = 0;
  let lastScroll = 0;             // 마지막으로 보던 스크롤 자리(숨기면 브라우저가 맨 위로 되돌린다)
  let heldScroll = null;          // 화면 배치를 다시 짓는 동안 지켜 둔 스크롤 자리(창을 떼었다 붙이면 맨 위로 돌아간다)
  let focusedPath = '';           // 나무에서 키보드로 고른 줄(나무를 다시 그려도 지킨다)
  let epoch = 0;                  // 프로젝트 폴더를 바꿀 때마다 오른다(옛 폴더의 늦은 결과를 가려낸다)
  let openSeq = 0;                // 파일 열기의 차례(먼저 연 느린 파일이 나중에 연 파일을 덮지 않게)
  let panelSeq = 0;               // 아래 결과 창에 넣을 찾기의 차례
  const opening = new Map();      // 읽고 있는 파일(소문자 경로 → 읽기). 같은 파일을 두 번 읽어 탭이 둘 생기지 않게 한다

  const button = (text, tip, action) => {
    const b = make('button', '', text);
    b.title = tip;
    b.onclick = action;
    return b;
  };

  // ── 틀 ──
  const $title = make('span', 'doc-title');
  const $changed = button('바뀐 파일', '바뀐 파일만 보기 (git 기준) ↔ 프로젝트 전체', () => { changedOnly = !changedOnly; renderTree(); changedState(); if (!changedOnly && current) revealInTree(current.path); });
  const $back = button('←', '뒤로 (Alt+←)', () => travel(-1));
  const $forward = button('→', '앞으로 (Alt+→)', () => travel(1));
  // 글꼴: 이름(비우면 앱의 고정폭 글꼴)과 크기. 여기서 바꾼 것은 코드 보기에만 적용된다.
  const $font = make('input', 'code-font');
  Object.assign($font, { placeholder: '글꼴 (기본)', spellcheck: false, autocomplete: 'off', title: '코드 글꼴 · 이름을 적거나 목록에서 고릅니다 · 비우면 앱의 고정폭 글꼴(설정 → 모양)' });
  const fontList = make('datalist');
  fontList.id = `code-fonts-${item.key}`;
  for (const name of CODE_FONTS) { const option = make('option'); option.value = name; fontList.append(option); }
  $font.setAttribute('list', fontList.id);
  $font.onchange = () => {
    const name = $font.value.trim();
    setCodeFont({ family: name });
    setStatus(!name ? '코드 글꼴: 앱의 고정폭 글꼴(설정 → 모양)을 씁니다.' : hasFont(name) ? `코드 글꼴: ${name}` : `"${name}" 글꼴이 이 컴퓨터에 없어 기본 글꼴로 보입니다.`);
  };
  $font.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); $font.blur(); $scroll.focus({ preventScroll: true }); } });
  const $size = make('span', 'code-size');
  $size.title = '코드 글자 크기 (Ctrl+휠, Ctrl+더하기/빼기 · Ctrl+0: 기본 크기)';
  const fontBox = make('span', 'code-fontbox');
  fontBox.append(
    $font, fontList,
    button('−', '글자 작게 (Ctrl+−, Ctrl+휠)', () => setCodeFont({ size: codeFont.size - 1 })),
    $size,
    button('+', '글자 크게 (Ctrl++, Ctrl+휠) · 기본 크기 (Ctrl+0)', () => setCodeFont({ size: codeFont.size + 1 })),
  );
  const bar = make('div', 'doc-bar');
  bar.append(
    $title,
    fontBox,
    button('폴더…', '볼 프로젝트 폴더 바꾸기', () => pickRoot()),
    $changed, $back, $forward,
    button('파일 찾기', '이름으로 파일 찾기 (Ctrl+P)', () => pickFile()),
    button('기호 찾기', '함수·클래스 이름으로 찾기 (Ctrl+T)', () => pickSymbol()),
    button('전체에서 찾기', '프로젝트 전체에서 글 찾기 (Ctrl+Shift+F)', () => openSearch()),
    button('구조', '이 파일의 구조에서 골라 이동 (Ctrl+F12)', () => pickOutline()),
  );

  const $tree = make('div', 'code-tree');
  $tree.tabIndex = 0;
  const $outlineHead = make('div', 'code-side-head', '구조');
  const $outline = make('div', 'code-outline');
  const side = make('div', 'code-side');
  side.append($tree, $outlineHead, $outline);
  const grip = make('div', 'code-grip');
  grip.title = '끌기: 너비 조절';

  const $tabs = make('div', 'code-tabs');
  // 탭은 초점을 받지 못하는 요소라, 그대로 두면 누른 뒤 초점이 창 밖으로 나가 단축키(Alt+←, Ctrl+F4 …)가 듣지 않는다.
  $tabs.addEventListener('mousedown', (ev) => { ev.preventDefault(); (current ? $scroll : $tree).focus({ preventScroll: true }); });
  const $crumbs = make('div', 'code-crumbs');
  const $query = make('input');
  Object.assign($query, { placeholder: '이 파일에서 찾기', spellcheck: false, autocomplete: 'off' });
  const $count = make('span', 'find-count');
  const $find = make('div', 'code-find');
  $find.hidden = true;
  const $findClose = button('×', '닫기 (Esc)', () => closeFind());
  $findClose.className = 'close';
  $find.append($query, $count, button('↑', '이전 (Shift+Enter)', () => stepFind(-1)), button('↓', '다음 (Enter)', () => stepFind(1)), $findClose);
  const $code = make('div', 'code-lines');
  const $empty = make('div', 'code-empty');
  const $scroll = make('div', 'code-scroll');
  $scroll.tabIndex = 0;
  $scroll.append($empty, $code);
  const $panelHead = make('div', 'code-panel-head');
  const $panelBody = make('div', 'code-panel-body');
  $panelBody.tabIndex = 0;
  const $panel = make('div', 'code-panel');
  $panel.hidden = true;
  $panel.append($panelHead, $panelBody);
  const $pick = make('div', 'code-pick');
  $pick.hidden = true;
  const main = make('div', 'code-main');
  main.append($tabs, $crumbs, $find, $scroll, $panel, $pick);
  const body = make('div', 'code-body');
  body.append(side, grip, main);
  const rootEl = make('div', 'doc code');
  rootEl.append(bar, body);
  host.append(rootEl);
  side.style.width = `${sideWidth}px`;

  /** 정해 둔 글꼴을 이 화면에 적용한다. 줄 높이가 바뀌므로 보던 줄을 지킨 채 코드를 다시 그린다. */
  function applyFont() {
    const top = current?.lines ? topLine() : 1;
    rootEl.style.setProperty('--code-font', codeFont.family ? `${quotedFont(codeFont.family)}, var(--mono-font)` : 'var(--mono-font)');
    rootEl.style.setProperty('--code-size', `${codeFont.size}px`);
    rootEl.style.setProperty('--code-lh', `${Math.round(codeFont.size * 1.46)}px`);
    if (document.activeElement !== $font) $font.value = codeFont.family;
    $size.textContent = String(codeFont.size);
    if (!current?.lines) return;
    renderCode(current);
    scrollToLine(top, false);
    repaint();
  }
  fontViews.add(applyFont);

  /** 프로젝트 폴더 안의 경로가 시작하는 글(소문자, 구분자로 끝남). 프로젝트 폴더가 드라이브 맨 위(`C:\`)여도 맞는다. */
  const rootPrefix = () => (lower(root).endsWith('\\') ? lower(root) : `${lower(root)}\\`);
  /** 프로젝트 기준 경로를 전체 경로로. 이미 전체 경로면 그대로. */
  const full = (relative) => (/^(?:[A-Za-z]:[\\/]|[\\/]{2})/.test(relative) ? norm(relative) : norm(`${root}\\${relative}`));
  /** 프로젝트 폴더 기준 경로(`/`로 구분). 프로젝트 밖의 파일은 전체 경로. */
  const relative = (path) => (lower(path).startsWith(rootPrefix()) ? slash(norm(path).slice(rootPrefix().length)) : norm(path));
  const title = () => `코드 · ${api.baseName(root) || '폴더 없음'}`;
  const changedState = () => { $changed.classList.toggle('on', changedOnly); onChange(); };

  function refreshBar() {
    $title.textContent = `${root}${git.branch ? `  ⎇ ${git.branch}` : ''}`;
    $title.title = `프로젝트 폴더: ${root}${git.branch ? `\n브랜치: ${git.branch}` : ''}${git.files.size ? `\n바뀐 파일 ${git.files.size}개` : ''}`;
    $changed.textContent = git.files.size ? `바뀐 파일 ${git.files.size}` : '바뀐 파일';
    $changed.classList.toggle('on', changedOnly);
    $back.disabled = !back.length;
    $forward.disabled = !forward.length;
  }

  // ── 프로젝트 나무 ──

  async function list(dir) {
    try {
      const entries = await invoke('code_list', { dir });
      listed.set(lower(dir), entries);
    } catch {
      listed.set(lower(dir), []);
    }
  }

  /** 파일의 git 상태에 따른 표시(줄의 클래스). */
  function statusClass(path) {
    const change = git.files.get(lower(path));
    if (!change) return '';
    return change.untracked || change.status === '추가' ? 'new' : change.status === '삭제' ? 'gone' : 'mod';
  }
  /** 그 폴더 안에 바뀐 파일이 있는지. */
  const dirChanged = (dir) => { const prefix = `${lower(dir)}\\`; for (const path of git.files.keys()) if (path.startsWith(prefix)) return true; return false; };

  /** 파일 종류의 표식(나무와 탭의 이름 앞). */
  function fileIcon(path) {
    const [mark, tone] = fileBadge(path);
    return make('i', `code-ico fi-${tone}`, mark);
  }

  function treeRow(path, name, depth, { dir = false, dim = false, note = '' } = {}) {
    const row = make('div', `code-row${dir ? ' dir' : ''}${dim ? ' dim' : ''}`);
    row.dataset.path = path;
    row.style.paddingLeft = `${6 + depth * 14}px`;
    const open = dir && expanded.has(lower(path));
    const status = dir ? (dirChanged(path) ? 'has' : '') : statusClass(path);
    if (status) row.classList.add(status);
    if (!dir && current && lower(current.path) === lower(path)) row.classList.add('current');
    if (!dir && touched.has(lower(path))) row.classList.add('touched');
    if (focusedPath && lower(path) === focusedPath) row.classList.add('focus');
    // 폴더는 펼침 표시, 파일은 종류의 표식(확장자마다 색이 다르다).
    row.append(dir ? make('span', 'code-twist', open ? '▾' : '▸') : fileIcon(path), make('span', 'code-name', name));
    if (note) row.append(make('span', 'code-note', note));
    row.title = `${path}${status === 'mod' ? '\n고친 파일' : status === 'new' ? '\n새 파일' : status === 'gone' ? '\n지운 파일' : ''}${touched.has(lower(path)) ? '\nClaude 세션이 방금 쓴 파일' : ''}`;
    return row;
  }

  function renderTree() {
    const rows = [];
    if (changedOnly) {
      // 바뀐 파일만: Claude 세션이 방금 쓴 것이 앞, 그 뒤로 이름순.
      const files = [...git.files.entries()].filter(([path]) => path.startsWith(rootPrefix()) || path === lower(root));
      files.sort((a, b) => (touched.get(b[0]) || 0) - (touched.get(a[0]) || 0) || a[1].rel.localeCompare(b[1].rel));
      for (const [, change] of files) {
        const path = change.path;
        rows.push(treeRow(path, nameOf(path), 0, { note: `${slash(dirOf(relative(path)))}${change.status ? `  ${change.status}` : ''}` }));
      }
      if (!rows.length) rows.push(make('div', 'code-note-row', git.root ? '바뀐 파일이 없습니다.' : 'git 저장소가 아니라 바뀐 파일을 알 수 없습니다.'));
    } else {
      const walk = (dir, depth) => {
        for (const entry of listed.get(lower(dir)) || []) {
          const path = `${dir}\\${entry.name}`;
          rows.push(treeRow(path, entry.name, depth, { dir: entry.dir, dim: entry.dim }));
          if (entry.dir && expanded.has(lower(path))) walk(path, depth + 1);
        }
      };
      walk(root, 0);
      if (!rows.length) rows.push(make('div', 'code-note-row', listed.has(lower(root)) ? '빈 폴더입니다.' : '읽는 중…'));
    }
    const keep = $tree.scrollTop;
    $tree.replaceChildren(...rows);
    $tree.scrollTop = keep;
  }

  async function toggleDir(path) {
    const key = lower(path);
    if (expanded.has(key)) expanded.delete(key);
    else {
      expanded.add(key);
      if (!listed.has(key)) await list(path);
    }
    renderTree();
    onChange();
  }

  /** 그 파일이 나무에서 보이게 위의 폴더를 모두 펼친다. */
  async function revealInTree(path) {
    if (changedOnly || !lower(path).startsWith(rootPrefix())) return;
    let dir = dirOf(path);
    const opened = [];
    while (lower(dir).length > lower(root).length) { opened.unshift(dir); dir = dirOf(dir); }
    for (const each of opened) {
      expanded.add(lower(each));
      if (!listed.has(lower(each))) await list(each);
    }
    renderTree();
    $tree.querySelector('.code-row.current')?.scrollIntoView({ block: 'nearest' });
  }

  $tree.addEventListener('click', (ev) => {
    const row = ev.target.closest('.code-row');
    if (!row) return;
    focusedPath = lower(row.dataset.path);
    if (row.classList.contains('dir')) toggleDir(row.dataset.path); else openFile(row.dataset.path);
  });
  $tree.addEventListener('contextmenu', (ev) => {
    const row = ev.target.closest('.code-row');
    ev.preventDefault();
    ev.stopPropagation();
    if (!row) return;
    const path = row.dataset.path;
    showMenu(ev, [
      { label: '경로 복사', action: () => copyText(path) },
      { label: '프로젝트 기준 경로 복사', action: () => copyText(relative(path)) },
      { label: 'Claude에 넣기 (경로)', action: () => api.toClaude?.(item, `${relative(path)} `, ev) },
      { label: '탐색기에서 보기', action: () => outside(path) },
    ]);
  });
  $tree.addEventListener('keydown', (ev) => {
    const rows = [...$tree.querySelectorAll('.code-row')];
    const at = rows.findIndex((row) => row.classList.contains('focus'));
    const move = (to) => {
      rows[at]?.classList.remove('focus');
      const next = rows[Math.max(0, Math.min(rows.length - 1, to))];
      if (!next) return;
      next.classList.add('focus');
      focusedPath = lower(next.dataset.path);
      next.scrollIntoView({ block: 'nearest' });
    };
    const row = rows[at];
    if (ev.key === 'ArrowDown') move(at + 1);
    else if (ev.key === 'ArrowUp') move(at < 0 ? 0 : at - 1);
    else if (ev.key === 'Enter' && row) row.click();
    else if (ev.key === 'ArrowRight' && row?.classList.contains('dir') && !expanded.has(lower(row.dataset.path))) row.click();
    else if (ev.key === 'ArrowLeft' && row?.classList.contains('dir') && expanded.has(lower(row.dataset.path))) row.click();
    else return;
    ev.preventDefault();
    ev.stopPropagation();
  });

  // 탭이 넘치면 휠로 좌우로 넘긴다.
  $tabs.addEventListener('wheel', (ev) => {
    if ($tabs.scrollWidth <= $tabs.clientWidth) return;
    ev.preventDefault();
    $tabs.scrollLeft += ev.deltaY || ev.deltaX;
  }, { passive: false });

  // 왼쪽 칸의 너비
  grip.onmousedown = (ev) => {
    ev.preventDefault();
    const left = side.getBoundingClientRect().left;
    const move = (e) => { sideWidth = Math.min(600, Math.max(140, Math.round(e.clientX - left))); side.style.width = `${sideWidth}px`; };
    const stop = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', stop); onChange(); };
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', stop);
  };

  // ── 바뀐 파일(git) ──

  async function refreshGit() {
    if (!root || disposed) return;
    const mine = epoch;
    let next = { root: '', branch: '', files: new Map() };
    try {
      const changes = await invoke('git_changes', { cwd: root });
      next = { root: norm(changes.root), branch: changes.branch, files: new Map() };
      for (const file of changes.files) {
        const path = norm(`${next.root}\\${file.path}`);
        next.files.set(lower(path), { status: file.status, untracked: file.untracked, rel: file.path, path });
      }
    } catch { /* git 저장소가 아니다 */ }
    if (mine !== epoch || disposed) return; // 그사이 프로젝트 폴더를 바꿨다
    const same = next.root === git.root && next.branch === git.branch && next.files.size === git.files.size
      && [...next.files].every(([path, change]) => git.files.get(path)?.status === change.status);
    git = next;
    if (same) return;
    refreshBar();
    renderTree();
    renderTabs();
    if (current) loadMarks(current).then(() => { if (current) applyMarks(current); });
  }

  /** 그 파일의 바뀐 줄(git 기준): 줄 번호 → 'add'(더한 줄) | 'mod'(고친 줄) | 'del'(이 줄 앞에서 지웠다) */
  async function loadMarks(tab) {
    const change = git.files.get(lower(tab.path));
    tab.marks = new Map();
    if (!change || !tab.lines) return;
    if (change.untracked) { for (let n = 1; n <= tab.lines.length; n++) tab.marks.set(n, 'add'); return; }
    try {
      const diff = await invoke('git_diff', { root: git.root, path: change.rel, untracked: false });
      let line = 0, removed = 0;
      for (const text of diff.split('\n')) {
        const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
        if (hunk) { line = Number(hunk[1]); removed = 0; continue; }
        if (!line || text.startsWith('\\')) continue;
        if (text.startsWith('-')) { removed += 1; continue; }
        if (text.startsWith('+')) {
          tab.marks.set(line, removed > 0 ? 'mod' : 'add');
          if (removed > 0) removed -= 1;
          line += 1;
          continue;
        }
        if (removed > 0 && !tab.marks.has(line)) tab.marks.set(line, 'del');
        removed = 0;
        line += 1;
      }
      if (removed > 0 && line <= tab.lines.length && !tab.marks.has(line)) tab.marks.set(line, 'del');
    } catch { /* 차이를 읽지 못하면 표시 없이 둔다 */ }
  }

  function applyMarks(tab) {
    if (tab !== current) return;
    for (const el of $code.querySelectorAll('.cl.add, .cl.mod, .cl.del')) el.classList.remove('add', 'mod', 'del');
    for (const [line, kind] of tab.marks || []) lineEl(line)?.classList.add(kind);
  }

  // ── 파일 열기와 그리기 ──

  const lineEl = (line) => $code.children[Math.floor((line - 1) / CHUNK)]?.children[(line - 1) % CHUNK] || null;

  /** 줄 하나의 내용을 그린다: 강조 조각은 span으로, 나머지는 글자 그대로. */
  function fillLine(el, text, tokens) {
    if (!tokens?.length) { el.textContent = text; return; }
    const parts = [];
    let at = 0;
    for (const [from, to, kind] of tokens) {
      if (from > at) parts.push(text.slice(at, from));
      const span = document.createElement('span');
      span.className = `t-${kind}`;
      span.textContent = text.slice(from, to);
      parts.push(span);
      at = to;
    }
    if (at < text.length) parts.push(text.slice(at));
    el.replaceChildren(...parts);
  }

  /** 화면 가까이 온 덩이를 강조해 그린다(처음에는 글자만 넣어 둔다). */
  const dresser = new IntersectionObserver((entries) => {
    let dressed = false;
    for (const entry of entries) {
      if (!entry.isIntersecting || entry.target.dataset.dressed) continue;
      dress(entry.target);
      dressed = true;
    }
    if (dressed) repaint();
  }, { root: $scroll, rootMargin: '900px 0px' });

  function dress(chunk) {
    const tab = current;
    if (!tab?.tokens || chunk.dataset.dressed) return;
    chunk.dataset.dressed = '1';
    const first = Number(chunk.dataset.first);
    for (let i = 0; i < chunk.children.length; i++) fillLine(chunk.children[i], tab.lines[first + i], tab.tokens[first + i]);
  }

  function renderCode(tab) {
    dresser.disconnect();
    const chunks = [];
    const total = tab.lines.length;
    const height = lineHeight();
    $code.style.setProperty('--digits', String(Math.max(3, String(total).length)));
    for (let first = 0; first < total; first += CHUNK) {
      const chunk = make('div', 'code-chunk');
      chunk.dataset.first = first;
      const count = Math.min(CHUNK, total - first);
      chunk.style.containIntrinsicBlockSize = `auto ${count * height}px`;
      for (let i = 0; i < count; i++) {
        const el = document.createElement('div');
        el.className = 'cl';
        el.dataset.n = first + i + 1;
        el.textContent = tab.lines[first + i];
        chunk.append(el);
      }
      chunks.push(chunk);
    }
    $code.replaceChildren(...chunks);
    if (tab.tokens) for (const chunk of chunks) dresser.observe(chunk);
    applyMarks(tab);
  }

  async function read(path) {
    const file = await invoke('reader_read', { path });
    const lines = file.text.split('\n');
    if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop();
    const lang = langOf(path);
    const tokens = lines.length <= MAX_HIGHLIGHT_LINES && lang !== 'plain' ? tokenize(lang, lines) : null;
    return { lines, lang, tokens, modified: file.modified, bytes: file.bytes };
  }

  /** 파일을 연다(이미 열려 있으면 그 탭으로 간다). `at`을 주면 그 줄·칸으로 가고 잠깐 표시한다. */
  async function openFile(path, at = null, { record = true } = {}) {
    path = norm(path);
    if (record) remember();
    const mine = epoch, seq = ++openSeq;
    const same = (other) => lower(other.path) === lower(path);
    let tab = tabs.find(same);
    if (!tab) {
      // 같은 파일을 읽는 중이면 그 읽기를 함께 기다린다.
      let pending = opening.get(lower(path));
      if (!pending) {
        pending = (async () => {
          const fresh = { path, lines: null, tokens: null, lang: langOf(path), modified: 0, top: 1, outline: [], marks: new Map(), usedAt: 0, missing: '' };
          try {
            Object.assign(fresh, await read(path));
          } catch (err) {
            fresh.missing = String(err);
          }
          return fresh;
        })();
        opening.set(lower(path), pending);
        pending.finally(() => opening.delete(lower(path)));
      }
      const fresh = await pending;
      if (disposed || mine !== epoch) return; // 그사이 프로젝트 폴더를 바꿨다
      tab = tabs.find(same);
      if (!tab) {
        tab = fresh;
        tabs.push(tab);
        while (tabs.length > MAX_TABS) {
          const oldest = tabs.filter((other) => other !== tab && other !== current).sort((a, b) => a.usedAt - b.usedAt)[0];
          if (!oldest) break;
          tabs = tabs.filter((other) => other !== oldest);
        }
      }
    }
    // 그사이 다른 파일을 열었으면 이 파일은 탭에만 올린다(나중에 연 것이 보여야 한다).
    if (seq !== openSeq) { renderTabs(); return; }
    await show(tab, at);
  }

  async function show(tab, at = null) {
    // 이미 보고 있는 파일: 다시 그리지 않는다(보던 자리를 지킨다). 갈 자리를 줬으면 그리로 간다.
    if (tab === current && tab.lines && $code.childElementCount) {
      tab.usedAt = Date.now();
      if (at) { scrollToLine(at.line, true); flash(at); }
      return;
    }
    if (current && current !== tab) current.top = topLine();
    current = tab;
    tab.usedAt = Date.now();
    caret = null;
    clearMarks();
    renderTabs();
    $empty.hidden = !!tab.lines;
    $code.hidden = !tab.lines;
    if (!tab.lines) {
      $empty.replaceChildren(make('b', '', '이 파일은 볼 수 없습니다'), make('span', '', tab.missing || ''), button('탐색기에서 보기', '탐색기에서 이 파일 보기', () => outside(tab.path)));
      $code.replaceChildren();
    } else {
      renderCode(tab);
      scrollToLine(at?.line || tab.top, !!at);
      if (at) flash(at);
    }
    renderCrumbs();
    renderTree();
    revealInTree(tab.path);
    onChange();
    if (findOpen) runFind(true);
    // 구조와 바뀐 줄은 뒤이어 읽는다(읽지 못한 파일은 구조도 묻지 않는다).
    if (tab.lines) invoke('code_outline', { path: tab.path }).then((outline) => { tab.outline = outline; if (current === tab) { renderOutline(); renderCrumbs(); } }).catch(() => {});
    loadMarks(tab).then(() => applyMarks(tab));
    renderOutline();
    // 다른 탭을 보는 동안 밖에서 고쳐졌을 수 있다. 다음 살핌을 기다리지 않고 바로 견준다.
    if (tab.lines) check();
  }

  function closeTab(tab) {
    closeTabs([tab]);
  }

  /** 탭을 여럿 한 번에 닫는다. 보던 탭이 닫히면 남은 탭 가운데 가까운 것(오른쪽 먼저)을 보인다. */
  function closeTabs(closing) {
    const drop = new Set(closing.filter((tab) => tabs.includes(tab)));
    if (!drop.size) return;
    const at = tabs.indexOf(current);
    const next = current && drop.has(current)
      ? tabs.slice(at + 1).find((tab) => !drop.has(tab)) || tabs.slice(0, at).reverse().find((tab) => !drop.has(tab))
      : null;
    const closedCurrent = !!current && drop.has(current);
    tabs = tabs.filter((tab) => !drop.has(tab));
    if (closedCurrent) {
      current = null;
      if (next) return void show(next);
      clearMarks();
      $code.replaceChildren();
      $code.hidden = true;
      $empty.hidden = false;
      welcome();
      renderCrumbs();
      renderOutline();
    }
    renderTabs();
    renderTree();
    onChange();
  }

  function welcome() {
    $empty.replaceChildren(
      make('b', '', '왼쪽 나무에서 파일을 고르세요'),
      make('span', '', 'Ctrl+P: 파일 찾기 · Ctrl+T: 기호 찾기 · Ctrl+Shift+F: 전체에서 찾기'),
      make('span', '', 'Ctrl+클릭·F12: 정의로 가기 · Alt+F7: 쓰는 곳 찾기 · Alt+←/→: 뒤로·앞으로'),
    );
  }

  function renderTabs() {
    const names = new Map();
    for (const tab of tabs) names.set(nameOf(tab.path).toLowerCase(), (names.get(nameOf(tab.path).toLowerCase()) || 0) + 1);
    $tabs.replaceChildren(...tabs.map((tab) => {
      const el = make('div', `code-tab${tab === current ? ' on' : ''}`);
      const status = statusClass(tab.path);
      if (status) el.classList.add(status);
      if (tab.gone) el.classList.add('gone'); // 파일이 없어졌다(보던 내용은 그대로 둔다)
      // 이름이 같은 파일이 여럿 열려 있으면 폴더 이름을 붙여 가린다.
      const twin = names.get(nameOf(tab.path).toLowerCase()) > 1;
      el.append(fileIcon(tab.path), make('span', '', twin ? `${nameOf(dirOf(tab.path))}/${nameOf(tab.path)}` : nameOf(tab.path)));
      const close = make('button', 'close', '×');
      close.title = '닫기 (가운데 단추로 눌러도 닫힙니다)';
      close.onclick = (ev) => { ev.stopPropagation(); closeTab(tab); };
      el.append(close);
      el.title = `${tab.path}
오른쪽 클릭: 탭 닫기 메뉴`;
      el.onclick = () => { if (tab !== current) { remember(); show(tab); } };
      el.onauxclick = (ev) => { if (ev.button === 1) { ev.preventDefault(); closeTab(tab); } };
      el.oncontextmenu = (ev) => { ev.preventDefault(); ev.stopPropagation(); tabMenu(tab, ev); };
      return el;
    }));
    $tabs.querySelector('.code-tab.on')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  /** 탭의 오른쪽 클릭 메뉴: 탭을 닫는 여러 방법과 그 파일의 경로를 쓰는 일. 닫을 것이 없는 줄은 내지 않는다. */
  function tabMenu(tab, ev) {
    const at = tabs.indexOf(tab);
    if (at < 0) return;
    const left = tabs.slice(0, at), right = tabs.slice(at + 1);
    // git 기준으로 바뀌지 않은 파일(고치지도 새로 만들지도 않은 것). 바뀐 파일만 남겨 두고 볼 때 쓴다.
    // 프로젝트 밖의 파일은 바뀌었는지 알 수 없으니 치지 않는다(닫지 않는다).
    const unchanged = git.root ? tabs.filter((other) => lower(other.path).startsWith(rootPrefix()) && !statusClass(other.path)) : [];
    const inProject = lower(tab.path).startsWith(rootPrefix());
    const items = [{ label: '탭 닫기', action: () => closeTabs([tab]) }];
    if (left.length + right.length) items.push({ label: `다른 탭 모두 닫기 (${left.length + right.length}개)`, action: () => closeTabs([...left, ...right]) });
    if (left.length && right.length) {
      items.push({ label: `왼쪽 탭 모두 닫기 (${left.length}개)`, action: () => closeTabs(left) });
      items.push({ label: `오른쪽 탭 모두 닫기 (${right.length}개)`, action: () => closeTabs(right) });
    }
    if (unchanged.length && unchanged.length < tabs.length) items.push({ label: `바뀌지 않은 파일의 탭 닫기 (${unchanged.length}개)`, action: () => closeTabs(unchanged) });
    if (tabs.length > 1) items.push({ label: `모든 탭 닫기 (${tabs.length}개)`, action: () => closeTabs(tabs) });
    items.push({ caption: '이 파일' });
    if (inProject && !changedOnly && !tab.gone) {
      items.push({
        label: '나무에서 보기',
        action: async () => {
          focusedPath = lower(tab.path);
          await revealInTree(tab.path);
          $tree.querySelector('.code-row.focus')?.scrollIntoView({ block: 'nearest' });
        },
      });
    }
    items.push(
      { label: '경로 복사', action: () => copyText(tab.path) },
      ...(inProject ? [{ label: '프로젝트 기준 경로 복사', action: () => copyText(relative(tab.path)) }] : []),
      { label: 'Claude에 넣기 (경로)', action: () => api.toClaude?.(item, `${inProject ? relative(tab.path) : tab.path} `, ev) },
      { label: '탐색기에서 보기', action: () => outside(tab.path) },
    );
    showMenu(ev, items);
  }

  /** 그 줄을 감싸고 있는 정의들(바깥 것부터). */
  function enclosing(tab, line) {
    const chain = [];
    for (const def of tab?.outline || []) {
      if (def.line > line) break;
      while (chain.length && chain[chain.length - 1].depth >= def.depth) chain.pop();
      const holds = !['field', 'variant', 'constant', 'variable', 'id', 'selector', 'heading', 'section'].includes(def.kind) && (def.end ?? def.line) >= line;
      if (holds || def.line === line) chain.push(def);
    }
    return chain;
  }

  function renderCrumbs() {
    if (!current) { $crumbs.replaceChildren(); return; }
    const parts = relative(current.path).split('/').map((part) => make('span', 'crumb', part));
    const line = caret?.line || topLine();
    for (const def of enclosing(current, line).slice(-3)) {
      const crumb = make('span', 'crumb sym', `${KIND_ICONS[def.kind] || '·'} ${def.name}`);
      crumb.title = `${KIND_NAMES[def.kind] || def.kind} ${def.name} — ${def.line}번째 줄`;
      crumb.onclick = () => { remember(); jump({ line: def.line, col: def.col, len: def.name.length }); };
      parts.push(crumb);
    }
    const info = make('span', 'crumb info', `${LANG_NAMES[current.lang] || ''}${current.lines ? ` · ${current.lines.length}줄` : ''}${caret ? ` · ${caret.line}:${caret.col + 1}` : ''}`);
    $crumbs.replaceChildren(...parts, info);
  }

  function renderOutline() {
    const defs = current?.outline || [];
    $outlineHead.textContent = `구조${defs.length ? ` (${defs.length})` : ''}`;
    const least = defs.reduce((low, def) => Math.min(low, def.depth), 99);
    $outline.replaceChildren(...defs.slice(0, 2000).map((def) => {
      const row = make('div', `code-sym k-${def.kind}`);
      row.style.paddingLeft = `${8 + (def.depth - least) * 12}px`;
      row.append(make('span', 'code-kind', KIND_ICONS[def.kind] || '·'), make('span', 'code-name', def.name));
      row.title = `${KIND_NAMES[def.kind] || def.kind} ${def.container ? `${def.container}.` : ''}${def.name} — ${def.line}번째 줄`;
      row.onclick = () => { remember(); jump({ line: def.line, col: def.col, len: def.name.length }); };
      return row;
    }));
    if (!defs.length) $outline.append(make('div', 'code-note-row', current ? '찾은 정의가 없습니다.' : ''));
  }

  // ── 자리: 스크롤, 화면의 글자 ↔ 줄·칸 ──

  /** 줄 하나의 높이(픽셀). 스타일에서 읽는다: 화면 밖의 덩이는 그려지지 않아 재 볼 수 없다. */
  const lineHeight = () => parseFloat(getComputedStyle($code).lineHeight) || 19;
  /** 지금 정말 보이는지(숨겨진 직후에는 `shown`이 아직 참일 수 있다). */
  const visibleNow = () => shown && heldScroll == null && $scroll.clientHeight > 0;
  const topLine = () => (current?.lines ? Math.max(1, Math.floor((visibleNow() ? $scroll.scrollTop : heldScroll ?? lastScroll) / lineHeight()) + 1) : 1);

  /** 그 줄이 보이게 한다. `center`이면 화면 가운데쯤에 오게 한다(멀리서 건너올 때). */
  function scrollToLine(line, center) {
    if (!current?.lines) return;
    const height = lineHeight();
    const top = (Math.max(1, Math.min(current.lines.length, line)) - 1) * height;
    // 숨겨져 있는 동안에는 화면의 스크롤 자리를 읽고 쓸 수 없다. 기억해 둔 자리(lastScroll)로 셈하고, 보일 때 그 자리로 간다.
    const live = shown && $scroll.clientHeight > 0;
    const view = live ? $scroll.clientHeight : 0;
    const now = live ? $scroll.scrollTop : heldScroll ?? lastScroll;
    let target = top;
    if (center) {
      if (view && top >= now + height * 2 && top <= now + view - height * 4) return; // 이미 보인다
      target = Math.max(0, top - (view || 600) / 3);
    }
    heldScroll = null; // 새로 갈 자리를 정했다: 배치를 바꾸기 전에 적어 둔 자리는 버린다
    lastScroll = target;
    if (live) $scroll.scrollTop = target;
  }

  /** 줄 안의 글자 자리를 화면의 글자 마디와 그 안의 자리로 바꾼다. */
  function nodeAt(el, col) {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    let left = col, last = null;
    while (walker.nextNode()) {
      last = walker.currentNode;
      if (left <= last.length) return [last, left];
      left -= last.length;
    }
    return last ? [last, last.length] : [el, 0];
  }

  function rangeOf(line, col, len) {
    const el = lineEl(line);
    if (!el) return null;
    try {
      const range = document.createRange();
      range.setStart(...nodeAt(el, col));
      range.setEnd(...nodeAt(el, col + len));
      return range;
    } catch {
      return null;
    }
  }

  /** 마우스 자리의 줄과 칸. 코드 밖이면 null. */
  function pointAt(ev) {
    const found = document.caretPositionFromPoint
      ? (() => { const pos = document.caretPositionFromPoint(ev.clientX, ev.clientY); return pos && { node: pos.offsetNode, offset: pos.offset }; })()
      : (() => { const range = document.caretRangeFromPoint?.(ev.clientX, ev.clientY); return range && { node: range.startContainer, offset: range.startOffset }; })();
    const el = (found?.node?.nodeType === 3 ? found.node.parentElement : found?.node)?.closest?.('.cl');
    if (!el || !$code.contains(el)) return null;
    let col = found.node.nodeType === 3 ? found.offset : 0;
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    while (walker.nextNode() && walker.currentNode !== found.node) col += walker.currentNode.length;
    return { line: Number(el.dataset.n), col };
  }

  /** 그 자리의 이름(식별자). 없으면 null. */
  function wordAt(line, col) {
    const text = current?.lines?.[line - 1];
    if (text == null) return null;
    const wide = wordPattern(current.lang);
    const narrow = wide.test('-') ? /[A-Za-z0-9_$-]/ : /[A-Za-z0-9_$]/;
    let from = Math.min(col, text.length), to = from;
    if (!wide.test(text[from] || '') && from > 0 && wide.test(text[from - 1])) { from -= 1; to = from; }
    if (!wide.test(text[from] || '')) return null;
    // 누른 글자가 영문·숫자면 영문·숫자가 이어진 데까지만 이름이다. 한글 같은 글자 위에서 누르면 그 글자들도 이름으로 본다.
    const isWord = narrow.test(text[from]) ? narrow : wide;
    while (from > 0 && isWord.test(text[from - 1])) from -= 1;
    while (to < text.length && isWord.test(text[to])) to += 1;
    // 이름 양끝의 `-`는 이름이 아니다(CSS·HTML에서만 `-`를 이름의 글자로 본다).
    let word = text.slice(from, to);
    while (word.startsWith('-') && !word.startsWith('--')) { word = word.slice(1); from += 1; }
    while (word.endsWith('-')) { word = word.slice(0, -1); to -= 1; }
    if (!word || /^\d+$/.test(word)) return null;
    return { word, line, from, to };
  }

  /** 그 낱말의 앞뒤에 붙으면 "다른 이름의 일부"가 되는 글자. 영문 이름이면 영문·숫자만 본다(앱의 쓰는 곳 찾기와 같게). */
  function edgePattern(word) {
    const wide = wordPattern(current.lang);
    // eslint-disable-next-line no-control-regex
    if (/[^\x00-\x7f]/.test(word)) return wide;
    return wide.test('-') ? /[A-Za-z0-9_$-]/ : /[A-Za-z0-9_$]/;
  }

  /** 그 자리가 주석이나 문자열 안인지(강조 조각으로 안다). */
  function inQuiet(line, col) {
    const token = current?.tokens?.[line - 1]?.find(([from, to]) => col >= from && col < to);
    return !!token && ['com', 'doc', 'str'].includes(token[2]);
  }

  // ── 표시(찾은 곳, 같은 이름) ──

  const marks = { hit: [], cur: [], same: [], link: [] };
  painted.set(marks, marks);
  let sameWord = '';

  function clearMarks() {
    sameWord = '';
    findHits = [];
    marks.hit = marks.cur = marks.same = marks.link = [];
    paint();
  }

  /** 덩이를 새로 그렸거나 내용이 바뀌었으면 표시를 다시 만든다(글자 마디가 바뀌어 예전 표시는 쓸 수 없다). */
  function repaint() {
    // 한 줄이 아주 길면(한 줄로 줄인 CSS·JSON) 표시 하나하나를 그리는 데 줄 길이만큼 걸린다. 그런 줄에 표시 수천 개를 걸면
    // 화면이 멈추므로, 긴 줄에는 "지금 자리" 하나만 표시한다(찾은 개수와 넘겨 보기는 그대로 된다).
    const short = (line) => (current?.lines?.[line - 1]?.length || 0) <= MAX_MARK_LINE;
    marks.hit = findHits.filter((hit) => short(hit.line)).map((hit) => rangeOf(hit.line, hit.col, hit.len)).filter(Boolean);
    const now = findHits[findAt];
    marks.cur = now ? [rangeOf(now.line, now.col, now.len)].filter(Boolean) : [];
    marks.same = [];
    if (sameWord && current?.lines) {
      const isWord = edgePattern(sameWord);
      current.lines.forEach((text, index) => {
        if (text.length > MAX_MARK_LINE) return;
        for (let at = text.indexOf(sameWord); at >= 0 && marks.same.length < 3000; at = text.indexOf(sameWord, at + sameWord.length)) {
          if (isWord.test(text[at - 1] || '') || isWord.test(text[at + sameWord.length] || '')) continue;
          const range = rangeOf(index + 1, at, sameWord.length);
          if (range) marks.same.push(range);
        }
      });
    }
    paint();
  }

  /** 건너온 자리를 잠깐 표시한다. */
  const flashing = new WeakMap(); // 표시 중인 줄 → 표시를 거둘 타이머
  function flash({ line, col = 0, len = 0 }) {
    const el = lineEl(line);
    if (!el) return;
    // 같은 줄을 잇달아 표시하면(같은 줄로 다시 건너옴) 앞의 표시가 끝나며 새 표시를 지우지 않게, 앞의 것을 거두고 새로 시작한다.
    clearTimeout(flashing.get(el));
    el.classList.remove('flash');
    void el.offsetWidth; // 움직임을 처음부터 다시
    el.classList.add('flash');
    flashing.set(el, setTimeout(() => el.classList.remove('flash'), 1400));
    caret = { line, col };
    if (len) {
      const text = current.lines[line - 1] || '';
      sameWord = text.slice(col, col + len);
      repaint();
    }
    if (col || len) {
      revealAcross(line, col, len);
      // 방금 그린 화면은 한두 틀 뒤에야 줄의 너비가 잡힌다(화면 밖 덩이는 그리기를 미룬다). 그때 한 번 더 맞춘다.
      const tab = current;
      setTimeout(() => { if (current === tab && caret?.line === line && caret?.col === col) revealAcross(line, col, len); }, 90);
    }
    renderCrumbs();
  }

  /** 그 자리가 가로로도 보이게 한다(긴 줄의 오른쪽에 있으면 가로로 민다). */
  function revealAcross(line, col, len) {
    if (!visibleNow()) return;
    const rect = rangeOf(line, col, Math.max(1, len))?.getBoundingClientRect();
    if (!rect || (!rect.width && !rect.height)) return;
    const box = $scroll.getBoundingClientRect();
    const gutter = 80;
    if (rect.right > box.right - 24) $scroll.scrollLeft += rect.right - box.right + box.width / 3;
    else if (rect.left < box.left + gutter) $scroll.scrollLeft = Math.max(0, $scroll.scrollLeft - (box.left + gutter - rect.left) - box.width / 3);
  }

  function jump(at) {
    scrollToLine(at.line, true);
    flash(at);
  }

  // ── 오간 자리(뒤로·앞으로) ──

  const here = () => (current ? { path: current.path, line: caret?.line || topLine(), col: caret?.col || 0 } : null);
  function remember() {
    const spot = here();
    if (!spot) return;
    const last = back[back.length - 1];
    if (last && lower(last.path) === lower(spot.path) && Math.abs(last.line - spot.line) < 3) return;
    back.push(spot);
    if (back.length > MAX_HISTORY) back.shift();
    forward = [];
    refreshBar();
  }
  async function travel(step) {
    const [from, to] = step < 0 ? [back, forward] : [forward, back];
    const spot = from.pop();
    if (!spot) return setStatus(step < 0 ? '더 뒤로 갈 곳이 없습니다.' : '더 앞으로 갈 곳이 없습니다.');
    const now = here();
    if (now) to.push(now);
    await openFile(spot.path, { line: spot.line, col: spot.col }, { record: false });
    refreshBar();
  }

  // ── 정의로 가기, 쓰는 곳 찾기 ──

  /** 이름 앞에 붙은 것: `Type::name`·`obj.name`의 앞 이름. 없으면 빈 글. */
  function qualifier(found) {
    const before = current.lines[found.line - 1].slice(0, found.from);
    const m = /([A-Za-z_$][\w$]*)\s*(?:::|\.|->|\?\.)\s*$/.exec(before);
    return m ? m[1] : '';
  }

  /**
   * 이 파일 안에서 그 이름을 선언한 줄을 찾는다(지역 변수, 인자, import). `from`번째 줄부터 `until`번째 줄까지 훑는다
   * (`until`이 더 크면 아래로). `skip`은 누른 자리다(그 자리 자체는 선언으로 치지 않는다).
   */
  function localDeclaration(name, from, until = 1, skip = null) {
    const lang = current.lang;
    const esc = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const word = `(?<![\\w$])${esc}(?![\\w$])`;
    const patterns = [
      new RegExp(`\\b(?:const|let|var|val|mut|auto|final)\\s+(?:mut\\s+)?${word}`),          // const x, let mut x, val x
      new RegExp(`\\b(?:const|let|var|val)\\s*[\\[{(][^=]*${word}[^=]*[\\]})]\\s*=`),          // const { x } =, let (a, x) =
      new RegExp(`\\b(?:for|foreach)\\s*\\(?\\s*(?:(?:const|let|var|final|auto)\\s+)?(?:[\\w<>\\[\\]]+\\s+)?${word}\\s*(?:in\\b|of\\b|:|=)`), // for x in / for (const x of
      new RegExp(`\\b(?:fn|function|def|func|fun)\\b[^(]*\\([^)]*${word}\\s*[:,)=]`),           // 함수의 인자
      new RegExp(`\\((?:[^()]*[,(]\\s*)?(?:[\\w.<>\\[\\]?]+\\s+)?${word}\\s*(?:[:,=][^()]*)?\\)\\s*(?:=>|\\{|->|:)`), // (a, x) => / 메서드의 인자
      new RegExp(`${word}\\s*=>`),                                                            // x =>
      // |x| (Rust의 클로저). 다른 언어의 `a || x || b`·`A | x | B`는 선언이 아니다.
      ...(lang === 'rust' ? [new RegExp(`(?:^|[^|])\\|(?:[^|]*[\\s,(&])?${word}(?:\\s*:[^|]*)?(?:\\s*,[^|]*)?\\|(?!\\|)`)] : []),
      new RegExp(`\\bcatch\\s*\\(\\s*(?:[\\w.<>|\\s]+\\s)?\\s*${word}\\s*\\)`),              // catch (e), catch (IOException e)
      new RegExp(`\\bexcept\\b[^:]*\\bas\\s+${word}`),                                        // except E as e
      new RegExp(`\\b(?:if|while)\\s+let\\b[^=]*${word}[^=]*=`),                              // if let Some(x) =
      new RegExp(`^\\s*(?:pub(?:\\([^)]*\\))?\\s+)?(?:import|use|from|export)\\b.*${word}`),     // 줄 머리의 import { x }, use a::x;
      // Type x = / Type x; (자료형을 앞에 적는 언어)
      ...(['python', 'yaml', 'json', 'css', 'markdown', 'toml', 'ini', 'shell', 'powershell'].includes(lang) ? [] : [
        new RegExp(`^\\s*(?!(?:return|throw|else|new|delete|case|yield|await|goto|import|package|in|not|and|or|typeof|void)\\b)(?:[A-Za-z_$][\\w$.<>\\[\\], ?]*\\s+)${word}\\s*(?:=[^=]|;)`),
      ]),
      // 선언하는 낱말 없이 값을 넣는 것이 곧 선언인 언어(Python, Go의 :=, 셸)
      ...(['python', 'shell', 'powershell'].includes(lang) ? [new RegExp(`^\\s*${word}\\s*=[^=>]`)] : []),
      ...(lang === 'go' ? [new RegExp(`(?:^|[\\s,])${word}\\s*(?:,\\s*[\\w]+\\s*)*:=`)] : []),
    ];
    const whole = new RegExp(word, 'g');
    const step = until >= from ? 1 : -1;
    for (let line = from; step > 0 ? line <= until : line >= until; line += step) {
      const text = current.lines[line - 1];
      if (text == null || !text.includes(name)) continue;
      for (const pattern of patterns) {
        const m = pattern.exec(text);
        if (!m) continue;
        whole.lastIndex = m.index;
        const col = whole.exec(text)?.index;
        if (col == null || inQuiet(line, col)) continue;
        if (skip && skip.line === line && skip.col === col) continue;
        return { line, col, len: name.length };
      }
    }
    return null;
  }

  /** 찾은 정의들을 가까운 순서로 늘어놓는다: 같은 파일, 앞에 붙은 이름이 감싼 것, 같은 폴더, 이 파일이 가져다 쓰는 파일, 같은 언어. */
  function rankDefs(defs, found) {
    const mine = lower(current.path);
    const owner = qualifier(found);
    const aroundMe = enclosing(current, found.line).map((def) => def.name);
    const text = current.lines.join('\n');
    const next = current.lines[found.line - 1].slice(found.to).trimStart()[0];
    const score = (def) => {
      const path = lower(full(def.file));
      let value = 0;
      if (path === mine) value += 50;
      if (owner && def.container === owner) value += 80;
      if (owner && /^(self|this|Self|super)$/.test(owner) && aroundMe.includes(def.container)) value += 60;
      if (!owner && aroundMe.includes(def.container)) value += 15;
      if (dirOf(path) === dirOf(mine)) value += 12;
      const stem = nameOf(def.file).replace(/\.[^.]+$/, '');
      if (stem.length > 2 && new RegExp(`(?:import|use|require|from|include|mod)\\b.*\\b${stem.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`).test(text)) value += 25;
      if (langOf(def.file) === current.lang) value += 10;
      // 부르는 자리(`이름(`)면 함수가, 그 밖에는 자료형·값이 먼저다.
      const callable = ['function', 'method', 'constructor', 'macro'].includes(def.kind);
      if (next === '(' || next === '!') value += callable || ['class', 'struct'].includes(def.kind) ? 6 : 0;
      else if (!callable) value += 4;
      return value;
    };
    return defs.map((def) => ({ def, value: score(def) })).sort((a, b) => b.value - a.value || a.def.file.localeCompare(b.def.file) || a.def.line - b.def.line);
  }

  async function goToDefinition(found = caretWord()) {
    if (!found || !current?.lines) return setStatus('이름 위에서 누르거나(Ctrl+클릭), 이름을 누른 뒤 F12를 누르세요.');
    const name = found.word;
    const tab = current;
    const clicked = { line: found.line, col: found.from };
    // 1. 이 함수 안의 지역 변수·인자: 감싼 함수의 첫 줄부터 지금 줄 사이에서 선언을 찾는다.
    const around = enclosing(current, found.line).filter((def) => ['function', 'method', 'constructor'].includes(def.kind)).pop();
    const owner = qualifier(found);
    if (around && !owner) {
      const local = localDeclaration(name, found.line, around.line, clicked);
      if (local) { remember(); return jump(local); }
    }
    // 2. 프로젝트 전체의 정의
    let defs = [];
    try {
      defs = await invoke('code_defs', { root, name });
    } catch (err) {
      return setStatus(`정의를 찾지 못했습니다: ${err}`);
    }
    if (disposed || current !== tab) return undefined; // 기다리는 사이 다른 파일로 갔다
    // 지금 누른 자리가 바로 그 정의면, 갈 곳이 없으니 쓰는 곳을 보여 준다.
    const onItself = (def) => lower(full(def.file)) === lower(current.path) && def.line === found.line && def.col === found.from;
    if (defs.some(onItself)) {
      const others = defs.filter((def) => !onItself(def));
      if (!others.length) return findUsages(found);
      defs = others;
    }
    if (!defs.length) {
      // 3. 프로젝트에서 못 찾았으면 이 파일의 위쪽 어디서든 선언을 찾아본다.
      // 위쪽에 없으면 아래쪽도 본다(함수 안에서 쓰는 곳보다 아래에 선언한 `const f = () => …`).
      const local = localDeclaration(name, found.line, 1, clicked) || localDeclaration(name, found.line + 1, tab.lines.length, clicked);
      if (local) { remember(); return jump(local); }
      return setStatus(`"${name}"의 정의를 찾지 못했습니다. 쓰는 곳은 Alt+F7로 찾습니다.`);
    }
    const ranked = rankDefs(defs, found);
    // 하나뿐이거나 맨 앞의 것이 뚜렷이 가까우면 바로 간다. 아니면 고르게 한다.
    if (ranked.length === 1 || ranked[0].value >= ranked[1].value + 40) {
      const { def } = ranked[0];
      return openFile(full(def.file), { line: def.line, col: def.col, len: name.length });
    }
    return pick({
      title: `"${name}"의 정의 ${ranked.length}개`,
      placeholder: '걸러 보기',
      source: (query) => ranked.map(({ def }) => def).filter((def) => !query || matchRank(query, `${def.file} ${def.container}`) >= 0),
      row: (def) => ({ icon: KIND_ICONS[def.kind] || '·', title: `${def.container ? `${def.container}.` : ''}${def.name}`, note: `${def.file}:${def.line}`, sub: def.text }),
      choose: (def) => openFile(full(def.file), { line: def.line, col: def.col, len: name.length }),
    });
  }

  async function findUsages(found = caretWord()) {
    if (!found) return setStatus('이름을 누른 뒤 Alt+F7을 누르세요(오른쪽 클릭 → 쓰는 곳 찾기).');
    setStatus(`"${found.word}" 쓰는 곳을 찾는 중…`);
    const seq = ++panelSeq;
    try {
      const result = await invoke('code_usages', { root, name: found.word });
      if (disposed || seq !== panelSeq) return;
      showPanel({ title: `"${found.word}" 쓰는 곳`, word: found.word, ...result });
      setStatus('');
    } catch (err) {
      setStatus(`쓰는 곳을 찾지 못했습니다: ${err}`);
    }
  }

  /** 마지막으로 누른 자리의 이름. 글자를 골라 두었으면 그 글자(한 줄 안의 이름일 때). */
  function caretWord() {
    const sel = getSelection();
    const picked = String(sel || '');
    if (sel && !sel.isCollapsed && picked && !/\s/.test(picked) && picked.length < 100 && $code.contains(sel.anchorNode)) {
      // 고른 글이 시작하는 줄과 칸을 화면에서 읽는다.
      const range = sel.getRangeAt(0);
      const el = (range.startContainer.nodeType === 3 ? range.startContainer.parentElement : range.startContainer)?.closest?.('.cl');
      if (el) {
        let col = range.startContainer.nodeType === 3 ? range.startOffset : 0;
        const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        while (walker.nextNode() && walker.currentNode !== range.startContainer) col += walker.currentNode.length;
        return { word: picked, line: Number(el.dataset.n), from: col, to: col + picked.length };
      }
    }
    return caret ? wordAt(caret.line, caret.col) : null;
  }

  // ── 아래 결과 창(쓰는 곳, 전체에서 찾기) ──

  function showPanel(next) {
    panel = next;
    $panel.hidden = false;
    const hits = next.hits || [];
    const files = new Set(hits.map((hit) => hit.file)).size;
    const counts = {};
    for (const hit of hits) counts[hit.role] = (counts[hit.role] || 0) + 1;
    const summary = Object.entries(counts).map(([role, count]) => `${ROLE_NAMES[role] || role} ${count}`).join(' · ');
    const close = make('button', 'close', '×');
    close.title = '닫기 (Esc)';
    close.onclick = () => closePanel();
    $panelHead.replaceChildren(
      make('b', '', next.title),
      make('span', 'code-panel-note', hits.length ? `${hits.length}곳 · 파일 ${files}개${summary ? ` — ${summary}` : ''}${next.truncated ? ' (많아서 앞의 것만 보입니다)' : ''}` : '찾은 곳이 없습니다'),
      ...(next.search ? [next.search] : []),
      close,
    );
    // 정의 → 쓰는 곳 → 문자열·주석 순서로, 그 안에서는 파일별로 묶는다.
    const order = { def: 0, use: 1, text: 1, string: 2, comment: 3 };
    const sorted = [...hits].sort((a, b) => (order[a.role] ?? 1) - (order[b.role] ?? 1) || a.file.localeCompare(b.file) || a.line - b.line || a.col - b.col);
    const rows = [];
    let lastFile = null, lastRole = null;
    for (const hit of sorted) {
      const group = hit.role === 'string' || hit.role === 'comment' ? 'quiet' : hit.role;
      if (group !== lastRole && sorted.some((other) => other.role !== hit.role)) {
        rows.push(make('div', 'code-hit-role', group === 'quiet' ? '주석·문자열 안' : ROLE_NAMES[hit.role] || hit.role));
        lastFile = null;
      }
      lastRole = group;
      if (hit.file !== lastFile) {
        const count = sorted.filter((other) => other.file === hit.file && ((other.role === 'string' || other.role === 'comment' ? 'quiet' : other.role) === group)).length;
        rows.push(make('div', 'code-hit-file', `${hit.file}  (${count})`));
        lastFile = hit.file;
      }
      const row = make('div', `code-hit ${hit.role}`);
      const text = make('span', 'code-hit-text');
      // 글에서 찾은 자리만 굵게 보인다(UTF-16 칸으로 자른다).
      text.append(hit.text.slice(0, hit.at), make('b', '', hit.text.slice(hit.at, hit.at + hit.len)), hit.text.slice(hit.at + hit.len));
      row.append(make('span', 'code-hit-line', String(hit.line)), text);
      row.tabIndex = -1;
      row.onclick = () => openFile(full(hit.file), { line: hit.line, col: hit.col, len: hit.len });
      rows.push(row);
    }
    $panelBody.replaceChildren(...rows);
    $panelBody.scrollTop = 0;
  }

  function closePanel() {
    panel = null;
    $panel.hidden = true;
    $scroll.focus({ preventScroll: true });
  }

  $panelBody.addEventListener('keydown', (ev) => {
    const rows = [...$panelBody.querySelectorAll('.code-hit')];
    const at = rows.indexOf(document.activeElement);
    if (ev.key === 'ArrowDown') rows[Math.min(rows.length - 1, at + 1)]?.focus();
    else if (ev.key === 'ArrowUp') rows[Math.max(0, at - 1)]?.focus();
    else if (ev.key === 'Enter') rows[at]?.click();
    else return;
    ev.preventDefault();
    ev.stopPropagation();
  });

  /** 프로젝트 전체에서 글 찾기: 아래 창에 찾는 칸과 결과를 보여 준다. */
  function openSearch(initial = '') {
    const input = make('input', 'code-search-input');
    Object.assign(input, { placeholder: '프로젝트 전체에서 찾을 글 (Enter)', spellcheck: false, autocomplete: 'off', value: initial || panel?.query || String(getSelection() || '').split('\n')[0].slice(0, 200) });
    const options = { case: false, word: false, regex: false, ...(panel?.options || {}) };
    const toggle = (key, text, tip) => {
      const b = make('button', options[key] ? 'on' : '', text);
      b.title = tip;
      b.onclick = () => { options[key] = !options[key]; b.classList.toggle('on', options[key]); run(); };
      return b;
    };
    const box = make('span', 'code-search');
    box.append(input, toggle('case', 'Aa', '대/소문자 구분'), toggle('word', '단어', '낱말 단위로'), toggle('regex', '.*', '정규식'));
    const run = async () => {
      const query = input.value;
      if (!query) return;
      setStatus(`"${query}"을(를) 찾는 중…`);
      const seq = ++panelSeq;
      try {
        const result = await invoke('code_search', { root, query, ...options });
        if (disposed || seq !== panelSeq) return;
        showPanel({ title: '전체에서 찾기', query, options, search: box, ...result });
        setStatus('');
        input.focus();
      } catch (err) {
        setStatus(String(err));
      }
    };
    input.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') { ev.preventDefault(); ev.stopPropagation(); run(); }
      else if (ev.key === 'ArrowDown') { ev.preventDefault(); $panelBody.querySelector('.code-hit')?.focus(); }
    });
    showPanel({ title: '전체에서 찾기', query: input.value, options, search: box, hits: panel?.query === input.value ? panel.hits : [] });
    input.focus();
    input.select();
    if (initial) run();
  }

  // ── 고르기 창(파일 찾기, 기호 찾기, 구조, 줄로 가기, 정의 고르기) ──

  let pickSeq = 0;
  /**
   * 코드 위에 뜨는 고르기 창. `source(query)`가 내놓은 것을 `row(item)`대로 보여 주고, 고르면 `choose(item)`을 부른다.
   * 위·아래로 고르고 Enter로 정한다. Esc나 바깥을 누르면 닫힌다.
   */
  function pick({ title, placeholder, source, row, choose, initial = '' }) {
    const seq = ++pickSeq;
    const input = make('input');
    Object.assign(input, { placeholder, spellcheck: false, autocomplete: 'off', value: initial });
    const $list = make('div', 'code-pick-list');
    const $head = make('div', 'code-pick-head', title);
    $pick.replaceChildren($head, input, $list);
    $pick.hidden = false;
    let items = [], at = 0, timer = 0, shownFor = null, loadSeq = 0;
    /** 창을 닫는다. `refocus`가 거짓이면(바깥을 눌러 닫힐 때) 초점을 코드로 가져오지 않는다: 사용자가 누른 곳에 그대로 둔다. */
    const close = (refocus = true) => { if (seq === pickSeq) { $pick.hidden = true; $pick.replaceChildren(); if (refocus) $scroll.focus({ preventScroll: true }); } };
    const mark = () => {
      [...$list.children].forEach((el, index) => el.classList.toggle('on', index === at));
      $list.children[at]?.scrollIntoView({ block: 'nearest' });
    };
    const draw = () => {
      $list.replaceChildren(...items.slice(0, MAX_PICK).map((each, index) => {
        const shown = row(each);
        const el = make('div', 'code-pick-row');
        el.append(make('span', 'code-kind', shown.icon || ''), make('span', 'code-pick-title', shown.title), make('span', 'code-pick-note', shown.note || ''));
        if (shown.sub) el.append(make('span', 'code-pick-sub', shown.sub));
        el.onmousedown = (ev) => { ev.preventDefault(); const typed = input.value; close(); choose(each, typed); };
        el.onmousemove = () => { if (at !== index) { at = index; mark(); } };
        return el;
      }));
      if (!items.length) $list.append(make('div', 'code-note-row', input.value ? '찾은 것이 없습니다.' : '찾을 글을 적으세요.'));
      else if (items.length > MAX_PICK) $list.append(make('div', 'code-note-row', `${items.length}개 가운데 앞의 ${MAX_PICK}개만 보입니다. 더 적어 좁히세요.`));
      at = 0;
      mark();
    };
    const load = async () => {
      const query = input.value.trim();
      const mine = ++loadSeq;
      let found = [];
      try {
        found = await source(query) || [];
      } catch {
        found = [];
      }
      // 늦게 온 옛 결과는 버린다.
      if (seq !== pickSeq || mine !== loadSeq) return;
      items = found;
      shownFor = query;
      draw();
    };
    /** 지금 적은 글의 결과가 보일 때까지 기다렸다가 고른 것을 돌려준다. */
    const settled = async () => {
      clearTimeout(timer);
      if (input.value.trim() !== shownFor) await load();
      return seq === pickSeq && input.value.trim() === shownFor ? items[at] : undefined;
    };
    input.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(load, 120); });
    input.addEventListener('keydown', (ev) => {
      const shownCount = Math.min(items.length, MAX_PICK);
      if (ev.key === 'ArrowDown') { at = Math.min(shownCount - 1, at + 1); mark(); }
      else if (ev.key === 'ArrowUp') { at = Math.max(0, at - 1); mark(); }
      else if (ev.key === 'PageDown') { at = Math.min(shownCount - 1, at + 10); mark(); }
      else if (ev.key === 'PageUp') { at = Math.max(0, at - 10); mark(); }
      else if (ev.key === 'Enter') { const typed = input.value; settled().then((chosen) => { if (seq !== pickSeq) return; close(); if (chosen) choose(chosen, typed); }); }
      else if (ev.key === 'Escape') close();
      else return;
      ev.preventDefault();
      ev.stopPropagation();
    });
    input.addEventListener('blur', () => setTimeout(() => { if (!$pick.contains(document.activeElement)) close(false); }, 100));
    input.focus();
    input.select();
    load();
  }

  async function pickFile() {
    // 목록은 열 때마다 다시 읽는다(Claude가 새 파일을 만들었을 수 있다). 읽는 동안에는 지난번 목록으로 찾는다.
    const loading = invoke('code_files', { root }).then((files) => { fileList = files; return files; }).catch(() => fileList || []);
    pick({
      title: '파일 찾기',
      placeholder: '파일 이름 (예: main, user.rs, src/ui) · 이름:줄번호',
      source: async (given) => {
        const files = fileList || await loading;
        const query = given.replace(/:\d*$/, '');
        if (!query) return tabs.slice().sort((a, b) => b.usedAt - a.usedAt).filter((tab) => tab !== current && lower(tab.path).startsWith(rootPrefix())).map((tab) => relative(tab.path));
        const ranked = [];
        const wantsPath = query.includes('/') || query.includes('\\');
        for (const file of files) {
          const name = file.slice(file.lastIndexOf('/') + 1);
          let rank = wantsPath ? -1 : matchRank(query, name);
          if (rank < 0) { const whole = matchRank(slash(query), file); if (whole >= 0) rank = whole + 5; }
          if (rank >= 0) ranked.push([rank, file]);
        }
        ranked.sort((a, b) => a[0] - b[0] || a[1].length - b[1].length || a[1].localeCompare(b[1]));
        return ranked.slice(0, 400).map(([, file]) => file);
      },
      row: (file) => ({ icon: git.files.has(lower(full(file))) ? '●' : '', title: file.slice(file.lastIndexOf('/') + 1), note: file.slice(0, Math.max(0, file.lastIndexOf('/'))) }),
      choose: (file, typed) => {
        const line = Number(/:(\d+)$/.exec(typed || '')?.[1]) || 0;
        openFile(full(file), line ? { line } : null);
      },
    });
  }

  function pickSymbol() {
    const word = caretWord()?.word || '';
    pick({
      title: '기호 찾기 (함수·클래스·필드 …)',
      placeholder: '이름 (머리글자도 됩니다: gU → getUser)',
      initial: word,
      source: (query) => (query ? invoke('code_symbols', { root, query }) : []),
      row: (def) => ({ icon: KIND_ICONS[def.kind] || '·', title: `${def.container ? `${def.container}.` : ''}${def.name}`, note: `${def.file}:${def.line}`, sub: def.text }),
      choose: (def) => openFile(full(def.file), { line: def.line, col: def.col, len: def.name.length }),
    });
  }

  function pickOutline() {
    if (!current) return setStatus('먼저 파일을 여세요.');
    const defs = current.outline || [];
    pick({
      title: `이 파일의 구조 — ${nameOf(current.path)}`,
      placeholder: '이름으로 걸러 보기',
      source: (query) => (query ? defs.map((def) => [matchRank(query, def.name), def]).filter(([rank]) => rank >= 0).sort((a, b) => a[0] - b[0] || a[1].line - b[1].line).map(([, def]) => def) : defs),
      row: (def) => ({ icon: KIND_ICONS[def.kind] || '·', title: `${'  '.repeat(Math.min(6, def.depth))}${def.name}`, note: `${KIND_NAMES[def.kind] || def.kind} · ${def.line}` }),
      choose: (def) => { remember(); jump({ line: def.line, col: def.col, len: def.name.length }); },
    });
  }

  function pickLine() {
    if (!current?.lines) return;
    pick({
      title: `줄로 가기 (1~${current.lines.length})`,
      placeholder: '줄 번호',
      source: (query) => { const line = Number(query); return Number.isInteger(line) && line >= 1 && line <= current.lines.length ? [line] : []; },
      row: (line) => ({ icon: '↳', title: `${line}번째 줄`, note: '', sub: (current.lines[line - 1] || '').trim().slice(0, 160) }),
      choose: (line) => { remember(); jump({ line }); },
    });
  }

  async function pickRoot() {
    const picked = await invoke('pick_folder', { start: root, title: '코드를 볼 프로젝트 폴더' }).catch(() => null);
    if (picked) setRoot(picked);
  }

  async function setRoot(next) {
    root = norm(next).replace(/\\+$/, '');
    if (/^[A-Za-z]:$/.test(root)) root += '\\'; // 드라이브 맨 위
    item.cwd = root; // 스테이지 맨 위의 경로와 "폴더 열기"도 새 폴더를 따른다
    epoch += 1; // 읽고 있던 옛 프로젝트의 결과는 버린다
    openSeq += 1;
    panelSeq += 1;
    tabs = [];
    current = null;
    back = [];
    forward = [];
    fileList = null;
    listed.clear();
    expanded.clear();
    touched.clear();
    git = { root: '', branch: '', files: new Map() };
    clearMarks();
    closePanel();
    $code.replaceChildren();
    $code.hidden = true;
    $empty.hidden = false;
    welcome();
    renderTabs();
    renderCrumbs();
    renderOutline();
    refreshBar();
    await list(root);
    renderTree();
    refreshGit();
    onChange();
  }

  // ── 이 파일에서 찾기 ──

  function openFind() {
    if (!current?.lines) return;
    findOpen = true;
    $find.hidden = false;
    const picked = String(getSelection() || '');
    if (picked && !picked.includes('\n') && picked.length < 200) $query.value = picked;
    $query.focus();
    $query.select();
    runFind(false);
  }
  function closeFind() {
    findOpen = false;
    $find.hidden = true;
    findHits = [];
    repaint();
    $scroll.focus({ preventScroll: true });
  }
  /** 찾는 글이 나온 자리를 모두 표시한다. `keep`이면 지금 고른 자리를 지킨다(파일이 다시 읽혔을 때). */
  function runFind(keep) {
    const query = $query.value.toLowerCase();
    findHits = [];
    if (query && current?.lines) {
      current.lines.forEach((text, index) => {
        const low = text.toLowerCase();
        for (let at = low.indexOf(query); at >= 0 && findHits.length < MAX_FIND; at = low.indexOf(query, at + query.length)) findHits.push({ line: index + 1, col: at, len: query.length });
      });
    }
    if (!keep) {
      // 지금 보이는 자리에서 가장 가까운(아래쪽) 것부터.
      const top = topLine();
      const near = findHits.findIndex((hit) => hit.line >= top);
      findAt = near < 0 ? 0 : near;
    } else {
      findAt = Math.min(findAt, Math.max(0, findHits.length - 1));
    }
    showFind(!keep);
  }
  function showFind(move) {
    // 찾은 곳이 많아 다 세지 않았으면 +를 붙인다.
    $count.textContent = $query.value ? (findHits.length ? `${findAt + 1}/${findHits.length}${findHits.length >= MAX_FIND ? '+' : ''}` : '없음') : '';
    const hit = findHits[findAt];
    if (move && hit) {
      scrollToLine(hit.line, true);
      revealAcross(hit.line, hit.col, hit.len);
      caret = { line: hit.line, col: hit.col }; // 경로 줄의 "지금 있는 곳"도 따라온다
      renderCrumbs();
    }
    repaint();
  }
  function stepFind(step) {
    if (!findHits.length) return;
    findAt = (findAt + step + findHits.length) % findHits.length;
    showFind(true);
  }
  $query.addEventListener('input', () => runFind(false));
  $query.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') { ev.preventDefault(); ev.stopPropagation(); stepFind(ev.shiftKey ? -1 : 1); }
  });

  // ── 그 밖의 동작 ──

  const copyText = (text) => invoke('clipboard_write', { text }).then(() => setStatus('복사했습니다.')).catch((err) => setStatus(String(err)));
  const outside = (path) => invoke('link_open', { target: path, cwd: dirOf(path) || root, openFile: false, line: null, col: null }).catch((err) => setStatus(String(err)));
  const selected = () => { const sel = getSelection(); return sel && !sel.isCollapsed && $code.contains(sel.anchorNode) ? String(sel) : ''; };

  /** 고른 글의 줄 범위(없으면 마지막으로 누른 줄). */
  function pickedLines() {
    const sel = getSelection();
    const lineOf = (node) => Number((node?.nodeType === 3 ? node.parentElement : node)?.closest?.('.cl')?.dataset.n) || 0;
    if (sel && !sel.isCollapsed && $code.contains(sel.anchorNode)) {
      const a = lineOf(sel.anchorNode), b = lineOf(sel.focusNode);
      if (a && b) return [Math.min(a, b), Math.max(a, b)];
    }
    return caret ? [caret.line, caret.line] : null;
  }

  /** 파일 경로와 줄 번호(고른 글이 있으면 그 코드도)를 Claude 세션 입력창에 넣는다. */
  function toClaude(ev, withCode) {
    if (!current) return;
    const lines = pickedLines();
    const where = `${relative(current.path)}${lines ? `:${lines[0]}${lines[1] > lines[0] ? `-${lines[1]}` : ''}` : ''}`;
    const code = withCode && lines ? `\n\`\`\`\n${current.lines.slice(lines[0] - 1, lines[1]).join('\n')}\n\`\`\`\n` : ' ';
    api.toClaude?.(item, `${where}${code}`, ev);
  }

  $code.addEventListener('click', (ev) => {
    const at = pointAt(ev);
    if (!at) return;
    caret = at;
    const found = wordAt(at.line, at.col);
    if (ev.ctrlKey && found) { ev.preventDefault(); goToDefinition(found); return; }
    // 누른 이름과 같은 이름을 이 파일에서 모두 옅게 표시한다(글자를 끌어 고르는 중이 아닐 때).
    if (!selected()) {
      sameWord = found && !inQuiet(at.line, at.col) ? found.word : '';
      repaint();
    }
    renderCrumbs();
  });
  // Ctrl을 누른 채 이름 위에 올리면 "누르면 정의로 간다"는 표시를 한다.
  $code.addEventListener('mousemove', (ev) => {
    const found = ev.ctrlKey ? (() => { const at = pointAt(ev); return at && wordAt(at.line, at.col); })() : null;
    const range = found ? rangeOf(found.line, found.from, found.to - found.from) : null;
    if (!range && !marks.link.length) return;
    marks.link = range ? [range] : [];
    $code.classList.toggle('linking', !!range);
    paint();
  });
  $code.addEventListener('mouseleave', () => { if (marks.link.length) { marks.link = []; $code.classList.remove('linking'); paint(); } });
  $code.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    const at = pointAt(ev);
    const picked = selected();
    if (at && !picked) caret = at;
    const found = picked && !/\s/.test(picked) ? caretWord() : at ? wordAt(at.line, at.col) : null;
    showMenu(ev, [
      ...(found ? [
        { caption: found.word },
        { label: '정의로 가기  (Ctrl+클릭, F12)', action: () => goToDefinition(found) },
        { label: '쓰는 곳 찾기  (Alt+F7)', action: () => findUsages(found) },
        { label: '전체에서 이 글 찾기', action: () => openSearch(found.word) },
      ] : []),
      ...(picked ? [{ label: '고른 글 복사  (Ctrl+C)', action: () => copyText(picked) }] : []),
      { label: '이 파일에서 찾기…  (Ctrl+F)', action: () => openFind() },
      { label: 'Claude에 넣기: 경로와 줄 번호', action: () => toClaude(ev, false) },
      ...(pickedLines() ? [{ label: 'Claude에 넣기: 경로와 이 코드', action: () => toClaude(ev, true) }] : []),
      { label: '경로 복사', action: () => copyText(current?.path || '') },
      { label: '탐색기에서 보기', action: () => current && outside(current.path) },
    ]);
  });
  $scroll.addEventListener('scroll', () => { if (shown && current && !caret) renderCrumbsSoon(); }, { passive: true });
  // Ctrl+휠: 코드 글자 크기
  $scroll.addEventListener('wheel', (ev) => {
    if (!ev.ctrlKey) return;
    ev.preventDefault();
    setCodeFont({ size: codeFont.size + (ev.deltaY < 0 ? 1 : -1) });
  }, { passive: false });
  let crumbTimer = 0;
  const renderCrumbsSoon = () => { clearTimeout(crumbTimer); crumbTimer = setTimeout(renderCrumbs, 150); };

  rootEl.addEventListener('keydown', (ev) => {
    const key = ev.key.toLowerCase();
    const plain = !ev.ctrlKey && !ev.altKey && !ev.shiftKey;
    const ctrl = ev.ctrlKey && !ev.altKey && !ev.shiftKey;
    const typing = ev.target.tagName === 'INPUT';
    let handled = true;
    if (key === 'escape' && !$pick.hidden) { pickSeq += 1; $pick.hidden = true; $scroll.focus({ preventScroll: true }); }
    else if (key === 'escape' && findOpen) closeFind();
    else if (key === 'escape' && panel) closePanel();
    else if (typing && !(ctrl && ['p', 't', 'g', 'f'].includes(key)) && key !== 'f12') handled = false;
    else if (key === 'f12' && ev.ctrlKey) pickOutline();
    else if ((key === 'f12' && plain) || (ctrl && key === 'b')) goToDefinition();
    else if ((key === 'f7' && ev.altKey) || (key === 'f12' && ev.shiftKey)) findUsages();
    else if (ctrl && key === 'f') openFind();
    else if (ctrl && key === 'p') pickFile();
    else if (ctrl && key === 't') pickSymbol();
    else if (ctrl && key === 'g') pickLine();
    else if (ctrl && key === 'c' && selected()) copyText(selected());
    else if (ev.ctrlKey && !ev.altKey && (key === '=' || key === '+')) setCodeFont({ size: codeFont.size + 1 });
    else if (ev.ctrlKey && !ev.altKey && key === '-') setCodeFont({ size: codeFont.size - 1 });
    else if (ctrl && key === '0') setCodeFont({ size: FONT_SIZE });
    else if (ev.altKey && !ev.ctrlKey && key === 'arrowleft') travel(-1);
    else if (ev.altKey && !ev.ctrlKey && key === 'arrowright') travel(1);
    else if (ctrl && key === 'f4' && current) closeTab(current);
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });

  // ── 바뀌면 다시 읽기 ──

  let checking = false;
  /** 보고 있는 파일이 바뀌었으면 다시 읽어 그린다(보던 자리는 지킨다). */
  async function check() {
    const tab = current;
    if (!tab || checking || disposed) return;
    checking = true;
    try {
      const stamp = await invoke('reader_stamp', { path: tab.path });
      if (stamp === (tab.triedAt ?? (tab.lines ? tab.modified : null)) || tab !== current) return;
      if (stamp == null) {
        // 파일이 없어졌다: 보던 내용은 그대로 두고 알리기만 한다.
        if (tab.lines && !tab.gone) { tab.gone = true; setStatus(`파일이 없어졌습니다: ${tab.path}`); renderTabs(); }
        return;
      }
      const top = shown ? $scroll.scrollTop : lastScroll;
      tab.triedAt = stamp; // 읽지 못해도 같은 내용을 2초마다 다시 읽지 않는다
      const fresh = await read(tab.path);
      if (tab !== current || disposed) return;
      const keepCaret = caret;
      const wasGone = tab.gone;
      Object.assign(tab, fresh, { missing: '', gone: false, triedAt: undefined });
      if (wasGone) renderTabs();
      $empty.hidden = true;
      $code.hidden = false;
      renderCode(tab);
      lastScroll = top;
      if (shown) $scroll.scrollTop = top;
      caret = keepCaret;
      if (findOpen) runFind(true); else repaint();
      invoke('code_outline', { path: tab.path }).then((outline) => { tab.outline = outline; if (current === tab) { renderOutline(); renderCrumbs(); } }).catch(() => {});
      await loadMarks(tab);
      applyMarks(tab);
    } catch { /* 읽지 못하면 다음에 다시 본다 */ } finally {
      checking = false;
    }
  }

  let gitTimer = 0;
  /** 바뀐 파일과 나무를 곧 다시 읽는다(잇달아 불러도 한 번만). */
  function refreshSoon() {
    clearTimeout(gitTimer);
    gitTimer = setTimeout(async () => {
      if (disposed) return;
      // 펼쳐 둔 폴더를 다시 읽는다(새 파일이 나무에 나타나게).
      const dirs = [root, ...[...listed.keys()].filter((dir) => expanded.has(dir))].slice(0, 60);
      const before = JSON.stringify(dirs.map((dir) => listed.get(lower(dir))));
      await Promise.all(dirs.map((dir) => list(dir)));
      if (JSON.stringify(dirs.map((dir) => listed.get(lower(dir)))) !== before) renderTree();
      await refreshGit();
    }, 300);
  }

  // 숨겨졌다 다시 보이면 보던 자리로 돌려놓고 바로 살핀다.
  $scroll.addEventListener('scroll', () => { if (shown && heldScroll == null && $scroll.clientHeight > 0) lastScroll = $scroll.scrollTop; }, { passive: true });
  const watcher = new ResizeObserver(() => {
    const now = $scroll.clientHeight > 0;
    if (now && heldScroll != null) { lastScroll = heldScroll; heldScroll = null; }
    if (now && !shown) {
      shown = true;
      if (Math.abs($scroll.scrollTop - lastScroll) > 1) $scroll.scrollTop = lastScroll;
      check();
      refreshSoon();
    } else if (!now) {
      shown = false;
    }
  });
  watcher.observe($scroll);
  const watchTimer = setInterval(() => { if (shown && !document.hidden) check(); }, WATCH_MS);
  const slowTimer = setInterval(() => { if (shown && !document.hidden) refreshSoon(); }, GIT_MS);

  const view = {
    ready: null,
    focus: () => (current ? $scroll : $tree).focus({ preventScroll: true }),
    title,
    /** 사이드바의 그 줄 아래에 보이는 글: 지금 보는 파일. */
    sub: () => (current ? relative(current.path) : ''),
    state: () => ({
      root,
      tabs: tabs.map((tab) => tab.path),
      current: current?.path || '',
      line: current?.lines ? topLine() : 1,
      expanded: [...expanded],
      side: sideWidth,
      changedOnly,
    }),
    close: () => Promise.resolve(true),
    dispose() {
      disposed = true;
      clearInterval(watchTimer);
      clearInterval(slowTimer);
      clearTimeout(gitTimer);
      clearTimeout(crumbTimer);
      watcher.disconnect();
      dresser.disconnect();
      painted.delete(marks);
      paint();
      fontViews.delete(applyFont);
    },
    /** 화면 배치를 다시 짓기 직전: 보던 자리를 적어 둔다(창을 떼었다 붙이면 스크롤이 맨 위로 돌아간다). */
    freeze() { if (heldScroll == null && shown && $scroll.clientHeight > 0) heldScroll = $scroll.scrollTop; },
    /** 배치를 다시 지은 뒤: 보던 자리로 되돌린다. */
    resync() {
      if (heldScroll != null) { lastScroll = heldScroll; heldScroll = null; }
      if ($scroll.clientHeight > 0 && Math.abs($scroll.scrollTop - lastScroll) > 1) $scroll.scrollTop = lastScroll;
    },
    /** Ctrl+Shift+F: 프로젝트 전체에서 찾기(이 파일에서 찾기는 Ctrl+F). */
    find: () => openSearch(),
    root: () => root,
    /** 이 항목이 그 파일을 보여 주고 있는지(열어 둔 탭). */
    shows: (path) => tabs.some((tab) => lower(tab.path) === lower(path)),
    /** 보고 있는 파일이 밖에서 고쳐졌다: 바로 다시 읽는다. */
    refresh: check,
    /** 같은 워크스페이스의 Claude 세션이 파일을 썼다: 이 프로젝트의 파일이면 나무·바뀐 파일을 곧 다시 읽고 "방금 쓴 파일"로 표시한다. */
    fileChanged(path) {
      if (!lower(path).startsWith(rootPrefix())) return;
      touched.set(lower(path), Date.now());
      refreshSoon();
      if (current && lower(current.path) === lower(path)) check();
    },
    open: (path, at) => openFile(path, at),
    // 시험·확인용
    goToDefinition, findUsages, pickFile, pickSymbol, openSearch, openFind, setRoot, setFont: setCodeFont, font: () => ({ ...codeFont }),
    current: () => current,
    wordAt, setCaret: (line, col) => { caret = { line, col }; },
  };

  welcome();
  refreshBar();
  renderOutline();
  applyFont();
  view.ready = (async () => {
    if (!root) return;
    await list(root);
    for (const dir of [...expanded]) {
      // 펼쳐 두었던 폴더를 읽는다(없어진 폴더는 접는다).
      await list(dir);
      if (!(listed.get(dir) || []).length) expanded.delete(dir);
    }
    renderTree();
    refreshGit();
    // 같은 파일이 표기만 달리 두 번 적혀 있으면 하나만 연다.
    const seen = new Set();
    const wanted = (Array.isArray(saved.tabs) ? saved.tabs : [])
      .filter((path) => typeof path === 'string' && path && !seen.has(lower(path)) && seen.add(lower(path)))
      .map(norm)
      .slice(0, MAX_TABS);
    // 지난번에 열어 둔 파일을 다시 연다. 마지막에 보던 파일이 맨 나중에 열려 그 파일이 보인다.
    const last = wanted.find((path) => lower(path) === lower(saved.current || '')) || wanted[wanted.length - 1];
    for (const path of wanted.filter((path) => path !== last)) {
      const tab = { path: norm(path), lines: null, tokens: null, lang: langOf(path), modified: 0, top: 1, outline: [], marks: new Map(), usedAt: 0, missing: '' };
      try { Object.assign(tab, await read(tab.path)); tabs.push(tab); } catch { /* 없어진 파일은 건너뛴다 */ }
    }
    if (last) {
      await openFile(last, null, { record: false });
      if (current?.lines && saved.line > 1) scrollToLine(saved.line, false);
    }
    renderTabs();
  })().catch(() => {});
  return view;
}
