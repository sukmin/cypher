// Cypher 화면: 세션 목록, 나란히 보는 터미널 창, 링크·붙여넣기·알림.
// 빌드 단계 없이 그대로 실행 파일에 들어간다.
import {
  initExtras, showSnippets, showHelp, showEngine, showMemory, showMenu, toggleDiff, showDiff, diffRefreshFor, openPopup,
} from './extras.js';
import { initHttp, createHttp } from './http.js';
import { initMemo, createMemo } from './memo.js';
import { initSite, createSite, syncSites } from './site.js';
import { initImage, createImage } from './image.js';
import { initDrop } from './drop.js';
import { initMindmap, createMindmap } from './mindmap.js';
import { initLook, showLook, setLook, currentLook } from './look.js';
import { initPalette, showPalette } from './palette.js';
import { initClosed, rememberClosed, reopenLast, showClosed, closedCount } from './closed.js';

const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// 화면 코드에서 난 오류를 모아 둔다(문제 확인용).
window.cypherErrors = [];
window.addEventListener('error', (ev) => window.cypherErrors.push(String(ev.message)));
window.addEventListener('unhandledrejection', (ev) => window.cypherErrors.push(String(ev.reason)));

const $ = (id) => document.getElementById(id);
const $terms = $('terms'), $list = $('sessions'), $folder = $('folder');
const $search = $('search'), $searchInput = $('search-input');

const BUSY_MS = 1200;           // 이 시간 안에 출력이 있었으면 "작업 중"으로 본다.
const HELD_STOP_MS = 30000;      // 서브에이전트 때문에 미룬 완료 알림을, 이만큼 조용하면 그냥 알린다.
const MAX_PANES = 8;            // 한 화면에 나눠 볼 수 있는 창 수
const MAX_SHOTS = 8;            // 세션마다 남길 이미지 미리보기 수
const FLOW_HIGH = 2 << 20;      // 화면에 못 그린 출력이 이만큼 쌓이면 읽기를 멈춘다.
const FLOW_LOW = 256 << 10;     // 이만큼으로 줄면 다시 읽는다.
const FOCUS_GUARD_MS = 250;     // 창을 고르려던 클릭이 링크를 열지 않게 하는 시간
const CONTEXT_WARN = 80;        // 컨텍스트 사용률이 이 이상이면 노란색,
const CONTEXT_DANGER = 90;      // 이 이상이면 빨간색으로 경고한다.
const COLORS = ['', '#e06c75', '#e5c07b', '#98c379', '#61afef', '#c678dd', '#56b6c2'];
const SIDEBAR_WIDTH = 240;      // 사이드바 기본 너비
const COMPOSER_HEIGHT = 110;    // 프롬프트 편집기 기본 높이
const MAX_FOLDERS = 8;          // 기억해 둘 최근 폴더 수
const MAX_PROMPTS = 50;         // 기억해 둘 보낸 프롬프트 수

/**
 * 사이드바에 놓을 수 있는 항목의 종류. 터미널 세션(Claude·PowerShell 등 프로필)은 따로 다루고,
 * 그 밖의 종류는 여기에 올린다. 새 종류를 더하려면 화면을 만드는 함수 하나를 등록하면 된다.
 *   create(item, host, saved, onChange) → { ready, focus(), title(), state(), close(), dispose(), insert?(text) }
 */
const KINDS = {
  memo: { label: '메모', icon: '✎', hint: '글자 편집기(찾기·바꾸기, 파일 열기·저장, 임시 저장)', create: createMemo },
  http: { label: 'HTTP', icon: '⇄', hint: 'HTTP 요청(.http 파일)을 쓰고 보내고 응답 보기', create: createHttp },
  site: { label: '사이트', icon: '◎', hint: '브라우저: 빈 페이지로 열고 위의 주소 칸에 주소를 쳐서 갑니다. 같은 그룹의 Claude 세션이 조작할 수 있습니다', create: createSite },
  image: { label: '이미지', icon: '▣', hint: '그림(파일, 붙여넣기, 사이트 화면 캡처)을 항목으로 두고 보기', create: createImage },
  mindmap: { label: '브레인스토밍', icon: '✺', hint: '마인드맵: 그림으로 가지를 뻗거나 코드(개요 글)로 쓰고, 표식을 붙이고, 저장하거나 PNG·PDF로 내보내기', create: createMindmap },
};

/** 그 종류의 항목을 새로 만든다. 종류에 따라 먼저 물어볼 것(사이트 주소 등)을 묻는다. */
async function createKind(kind, opts = {}) {
  const def = KINDS[kind];
  if (!def) return null;
  let { saved } = opts;
  if (!saved && def.prepare) {
    saved = await def.prepare();
    if (!saved) return null;
  }
  return newItem(kind, { ...opts, saved });
}

/** @type {Map<number, object>} 화면용 번호(key) → 항목(터미널 세션, 메모, HTTP …) */
const sessions = new Map();
let visible = [];               // 나란히 보이는 세션(왼쪽 위부터 순서대로)
let active = null;
let seq = 0;
let config = null;
let folder = '';
let lastProfile = 'claude';
let broadcast = false;
let closing = false;
let limits = null;              // 계정 사용 한도(어느 Claude 세션이든 마지막으로 알려 준 값)
let filterWaiting = false;      // 대기 중인 세션만 목록에 보이기
let collapsed = new Set();      // 접어 둔 묶음(그룹·폴더)의 열쇠
let drag = null;                // 목록에서 끌고 있는 세션 { s, x, y, active, ghost }
let restoring = false;          // 지난번 항목을 되살리는 중(이 동안에는 상태를 적지 않는다)
let renaming = null;            // 사이드바에서 별칭을 고치고 있는 항목의 번호
let composerFor = null;         // 프롬프트 편집기가 지금 맡고 있는 세션
try { collapsed = new Set(JSON.parse(localStorage.getItem('cypher.collapsed') || '[]')); } catch { /* 무시 */ }
let storeDirs = {};            // 메모·브레인스토밍·이미지를 두는 폴더(같은 그룹의 Claude 세션에 경로를 알려 줄 때 쓴다)
let groupList = [];            // 만들어 둔 그룹 이름(세션이 없어도 남는다)
try { groupList = JSON.parse(localStorage.getItem('cypher.groups') || '[]'); } catch { /* 무시 */ }
let recentFolders = [];        // 세션을 시작했던 폴더(최근 것이 앞)
try { recentFolders = JSON.parse(localStorage.getItem('cypher.folders') || '[]'); } catch { /* 무시 */ }
let promptLog = [];            // 프롬프트 편집기에서 보낸 글(최근 것이 뒤)
try { promptLog = JSON.parse(localStorage.getItem('cypher.prompts') || '[]'); } catch { /* 무시 */ }
if (!Array.isArray(recentFolders)) recentFolders = [];
if (!Array.isArray(promptLog)) promptLog = [];
let searchOptions = { caseSensitive: false, regex: false }; // 터미널 화면에서 찾기의 방식
try { searchOptions = { ...searchOptions, ...JSON.parse(localStorage.getItem('cypher.search') || '{}') }; } catch { /* 무시 */ }

// ───────────────────────── 시작 ─────────────────────────

async function init() {
  const loaded = await invoke('app_config');
  config = loaded.config;
  if (loaded.error) setStatus(`설정 파일 오류(기본값으로 동작): ${loaded.error}`);
  folder = localStorage.getItem('cypher.folder') || await invoke('home_dir');
  showFolder();
  storeDirs = await invoke('store_dirs').catch(() => ({}));
  // 테마·글꼴·글자 크기를 적용한다. 바뀔 때마다 열려 있는 터미널과 항목 화면을 맞춘다.
  initLook({ invoke, make, openPopup, config, onChange: applyLook });

  const addButton = (label, title, onClick) => {
    const b = document.createElement('button');
    b.textContent = label;
    b.title = `${title} (Ctrl+클릭: 나란히 열기)`;
    b.onclick = (ev) => onClick(ev.ctrlKey ? 'split' : 'replace');
    $('new-buttons').append(b);
  };
  for (const p of config.profiles) {
    addButton(`+ ${p.label}`, `${p.program} ${p.args.join(' ')}`.trim(), (show) => newSession(p.id, { show }));
  }
  if (config.profiles.some((p) => p.id === 'claude')) {
    addButton('+ 워크트리', '새 git 워크트리에서 Claude 시작', newWorktree);
  }
  // 터미널이 아닌 항목(메모, HTTP …)
  for (const [kind, def] of Object.entries(KINDS)) {
    addButton(`+ ${def.label}`, def.hint, (show) => createKind(kind, { show }));
  }
  // 새 그룹 만들기
  const groupButton = make('button', '', '+ 그룹');
  groupButton.title = '이름 붙인 새 그룹 만들기(업무 하나를 한 그룹으로). 항목을 끌어다 놓거나 그룹 머리의 +로 채웁니다';
  groupButton.onclick = newGroup;
  $('new-buttons').append(groupButton);

  initExtras({
    invoke, make, ask, setStatus, newSession, label,
    getActive: () => active,
    getFolder: () => folder,
    focusActive: () => focusItem(active),
    pasteText: (text) => pasteInto(active, text),
  });
  initHttp({ invoke, make, ask, setStatus, baseName });
  initMemo({
    invoke, make, ask, setStatus, baseName, showMenu,
    openMemo: (path) => newItem('memo', { saved: { open: path }, show: 'split' }),
  });
  initSite({
    invoke, listen, make, setStatus, showMenu, captured: onCaptured,
    zoom: () => currentLook().zoom,
    // 이 브라우저를 넘겨받을 수 있는 Claude 세션: 같은 그룹에서 돌고 있는 것.
    claudes: (item) => claudesOf(item).map((s) => ({ s, label: label(s) })),
    // 지금 이 브라우저를 조작하는 Claude 세션의 이름(없으면 빈 글).
    controller: (item) => { const by = [...sessions.values()].find((s) => s.browser === item.key && !s.exited); return by ? label(by) : ''; },
    handoff: handoffSite,
  });
  initMindmap({ invoke, make, setStatus, showMenu, baseName, addImages });
  initImage({
    invoke, make, setStatus, showMenu, addImages, toClaude: imageToClaude,
    // 같은 묶음의 이미지 항목들(사이드바에 놓인 순서). 이미지 화면에서 이전/다음으로 넘길 때 쓴다.
    siblings: (s) => [...sessions.values()].filter((other) => other.kind === 'image' && groupKey(other) === groupKey(s)),
    show: (s) => activate(s),
  });
  initDrop({
    invoke, listen, setStatus, quote, newItem, addImages, pasteInto, reveal,
    itemOfPane: (pane) => (pane ? [...sessions.values()].find((s) => s.pane === pane) : null),
    itemOfRow: (row) => sessions.get(Number(row.dataset.key)),
    contextOfHead: (head) => ({ group: head.dataset.group || '', cwd: head.dataset.cwd || folder }),
    screenEmpty: () => !visible.length,
  });
  initPalette({ make, openPopup, entries: paletteEntries });
  initClosed({
    invoke, make, openPopup, setStatus, notice, baseName,
    revive: (record) => revive(record),
    icon: (record) => (KINDS[record.kind]?.icon || '›'),
    kindLabel: (record) => (KINDS[record.kind]?.label || config.profiles.find((p) => p.id === record.profile)?.label || '세션'),
  });
  initSidebarGrip();
  initComposer();
  // 사이드바 아래에는 자주 쓰는 것만 둔다. 나머지는 창 상단 메뉴에 있다. 브라우저는 사이드바의 사이트 항목이다.
  $('waiting').onclick = () => { filterWaiting = !filterWaiting; render(); };
  $('changes').onclick = () => toggleDiff();
  if (chromeEngine()) {
    // 설정에서 브라우저 엔진을 "chrome"으로 바꿨을 때만: 사이트 항목 대신 설치된 Chrome을 띄워 Claude가 조작한다.
    // 그 브라우저는 Cypher 창 밖에 있으므로 여는 단추·넘기기·캡처를 여기에 둔다.
    const tool = (text, id, title, onClick, onMenu) => {
      const b = make('button', '', text);
      Object.assign(b, { id, title, onclick: onClick });
      if (onMenu) b.oncontextmenu = (ev) => { ev.preventDefault(); ev.stopPropagation(); onMenu(ev); };
      return b;
    };
    $('tools').append(
      tool('Chrome', 'browser', '설치된 Chrome(또는 Edge)을 Cypher 전용 프로필로 엽니다. 여기서 로그인하고 작업하던 것을 Claude 세션이 이어서 조작할 수 있습니다 (Ctrl+Shift+O)', openChrome),
      tool('넘기기', 'handoff', 'Chrome에서 보고 있는 페이지를 Claude 세션에 넘기기 (오른쪽 클릭: 새 Claude 세션에 넘기기)', () => handoffChrome(false), () => handoffChrome(true)),
      tool('캡처', 'capture', 'Chrome에 보이는 화면을 찍어 지금 보고 있는 항목의 묶음에 이미지 항목으로 넣기 (Shift+클릭: 페이지 전체)', (ev) => captureBrowser(ev.shiftKey)),
    );
    listen('image-captured', ({ payload }) => onCaptured(payload, null));
  }
  // 같은 그룹의 Claude 세션이 Cypher 도구로 물어 오는 것(조작할 브라우저, 그룹의 항목, 브레인스토밍 쓰기)에 답한다.
  listen('mcp-request', ({ payload }) => onMcpRequest(payload));
  initSidebarKeys();
  listen('menu', ({ payload }) => runMenu(payload));

  $('pick').onclick = pickFolder;
  $('recent').onclick = folderMenu;
  // 사이드바의 빈 곳(세션·버튼이 아닌 곳)을 오른쪽 클릭하면 할 수 있는 일을 메뉴로 보여 준다.
  $('sidebar').addEventListener('contextmenu', (ev) => {
    if (ev.target.closest('li, button')) return;
    ev.preventDefault();
    sidebarMenu(ev);
  });
  $('search-prev').onclick = () => find(true);
  $('search-next').onclick = () => find(false);
  $('search-close').onclick = closeSearch;
  $('search-case').onclick = () => setSearchOption('caseSensitive');
  $('search-regex').onclick = () => setSearchOption('regex');
  setSearchOption(); // 지난번에 켜 둔 방식을 단추에 표시한다.
  // 글자를 치는 대로 찾는다(치는 동안에는 지금 찾은 자리에서 벗어나지 않는다).
  $searchInput.addEventListener('input', () => find(false, true));
  $searchInput.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') find(ev.shiftKey);
    else if (ev.key === 'Escape') closeSearch();
    else if (ev.altKey && ev.key.toLowerCase() === 'c') { ev.preventDefault(); setSearchOption('caseSensitive'); }
    else if (ev.altKey && ev.key.toLowerCase() === 'x') { ev.preventDefault(); setSearchOption('regex'); }
  });

  listen('session-exit', ({ payload }) => onExit(payload));
  listen('session-status', ({ payload }) => onStatus(payload));
  listen('session-hook', ({ payload }) => onHook(payload));
  listen('system-usage', ({ payload }) => onUsage(payload));
  // 창에 끌어다 놓은 파일·글자·링크·그림은 drop.js가 받는다(터미널에는 입력하고, 그 밖에는 항목으로 만든다).

  new ResizeObserver(fitVisible).observe($terms);
  setInterval(refreshActivity, 400);
  window.addEventListener('beforeunload', () => {
    // 메모의 임시 저장과 HTTP 편집기의 고친 내용을 밀어 넣어 본다(끝까지 기다릴 수는 없다).
    for (const s of sessions.values()) s.view?.flush?.();
    writeState();
    closing = true;
  });

  const auto = await invoke('autostart');
  if (auto) {
    // 바로 시작 모드: 지난 세션은 되살리지 않지만 메모·HTTP 항목은 사이드바에 남긴다.
    folder = auto[1];
    showFolder();
    await restore({ docsOnly: true });
    await newSession(auto[0]);
  } else {
    await restore();
  }
}

/** 창 상단 메뉴(도구·설정·도움말)에서 고른 항목을 실행한다. 항목 번호는 앱 쪽 메뉴 정의와 맞춘다. */
function runMenu(id) {
  const actions = {
    palette: showPalette,
    reopen: showClosed,
    snippets: showSnippets,
    broadcast: () => setBroadcast(!broadcast),
    memory: () => showMemory(),
    look: showLook,
    settings: () => {
      invoke('open_config').catch((err) => setStatus(String(err)));
      setStatus('설정 파일은 저장한 뒤 Cypher를 다시 시작하면 적용됩니다. (테마·글꼴·글자 크기는 설정 → 모양에서 바로 바꿉니다)');
    },
    // Claude Code의 사용자 공통 파일(모든 폴더에 적용되는 설정과 지침)
    'claude-settings': () => openClaudeFile('settings'),
    'claude-memory': () => openClaudeFile('memory'),
    help: showHelp,
    engine: showEngine,
  };
  actions[id]?.();
}

function openClaudeFile(which) {
  invoke('open_claude_file', { which })
    .then((path) => setStatus(`열었습니다: ${path} (고친 내용은 새로 여는 Claude 세션부터 적용됩니다)`))
    .catch((err) => setStatus(String(err)));
}

function showFolder() {
  // 오른쪽 정렬 말줄임에서 글자 순서가 뒤집히지 않도록 방향 표시 문자를 붙인다.
  $folder.textContent = '‎' + folder;
  $folder.title = `새 세션이 시작될 폴더: ${folder}`;
}

/** 새 세션이 시작될 폴더를 바꾼다. */
function useFolder(path) {
  folder = path;
  localStorage.setItem('cypher.folder', folder);
  showFolder();
  setStatus(`새 세션 폴더: ${folder}`);
}

/** 세션을 시작한 폴더를 최근 폴더 목록 맨 앞에 올린다. */
function rememberFolder(path) {
  recentFolders = [path, ...recentFolders.filter((other) => other.toLowerCase() !== path.toLowerCase())].slice(0, MAX_FOLDERS);
  localStorage.setItem('cypher.folders', JSON.stringify(recentFolders));
}

/** 새 세션 폴더로 고를 만한 곳: 지금 보고 있는 항목의 폴더와 최근에 세션을 시작한 폴더(지금 폴더는 뺀다). */
function folderChoices() {
  const here = active && sessions.has(active.key) ? (active.project || active.startCwd) : '';
  const seen = new Set([folder.toLowerCase()]);
  const choices = [];
  for (const [path, note] of [[here, '지금 항목의 폴더'], ...recentFolders.map((path) => [path, '최근 폴더'])]) {
    if (!path || seen.has(path.toLowerCase())) continue;
    seen.add(path.toLowerCase());
    choices.push({ path, note });
  }
  return choices;
}

/** 폴더 줄의 ▾ 단추: 최근 폴더에서 골라 새 세션 폴더를 바꾼다. */
function folderMenu(ev) {
  showMenu(ev, [
    { caption: `지금: ${folder}` },
    ...folderChoices().map(({ path, note }) => ({ label: `${baseName(path)}  —  ${path}${note === '지금 항목의 폴더' ? '  (지금 항목)' : ''}`, action: () => useFolder(path) })),
    { label: '다른 폴더 고르기…', action: pickFolder },
  ]);
}

const quote = (p) => (p.includes(' ') ? `"${p}"` : p);
const baseName = (p) => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p;
const setStatus = (text) => { $('status-text').textContent = text; };
/** 항목의 원래 이름(종류와 폴더, 메모는 파일 이름이나 첫 줄). */
const baseLabel = (s) => (s.kind === 'term'
  ? `${s.profile.label} · ${s.worktree || baseName(s.cwd)}`
  : s.view?.title() || KINDS[s.kind].label);
/** 화면에 보일 이름: 별칭이 있으면 별칭, 없으면 원래 이름. */
const label = (s) => s.name || baseLabel(s);
const isTerm = (s) => s?.kind === 'term';

/** 항목에 초점을 준다(터미널이면 입력, 그 밖에는 그 화면의 편집기). */
function focusItem(s) {
  if (!s) return;
  if (s.term) s.term.focus(); else s.view?.focus();
}

/** 글자를 그 항목에 넣는다(터미널은 붙여넣기, 메모·HTTP는 커서 위치에 넣기). */
function pasteInto(s, text) {
  if (!s) return;
  if (s.term) s.term.paste(text); else s.view?.insert?.(text);
}

function make(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/** 한 줄 입력을 묻는다. 취소하면 null. */
function ask(title, placeholder = '', value = '') {
  return new Promise((resolve) => {
    const dialog = $('ask'), input = $('ask-input');
    $('ask-title').textContent = title;
    input.value = value;
    input.placeholder = placeholder;
    dialog.returnValue = '';
    dialog.onclose = () => {
      resolve(dialog.returnValue === 'ok' ? input.value.trim() : null);
      focusItem(active);
    };
    dialog.showModal();
    input.focus();
  });
}

// ───────────────────────── 세션 ─────────────────────────

/**
 * 세션을 새로 연다.
 * opts.show: 'replace'(지금 창 자리에), 'split'(나란히 추가), 'hidden'(목록에만)
 * opts.cwd / opts.resume(이어갈 Claude 세션 번호) / opts.worktree(워크트리 이름) / opts.group(그룹)
 */
async function newSession(profileId, opts = {}) {
  const profile = config.profiles.find((p) => p.id === profileId);
  if (!profile) return null;
  if (!opts.restoring) lastProfile = profileId;
  $('empty').hidden = true;
  // 같은 폴더가 표기만 달라 다른 묶음으로 갈리지 않게 역슬래시로 통일한다.
  const cwd = (opts.cwd || folder).replace(/\//g, '\\').replace(/(?<=[^:\\])\\+$/, '');

  const pane = make('div', 'pane hidden');
  const head = make('div', 'pane-head');
  const host = make('div', 'term');
  pane.append(head, host);
  $('parked').append(pane); // 화면에 놓이기 전까지 보관 칸에 둔다.

  const look = currentLook();
  const term = new Terminal({
    fontFamily: look.termFamily, fontSize: look.termPixels, theme: look.termTheme, minimumContrastRatio: look.contrast,
    cursorBlink: true, scrollback: config.session.scrollback, allowProposedApi: true,
  });
  const fitAddon = new FitAddon.FitAddon();
  const search = new SearchAddon.SearchAddon();
  term.loadAddon(fitAddon);
  term.loadAddon(search);
  term.loadAddon(new Unicode11Addon.Unicode11Addon());
  term.unicode.activeVersion = '11';
  term.open(host);
  try {
    const gl = new WebglAddon.WebglAddon();
    gl.onContextLoss(() => gl.dispose());
    term.loadAddon(gl);
  } catch { /* WebGL을 못 쓰면 기본 그리기로 동작한다 */ }

  const s = {
    key: ++seq, kind: 'term', id: null, profile, term, fitAddon, search, pane, head, view: null,
    startCwd: cwd, cwd, project: '', worktree: opts.worktree || '',
    name: opts.name || '', color: opts.color || '',
    // 이름 붙인 그룹. 비어 있으면 시작 폴더별로 묶인다. 새 세션은 지금 세션의 그룹을 따른다.
    group: opts.group ?? ((!opts.restoring && active?.group) || ''),
    title: '', model: '', effort: '', context: null, cost: 0,
    usage: '', usageEl: null,
    agents: new Map(), // 지금 돌고 있는 서브에이전트: 번호 → 종류
    waiting: '', // '' | 'approval'(승인·입력 대기) | 'done'(응답 완료, 다음 지시 대기)
    claudeSession: opts.resume || '', resumable: !!opts.resume,
    lastOutput: 0, busy: false, attention: false, exited: false,
    pending: '', sending: false, queued: 0, paused: false,
    focusedAt: 0, shots: [],
    draft: opts.draft || '', // 프롬프트 편집기에 쓰던 글
    workStart: 0, took: 0,   // 지금 요청을 시작한 시각(일하는 중이 아니면 0)과 지난 요청에 걸린 시간(ms)
  };
  sessions.set(s.key, s);
  ensureGroup(s.group);
  if (!opts.restoring) rememberFolder(cwd);
  wirePane(s);
  wireTerminal(s);
  // 찾기 결과의 개수와 지금 몇 번째인지를 찾기 줄에 보여 준다.
  search.onDidChangeResults?.((result) => { if (s === active && !$search.hidden) showSearchCount(result); });

  if (opts.show === 'split' && visible.length < MAX_PANES) placeBeside(s);
  else if (opts.show !== 'hidden') placeInActive(s);
  layout();
  fitVisible(); // 띄우기 전에 실제 크기를 재 둔다.
  if (opts.show !== 'hidden') activate(s);

  const channel = new Channel();
  channel.onmessage = (data) => {
    const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data);
    s.queued += bytes.length;
    term.write(bytes, () => {
      s.queued -= bytes.length;
      if (s.paused && s.queued < FLOW_LOW) setPaused(s, false);
    });
    if (!s.paused && s.queued > FLOW_HIGH) setPaused(s, true);
    s.lastOutput = performance.now();
  };
  try {
    s.id = await invoke('session_create', {
      profile: profileId, cwd, cols: term.cols, rows: term.rows,
      resume: opts.resume || null, worktree: opts.worktree || null, onOutput: channel,
    });
    // 띄우는 사이에 사용자가 이미 닫았으면, 방금 뜬 프로세스를 끝내고 손을 뗀다.
    if (!sessions.has(s.key)) { invoke('session_close', { id: s.id }); return s; }
    flush(s); // 띄우는 동안 눌린 키를 보낸다.
  } catch (err) {
    if (!sessions.has(s.key)) return s;
    term.write(`\x1b[31m세션을 시작하지 못했습니다: ${err}\x1b[0m\r\n`);
    s.exited = true;
  }
  render();
  saveState();
  return s;
}

/**
 * 터미널이 아닌 항목(메모, HTTP …)을 새로 만든다. 사이드바에 세션처럼 놓이고, 화면에서도 창 하나를 차지한다.
 * opts.show / opts.cwd / opts.group / opts.name / opts.color 는 newSession과 같고,
 * opts.saved 는 그 종류가 되살릴 내용(메모 번호, 열어 둔 파일 등)이다.
 */
async function newItem(kind, opts = {}) {
  const def = KINDS[kind];
  if (!def) return null;
  $('empty').hidden = true;
  // 폴더는 지금 보고 있는 항목의 것을 따른다(같은 업무의 요청 파일·메모가 한 폴더에 모이게).
  const base = !opts.restoring && active ? (active.project || active.startCwd) : folder;
  const cwd = (opts.cwd || base).replace(/\//g, '\\').replace(/(?<=[^:\\])\\+$/, '');

  const pane = make('div', 'pane hidden');
  const head = make('div', 'pane-head');
  const host = make('div', 'doc-host');
  pane.append(head, host);
  $('parked').append(pane);

  const s = {
    key: ++seq, kind, id: null, profile: null, term: null, pane, head, view: null,
    startCwd: cwd, cwd, project: '', worktree: '',
    name: opts.name || '', color: opts.color || '',
    group: opts.group ?? ((!opts.restoring && active?.group) || ''),
    title: '', model: '', effort: '', context: null, cost: 0,
    usage: '', usageEl: null, agents: new Map(), waiting: '',
    lastOutput: -Infinity, busy: false, attention: false, exited: false,
    shots: [], draft: '', workStart: 0, took: 0,
  };
  // 제목(파일 이름, 메모 첫 줄)이 바뀌면 사이드바를 다시 그리고 상태를 적어 둔다.
  s.view = def.create(s, host, opts.saved || {}, () => { if (sessions.has(s.key)) { render(); saveState(); } });
  sessions.set(s.key, s);
  ensureGroup(s.group);
  wirePane(s);

  if (opts.show === 'split' && visible.length < MAX_PANES) placeBeside(s);
  else if (opts.show !== 'hidden') placeInActive(s);
  layout();
  if (opts.show !== 'hidden') activate(s);
  await s.view.ready;
  render();
  saveState();
  return s;
}

/** 모든 종류의 창에 공통인 동작: 나란히 보기에서 창을 누르면 그 창이 활성화된다. */
function wirePane(s) {
  s.pane.addEventListener('mousedown', () => { if (active !== s) activate(s); }, true);
}

async function newWorktree(show) {
  const name = await ask('새 워크트리 이름 (영문·숫자·. _ -)', 'feature-x');
  if (name) newSession('claude', { worktree: name, show });
}

/** 화면이 출력을 못 따라가면 읽기를 멈추고, 따라잡으면 다시 읽게 한다. */
function setPaused(s, paused) {
  s.paused = paused;
  if (s.id != null && !s.exited) invoke('session_pause', { id: s.id, paused });
}

// ───────────────────────── 화면 분할 ─────────────────────────
// 화면 배치는 나무 구조다. 잎은 세션 하나({ s }), 가지는 좌우(row)나 상하(col)로 나눈 둘({ dir, a, b }).
// `visible`은 이 나무의 잎을 화면 순서대로 늘어놓은 목록이고, layout()이 나무에서 다시 만든다.

let tree = null;

const leavesOf = (node) => (!node ? [] : node.s ? [node.s] : [...leavesOf(node.a), ...leavesOf(node.b)]);

function findLeaf(s, node = tree) {
  if (!node) return null;
  if (node.s) return node.s === s ? node : null;
  return findLeaf(s, node.a) || findLeaf(s, node.b);
}

/** 그 세션을 뺀 나무. 짝이던 쪽이 빈자리를 다 차지한다. */
function without(node, s) {
  if (!node) return null;
  if (node.s) return node.s === s ? null : node;
  const a = without(node.a, s), b = without(node.b, s);
  return a && b ? { dir: node.dir, ratio: node.ratio, a, b } : (a || b);
}

/** 나눈 둘 중 앞쪽(a)이 차지하는 비율. 경계선을 끌어 바꾼다. 너무 좁아지지 않게 범위를 둔다. */
const ratioOf = (node) => Math.min(0.9, Math.max(0.1, Number(node.ratio) || 0.5));

/** 가지 하나의 두 칸에 비율을 적용한다. */
function applyRatio(node, first, second) {
  const ratio = ratioOf(node);
  first.style.flex = `${ratio} 1 0`;
  second.style.flex = `${1 - ratio} 1 0`;
}

/** 경계선을 끄는 동안 비율을 바꾼다. 놓으면 저장한다. */
function startResize(ev, node, box, first, second, bar) {
  ev.preventDefault();
  bar.classList.add('active');
  document.body.classList.add(node.dir === 'row' ? 'resizing-row' : 'resizing-col');
  let queued = false;
  const move = (e) => {
    const rect = box.getBoundingClientRect();
    node.ratio = node.dir === 'row' ? (e.clientX - rect.left) / rect.width : (e.clientY - rect.top) / rect.height;
    applyRatio(node, first, second);
    if (queued) return; // 터미널 크기 다시 재기는 한 화면에 한 번만
    queued = true;
    requestAnimationFrame(() => { queued = false; fitVisible(); });
  };
  const stop = () => {
    document.removeEventListener('mousemove', move);
    document.removeEventListener('mouseup', stop);
    bar.classList.remove('active');
    document.body.classList.remove('resizing-row', 'resizing-col');
    node.ratio = ratioOf(node);
    fitVisible();
    saveState();
  };
  document.addEventListener('mousemove', move);
  document.addEventListener('mouseup', stop);
}

/** target 세션의 자리를 둘로 나눠 한쪽(side: left | right | top | bottom)에 s를 놓는다. */
function splitAt(target, s, side) {
  const leaf = findLeaf(target);
  if (!leaf) { tree = tree ? { dir: 'row', a: tree, b: { s } } : { s }; return; }
  const fresh = { s }, old = { s: leaf.s };
  const first = side === 'left' || side === 'top';
  delete leaf.s;
  Object.assign(leaf, { dir: side === 'left' || side === 'right' ? 'row' : 'col', a: first ? fresh : old, b: first ? old : fresh });
}

/** 놓을 자리를 정하지 않았을 때: 지금 창을 나눠 옆(넓으면 오른쪽, 좁으면 아래)에 놓는다. */
function placeBeside(s) {
  const target = visible.includes(active) ? active : visible[visible.length - 1];
  if (!target) { tree = { s }; return; }
  const rect = target.pane.getBoundingClientRect();
  splitAt(target, s, rect.width >= rect.height * 1.2 ? 'right' : 'bottom');
}

/** 지금 창의 자리에 s를 놓는다(있던 세션은 화면에서만 빠지고 계속 살아 있다). */
function placeInActive(s) {
  const leaf = findLeaf(active);
  if (leaf) leaf.s = s; else if (!tree) tree = { s }; else placeBeside(s);
}

/** 세션 여러 개를 고르게 나눈 배치. */
function balanced(list, dir = 'row') {
  if (list.length < 2) return list.length ? { s: list[0] } : null;
  const mid = Math.ceil(list.length / 2), next = dir === 'row' ? 'col' : 'row';
  return { dir, a: balanced(list.slice(0, mid), next), b: balanced(list.slice(mid), next) };
}

/** 나무대로 창을 배치한다. 화면에 없는 세션의 창은 보이지 않는 보관 칸으로 옮겨 둔다. */
function layout() {
  visible = leavesOf(tree);
  for (const s of sessions.values()) {
    const shown = visible.includes(s);
    s.pane.classList.toggle('hidden', !shown);
    if (!shown && s.pane.parentElement !== $('parked')) $('parked').append(s.pane);
  }
  const build = (node) => {
    if (node.s) return node.s.pane;
    const box = make('div', `split split-${node.dir}`);
    const first = build(node.a), second = build(node.b);
    // 둘 사이의 경계선: 끌어서 크기를 조절하고, 두 번 누르면 반반으로 되돌린다.
    const bar = make('div', 'divider');
    bar.title = '끌어서 크기 조절 · 두 번 눌러 반반으로';
    bar.onmousedown = (ev) => startResize(ev, node, box, first, second, bar);
    bar.ondblclick = () => { node.ratio = 0.5; applyRatio(node, first, second); fitVisible(); saveState(); };
    applyRatio(node, first, second);
    box.append(first, bar, second);
    return box;
  };
  const root = tree ? build(tree) : null;
  if (root) root.style.flex = ''; // 맨 위 칸은 화면 전체를 쓴다(예전 비율이 남아 있지 않게).
  for (const old of [...$terms.children]) if (old !== root && old.id !== 'empty') old.remove();
  if (root && root.parentElement !== $terms) $terms.append(root);
  $terms.classList.toggle('multi', visible.length > 1);
  $('empty').hidden = visible.length > 0;
  $('empty').textContent = sessions.size
    ? '화면에 띄운 항목이 없습니다. 사이드바에서 항목을 누르거나 화면으로 끌어다 놓으세요.'
    : '왼쪽에서 세션이나 메모·HTTP·사이트·이미지·브레인스토밍 항목을 시작하세요.';
  syncComposer();
  syncSites(); // 사이트 항목의 웹뷰를 새 창 자리에 맞춘다.
  requestAnimationFrame(() => {
    fitVisible();
    visible.filter(isTerm).forEach(resyncScroll);
  });
}

/**
 * 숨겨졌다 다시 보이는 창의 스크롤 막대를 터미널이 기억하는 위치에 다시 맞춘다.
 *
 * 창을 숨기면(display: none) 브라우저가 스크롤 막대 위치를 맨 위(0)로 되돌린다. 터미널(xterm.js)이
 * 기억하는 위치는 그대로라서 둘이 어긋나고, 그 상태에서 휠을 굴리거나 화면이 갱신되면 막대 기준으로
 * 맞춰지면서 맨 위로 튄다. xterm.js는 위치가 실제로 바뀔 때만 막대를 다시 맞추므로, 한 줄 움직였다가
 * 원래 자리로 돌려 맞추게 한다.
 */
function resyncScroll(s) {
  const buffer = s.term.buffer.active;
  const bar = s.pane.querySelector('.xterm-viewport');
  if (!bar || buffer.viewportY === 0 || bar.scrollTop > 0) return; // 어긋나지 않았다
  const y = buffer.viewportY;
  s.term.scrollLines(-1);
  if (y >= buffer.baseY) s.term.scrollToBottom(); else s.term.scrollToLine(y);
}

function fitVisible() {
  for (const s of visible) {
    try { s.fitAddon?.fit(); } catch { /* 아직 크기를 잴 수 없는 상태 */ }
  }
}

/**
 * 사이드바에서 끌어온 세션을 창(target) 위에 놓았을 때.
 * 가장자리에 놓으면 그 창을 좌우·상하로 나눠 그쪽에 열고, 가운데에 놓으면 그 자리에 연다
 * (이미 화면에 있던 세션이면 둘의 자리를 바꾼다).
 */
function dropOnPane(s, target, zone) {
  if (s === target) return;
  if (zone === 'center') {
    const from = findLeaf(s), to = findLeaf(target);
    if (from) from.s = target;
    to.s = s;
  } else {
    if (!visible.includes(s) && visible.length >= MAX_PANES) return setStatus(`한 화면에 볼 수 있는 창은 ${MAX_PANES}개까지입니다.`);
    tree = without(tree, s);
    splitAt(target, s, zone);
  }
  layout();
  activate(s);
}

/** 그 위치에 있는 창과, 창 안에서의 구역(left | right | top | bottom | center). */
function paneZoneAt(x, y) {
  const el = document.elementFromPoint(x, y)?.closest?.('.pane');
  const target = el && visible.find((v) => v.pane === el);
  if (!target) return null;
  const rect = el.getBoundingClientRect();
  const fx = (x - rect.left) / rect.width, fy = (y - rect.top) / rect.height;
  const [nearest, distance] = [['left', fx], ['right', 1 - fx], ['top', fy], ['bottom', 1 - fy]].sort((p, q) => p[1] - q[1])[0];
  return { target, rect, zone: distance < 0.3 ? nearest : 'center' };
}

const DROP_TEXT = { left: '왼쪽에 나눠 열기', right: '오른쪽에 나눠 열기', top: '위에 나눠 열기', bottom: '아래에 나눠 열기', center: '이 자리에 열기' };

/** 끄는 동안, 놓으면 새 창이 차지할 영역을 미리 보여 준다. */
function showDropHint(hit) {
  const hint = $('drop-hint');
  hint.hidden = !hit;
  if (!hit) return;
  const { rect: r, zone } = hit;
  const box = {
    left: [r.left, r.top, r.width / 2, r.height],
    right: [r.left + r.width / 2, r.top, r.width / 2, r.height],
    top: [r.left, r.top, r.width, r.height / 2],
    bottom: [r.left, r.top + r.height / 2, r.width, r.height / 2],
    center: [r.left, r.top, r.width, r.height],
  }[zone];
  Object.assign(hint.style, { left: `${box[0]}px`, top: `${box[1]}px`, width: `${box[2]}px`, height: `${box[3]}px` });
  hint.textContent = DROP_TEXT[zone];
}

/** 항목을 고른다(화면에 없으면 지금 창 자리에 놓는다). `keepFocus`이면 초점을 옮기지 않는다(사이드바에서 방향키로 오갈 때). */
function activate(s, { keepFocus = false } = {}) {
  if (!visible.includes(s)) {
    placeInActive(s);
    layout();
  }
  active = s;
  s.attention = false;
  s.usedAt = Date.now();
  for (const other of sessions.values()) other.pane.classList.toggle('active', other === s);
  renderShots();
  if (!$search.hidden) closeSearch();
  showDiff(s);
  syncComposer();
  if (!keepFocus) focusItem(s);
  render();
  saveState();
}

/** 나란히 보기에 넣거나 뺀다(세션은 계속 살아 있다). */
function toggleShown(s) {
  if (!visible.includes(s)) {
    if (visible.length >= MAX_PANES) return setStatus(`한 화면에 볼 수 있는 창은 ${MAX_PANES}개까지입니다.`);
    placeBeside(s);
    layout();
    activate(s);
  } else {
    // 화면에서만 숨긴다. 마지막 창도 숨길 수 있고, 그때는 화면이 비고 세션은 사이드바에 남는다.
    tree = without(tree, s);
    layout();
    if (active === s && visible[0]) activate(visible[0]); else { render(); saveState(); }
  }
}

/**
 * 닫은 항목을 다시 열 수 있게 적어 둘 내용. 되살릴 것이 없으면(빈 메모, 빈 페이지, 시작하지 못한 세션) null.
 * 닫기 전에 만들어야 한다(닫은 뒤에는 화면이 제목과 상태를 내놓지 못한다).
 */
function closedRecord(s) {
  if (isTerm(s) && s.id == null) return null;
  if (s.kind === 'memo' && !s.view.text().trim() && !s.view.state().path) return null;
  if (s.kind === 'image' && s.view.empty()) return null;
  if (s.kind === 'site' && !s.view.url()) return null;
  try {
    return { ...describe(s), label: label(s) };
  } catch {
    return null;
  }
}

/** 항목을 닫는다. `forget`이면 닫은 항목 목록에 남기지 않는다(다시 시작하려고 닫을 때). */
async function closeSession(s, { forget = false } = {}) {
  const record = forget ? null : closedRecord(s);
  if (s.view) {
    // 메모·HTTP: 닫기 전에 저장할 것을 저장한다. 거짓이면 사용자가 닫기를 취소한 것이다.
    if (!(await s.view.close())) return;
    s.view.dispose();
  } else {
    if (s.id != null && !s.exited) invoke('session_close', { id: s.id });
    s.term.dispose();
  }
  if (!sessions.has(s.key)) return; // 기다리는 사이 이미 닫혔다.
  if (record) {
    rememberClosed(record);
    setStatus(`닫았습니다: ${record.label} · 다시 열기: Ctrl+Shift+R`);
  }
  s.pane.remove();
  s.shots.forEach((shot) => URL.revokeObjectURL(shot.url));
  sessions.delete(s.key);
  tree = without(tree, s);
  visible = leavesOf(tree);
  if (active === s) {
    active = null;
    const next = visible[0] || [...sessions.values()].pop();
    if (next) activate(next);
  }
  layout();
  renderShots();
  render();
  saveState();
}

/**
 * 종료된 세션을 같은 자리에서 다시 시작한다. Claude 세션은 하던 대화를 이어 가고, 다른 터미널은 같은 폴더에서 새로 뜬다.
 * 사이드바의 순서, 화면에서의 자리, 별칭·색·그룹은 그대로 둔다.
 */
async function restartSession(s) {
  if (!isTerm(s) || !sessions.has(s.key)) return;
  const all = [...sessions.values()];
  const after = all[all.indexOf(s) + 1];
  const fresh = await revive(describe(s), { show: 'hidden' });
  if (!fresh || !sessions.has(s.key)) return;
  if (after && sessions.has(after.key)) placeInOrder(fresh, after, false);
  const leaf = findLeaf(s);
  if (leaf) leaf.s = fresh;
  const wasActive = active === s;
  await closeSession(s, { forget: true });
  if (wasActive || leaf) activate(fresh); else render();
}

/** 사이드바에 보이는 차례대로의 항목(접어 둔 묶음 안의 것도 포함). */
const inSidebarOrder = () => groupsInOrder().flatMap((group) => group.list);

function cycle(step) {
  const all = inSidebarOrder();
  if (all.length < 2 || !active) return;
  activate(all[(all.indexOf(active) + step + all.length) % all.length]);
}

function focusPane(step) {
  if (visible.length < 2 || !active) return;
  activate(visible[(visible.indexOf(active) + step + visible.length) % visible.length]);
}

function setBroadcast(on) {
  broadcast = on;
  $terms.classList.toggle('broadcast', on);
  $('status-mode').textContent = on ? '동시 입력 켜짐: 나란히 보이는 모든 세션에 입력됩니다' : '';
  focusItem(active);
}

const byId = (id) => [...sessions.values()].find((s) => s.id === id);

function onExit({ id, code }) {
  const s = byId(id);
  if (!s) return;
  s.exited = true;
  s.agents.clear();
  s.workStart = 0;
  s.term.write(`\r\n\x1b[90m[세션 종료${code == null ? '' : `: 코드 ${code}`} · 사이드바에서 오른쪽 클릭 → 다시 시작]\x1b[0m\r\n`);
  render();
  saveState();
}

function onStatus({ id, model, effort, context, cwd, project, session, worktree, cost, limits: newLimits }) {
  const s = byId(id);
  if (!s) return;
  s.model = model || '';
  s.effort = effort || '';
  s.context = context;
  s.cost = cost || 0;
  // 컨텍스트가 거의 찼을 때 한 번 알린다(/compact 등으로 줄었다가 다시 차면 또 알린다).
  if (context != null && context >= CONTEXT_DANGER && !s.contextAlerted) {
    s.contextAlerted = true;
    if (config.notify.in_app) toast(s, label(s), `컨텍스트가 ${Math.round(context)}% 찼습니다. /compact 또는 새 세션을 고려하세요.`);
  } else if (context != null && context < CONTEXT_WARN) {
    s.contextAlerted = false;
  }
  if (newLimits) { limits = newLimits; renderLimits(); }
  if (cwd) s.cwd = cwd; // 상대경로 링크의 기준 폴더도 따라간다.
  if (worktree) s.worktree = worktree;
  if ((project && project !== s.project) || (session && session !== s.claudeSession)) {
    s.project = project || s.project;
    s.claudeSession = session || s.claudeSession;
    saveState();
  }
  render();
}

/** Claude Code 훅: 요청 시작, 응답 완료(Stop), 입력·승인 대기(Notification), 서브에이전트 시작·종료. */
function onHook({ id, event, kind, message, agent, agent_type: agentType, force }) {
  const s = byId(id);
  if (!s) return;
  if (!s.resumable) { s.resumable = true; saveState(); } // 대화가 생겼으니 다음에 이어갈 수 있다.
  // 서브에이전트: 돌고 있는 것의 번호와 종류를 모아 두고 사이드바에 보여 준다.
  if (event === 'SubagentStart' || event === 'SubagentStop') {
    if (agent) {
      if (event === 'SubagentStart') s.agents.set(agent, agentType || ''); else s.agents.delete(agent);
    }
    if (s.heldStop) s.heldAt = performance.now(); // 서브에이전트에 움직임이 있으면 조용한 시간을 다시 잰다.
    return render();
  }
  if (event !== 'UserPromptSubmit' && event !== 'Stop' && event !== 'Notification') return;
  if (event === 'UserPromptSubmit') {
    s.waiting = '';
    s.heldStop = false;
    s.workStart = Date.now(); // 이 요청에 걸리는 시간을 잰다.
    s.took = 0;
    return render();
  }
  if (event === 'Notification' && kind === 'idle_prompt') return; // 완료 때 이미 알렸다.
  // 서브에이전트가 아직 돌고 있을 때의 완료 신호는 "잠깐 멈춤"이다(끝나면 Claude가 이어서 답한다).
  // 이때는 끝났다고 알리지 않고, 서브에이전트가 모두 끝난 뒤의 완료 신호를 기다린다.
  // 그 신호가 끝내 오지 않는 경우(서브에이전트 종료 신호가 빠짐 등)는 refreshActivity()가 챙긴다.
  if (event === 'Stop' && s.agents.size && !force) {
    s.heldStop = true;
    s.heldAt = performance.now();
    s.heldEpoch = Date.now();
    s.waiting = '';
    return render();
  }
  if (event === 'Stop') {
    s.heldStop = false;
    // 걸린 시간: 요청을 보낸 때부터 지금까지. 미뤄 둔 완료를 뒤늦게 알리는 경우에는 그 완료 신호가 온 때까지.
    if (s.workStart) s.took = (force && s.heldEpoch ? s.heldEpoch : Date.now()) - s.workStart;
    s.workStart = 0;
  }
  if (force) s.agents.clear(); // 한참 조용했으니 남아 있던 서브에이전트 표시는 끝난 것으로 본다.
  s.waiting = event === 'Stop' ? 'done' : 'approval';
  if (event === 'Stop') diffRefreshFor(s);
  render();
  const title = label(s);
  const body = event === 'Stop'
    ? `요청한 작업이 끝났습니다.${s.took >= 1000 ? ` (${duration(s.took)} 걸림)` : ''}`
    : (message || '입력을 기다리고 있습니다.');
  const focused = document.hasFocus();
  if (!focused && config.notify.system) invoke('notify', { title, body }); // Windows 알림 + 작업 표시줄 깜빡임
  if (!focused || s !== active) {
    s.attention = true;
    if (config.notify.in_app) toast(s, title, body);
    render();
  }
}

/** 세션과 무관한 알림(오류 등). 상태줄과 달리 다른 표시에 덮이지 않고 누를 때까지 12초간 남는다. */
function notice(title, body) {
  const box = make('div', 'toast');
  box.append(make('b', '', title), make('span', '', body));
  box.onclick = () => box.remove();
  $('toasts').append(box);
  setTimeout(() => box.remove(), 12000);
}

/** 창 오른쪽 아래에 잠깐 뜨는 알림. 누르면 그 세션으로 이동한다. */
function toast(s, title, body) {
  const box = make('div', 'toast');
  box.append(make('b', '', title), make('span', '', body));
  const remove = () => box.remove();
  box.onclick = () => { remove(); if (sessions.has(s.key)) activate(s); };
  $('toasts').append(box);
  setTimeout(remove, 8000);
}

/** 출력 활동으로 "작업 중"을 추정하고, 뒤에 있던 세션이 멈추면 주의 표시를 한다. */
function refreshActivity() {
  const now = performance.now();
  let changed = false;
  for (const s of sessions.values()) {
    const busy = isTerm(s) && !s.exited && now - s.lastOutput < BUSY_MS;
    // 미뤄 둔 완료 알림: 서브에이전트 때문에 미뤘는데 그 뒤로 한참 조용하면(출력도, 서브에이전트 신호도 없음)
    // 마지막 완료 신호나 서브에이전트 종료 신호가 빠진 것으로 보고 여기서 알린다. 승인 대기 중에는 건드리지 않는다.
    if (s.heldStop && !busy && !s.exited && !s.waiting && now - Math.max(s.lastOutput, s.heldAt || 0) > HELD_STOP_MS) {
      onHook({ id: s.id, event: 'Stop', force: true });
    }
    // 일하는 중인 세션의 경과 시간은 목록을 다시 그리지 않고 그 칸만 고친다.
    if (s.workStart && s.timeEl?.isConnected) s.timeEl.textContent = clock(Date.now() - s.workStart);
    if (busy === s.busy) continue;
    if (!busy && s !== active) s.attention = true;
    s.busy = busy;
    changed = true;
  }
  if (changed) render();
}

const WAITING_TEXT = { approval: '승인·입력 대기', done: '완료 · 다음 지시 대기' };

/** 경과 시간을 시계 모양으로: 3:24, 1:02:07. */
function clock(ms) {
  const total = Math.max(0, Math.floor(ms / 1000));
  const two = (n) => String(n).padStart(2, '0');
  const hours = Math.floor(total / 3600), minutes = Math.floor((total % 3600) / 60), seconds = total % 60;
  return hours ? `${hours}:${two(minutes)}:${two(seconds)}` : `${minutes}:${two(seconds)}`;
}

/** 걸린 시간을 말로: 42초, 5분 12초, 1시간 3분. */
function duration(ms) {
  const total = Math.max(0, Math.round(ms / 1000));
  const hours = Math.floor(total / 3600), minutes = Math.floor((total % 3600) / 60), seconds = total % 60;
  if (hours) return `${hours}시간${minutes ? ` ${minutes}분` : ''}`;
  if (minutes) return `${minutes}분${seconds ? ` ${seconds}초` : ''}`;
  return `${seconds}초`;
}

/** 사이드바의 세션 한 줄. */
function sessionItem(s) {
  const li = make('li', [
    s === active && 'active', visible.includes(s) && 'shown', s.waiting,
    s.exited ? 'exited' : s.busy ? 'busy' : s.attention && 'attention',
  ].filter(Boolean).join(' '));
  li.title = '화면으로 끌어다 놓기: 놓은 위치대로 좌우·상하 분할 · 다른 줄 위·아래로 끌기: 순서 바꾸기 · 묶음 머리로 끌기: 그 묶음으로 옮기기 · 두 번 누르기: 별칭 · 오른쪽 클릭: 메뉴';
  if (s.color) li.style.borderLeftColor = s.color;
  li.onclick = (ev) => {
    if (ev.target.closest('input')) return; // 별칭을 고치는 중
    if (ev.ctrlKey) toggleShown(s); else activate(s);
  };
  li.onmousedown = (ev) => {
    if (ev.button === 0 && !ev.ctrlKey && !ev.target.closest('button, input')) drag = { s, x: ev.clientX, y: ev.clientY, active: false };
  };
  li.dataset.key = s.key;
  li.ondblclick = (ev) => { if (!ev.target.closest('button, input')) rename(s); };
  li.oncontextmenu = (ev) => { ev.preventDefault(); sessionMenu(s, ev); };

  // 이름: 별칭이 있으면 별칭을 크게, 원래 이름(종류·폴더)은 옆에 흐리게 보여 준다.
  let name;
  if (renaming === s.key) {
    name = renameInput(s);
  } else {
    name = make('span', 'name');
    name.append(make('span', 'alias', label(s)));
    if (s.name) name.append(make('span', 'origin', baseLabel(s)));
    name.title = `${s.name ? `${baseLabel(s)}\n` : ''}${s.cwd}`;
    if (s.color) name.style.color = s.color;
  }
  const close = make('button', 'close', '×');
  close.title = isTerm(s) ? '세션 닫기' : '항목 닫기';
  close.onclick = (ev) => { ev.stopPropagation(); closeSession(s); };
  const pencil = make('button', 'close', '✎');
  pencil.title = '별칭 붙이기 (세션에는 영향 없이 사이드바에 보이는 이름만 바꿉니다)';
  pencil.onclick = (ev) => { ev.stopPropagation(); rename(s); };
  // 화면 표시 토글: 오른쪽 화면에 보이기/숨기기(세션은 계속 실행된다). 항상 보이는 작은 단추.
  const shown = visible.includes(s);
  const eye = make('button', `eye${shown ? ' on' : ''}`, shown ? '◉' : '○');
  eye.title = shown ? '화면에서 숨기기 (세션은 계속 실행됩니다)' : '화면에 보이기';
  eye.onclick = (ev) => { ev.stopPropagation(); toggleShown(s); };
  const buttons = make('span', 'row-buttons');
  if (s.agents.size) buttons.append(agentPill(s.agents.size, agentSummary(s)));
  buttons.append(pencil, eye, close);

  // 터미널이 아닌 항목은 점 대신 종류 기호를 보여 준다.
  const dot = isTerm(s) ? make('span', 'dot') : make('span', 'dot kind', KINDS[s.kind].icon);
  if (!isTerm(s)) dot.title = KINDS[s.kind].label;
  const context = s.context == null ? null : Math.round(s.context);
  const badge = make('span', 'badge', [
    s.model, s.effort, context == null ? '' : `컨텍스트 ${context}%`, s.cost ? `$${s.cost.toFixed(2)}` : '',
  ].filter(Boolean).join(' · '));
  if (context >= CONTEXT_DANGER) badge.classList.add('danger');
  else if (context >= CONTEXT_WARN) badge.classList.add('warn');
  if (context >= CONTEXT_WARN) badge.title = '컨텍스트가 거의 찼습니다. /compact 또는 새 세션을 고려하세요.';
  // 돌고 있는 서브에이전트: 눈에 띄는 한 줄로 따로 보여 준다.
  const agents = make('span', 'agents-line');
  if (s.agents.size) {
    agents.append(make('i', 'spin'), make('span', '', `서브에이전트 ${s.agents.size}개 실행 중${agentKinds(s) ? ` · ${agentKinds(s)}` : ''}`));
    agents.title = agentSummary(s);
  }
  const sub = make('span', 'sub');
  // 요청을 처리하는 중이면 경과 시간을 앞에 보여 준다(승인을 기다리는 동안에는 대기 표시가 먼저다).
  s.timeEl = null;
  if (s.workStart && !s.exited && !s.waiting) {
    s.timeEl = make('b', 'elapsed', clock(Date.now() - s.workStart));
    s.timeEl.title = '이 요청을 처리한 지 지난 시간';
    sub.append(s.timeEl);
  }
  const subText = s.exited ? '종료됨 (오른쪽 클릭: 다시 시작)'
    : s.waiting === 'done' && s.took >= 1000 ? `완료 · ${duration(s.took)} 걸림 · 다음 지시 대기`
    : WAITING_TEXT[s.waiting] || (s.heldStop && s.agents.size ? '서브에이전트가 끝나기를 기다리는 중' : s.title);
  if (subText) sub.append(subText);
  // 사용량은 3초마다 바뀌므로 목록을 다시 그리지 않고 이 칸만 고친다.
  s.usageEl = make('span', 'usage', s.exited ? '' : s.usage);
  li.append(dot, name, buttons, agents, badge, sub, s.usageEl);
  return li;
}

/** 서브에이전트의 종류별 개수: "Explore 2, general-purpose 1". 종류를 모르면 빈 글. */
function agentKinds(s) {
  const counts = new Map();
  for (const kind of s.agents.values()) if (kind) counts.set(kind, (counts.get(kind) || 0) + 1);
  return [...counts].map(([kind, count]) => `${kind} ${count}`).join(', ');
}

const agentSummary = (s) => `${label(s)}: 서브에이전트 ${s.agents.size}개 실행 중${agentKinds(s) ? ` (${agentKinds(s)})` : ''}`;

/** 돌고 있는 서브에이전트 수를 보여 주는 알약 모양 표시(도는 고리 + 숫자). */
function agentPill(count, title) {
  const pill = make('span', 'agents');
  pill.append(make('i', 'spin'), make('b', '', String(count)));
  pill.title = title;
  return pill;
}

/** 사이드바에서 바로 고치는 별칭 입력칸. Enter·다른 곳 클릭: 확정, Esc: 취소. */
function renameInput(s) {
  const input = make('input', 'rename');
  Object.assign(input, { value: s.name, placeholder: baseLabel(s), spellcheck: false, autocomplete: 'off' });
  input.title = '별칭 (비우면 원래 이름으로)';
  let done = false;
  const finish = (commit, byKey) => {
    if (done) return;
    done = true;
    renaming = null;
    if (commit) { s.name = input.value.trim(); saveState(); }
    if (byKey) {
      render();
      focusItem(active); // 입력칸이 사라졌으니 키 입력이 다시 세션으로 가게 한다.
    } else {
      // 다른 곳을 눌러 끝난 경우: 이 줄만 바꾼다. 목록 전체를 다시 그리면 방금 누른 줄·단추가 사라져
      // 그 클릭이 먹히지 않는다. (전체는 곧 다음 갱신 때 다시 그려진다.)
      input.closest('li')?.replaceWith(sessionItem(s));
    }
  };
  input.onkeydown = (ev) => {
    ev.stopPropagation();
    if (ev.key === 'Enter') finish(true, true);
    else if (ev.key === 'Escape') finish(false, true);
  };
  input.onblur = () => finish(true, false);
  return input;
}

const gigabytes = (bytes) => (bytes / 2 ** 30).toFixed(1);

// 수치 표시는 자릿수를 미리 잡아 둔다(고정폭 글꼴 + 앞쪽 공백). 값이 바뀌어도 글자 위치가 흔들리지 않는다.
const pad = (value, width) => String(value).padStart(width, ' ');
const percent = (value) => `${pad(Math.min(999, Math.round(value)), 3)}%`;

/** 세션별 사용량(그 세션이 띄운 프로세스 전부의 합)을 사이드바에 보여 준다. */
function onUsage({ sessions: perSession }) {
  for (const item of perSession) {
    const s = byId(item.id);
    if (!s) continue;
    const memory = item.memory >= 2 ** 30 ? `${pad(gigabytes(item.memory), 4)}GB` : `${pad(Math.round(item.memory / 2 ** 20), 4)}MB`;
    s.usage = `CPU ${pad(Math.min(999.9, item.cpu).toFixed(1), 5)}% · ${memory}`;
    if (s.usageEl && !s.exited) s.usageEl.textContent = s.usage;
  }
}

// ───────────────────────── 세션 묶음(그룹) ─────────────────────────
// 세션은 기본으로 시작 폴더별로 묶인다. 그룹 이름을 지정한 세션은 폴더와 무관하게 그 그룹으로 묶인다.

const groupKey = (s) => (s.group ? `g:${s.group}` : `d:${s.startCwd}`);
function saveGroups() {
  localStorage.setItem('cypher.groups', JSON.stringify(groupList));
}

/** 그룹을 목록에 올린다. 올려 둔 그룹은 세션이 없어도 사이드바에 남는다. */
function ensureGroup(name) {
  if (!name || groupList.includes(name)) return;
  groupList.push(name);
  saveGroups();
}

/** "+ 그룹" 버튼: 빈 그룹을 만든다. 세션을 끌어다 놓거나 그룹 머리의 +로 세션을 열어 채운다. */
async function newGroup() {
  const name = await ask('새 그룹 이름', '예: 결제 개편');
  if (!name) return;
  ensureGroup(name);
  collapsed.delete(`g:${name}`);
  render();
}

/** 세션을 그룹으로 옮긴다. 이름이 비면 폴더별 묶음으로 돌아간다. */
function moveToGroup(list, name) {
  ensureGroup(name);
  for (const s of list) s.group = name;
  render();
  saveState();
}

async function askGroup(list, current = '') {
  const name = await ask('그룹 이름 (비우면 폴더별 묶음으로)', groupList.length ? `있는 그룹: ${groupList.join(', ')}` : '예: 결제 개편', current);
  if (name != null) moveToGroup(list, name);
}

async function renameGroup(group, live) {
  const name = await ask('그룹 이름', '', group.name);
  if (!name || name === group.name) return;
  // 같은 이름의 그룹이 이미 있으면 그쪽으로 합쳐진다.
  groupList = [...new Set(groupList.map((g) => (g === group.name ? name : g)))];
  saveGroups();
  moveToGroup(live, name);
}

/** 그룹을 없앤다. 안에 있던 세션은 닫지 않고 폴더별 묶음으로 돌려보낸다. */
function deleteGroup(group, live) {
  groupList = groupList.filter((g) => g !== group.name);
  saveGroups();
  moveToGroup(live, '');
}

/** 그 묶음에 새 항목을 넣을 때 쓸 그룹 이름과 폴더. */
function groupContext(group) {
  const first = group.list.find((s) => sessions.has(s.key));
  // 폴더 묶음은 시작 폴더로 묶이므로 그 폴더를 그대로 써야 새 항목이 같은 묶음에 들어간다.
  const cwd = !first ? folder : group.logical ? (first.project || first.startCwd) : first.startCwd;
  return { group: group.logical ? group.name : '', cwd };
}

/**
 * 그 묶음에 새 항목을 더하는 메뉴 항목들: 터미널 프로필들과 그 밖의 종류(메모, HTTP …).
 * 폴더는 그 묶음에 이미 있는 항목의 것을 따른다(없으면 새 세션 폴더).
 */
function addItems(group) {
  const opts = groupContext(group);
  return [
    ...config.profiles.map((p) => ({ label: `+ ${p.label} 세션`, action: () => newSession(p.id, opts) })),
    ...Object.entries(KINDS).map(([kind, def]) => ({ label: `+ ${def.label}`, action: () => createKind(kind, opts) })),
  ];
}

function groupMenu(group, ev) {
  const live = group.list.filter((s) => sessions.has(s.key));
  const items = [];
  items.push(...addItems(group));
  if (group.logical) {
    items.push({ label: '그룹 이름 바꾸기', action: () => renameGroup(group, live) });
    items.push({ label: '그룹 삭제 (세션은 폴더별 묶음으로)', action: () => deleteGroup(group, live) });
  } else {
    items.push({ label: '이 묶음을 그룹으로 만들기…', action: () => askGroup(live) });
  }
  items.push({
    label: '이 묶음만 나란히 보기',
    action: () => {
      tree = balanced(live.slice(0, MAX_PANES));
      layout();
      if (visible[0]) activate(visible[0]);
    },
  });
  items.push({
    label: '이 묶음의 세션 모두 닫기',
    action: () => {
      if (confirm(`"${group.name}" 묶음의 세션 ${live.length}개를 모두 닫을까요?`)) live.forEach(closeSession);
    },
  });
  showMenu(ev, items);
}

// 목록에서 세션을 끌어 묶음 머리에 놓으면 그 묶음으로 옮긴다.
// (창에 파일을 끌어다 놓는 기능과 겹치지 않도록 브라우저의 끌어다 놓기 대신 마우스 이동으로 처리한다.)
document.addEventListener('mousemove', (ev) => {
  if (!drag) return;
  if (!drag.active) {
    if (Math.hypot(ev.clientX - drag.x, ev.clientY - drag.y) < 6) return;
    drag.active = true;
    drag.ghost = make('div', 'drag-ghost', label(drag.s));
    document.body.append(drag.ghost);
    $list.classList.add('dragging');
    render(); // "새 그룹" 놓을 자리를 보여 준다.
  }
  drag.ghost.style.left = `${ev.clientX + 12}px`;
  drag.ghost.style.top = `${ev.clientY + 8}px`;
  for (const row of $list.querySelectorAll('li.drop, li.before, li.after')) row.classList.remove('drop', 'before', 'after');
  const point = document.elementFromPoint(ev.clientX, ev.clientY);
  point?.closest?.('li.group[data-drop]')?.classList.add('drop');
  // 다른 항목의 줄 위에서는 놓으면 끼어들 자리(그 줄의 위나 아래)를 선으로 보여 준다.
  const over = rowUnder(point, ev.clientY);
  if (over && over.target !== drag.s) over.row.classList.add(over.after ? 'after' : 'before');
  // 터미널 화면 위에서는 놓으면 창이 어떻게 나뉘는지 미리 보여 준다.
  showDropHint(paneZoneAt(ev.clientX, ev.clientY));
});

document.addEventListener('mouseup', (ev) => {
  if (!drag) return;
  const { s, active: moved, ghost } = drag;
  drag = null;
  if (!moved) return; // 그냥 클릭
  ghost.remove();
  $list.classList.remove('dragging');
  showDropHint(null);
  const point = document.elementFromPoint(ev.clientX, ev.clientY);
  const head = point?.closest?.('li.group[data-drop]');
  if (head) {
    // 묶음 머리에 놓음: 그 묶음으로 옮긴다.
    if (head.dataset.drop === 'new') askGroup([s]).then(render);
    else moveToGroup([s], head.dataset.drop === 'logical' ? head.dataset.name : '');
    return;
  }
  // 다른 항목의 줄에 놓음: 그 줄의 위나 아래로 순서를 옮긴다. 다른 그룹의 줄이면 그 그룹으로도 옮긴다.
  const over = rowUnder(point, ev.clientY);
  if (over) {
    if (over.target !== s) {
      if (over.target.group) ensureGroup(over.target.group);
      s.group = over.target.group;
      placeInOrder(s, over.target, over.after);
      saveState();
    }
    return render();
  }
  // 터미널 화면에 놓음: 놓은 위치에 따라 그 창을 나누거나 그 자리에 연다.
  const hit = paneZoneAt(ev.clientX, ev.clientY);
  if (hit) dropOnPane(s, hit.target, hit.zone);
  else if (point?.closest?.('#terms')) activate(s);
  render();
});

/** 그 자리(point)에 있는 사이드바의 항목 줄과, 세로 위치로 본 끼어들 자리(줄의 아래쪽 절반이면 뒤). */
function rowUnder(point, y) {
  const row = point?.closest?.('#sessions li[data-key]');
  const target = row && sessions.get(Number(row.dataset.key));
  if (!target) return null;
  const rect = row.getBoundingClientRect();
  return { row, target, after: y > rect.top + rect.height / 2 };
}

/** 항목 s를 사이드바 순서에서 target의 바로 앞(after이면 바로 뒤)으로 옮긴다. 이 순서는 다시 시작해도 유지된다. */
function placeInOrder(s, target, after) {
  const order = [...sessions.values()].filter((other) => other !== s);
  const at = order.indexOf(target);
  if (at < 0) return;
  order.splice(at + (after ? 1 : 0), 0, s);
  sessions.clear();
  for (const item of order) sessions.set(item.key, item);
}

/**
 * 사이드바에 보이는 차례대로의 묶음들: 만들어 둔 그룹이 먼저(비어 있어도), 그 뒤로 폴더 묶음이 처음 나온 순서대로.
 * `waitingOnly`이면 대기 중인 세션만 담고 빈 그룹은 뺀다.
 */
function groupsInOrder({ waitingOnly = false } = {}) {
  const groups = new Map();
  if (!waitingOnly) {
    for (const name of groupList) groups.set(`g:${name}`, { key: `g:${name}`, logical: true, name, hint: `그룹: ${name}`, list: [] });
  }
  for (const s of sessions.values()) {
    if (waitingOnly && !s.waiting) continue;
    const key = groupKey(s);
    if (!groups.has(key)) {
      groups.set(key, { key, logical: !!s.group, name: s.group || baseName(s.startCwd), hint: s.group ? `그룹: ${s.group}` : s.startCwd, list: [] });
    }
    groups.get(key).list.push(s);
  }
  return [...groups.values()];
}

function render() {
  // 끄는 중에 목록을 다시 그리면 놓을 자리가 사라지므로 미뤄 둔다.
  if (drag?.active && $list.querySelector('li.group.new')) return;
  // 별칭을 고치는 중에도 입력칸이 사라지지 않게 미룬다(끝나면 다시 그린다).
  if (renaming != null && $list.querySelector('input.rename')) return;

  const rows = [];
  for (const group of groupsInOrder({ waitingOnly: filterWaiting })) {
    const folded = collapsed.has(group.key);
    const head = make('li', [
      'group', group.logical && 'logical', group.list.some((s) => s.attention || s.waiting) && 'attention',
    ].filter(Boolean).join(' '));
    head.title = `${group.hint}\n클릭: 접기/펼치기 · 오른쪽 클릭: 묶음 메뉴 · 세션을 끌어다 놓으면 이 묶음으로 · 파일·글자·링크·그림을 끌어다 놓으면 이 묶음의 항목으로`;
    head.dataset.drop = group.logical ? 'logical' : 'folder';
    head.dataset.name = group.name;
    // 밖에서 끌어다 놓은 것을 이 묶음의 항목으로 만들 때 쓴다(drop.js).
    const context = groupContext(group);
    head.dataset.group = context.group;
    head.dataset.cwd = context.cwd;
    head.append(
      make('span', '', folded ? '▸' : '▾'),
      make('span', 'name', group.logical ? `◆ ${group.name}` : group.name),
    );
    // 묶음 안에서 돌고 있는 서브에이전트 수(접어 두어도 보인다).
    const running = group.list.reduce((sum, s) => sum + s.agents.size, 0);
    head.append(running ? agentPill(running, `이 묶음에서 서브에이전트 ${running}개 실행 중`) : make('span'));
    const add = make('button', 'close', '+');
    add.title = '이 묶음에 새 항목 더하기: 세션, 메모, HTTP …';
    add.onclick = (ev) => { ev.stopPropagation(); showMenu(ev, addItems(group)); };
    head.append(add);
    head.append(make('span', 'count', String(group.list.length)));
    head.onclick = () => {
      if (folded) collapsed.delete(group.key); else collapsed.add(group.key);
      localStorage.setItem('cypher.collapsed', JSON.stringify([...collapsed]));
      render();
    };
    head.oncontextmenu = (ev) => { ev.preventDefault(); groupMenu(group, ev); };
    rows.push(head);
    if (!folded) rows.push(...group.list.map(sessionItem));
  }
  if (renaming != null && !sessions.has(renaming)) renaming = null;
  if (filterWaiting && !rows.length) rows.push(make('li', 'group', '대기 중인 세션이 없습니다.'));
  if (drag?.active) {
    const fresh = make('li', 'group new', '+ 새 그룹으로 (여기에 놓기)');
    fresh.dataset.drop = 'new';
    rows.push(fresh);
  }
  $list.replaceChildren(...rows);

  // 창 아래 상태줄에도 지금 세션의 모델·effort를 항상 보여 준다.
  renderWhere();
  $('status-model').textContent = active?.model
    ? [active.model, active.effort, active.context == null ? '' : `컨텍스트 ${percent(active.context)}`,
      active.agents.size ? `서브에이전트 ${active.agents.size}` : ''].filter(Boolean).join(' · ')
    : '';

  const waitingCount = [...sessions.values()].filter((s) => s.waiting).length;
  const $waiting = $('waiting');
  $waiting.textContent = `대기 ${waitingCount}`;
  $waiting.classList.toggle('has', waitingCount > 0);
  $waiting.classList.toggle('on', filterWaiting);

  // 화면을 나눠 볼 때 창마다 붙는 제목 줄: 이 창만 남기기, 이 창 숨기기
  for (const s of visible) {
    const only = make('button', 'pane-button', '이 창만');
    only.title = '분할을 풀고 이 창만 보기 (다른 세션은 계속 실행됩니다 · 제목 줄을 두 번 눌러도 됩니다)';
    only.onclick = (ev) => { ev.stopPropagation(); showOnly(s); };
    const hide = make('button', 'pane-button', '숨기기');
    hide.title = '이 창을 화면에서 숨기기 (세션은 계속 실행되고 사이드바에 남습니다)';
    hide.onclick = (ev) => { ev.stopPropagation(); toggleShown(s); };
    s.head.replaceChildren(
      make('span', 'title', label(s)),
      ...(s.agents.size ? [agentPill(s.agents.size, agentSummary(s))] : []),
      make('span', 'model', [s.model, s.effort].filter(Boolean).join(' · ')),
      only, hide,
    );
    s.head.ondblclick = () => showOnly(s);
  }
  syncComposer();
}

/** 작업영역 맨 위에 지금 보고 있는 항목을 알려 준다: 그룹(또는 폴더 묶음) › 별칭·이름 · 원래 이름 · 폴더. */
function renderWhere() {
  const $where = $('where');
  const s = active && sessions.has(active.key) ? active : null;
  $where.hidden = !s;
  if (!s) return;
  const name = make('span', 'where-name', label(s));
  if (s.color) name.style.color = s.color;
  const parts = [
    make('span', 'where-group', s.group ? `◆ ${s.group}` : baseName(s.startCwd)),
    make('span', 'where-sep', '›'),
  ];
  if (!isTerm(s)) parts.push(make('span', 'where-kind', `${KINDS[s.kind].icon} ${KINDS[s.kind].label}`));
  parts.push(name);
  if (s.name) parts.push(make('span', 'where-origin', baseLabel(s)));
  parts.push(make('span', 'where-path', `‎${s.cwd}`));
  $where.replaceChildren(...parts);
  $where.title = `${s.group ? `그룹: ${s.group}` : `폴더 묶음: ${s.startCwd}`}\n${s.name ? `별칭: ${s.name}\n` : ''}${baseLabel(s)}\n${s.cwd}`;
}

/**
 * 사이드바의 빈 곳을 눌러 목록에 초점을 두면 ↑/↓(Home/End)로 항목 사이를 오간다.
 * 고른 항목이 화면에 나오고, 초점은 목록에 남는다. Enter나 Esc를 누르면 그 항목으로 초점이 간다.
 */
function initSidebarKeys() {
  $('sidebar').addEventListener('mousedown', (ev) => {
    if (ev.button !== 0 || ev.target.closest('li, button, input')) return;
    ev.preventDefault();
    $list.focus();
  });
  $list.addEventListener('keydown', (ev) => {
    if (ev.target !== $list || ev.ctrlKey || ev.altKey) return;
    const rows = [...$list.querySelectorAll('li[data-key]')].map((li) => sessions.get(Number(li.dataset.key))).filter(Boolean);
    const at = rows.indexOf(active);
    let next = null;
    if (ev.key === 'ArrowDown') next = rows[at < 0 ? 0 : Math.min(rows.length - 1, at + 1)];
    else if (ev.key === 'ArrowUp') next = rows[at < 0 ? rows.length - 1 : Math.max(0, at - 1)];
    else if (ev.key === 'Home') next = rows[0];
    else if (ev.key === 'End') next = rows[rows.length - 1];
    else if (ev.key === 'Enter' || ev.key === 'Escape') { ev.preventDefault(); return focusItem(active); }
    else return;
    ev.preventDefault();
    if (!next) return;
    if (next !== active) activate(next, { keepFocus: true });
    $list.querySelector(`li[data-key="${next.key}"]`)?.scrollIntoView({ block: 'nearest' });
  });
}

/** 테마·글꼴·글자 크기가 바뀌면 열려 있는 터미널과 항목 화면을 맞춘다. */
function applyLook(look) {
  for (const s of sessions.values()) {
    if (!s.term) continue;
    Object.assign(s.term.options, {
      theme: look.termTheme, fontFamily: look.termFamily, fontSize: look.termPixels, minimumContrastRatio: look.contrast,
    });
  }
  window.dispatchEvent(new Event('cypher-look')); // 브레인스토밍 그림처럼 색을 직접 칠하는 화면이 다시 그린다.
  requestAnimationFrame(() => { fitVisible(); syncSites(); });
}

/** 분할을 풀고 이 세션 하나만 화면에 남긴다. 다른 세션은 닫지 않는다. */
function showOnly(s) {
  tree = { s };
  layout();
  activate(s);
}

/** 사이드바의 빈 곳을 오른쪽 클릭했을 때의 메뉴: 새로 열기, 그룹, 보기 방식. */
function sidebarMenu(ev) {
  const items = config.profiles.map((p) => ({ label: `새 ${p.label} 세션`, action: () => newSession(p.id) }));
  if (config.profiles.some((p) => p.id === 'claude')) {
    items.push({ label: '새 워크트리 세션…', action: () => newWorktree('replace') });
  }
  for (const [kind, def] of Object.entries(KINDS)) items.push({ label: `새 ${def.label}`, action: () => createKind(kind) });
  items.push({ label: '새 그룹 만들기…', action: newGroup });
  items.push({ label: '새 세션 폴더 바꾸기…', action: pickFolder });
  items.push({ label: '빠른 이동·명령 찾기…  (Ctrl+Shift+K)', action: showPalette });
  if (closedCount()) items.push({ label: '닫은 항목 다시 열기…  (Ctrl+Shift+R)', action: showClosed });
  if (visible.length > 1) {
    items.push({ label: '분할 풀기 (지금 창만 보기)', action: () => active && showOnly(active) });
  }
  items.push({
    label: filterWaiting ? '모든 세션 보기' : '대기 중인 세션만 보기',
    action: () => { filterWaiting = !filterWaiting; render(); },
  });
  const keys = [...new Set([...sessions.values()].map(groupKey)), ...groupList.map((name) => `g:${name}`)];
  if (keys.length) {
    const allFolded = keys.every((key) => collapsed.has(key));
    items.push({
      label: allFolded ? '묶음 모두 펼치기' : '묶음 모두 접기',
      action: () => {
        collapsed = allFolded ? new Set() : new Set(keys);
        localStorage.setItem('cypher.collapsed', JSON.stringify([...collapsed]));
        render();
      },
    });
  }
  showMenu(ev, items);
}

async function pickFolder() {
  const picked = await invoke('pick_folder', { start: folder });
  if (picked) useFolder(picked);
}

/**
 * 빠른 이동 창에 올릴 것: 사이드바의 항목(최근에 본 순서, 지금 보고 있는 것은 맨 뒤)과 앱의 명령.
 * 아무것도 치지 않고 Enter를 누르면 바로 전에 보던 항목으로 간다.
 */
function paletteEntries() {
  const items = [...sessions.values()]
    .sort((a, b) => (a === active) - (b === active) || (b.usedAt || 0) - (a.usedAt || 0))
    .map((s) => {
      const state = !isTerm(s) ? '' : s.exited ? '종료됨' : WAITING_TEXT[s.waiting] || (s.busy || s.workStart ? '작업 중' : '');
      return {
        icon: isTerm(s) ? '›' : KINDS[s.kind].icon,
        title: label(s),
        note: [s.group ? `◆ ${s.group}` : baseName(s.startCwd), s.name ? baseLabel(s) : '', state, s === active ? '지금 보는 항목' : ''].filter(Boolean).join(' · '),
        keywords: [s.cwd, s.title, isTerm(s) ? '세션 터미널' : KINDS[s.kind].label, s.model].filter(Boolean).join(' '),
        tone: s.waiting === 'approval' ? 'alert' : s.waiting === 'done' ? 'done' : '',
        action: () => activate(s),
      };
    });
  const command = (title, action, note = '명령') => ({ icon: '⌘', title, note, action });
  const commands = [
    ...config.profiles.map((p) => command(`새 ${p.label} 세션`, () => newSession(p.id), `명령 · 폴더: ${folder}`)),
    ...(config.profiles.some((p) => p.id === 'claude') ? [command('새 워크트리 세션…', () => newWorktree('replace'))] : []),
    ...Object.entries(KINDS).map(([kind, def]) => command(`새 ${def.label}`, () => createKind(kind))),
    command('새 그룹 만들기…', newGroup),
    command('닫은 항목 다시 열기…', showClosed, '명령 · Ctrl+Shift+R: 가장 나중에 닫은 것'),
    command('다음 대기 세션으로', nextWaiting, '명령 · Ctrl+Shift+N'),
    command(filterWaiting ? '모든 세션 보기' : '대기 중인 세션만 보기', () => { filterWaiting = !filterWaiting; render(); }),
    command('변경 내용 옆 창 열기/닫기', () => toggleDiff(), '명령 · Ctrl+Shift+G'),
    command('화면에서 찾기', openSearch, '명령 · Ctrl+Shift+F'),
    command('자주 쓰는 지시', showSnippets, '명령 · Ctrl+Shift+P'),
    command(broadcast ? '동시 입력 끄기' : '동시 입력 켜기', () => setBroadcast(!broadcast), '명령 · Ctrl+Shift+B'),
    command('프롬프트 편집기로 가기', focusComposer, '명령 · Ctrl+Shift+E'),
    ...(visible.length > 1 && active ? [command('분할 풀기 (지금 창만 보기)', () => showOnly(active))] : []),
    command('기억 내보내기·가져오기', () => showMemory()),
    command('모양: 테마·글꼴·글자 크기', showLook, '설정'),
    command('Cypher 설정 파일 열기', () => runMenu('settings'), '설정'),
    command('Claude 공통 설정 열기 (settings.json)', () => runMenu('claude-settings'), '설정'),
    command('Claude 공통 지침 열기 (CLAUDE.md)', () => runMenu('claude-memory'), '설정'),
    command('단축키 보기', showHelp, '도움말 · F1'),
    command('새 세션 폴더 고르기…', pickFolder, `폴더 · 지금: ${folder}`),
    ...folderChoices().map(({ path, note }) => command(`새 세션 폴더를 ${baseName(path)}(으)로`, () => useFolder(path), `${note} · ${path}`)),
  ];
  return [...items, ...commands];
}

/** 창 아래에 5시간·주간 사용 한도를 항상 보여 준다. */
function renderLimits() {
  const $limits = $('status-limits');
  const when = (epoch) => {
    if (!epoch) return '';
    // 날짜와 시각은 늘 같은 자릿수로 적는다(11/13 18:00). 오늘이든 아니든 폭이 같다.
    const d = new Date(epoch * 1000);
    const two = (n) => String(n).padStart(2, '0');
    return ` (${two(d.getMonth() + 1)}/${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())} 초기화)`;
  };
  const parts = [['five_hour', '5시간'], ['seven_day', '주간']]
    .filter(([key]) => limits?.[key]?.used_percentage != null)
    .map(([key, name]) => ({ pct: limits[key].used_percentage, text: `${name} ${percent(limits[key].used_percentage)}${when(limits[key].resets_at)}` }));
  $limits.textContent = parts.length ? `사용 한도: ${parts.map((p) => p.text).join(' · ')}` : '';
  const worst = Math.max(0, ...parts.map((p) => p.pct));
  $limits.className = worst >= 95 ? 'danger' : worst >= 80 ? 'warn' : '';
}

// ───────────────────────── 세션 꾸미기·내보내기 ─────────────────────────

/**
 * 별칭 붙이기: 사이드바의 그 줄에서 바로 고친다. 세션(실행 중인 프로그램)에는 영향이 없고
 * 사이드바·창 제목·알림에 보이는 이름만 바뀐다. 비우면 원래 이름으로 돌아간다.
 */
function rename(s) {
  // 접힌 묶음 안에 있으면 펼쳐서 입력칸이 보이게 한다.
  collapsed.delete(groupKey(s));
  renaming = null;
  render();
  renaming = s.key;
  $list.querySelector(`li[data-key="${s.key}"]`)?.replaceWith(sessionItem(s));
  const input = $list.querySelector('input.rename');
  if (!input) { renaming = null; return; }
  input.focus();
  input.select();
}

/** 세션 화면에 남아 있는 글자 전체. */
function sessionText(s) {
  const buffer = s.term.buffer.active;
  const lines = [];
  for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i).translateToString(true));
  return lines.join('\n').trimEnd();
}

async function exportText(s) {
  try {
    const name = `${label(s).replace(/[\\/:*?"<>|]/g, '_')}.txt`;
    const saved = await invoke('save_text', { name, text: `${sessionText(s)}\n` });
    if (saved) setStatus(`저장했습니다: ${saved}`);
  } catch (err) {
    setStatus(`저장 실패: ${err}`);
  }
}

function sessionMenu(s, ev) {
  const term = isTerm(s);
  showMenu(ev, [
    { label: s.name ? `별칭 바꾸기… (지금: ${s.name})` : '별칭 붙이기…', action: () => rename(s) },
    ...(s.name ? [{ label: '별칭 지우기 (원래 이름으로)', action: () => { s.name = ''; render(); saveState(); } }] : []),
    { colors: COLORS, action: (color) => { s.color = color; render(); saveState(); } },
    { label: s.group ? `그룹 바꾸기… (지금: ${s.group})` : '그룹 지정…', action: () => askGroup([s], s.group) },
    ...(visible.length > 1 || !visible.includes(s) ? [{ label: '이것만 보기 (나머지는 화면에서 숨기기)', action: () => showOnly(s) }] : []),
    visible.includes(s)
      ? { label: '화면에서 숨기기', action: () => toggleShown(s) }
      : { label: '화면에 나눠 보기', action: () => toggleShown(s) },
    ...(term ? [
      ...(s.exited ? [{
        label: s.profile.claude && s.resumable && s.claudeSession ? '다시 시작 (대화 이어 가기)' : '다시 시작 (같은 폴더에서)',
        action: () => restartSession(s),
      }] : []),
      {
        label: `같은 폴더에 새 ${s.profile.label} 세션`,
        action: () => newSession(s.profile.id, { ...besideOf(s), show: visible.includes(s) ? 'split' : 'replace' }),
      },
      { label: '변경 내용 보기', action: () => { activate(s); toggleDiff(true); } },
      { label: '출력 저장…', action: () => exportText(s) },
      { label: '기억 내보내기·가져오기', action: () => showMemory(s) },
    ] : []),
    { label: '폴더를 탐색기에서 열기', action: () => openFolder(s) },
    { label: '폴더 경로 복사', action: () => invoke('clipboard_write', { text: s.cwd }).then(() => setStatus(`복사했습니다: ${s.cwd}`)).catch((err) => setStatus(String(err))) },
    { label: term ? '세션 닫기' : `${KINDS[s.kind].label} 닫기`, action: () => closeSession(s) },
  ]);
}

/** 항목의 폴더를 탐색기에서 연다. */
function openFolder(s) {
  invoke('link_open', { target: s.cwd, cwd: s.cwd, openFile: false, line: null, col: null }).catch((err) => setStatus(String(err)));
}

// ───────────────────────── 브라우저(사이트 항목)와 Claude 세션 ─────────────────────────
// 브라우저는 사이드바의 사이트 항목이다. 같은 그룹의 Claude 세션이 Cypher 도구로 그 브라우저를 조작한다.
// 어느 브라우저를 조작할지: 사용자가 넘겨준 것 → 같은 그룹에서 마지막으로 본 것 → 없으면 새로 만든다.

const chromeEngine = () => config.browser.engine?.toLowerCase() === 'chrome';
const aliveClaude = (s) => !!s.profile?.claude && !s.exited;
/** 그 항목과 같은 묶음에서 돌고 있는 Claude 세션들. */
const claudesOf = (item) => [...sessions.values()].filter((s) => aliveClaude(s) && groupKey(s) === groupKey(item));
/** 그 세션과 같은 묶음의 사이트 항목들. */
const sitesOf = (s) => [...sessions.values()].filter((other) => other.kind === 'site' && groupKey(other) === groupKey(s));
/** 그 항목과 같은 묶음에 새 항목을 만들 때 쓸 그룹 이름과 폴더. */
const besideOf = (s) => ({ group: s.group, cwd: s.group ? (s.project || s.startCwd) : s.startCwd });

/** 새로 연 Claude 세션이 입력을 받을 준비가 될 때까지(상태줄이 오면 준비된 것) 기다린다. */
async function untilReady(s) {
  for (let waited = 0; waited < 20000 && !s.model && !s.exited; waited += 300) {
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
}

/**
 * 사이트 항목의 [Claude에게]: 그 브라우저를 Claude 세션에 넘긴다. 세션이 없으면(target이 null) 같은 묶음에 새로 연다.
 * 넘겨받은 세션의 브라우저 도구는 이 사이트를 조작하고, 입력창에는 이어서 할 일을 적을 머리말이 들어간다.
 */
async function handoffSite(item, target) {
  if (!sessions.has(item.key)) return;
  if (!target) {
    target = await newSession('claude', { ...besideOf(item), show: 'split' });
    if (!target || target.exited) return notice('브라우저', '새 Claude 세션을 열지 못했습니다.');
    await untilReady(target);
  }
  // 한 브라우저는 한 세션이 맡는다(넘겨주면 앞서 맡던 세션에서는 떼어 낸다).
  for (const other of sessions.values()) if (other.browser === item.key) other.browser = null;
  target.browser = item.key;
  // 세션이 화면에 없으면 브라우저 옆에 나란히 놓아 둘 다 보이게 한다.
  if (!visible.includes(target) && visible.includes(item) && visible.length < MAX_PANES) {
    splitAt(item, target, 'right');
    layout();
  }
  activate(target);
  const url = item.view.url();
  target.term.paste(`Cypher 브라우저 "${label(item)}"${url ? `(${url})` : ''}에서 이어서 작업해줘: `);
  syncSites();
}

/** 그 세션이 지금 조작하는 브라우저: 넘겨받은 것, 없으면 같은 묶음에서 마지막으로 본 것. */
function browserOf(s) {
  const bound = s.browser != null ? sessions.get(s.browser) : null;
  if (bound?.kind === 'site') return bound;
  return sitesOf(s).sort((a, b) => (b.usedAt || 0) - (a.usedAt || 0))[0] || null;
}

const siteLine = (site) => `#${site.key} "${label(site)}" — ${site.view.url() || '빈 페이지'}`;

/** 브라우저 도구가 조작할 사이트를 정한다. `create`이면(주소로 이동하라는 요청) 묶음에 브라우저가 없을 때 새로 만든다. */
async function browserTarget(s, create) {
  let site = browserOf(s);
  const created = !site;
  if (!site) {
    if (!create) throw new Error('이 세션의 그룹에 브라우저(사이트 항목)가 없습니다. browser_navigate로 주소를 열면 새 브라우저가 만들어집니다.');
    site = await newItem('site', { ...besideOf(s), saved: {}, show: 'hidden' });
    // 세션이 화면에 보이면 그 옆에 나란히 놓아 Claude가 하는 일을 볼 수 있게 한다(초점은 세션에 그대로 둔다).
    if (visible.includes(s) && visible.length < MAX_PANES) {
      splitAt(s, site, 'right');
      layout();
    }
    reveal([site]);
  }
  if (s.browser !== site.key) {
    for (const other of sessions.values()) if (other.browser === site.key) other.browser = null;
    s.browser = site.key;
    syncSites();
  }
  const others = sitesOf(s).filter((other) => other !== site);
  return {
    key: site.key,
    url: site.view.url() || 'about:blank',
    // 도구의 답 끝에 덧붙일 말: 새로 만들었거나, 브라우저가 여러 개라 어느 것을 조작했는지 알려야 할 때만.
    note: created ? `이 그룹에 새 브라우저(사이트 항목 #${site.key})를 만들어 열었습니다.`
      : others.length ? `조작한 브라우저: 사이트 항목 #${site.key}. 같은 그룹에 다른 브라우저가 ${others.length}개 더 있습니다(browser_tabs로 보고 browser_select로 바꿉니다).` : '',
  };
}

function browserTabs(s) {
  const sites = sitesOf(s);
  if (!sites.length) return '이 세션의 그룹에 브라우저(사이트 항목)가 없습니다. browser_navigate로 주소를 열면 새로 만들어집니다.';
  const current = browserOf(s);
  return `이 세션의 그룹에 있는 브라우저(사이트 항목):\n${sites.map((site) => `${site === current ? '●' : ' '} ${siteLine(site)}${site === current ? '  ← 지금 조작 대상' : ''}`).join('\n')}`;
}

function browserSelect(s, id) {
  const site = sessions.get(Number(id));
  if (site?.kind !== 'site' || groupKey(site) !== groupKey(s)) throw new Error(`#${id}은(는) 이 세션의 그룹에 있는 브라우저가 아닙니다. browser_tabs로 번호를 확인하세요.`);
  for (const other of sessions.values()) if (other.browser === site.key) other.browser = null;
  s.browser = site.key;
  syncSites();
  return `조작 대상을 바꿨습니다: ${siteLine(site)}`;
}

/** 같은 묶음의 항목을 Claude 세션이 알 수 있게 글로 적는다: 종류, 이름, 파일 경로나 주소. */
function groupItemsText(s) {
  const dir = (name) => storeDirs[name] || `%APPDATA%\\cypher\\${name}`;
  const lines = [...sessions.values()].filter((other) => groupKey(other) === groupKey(s)).map((other) => {
    const head = `#${other.key}`;
    const name = `"${label(other)}"`;
    if (other === s) return `- ${head} 이 세션 ${name} — 폴더: ${other.cwd}`;
    if (isTerm(other)) {
      const state = other.exited ? '종료됨' : WAITING_TEXT[other.waiting] || (other.busy ? '작업 중' : '대기');
      return `- ${head} ${other.profile.label} 세션 ${name} — 폴더: ${other.cwd} · ${state}`;
    }
    const state = other.view.state();
    if (other.kind === 'memo') {
      const file = state.path && !state.dirty ? state.path : `${dir('memos')}\\${state.memo}.txt`;
      return `- ${head} 메모 ${name} — 파일: ${file}${state.path && state.dirty ? ` (저장하지 않은 고침이 있음. 원래 파일: ${state.path})` : ''}`;
    }
    if (other.kind === 'http') return `- ${head} HTTP 요청 ${name} — 요청 폴더: ${other.startCwd}${state.file ? ` · 열린 파일: ${state.file}` : ''} (http_list / http_run으로 다룬다)`;
    if (other.kind === 'site') return `- ${head} 브라우저(사이트) ${name} — 주소: ${other.view.url() || '빈 페이지'} (browser_* 도구로 조작)`;
    if (other.kind === 'image') return `- ${head} 이미지 ${name} — 파일: ${other.view.path() || '(아직 그림 없음)'}${state.source ? ` · 출처: ${state.source}` : ''}`;
    if (other.kind === 'mindmap') return `- ${head} 브레인스토밍 ${name} — 개요 글 파일: ${dir('mindmaps')}\\${other.view.store()}.md (고칠 때는 mindmap_write에 id ${other.key})`;
    return `- ${head} ${KINDS[other.kind]?.label || other.kind} ${name}`;
  });
  const where = s.group ? `그룹 "${s.group}"` : `폴더 묶음 ${s.startCwd} (이름 붙인 그룹은 아님)`;
  return `${where}의 Cypher 항목 ${lines.length}개:\n${lines.join('\n')}`;
}

/** 브레인스토밍 항목을 개요 글로 만들거나 고친다. */
async function mindmapWrite(s, { item, title, text }) {
  if (item != null) {
    const target = sessions.get(Number(item));
    if (target?.kind !== 'mindmap') throw new Error(`#${item}은(는) 브레인스토밍 항목이 아닙니다. cypher_items로 번호를 확인하세요.`);
    await target.view.ready;
    if (target.view.setText(text) === false) {
      throw new Error(`브레인스토밍 #${target.key}은(는) 사용자가 코드 화면에서 고치는 중이라 바꾸지 않았습니다(오류가 있어 아직 반영되지 않은 글이 있습니다). 사용자에게 알리고, 사용자가 고친 뒤에 다시 시도하세요.`);
    }
    if (title) target.name = title;
    reveal([target]);
    saveState();
    return `브레인스토밍 #${target.key} "${label(target)}"의 내용을 바꿨습니다.`;
  }
  const made = await newItem('mindmap', { ...besideOf(s), saved: { text }, name: title || '', show: 'hidden' });
  reveal([made]);
  return `브레인스토밍 항목 #${made.key} "${label(made)}"을 만들었습니다(사이드바의 같은 그룹). 고칠 때는 mindmap_write에 id ${made.key}를 주세요.`;
}

/** Claude 세션의 Cypher 도구가 물어 온 것에 답한다. `req.session`은 도구를 부른 세션의 번호. */
async function mcpAction(req) {
  const s = (req.session != null && byId(req.session)) || (active && aliveClaude(active) ? active : null);
  if (!s) throw new Error('이 도구를 부른 Claude 세션을 Cypher에서 찾지 못했습니다.');
  switch (req.action) {
    case 'browser_target': return browserTarget(s, !!req.create);
    case 'browser_tabs': return { text: browserTabs(s) };
    case 'browser_select': return { text: browserSelect(s, req.id) };
    case 'items': return { text: groupItemsText(s) };
    case 'mindmap_write': return { text: await mindmapWrite(s, req) };
    default: throw new Error(`모르는 요청입니다: ${req.action}`);
  }
}

async function onMcpRequest(req) {
  let ok = true, value;
  try {
    value = await mcpAction(req);
  } catch (err) {
    ok = false;
    value = String(err?.message || err);
  }
  invoke('mcp_reply', { id: req.id, ok, value });
}

// ── "chrome" 엔진(설정에서 바꿨을 때만): 설치된 Chrome을 Cypher 전용 프로필로 띄워 조작한다 ──

async function openChrome() {
  try {
    await invoke('browser_open', { url: null });
  } catch (err) {
    notice('브라우저를 열지 못했습니다', String(err));
  }
}

/** Chrome에 지금 열려 있는 페이지를 Claude 세션에 넘긴다. `fresh`이면 새 Claude 세션을 연다. */
async function handoffChrome(fresh) {
  const url = await invoke('browser_where').catch(() => null);
  if (!url) return notice('브라우저', 'Chrome이 열려 있지 않습니다. 먼저 [Chrome]을 눌러 여세요.');
  const current = active && aliveClaude(active) ? active : [...sessions.values()].find(aliveClaude);
  let target = fresh ? null : current;
  if (!target) {
    const base = current || active;
    target = await newSession('claude', { ...(base ? besideOf(base) : { cwd: folder }), show: 'split' });
    if (!target || target.exited) return notice('브라우저', '새 Claude 세션을 열지 못했습니다.');
    await untilReady(target);
  }
  activate(target);
  target.term.paste(`Cypher 브라우저에 열려 있는 페이지(${url})에서 이어서 작업해줘: `);
}

// ───────────────────────── 이미지 항목 ─────────────────────────

/**
 * 저장소에 넣은 그림들을 이미지 항목으로 만든다. 만든(또는 채운) 항목들을 돌려준다.
 * opts.near: 이 항목과 같은 묶음·폴더에 만든다. 그것이 빈 이미지 항목이면 첫 장으로 그 항목을 채운다.
 * opts.group / opts.cwd: 묶음을 직접 정한다. 아무것도 없으면 지금 보고 있는 항목의 묶음에 만든다.
 * opts.source: 그림이 온 곳(캡처한 페이지 주소 등) · opts.shot / opts.full: 화면 캡처인지, 페이지 전체인지 · opts.show: 기본은 사이드바에만 놓는다('hidden').
 */
async function addImages(list, { near = null, group, cwd, source = '', shot = false, full = false, show = 'hidden' } = {}) {
  const base = near && sessions.has(near.key) ? near : null;
  const made = [];
  for (const stored of list) {
    if (base?.kind === 'image' && base.view.empty()) {
      await base.view.fill(stored, { source, shot, full });
      made.push(base);
      continue;
    }
    made.push(await newItem('image', {
      group: group ?? base?.group,
      cwd: cwd ?? (base ? base.project || base.startCwd : undefined),
      saved: { image: stored.id, ext: stored.ext, name: stored.name, source, at: Date.now(), shot, full },
      show: made.length ? 'hidden' : show,
    }));
  }
  reveal(made);
  return made.filter(Boolean);
}

/** 방금 만든 항목들이 사이드바에서 눈에 띄게 한다: 접힌 묶음을 펴고, 화면에 없는 것은 주의 표시를 한다. */
function reveal(items) {
  const fresh = items.filter((s) => s && sessions.has(s.key));
  if (!fresh.length) return;
  for (const s of fresh) {
    if (!visible.includes(s)) s.attention = true;
    collapsed.delete(groupKey(s));
  }
  localStorage.setItem('cypher.collapsed', JSON.stringify([...collapsed]));
  render();
  $list.querySelector(`li[data-key="${fresh[fresh.length - 1].key}"]`)?.scrollIntoView({ block: 'nearest' });
}

/**
 * 화면을 찍은 그림을 이미지 항목으로 넣는다. `near`가 있으면(사이트 항목에서 찍음) 그 항목의 묶음에,
 * 없으면("chrome" 엔진의 브라우저에서 찍음) 지금 보고 있는 항목의 묶음에 넣는다.
 */
async function onCaptured(captured, near) {
  const [s] = await addImages([captured.image], { near, source: captured.source, shot: true, full: captured.full });
  if (!s) return;
  const where = s.group ? `그룹 "${s.group}"` : `"${baseName(s.startCwd)}" 묶음`;
  toast(s, captured.full ? '페이지 전체 캡처' : '화면 캡처',
    `${label(s)} — ${where}에 이미지 항목으로 넣었습니다.${captured.cut ? ' 페이지가 너무 길어 아래쪽은 잘렸습니다.' : ''} (누르면 엽니다)`);
}

/** 본창에서 브라우저 화면을 찍는다. 찍힌 그림은 `image-captured` 사건으로 돌아온다. */
async function captureBrowser(full) {
  setStatus('브라우저 화면을 찍는 중…');
  try {
    await invoke('browser_capture', { full });
    setStatus('');
  } catch (err) {
    setStatus('');
    notice('캡처하지 못했습니다', String(err));
  }
}

/** 이미지 파일의 경로를 Claude 세션 입력창에 넣는다. 같은 묶음의 세션을 먼저 고르고, 여럿이면 고르게 한다. */
function imageToClaude(item, path, ev) {
  if (!path) return setStatus('아직 그림이 없습니다.');
  const claudes = [...sessions.values()].filter((s) => s.profile?.claude && !s.exited);
  if (!claudes.length) return setStatus('열려 있는 Claude 세션이 없습니다. 먼저 Claude 세션을 여세요.');
  const give = (target) => {
    activate(target);
    target.term.paste(`${quote(path)} `);
  };
  const same = claudes.filter((s) => groupKey(s) === groupKey(item));
  const pool = same.length ? same : claudes;
  if (pool.length === 1) return give(pool[0]);
  showMenu(ev, pool.map((s) => ({ label: `${label(s)}에 넣기`, action: () => give(s) })));
}

/** 대기 중인 다음 세션으로 간다. 승인 대기가 있으면 그쪽을 먼저 돈다. */
function nextWaiting() {
  const all = inSidebarOrder();
  const approvals = all.filter((s) => s.waiting === 'approval');
  const pool = approvals.length ? approvals : all.filter((s) => s.waiting);
  if (!pool.length) return setStatus('대기 중인 세션이 없습니다.');
  const from = all.indexOf(active);
  const next = pool.find((s) => all.indexOf(s) > from) || pool[0];
  activate(next);
}

// ───────────────────────── 사이드바 너비 ─────────────────────────

/** 사이드바와 화면 사이의 손잡이를 끌어 사이드바 너비를 바꾼다. 두 번 누르면 기본 너비로 돌아간다. */
function initSidebarGrip() {
  const grip = $('sidebar-grip');
  const apply = (width) => {
    const clamped = Math.min(Math.max(160, Math.round(width)), Math.max(200, innerWidth - 320));
    document.body.style.setProperty('--sidebar', `${clamped}px`);
    return clamped;
  };
  apply(Number(localStorage.getItem('cypher.sidebar')) || SIDEBAR_WIDTH);
  grip.onmousedown = (ev) => {
    ev.preventDefault();
    grip.classList.add('active');
    document.body.classList.add('resizing-row');
    let width = 0;
    const move = (e) => { width = apply(e.clientX); };
    const stop = () => {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', stop);
      grip.classList.remove('active');
      document.body.classList.remove('resizing-row');
      if (width) localStorage.setItem('cypher.sidebar', String(width));
      fitVisible();
    };
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', stop);
  };
  grip.ondblclick = () => {
    localStorage.setItem('cypher.sidebar', String(apply(SIDEBAR_WIDTH)));
    fitVisible();
  };
}

// ───────────────────────── 프롬프트 편집기 ─────────────────────────
// 화면에 세션 하나만 보일 때(분할하지 않았을 때) 세션 아래에 놓이는 글 편집기.
// 긴 프롬프트를 여기서 편하게 쓰고 [프롬프트]를 누르면 그 세션의 입력창에 들어간다.
// 쓰던 글은 세션마다 따로 기억한다.

/**
 * 프롬프트 편집기가 맡을 세션: 화면에 하나만 보이는 터미널 세션.
 * 세션이 종료돼도 편집기는 남긴다(쓰던 글을 복사해 갈 수 있게). 보내기만 막는다.
 */
const composerTarget = () => (visible.length === 1 && isTerm(visible[0]) ? visible[0] : null);

/** 프롬프트 편집기를 접거나 편다. 단추 글자와 저장값을 함께 맞춘다. */
function setComposerFolded(folded) {
  $('composer').classList.toggle('folded', folded);
  $('composer-fold').textContent = folded ? '프롬프트 편집기 ▴' : '프롬프트 편집기 ▾';
  localStorage.setItem('cypher.composer.folded', folded ? '1' : '');
}

function initComposer() {
  const $text = $('composer-text'), $box = $('composer');
  const setHeight = (height) => {
    const clamped = Math.min(Math.max(56, Math.round(height)), Math.max(120, innerHeight - 260));
    $box.style.setProperty('--composer', `${clamped}px`);
    return clamped;
  };
  setHeight(Number(localStorage.getItem('cypher.composer.height')) || COMPOSER_HEIGHT);
  setComposerFolded(!!localStorage.getItem('cypher.composer.folded'));
  $('composer-fold').onclick = () => {
    setComposerFolded(!$box.classList.contains('folded'));
    if (!$box.classList.contains('folded')) $text.focus(); else focusItem(active);
  };
  $('composer-enter').checked = !!localStorage.getItem('cypher.composer.enter');
  $('composer-enter').onchange = (ev) => localStorage.setItem('cypher.composer.enter', ev.target.checked ? '1' : '');
  $('composer-send').onclick = () => sendPrompt();
  $('composer-history').onclick = promptMenu;
  $text.addEventListener('input', () => {
    if (!recalling) promptAt = -1; // 직접 고치기 시작하면 기록을 넘기던 자리를 잊는다.
    if (composerFor) { composerFor.draft = $text.value; saveState(); }
  });
  $text.addEventListener('keydown', (ev) => {
    if (ev.ctrlKey && !ev.shiftKey && ev.key === 'Enter') { ev.preventDefault(); sendPrompt(); }
    else if (ev.altKey && !ev.ctrlKey && (ev.key === 'ArrowUp' || ev.key === 'ArrowDown')) { ev.preventDefault(); recallPrompt(ev.key === 'ArrowUp' ? 1 : -1); }
    else if (ev.key === 'Escape') focusItem(composerFor);
  });
  // 위쪽 손잡이를 끌어 높이를 바꾼다.
  const grip = $('composer-grip');
  grip.onmousedown = (ev) => {
    ev.preventDefault();
    document.body.classList.add('resizing-col');
    let height = 0;
    const bottom = $box.getBoundingClientRect().bottom;
    const move = (e) => { height = setHeight(bottom - e.clientY - 4); };
    const stop = () => {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', stop);
      document.body.classList.remove('resizing-col');
      if (height) localStorage.setItem('cypher.composer.height', String(height));
    };
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', stop);
  };
  grip.ondblclick = () => localStorage.setItem('cypher.composer.height', String(setHeight(COMPOSER_HEIGHT)));
}

/** 화면 상태에 맞춰 프롬프트 편집기를 보이거나 숨기고, 세션이 바뀌면 그 세션이 쓰던 글로 바꾼다. */
function syncComposer() {
  const target = composerTarget();
  const $text = $('composer-text');
  if (composerFor !== target) {
    if (composerFor && sessions.has(composerFor.key)) composerFor.draft = $text.value;
    promptAt = -1;
    composerFor = target;
    $text.value = target?.draft || '';
  }
  $('composer').hidden = !target;
  $('composer-send').disabled = !!target?.exited;
  $('composer-send').title = target?.exited
    ? '이 세션은 종료됐습니다. 쓰던 글은 복사해서 다른 세션에 쓸 수 있습니다.'
    : '편집기에 쓴 내용을 지금 보고 있는 세션의 입력창에 넣습니다 (Ctrl+Enter)';
  if (target) {
    $text.placeholder = `"${label(target)}"에 보낼 긴 프롬프트를 여기에 씁니다. [프롬프트] 또는 Ctrl+Enter를 누르면 세션 입력창에 들어갑니다. (Ctrl+Shift+E: 여기로 오기)`;
  }
}

function focusComposer() {
  if (!composerTarget()) return setStatus('프롬프트 편집기는 화면에 세션 하나만 보일 때 쓸 수 있습니다.');
  setComposerFolded(false);
  $('composer-text').focus();
}

// 보낸 프롬프트 기록: 편집기에서 보낸 글을 기억해 두고 Alt+↑/↓나 [기록] 단추로 다시 불러온다(세션과 무관하게 함께 쓴다).

let promptAt = -1;      // 기록을 넘겨 보는 자리(0이 가장 최근). -1이면 쓰던 글을 보고 있다.
let promptDraft = '';   // 기록을 넘기기 전에 쓰던 글
let recalling = false;  // 기록의 글을 편집기에 넣는 중(이때의 입력 사건은 사용자가 친 것이 아니다)

function rememberPrompt(text) {
  promptLog = [...promptLog.filter((old) => old !== text), text].slice(-MAX_PROMPTS);
  try { localStorage.setItem('cypher.prompts', JSON.stringify(promptLog)); } catch { /* 저장 공간 문제는 무시 */ }
}

/** 편집기의 글을 통째로 바꾼다. 되돌리기(Ctrl+Z)로 앞의 글을 되살릴 수 있게 편집 명령으로 넣는다. */
function setComposerText(text) {
  const $text = $('composer-text');
  recalling = true;
  try {
    $text.focus();
    $text.select();
    if (text) document.execCommand('insertText', false, text); else document.execCommand('delete');
    if ($text.value !== text) $text.value = text; // 편집 명령이 듣지 않는 경우
  } finally {
    recalling = false;
  }
  if (composerFor) { composerFor.draft = $text.value; saveState(); }
}

/** 보낸 프롬프트 기록을 넘겨 본다. step 1: 더 예전 것, -1: 더 최근 것(끝까지 오면 쓰던 글로 돌아온다). */
function recallPrompt(step) {
  if (!promptLog.length) return setStatus('보낸 프롬프트 기록이 없습니다.');
  const next = Math.min(promptLog.length - 1, Math.max(-1, promptAt + step));
  if (next === promptAt) return;
  if (promptAt === -1) promptDraft = $('composer-text').value;
  promptAt = next;
  setComposerText(next === -1 ? promptDraft : promptLog[promptLog.length - 1 - next]);
  setStatus(next === -1 ? '쓰던 글로 돌아왔습니다.' : `보낸 프롬프트 ${next + 1}/${promptLog.length} (Alt+↑: 더 예전 것 · Alt+↓: 더 최근 것)`);
}

/** [기록] 단추: 최근에 보낸 프롬프트를 골라 편집기에 넣는다. */
function promptMenu(ev) {
  if (!promptLog.length) return setStatus('보낸 프롬프트 기록이 없습니다. 편집기에서 보낸 글이 여기에 쌓입니다.');
  const brief = (text) => { const line = text.trim().split('\n')[0]; return line.length > 60 ? `${line.slice(0, 60)}…` : line; };
  showMenu(ev, [
    { caption: '최근에 보낸 프롬프트 (누르면 편집기에 넣습니다 · Alt+↑/↓로도 넘깁니다)' },
    ...promptLog.slice(-15).reverse().map((text) => ({ label: brief(text) + (text.includes('\n') ? '  ⏎' : ''), action: () => { promptAt = -1; setComposerText(text); } })),
    { label: '기록 지우기', action: () => { promptLog = []; localStorage.removeItem('cypher.prompts'); promptAt = -1; setStatus('보낸 프롬프트 기록을 지웠습니다.'); } },
  ]);
}

/**
 * 프롬프트 편집기에 쓴 글을 지금 보고 있는 세션의 입력창에 넣는다.
 * "바로 전송"이 켜져 있으면 Enter까지 눌러 보낸다. 넣은 뒤 편집기는 비운다(Ctrl+Z로 되살릴 수 있다).
 */
function sendPrompt() {
  const s = composerFor, $text = $('composer-text');
  const text = $text.value.replace(/\s+$/, '');
  if (!s || !text) return;
  if (s.exited) return setStatus('이 세션은 종료됐습니다. 쓰던 글은 편집기에 그대로 있으니 복사해서 다른 세션에 쓰세요.');
  s.term.paste(text); // 여러 줄도 한 번의 붙여넣기로 들어간다.
  rememberPrompt(text);
  promptAt = -1;
  if ($('composer-enter').checked) setTimeout(() => send(s, '\r'), 200);
  if (s.waiting) s.waiting = '';
  $text.focus();
  $text.select();
  document.execCommand('delete');
  s.draft = '';
  saveState();
  s.term.focus();
  render();
}

// ───────────────────────── 배치 기억 ─────────────────────────

let saveTimer = 0;

function saveState() {
  if (restoring) return;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(writeState, 300);
}

/** 열린 세션과 배치를 적어 둔다. 다시 시작하면 restore()가 되살린다. */
function writeState() {
  if (closing || restoring) return; // 창이 닫히는 중의 종료 알림이나, 되살리는 도중의 덜 찬 목록을 적지 않는다.
  if (composerFor && sessions.has(composerFor.key)) composerFor.draft = $('composer-text').value;
  const live = [...sessions.values()].filter((s) => (isTerm(s) ? !s.exited && s.id != null : !!s.view));
  const state = {
    sessions: live.map((s) => ({ ...describe(s), shown: visible.includes(s) })),
    active: live.indexOf(active),
    // 화면 분할 모양. 잎은 세션의 순번({ i }), 가지는 { dir, a, b }.
    layout: (function pack(node) {
      if (!node) return null;
      return node.s ? { i: live.indexOf(node.s) } : { dir: node.dir, ratio: ratioOf(node), a: pack(node.a), b: pack(node.b) };
    })(tree),
  };
  try { localStorage.setItem('cypher.state', JSON.stringify(state)); } catch { /* 저장 공간 문제는 무시 */ }
}

/**
 * 항목을 다시 만들 수 있게 적어 둘 내용: 종류, 폴더, 별칭·그룹·색, 그리고 세션이면 프로필과 이어갈 대화 번호,
 * 그 밖의 항목이면 그 화면이 내놓은 상태. 배치 기억(writeState)과 닫은 항목 목록이 함께 쓴다.
 */
function describe(s) {
  return {
    kind: s.kind,
    cwd: s.project || s.startCwd,
    name: s.name,
    group: s.group,
    color: s.color,
    ...(isTerm(s) ? {
      profile: s.profile.id,
      resume: s.resumable && s.claudeSession ? s.claudeSession : null,
      draft: s.draft || undefined,
    } : { saved: s.view.state() }),
  };
}

/** describe()로 적어 둔 내용으로 항목을 다시 만든다. `opts`는 newSession·newItem에 그대로 더해진다(show, restoring 등). */
function revive(item, opts = {}) {
  const common = { cwd: item.cwd, name: item.name, color: item.color, group: item.group || '', ...opts };
  // 종류 표시가 없는 예전 기록은 터미널 세션이다.
  const term = !item.kind || item.kind === 'term';
  return term
    ? newSession(item.profile, { ...common, resume: item.resume, draft: item.draft })
    : newItem(item.kind, { ...common, saved: item.saved });
}

/** 저장해 둔 분할 모양을 되살린다. 없어진 세션의 자리는 짝이 차지한다. */
function unpack(data, list) {
  if (!data) return null;
  if ('i' in data) return list[data.i] ? { s: list[data.i] } : null;
  const a = unpack(data.a, list), b = unpack(data.b, list);
  return a && b ? { dir: data.dir === 'col' ? 'col' : 'row', ratio: data.ratio, a, b } : (a || b);
}

/**
 * 지난번 항목과 배치를 되살린다.
 * `docsOnly`이면(세션 복원을 꺼 두었거나 바로 시작 모드) 터미널 세션은 되살리지 않고 메모·HTTP 항목만
 * 사이드바에 되살린다. 메모는 사용자가 쓴 글이라, 세션 복원 설정과 무관하게 목록에서 사라지면 안 된다.
 */
async function restore({ docsOnly = false } = {}) {
  docsOnly = docsOnly || !config.session.restore;
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem('cypher.state') || 'null'); } catch { /* 깨진 기록은 버린다 */ }
  if (!saved?.sessions?.length) return;
  const restored = [];
  // 되살리는 동안에는 상태를 적지 않는다. 도중에 적으면 아직 되살리지 않은 뒤쪽 항목이 기록에서 빠져,
  // 그때 앱이 꺼지면 그 항목(메모 포함)이 목록에서 사라진다.
  restoring = true;
  try {
    for (const item of saved.sessions) {
      const term = !item.kind || item.kind === 'term';
      restored.push(term && docsOnly ? null : await revive(item, { show: 'hidden', restoring: true }));
    }
  } finally {
    restoring = false;
  }
  if (docsOnly) { render(); saveState(); return; } // 메모·HTTP는 사이드바에만 되살린다.
  // 분할 모양을 되살린다. 저장된 모양이 없으면(예전 기록) 보이던 세션을 고르게 나눈다.
  // 되살리는 도중에 사용자가 닫은 항목은 배치에서 뺀다(순번은 저장 때와 맞아야 하므로 자리는 비워 둔다).
  const alive = restored.map((s) => (s && sessions.has(s.key) ? s : null));
  tree = unpack(saved.layout, alive)
    || balanced(alive.filter((s, index) => s && saved.sessions[index].shown).slice(0, MAX_PANES));
  layout();
  const target = alive[saved.active] || alive.find(Boolean);
  if (target) activate(target);
}

// ───────────────────────── 입력 ─────────────────────────

/** 키 입력을 순서대로 보낸다. 동시 입력이 켜져 있으면 나란히 보이는 모든 세션에 보낸다. */
function send(s, data) {
  const targets = broadcast && visible.includes(s) ? visible.filter((v) => isTerm(v) && !v.exited) : [s];
  for (const t of targets) {
    t.pending += data;
    if (!t.sending) flush(t);
  }
}

/** 보내는 중에 들어온 입력은 모아서 다음에 한 번에 보낸다(순서 보장). */
async function flush(s) {
  if (s.id == null || s.sending) return;
  s.sending = true;
  while (s.pending && !s.exited) {
    const data = s.pending;
    s.pending = '';
    try { await invoke('session_write', { id: s.id, data }); } catch { break; }
  }
  s.sending = false;
}

function wireTerminal(s) {
  const { term } = s;
  term.onData((data) => send(s, data));
  term.onResize(({ cols, rows }) => {
    if (s.id != null && !s.exited) invoke('session_resize', { id: s.id, cols, rows });
  });
  term.onTitleChange((title) => { s.title = title; render(); });
  term.onBell(() => { if (s !== active) { s.attention = true; render(); } });
  term.textarea?.addEventListener('focus', () => { s.focusedAt = performance.now(); });

  // 셸이 알려 주는 현재 폴더(OSC 9;9 경로, OSC 7 file://…)를 따라간다.
  term.parser.registerOscHandler(9, (data) => {
    if (!data.startsWith('9;')) return false;
    s.cwd = data.slice(2).replace(/^"|"$/g, '');
    render();
    return true;
  });
  term.parser.registerOscHandler(7, (data) => {
    const m = /^file:\/\/[^/]*\/(.+)$/.exec(data);
    if (m) { s.cwd = decodeURIComponent(m[1]).replace(/\//g, '\\'); render(); }
    return true;
  });

  term.attachCustomKeyEventHandler((ev) => {
    const key = ev.key.toLowerCase();
    // Shift+Enter는 줄바꿈으로 전달한다(Claude Code 여러 줄 입력).
    if (key === 'enter' && ev.shiftKey && !ev.ctrlKey && !ev.altKey) {
      if (ev.type === 'keydown') send(s, '\x1b\r');
      ev.preventDefault();
      return false;
    }
    // 사용자가 이 세션에 직접 입력하면 대기 표시를 푼다.
    if (ev.type === 'keydown' && s.waiting && (key.length === 1 || key === 'enter' || key === 'escape')) {
      s.waiting = '';
      render();
    }
    if (ev.type !== 'keydown' || !ev.ctrlKey || ev.altKey) return true;
    if (key === 'v') {
      ev.preventDefault(); // 웹뷰 자체 붙여넣기와 겹치지 않게 한다.
      paste(s, ev.shiftKey);
      return false;
    }
    if (key === 'c' && (ev.shiftKey || term.hasSelection())) {
      ev.preventDefault();
      copy(s);
      return false;
    }
    return true;
  });

  // 오른쪽 클릭: 선택 영역이 있으면 복사, 없으면 붙여넣기.
  s.pane.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    if (term.hasSelection()) copy(s); else paste(s, false);
  });

  // 프로그램이 직접 심은 링크(OSC 8).
  term.options.linkHandler = {
    allowNonHttpProtocols: true,
    activate: (ev, uri) => openLink(s, { target: uri }, ev),
    hover: (_ev, uri) => setStatus(`링크 열기: ${uri}`),
    leave: () => setStatus(''),
  };
  term.registerLinkProvider({ provideLinks: (y, cb) => provideLinks(s, y, cb) });
}

function copy(s) {
  const text = s.term.getSelection();
  if (text) invoke('clipboard_write', { text });
  s.term.clearSelection();
}

/**
 * 붙여넣기. 클립보드에 이미지가 있으면
 * - Claude 세션: Alt+V 신호를 보내 Claude Code가 클립보드를 직접 읽게 한다.
 * - 그 밖의 세션, 또는 Ctrl+Shift+V: PNG 파일로 저장하고 그 경로를 붙여넣는다.
 */
async function paste(s, forceFile) {
  try {
    const clip = await invoke('clipboard_read');
    if (clip.kind === 'empty') return;
    if (clip.kind !== 'image') return s.term.paste(clip.text);
    if (s.profile.claude && !forceFile) send(s, '\x1bv');
    else s.term.paste(clip.text);
    addShot(s, clip);
  } catch (err) {
    setStatus(`붙여넣기 실패: ${err}`);
  }
}

// ───────────────────────── 이미지 미리보기 ─────────────────────────

async function addShot(s, clip) {
  try {
    const bytes = await invoke('paste_preview', { path: clip.path });
    const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
    s.shots.push({ url, path: clip.path, size: `${clip.width}×${clip.height}` });
    while (s.shots.length > MAX_SHOTS) URL.revokeObjectURL(s.shots.shift().url);
    renderShots();
  } catch (err) {
    setStatus(`미리보기 실패: ${err}`);
  }
}

/** 활성 세션에 붙여넣은 이미지를 터미널 아래에 작게 보여 준다. 클릭하면 원본을 연다. */
function renderShots() {
  const $shots = $('shots');
  const shots = active?.shots ?? [];
  $shots.hidden = !shots.length;
  $shots.replaceChildren(...shots.map((shot) => {
    const box = make('div', 'shot');
    box.title = `클릭: 원본 열기 — ${shot.path}`;
    box.onclick = () => invoke('link_open', { target: shot.path, cwd: active.cwd, openFile: true });
    const img = make('img');
    img.src = shot.url;
    const close = make('button', '', '×');
    close.title = '미리보기 지우기';
    close.onclick = (ev) => {
      ev.stopPropagation();
      URL.revokeObjectURL(shot.url);
      active.shots.splice(active.shots.indexOf(shot), 1);
      renderShots();
    };
    box.append(img, make('span', '', shot.size), close);
    return box;
  }));
}

// ───────────────────────── 찾기 ─────────────────────────

function openSearch() {
  if (!active) return;
  if (!isTerm(active)) return active.view.find?.(); // 메모는 자기 찾기 줄을 쓴다.
  $search.hidden = false;
  $searchInput.focus();
  $searchInput.select();
  if ($searchInput.value) find(false, true); // 지난번에 찾던 글이 남아 있으면 개수를 바로 보여 준다.
}

function closeSearch() {
  $search.hidden = true;
  active?.search?.clearDecorations();
  $('search-count').textContent = '';
  focusItem(active);
}

/** 찾기 방식(대/소문자 구분, 정규식)을 켜거나 끈다. `key` 없이 부르면 단추 표시만 맞춘다. */
function setSearchOption(key) {
  if (key) {
    searchOptions[key] = !searchOptions[key];
    localStorage.setItem('cypher.search', JSON.stringify(searchOptions));
  }
  $('search-case').classList.toggle('on', !!searchOptions.caseSensitive);
  $('search-regex').classList.toggle('on', !!searchOptions.regex);
  if (key && !$search.hidden) { find(false, true); $searchInput.focus(); }
}

/** 찾은 개수와 지금 몇 번째인지: "3/15". 너무 많아 다 세지 못했으면 "많음". */
function showSearchCount({ resultIndex, resultCount } = {}) {
  const $count = $('search-count');
  if (!$searchInput.value) $count.textContent = '';
  else if (!resultCount) $count.textContent = '없음';
  else $count.textContent = resultIndex >= 0 ? `${resultIndex + 1}/${resultCount}` : `${resultCount}개`;
  $count.classList.toggle('none', !!$searchInput.value && !resultCount);
}

/** 터미널 화면에서 찾는다. `typing`이면(글자를 치는 중) 지금 찾은 자리부터 다시 맞춰 본다(다음 것으로 넘어가지 않는다). */
function find(backward, typing = false) {
  const query = $searchInput.value;
  if (!isTerm(active)) return;
  if (!query) {
    active.search.clearDecorations();
    showSearchCount();
    return setStatus('');
  }
  const options = {
    decorations: currentLook().searchColors, caseSensitive: !!searchOptions.caseSensitive, regex: !!searchOptions.regex, incremental: typing,
  };
  if (options.regex) {
    // 쓰다 만 정규식("(" 등)은 찾기에 넘기지 않는다(넘기면 출력이 올 때마다 오류가 난다).
    try { new RegExp(query); } catch {
      active.search.clearDecorations();
      showSearchCount({ resultCount: 0 });
      return setStatus(`정규식이 올바르지 않습니다: ${query}`);
    }
  }
  const found = backward ? active.search.findPrevious(query, options) : active.search.findNext(query, options);
  if (!found) showSearchCount({ resultCount: 0 });
  setStatus(found ? '' : `찾는 내용이 없습니다: ${query}`);
}

// ───────────────────────── 링크 ─────────────────────────

/** 클릭된 링크를 연다. 창을 고르려던 클릭과, Ctrl이 필요한 설정에서의 맨 클릭은 무시한다. */
function openLink(s, span, ev) {
  if (performance.now() - s.focusedAt < FOCUS_GUARD_MS) return;
  if (config.link.require_ctrl && !ev?.ctrlKey) return;
  const openFile = config.link.require_ctrl ? !!ev?.shiftKey : !!ev?.ctrlKey;
  invoke('link_open', {
    target: span.target, cwd: s.cwd, openFile, line: span.line ?? null, col: span.col ?? null,
  }).catch((err) => setStatus(String(err)));
}

/**
 * 화면 줄 y가 속한 "논리적인 한 줄"(터미널이 자동으로 접은 줄을 이은 것)의 글자와,
 * 글자 위치(UTF-16) → 화면 칸 대응표를 만든다. 한글처럼 두 칸짜리 글자 때문에 필요하다.
 */
function readLogicalLine(term, y) {
  const buffer = term.buffer.active;
  let top = y - 1;
  while (top > 0 && buffer.getLine(top)?.isWrapped && y - 1 - top < 20) top--;
  let text = '';
  const first = [], last = [];
  for (let row = top; row < buffer.length && row - top < 40; row++) {
    const line = buffer.getLine(row);
    if (!line || (row > top && !line.isWrapped)) break;
    for (let x = 0; x < line.length; x++) {
      const cell = line.getCell(x);
      const width = cell?.getWidth() ?? 0;
      if (!width) continue; // 두 칸 글자의 뒤쪽 칸
      const chars = cell.getChars() || ' ';
      for (let i = 0; i < chars.length; i++) {
        first.push({ x: x + 1, y: row + 1 });
        last.push({ x: x + width, y: row + 1 });
      }
      text += chars;
    }
  }
  return { text, first, last };
}

const detectCache = new Map(); // "폴더\n줄 내용" → { at, promise }

function detect(text, cwd) {
  const cacheKey = `${cwd}\n${text}`;
  const hit = detectCache.get(cacheKey);
  const now = performance.now();
  if (hit && now - hit.at < 3000) return hit.promise;
  if (detectCache.size > 500) detectCache.clear();
  const promise = invoke('link_detect', { text, cwd }).catch(() => []);
  detectCache.set(cacheKey, { at: now, promise });
  return promise;
}

function hint(span) {
  const click = config.link.require_ctrl ? 'Ctrl+클릭' : '클릭';
  const open = config.link.require_ctrl ? 'Ctrl+Shift+클릭' : 'Ctrl+클릭';
  if (span.kind === 'url') return `${click}: 브라우저에서 열기 — ${span.target}`;
  if (span.kind === 'dir') return `${click}: 탐색기에서 열기 — ${span.target}`;
  return `${click}: 탐색기에서 보기 · ${open}: 열기 — ${span.target}`;
}

async function provideLinks(s, y, callback) {
  const row = readLogicalLine(s.term, y);
  if (!row.text.trim()) return callback(undefined);
  const spans = await detect(row.text.trimEnd(), s.cwd);
  const links = spans
    .filter((sp) => sp.end > sp.start && sp.end <= row.first.length)
    .map((sp) => ({
      range: { start: row.first[sp.start], end: row.last[sp.end - 1] },
      text: row.text.slice(sp.start, sp.end),
      activate: (ev) => openLink(s, sp, ev),
      hover: () => setStatus(hint(sp)),
      leave: () => setStatus(''),
    }))
    .filter((link) => link.range.start.y <= y && y <= link.range.end.y);
  callback(links.length ? links : undefined);
}

// ───────────────────────── 앱 단축키 ─────────────────────────

window.addEventListener('keydown', (ev) => {
  if ($('ask').open || $('popup').open) return;
  const key = ev.key.toLowerCase();
  const combo = ev.ctrlKey && ev.shiftKey && !ev.altKey;
  let handled = true;
  if (combo && key === 't') newSession(lastProfile);
  else if (combo && key === 'd') newSession(lastProfile, { show: 'split' });
  else if (combo && key === 'w') { if (active) closeSession(active); }
  else if (combo && key === 'x') { if (active) toggleShown(active); }
  else if (combo && key === 'f') openSearch();
  else if (combo && key === 'b') setBroadcast(!broadcast);
  else if (combo && key === 'n') nextWaiting();
  else if (combo && key === 'g') toggleDiff();
  else if (combo && key === 'p') showSnippets();
  else if (combo && key === 'o') { if (chromeEngine()) openChrome(); else newItem('site'); }
  else if (combo && key === 'h') newItem('http');
  else if (combo && key === 'm') newItem('memo');
  else if (combo && key === 'e') focusComposer();
  else if (combo && key === 'k') showPalette();
  else if (combo && key === 'r') reopenLast();
  else if (key === 'f1') showHelp();
  else if (combo && key === 'arrowright') focusPane(1);
  else if (combo && key === 'arrowleft') focusPane(-1);
  else if (ev.ctrlKey && key === 'tab') cycle(ev.shiftKey ? -1 : 1);
  // 터미널 밖에서 새로 고침이 눌리면 세션 화면이 날아가므로 막는다.
  // (메모에서는 F5가 시간/날짜, Ctrl+R이 바꾸기라서 메모가 직접 처리한다.)
  else if ((key === 'f5' || (ev.ctrlKey && key === 'r')) && !ev.target.closest?.('.term, .memo')) { /* 막기만 한다 */ }
  else handled = false;
  if (handled) { ev.preventDefault(); ev.stopPropagation(); }
}, true);

document.addEventListener('contextmenu', (ev) => ev.preventDefault());

// 자동 검증과 문제 확인용 창구.
window.cypher = {
  sessions, newSession, newItem, activate, closeSession, toggleShown, send, setBroadcast, provideLinks, paste, moveToGroup, runMenu, showOnly,
  onHook, onStatus, nextWaiting, writeState, rename, sendPrompt, text: sessionText, addImages, onCaptured, captureBrowser,
  setLook, currentLook, mcpAction, handoffSite, onMcpRequest,
  showPalette, paletteEntries, showClosed, reopenLast, restartSession, placeInOrder, inSidebarOrder, cycle, find, recallPrompt, useFolder, folderChoices,
  get visible() { return visible; },
  get active() { return active; },
};

init();
