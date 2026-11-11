// Cypher 화면: 세션 목록, 나란히 보는 터미널 창, 링크·붙여넣기·알림.
// 빌드 단계 없이 그대로 실행 파일에 들어간다.
import {
  initExtras, showSnippets, showHelp, showMemory, showMenu, toggleDiff, showDiff, diffRefreshFor,
} from './extras.js';
import { initHttp, toggleHttp, showHttp } from './http.js';

const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// 화면 코드에서 난 오류를 모아 둔다(문제 확인용).
window.cypherErrors = [];
window.addEventListener('error', (ev) => window.cypherErrors.push(String(ev.message)));
window.addEventListener('unhandledrejection', (ev) => window.cypherErrors.push(String(ev.reason)));

const $ = (id) => document.getElementById(id);
const $terms = $('terms'), $list = $('sessions'), $folder = $('folder');
const $search = $('search'), $searchInput = $('search-input');

const THEME = {
  background: '#16181d', foreground: '#d7dae0', cursor: '#d97757',
  selectionBackground: '#3e4451',
};
const SEARCH_COLORS = {
  matchBackground: '#5c4a1f', activeMatchBackground: '#d97757',
  matchOverviewRuler: '#5c4a1f', activeMatchColorOverviewRuler: '#d97757',
};
const BUSY_MS = 1200;           // 이 시간 안에 출력이 있었으면 "작업 중"으로 본다.
const MAX_PANES = 8;            // 한 화면에 나눠 볼 수 있는 창 수
const MAX_SHOTS = 8;            // 세션마다 남길 이미지 미리보기 수
const FLOW_HIGH = 2 << 20;      // 화면에 못 그린 출력이 이만큼 쌓이면 읽기를 멈춘다.
const FLOW_LOW = 256 << 10;     // 이만큼으로 줄면 다시 읽는다.
const FOCUS_GUARD_MS = 250;     // 창을 고르려던 클릭이 링크를 열지 않게 하는 시간
const CONTEXT_WARN = 80;        // 컨텍스트 사용률이 이 이상이면 노란색,
const CONTEXT_DANGER = 90;      // 이 이상이면 빨간색으로 경고한다.
const COLORS = ['', '#e06c75', '#e5c07b', '#98c379', '#61afef', '#c678dd', '#56b6c2'];

/** @type {Map<number, object>} 화면용 번호(key) → 세션 */
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
try { collapsed = new Set(JSON.parse(localStorage.getItem('cypher.collapsed') || '[]')); } catch { /* 무시 */ }
let groupList = [];            // 만들어 둔 그룹 이름(세션이 없어도 남는다)
try { groupList = JSON.parse(localStorage.getItem('cypher.groups') || '[]'); } catch { /* 무시 */ }

// ───────────────────────── 시작 ─────────────────────────

async function init() {
  const loaded = await invoke('app_config');
  config = loaded.config;
  if (loaded.error) setStatus(`설정 파일 오류(기본값으로 동작): ${loaded.error}`);
  folder = localStorage.getItem('cypher.folder') || await invoke('home_dir');
  showFolder();

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
  // 새 그룹 만들기
  const groupButton = make('button', '', '+ 그룹');
  groupButton.title = '이름 붙인 새 그룹 만들기. 세션을 끌어다 놓거나 그룹 머리의 +로 세션을 엽니다';
  groupButton.onclick = newGroup;
  $('new-buttons').append(groupButton);

  initExtras({
    invoke, make, ask, setStatus, newSession, label,
    getActive: () => active,
    getFolder: () => folder,
    pasteText: (text) => active?.term.paste(text),
  });
  initHttp({
    invoke, make, ask, setStatus, baseName,
    getActive: () => active,
    getFolder: () => folder,
  });
  // 사이드바 아래에는 자주 쓰는 네 가지만 둔다. 나머지는 창 상단 메뉴에 있다.
  $('waiting').onclick = () => { filterWaiting = !filterWaiting; render(); };
  $('changes').onclick = () => toggleDiff();
  $('browser').onclick = openBrowser;
  $('http-button').onclick = () => toggleHttp();
  listen('browser-handoff', ({ payload }) => handoffFromBrowser(payload));
  listen('menu', ({ payload }) => runMenu(payload));

  $('pick').onclick = async () => {
    const picked = await invoke('pick_folder', { start: folder });
    if (picked) { folder = picked; localStorage.setItem('cypher.folder', folder); showFolder(); }
  };
  $('search-prev').onclick = () => find(true);
  $('search-next').onclick = () => find(false);
  $('search-close').onclick = closeSearch;
  $searchInput.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') find(ev.shiftKey);
    else if (ev.key === 'Escape') closeSearch();
  });

  listen('session-exit', ({ payload }) => onExit(payload));
  listen('session-status', ({ payload }) => onStatus(payload));
  listen('session-hook', ({ payload }) => onHook(payload));
  listen('system-usage', ({ payload }) => onUsage(payload));
  listen('tauri://drag-drop', ({ payload }) => {
    // 파일을 끌어다 놓으면 경로를 입력한다.
    if (active && payload.paths?.length) active.term.paste(payload.paths.map(quote).join(' '));
  });

  new ResizeObserver(fitVisible).observe($terms);
  setInterval(refreshActivity, 400);
  window.addEventListener('beforeunload', () => { writeState(); closing = true; });

  const auto = await invoke('autostart');
  if (auto) { folder = auto[1]; showFolder(); await newSession(auto[0]); }
  else await restore();
}

/** 창 상단 메뉴(도구·설정·도움말)에서 고른 항목을 실행한다. 항목 번호는 앱 쪽 메뉴 정의와 맞춘다. */
function runMenu(id) {
  const actions = {
    snippets: showSnippets,
    broadcast: () => setBroadcast(!broadcast),
    memory: () => showMemory(),
    settings: () => {
      invoke('open_config').catch((err) => setStatus(String(err)));
      setStatus('설정은 저장한 뒤 Cypher를 다시 시작하면 적용됩니다.');
    },
    // Claude Code의 사용자 공통 파일(모든 폴더에 적용되는 설정과 지침)
    'claude-settings': () => openClaudeFile('settings'),
    'claude-memory': () => openClaudeFile('memory'),
    help: showHelp,
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

const quote = (p) => (p.includes(' ') ? `"${p}"` : p);
const baseName = (p) => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p;
const setStatus = (text) => { $('status-text').textContent = text; };
const label = (s) => s.name || `${s.profile.label} · ${s.worktree || baseName(s.cwd)}`;

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
      active?.term.focus();
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

  const term = new Terminal({
    fontFamily: config.font.family, fontSize: config.font.size, theme: THEME, cursorBlink: true,
    scrollback: config.session.scrollback, allowProposedApi: true,
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
    key: ++seq, id: null, profile, term, fitAddon, search, pane, head,
    startCwd: cwd, cwd, project: '', worktree: opts.worktree || '',
    name: opts.name || '', color: opts.color || '',
    // 이름 붙인 그룹. 비어 있으면 시작 폴더별로 묶인다. 새 세션은 지금 세션의 그룹을 따른다.
    group: opts.group ?? ((!opts.restoring && active?.group) || ''),
    title: '', model: '', effort: '', context: null, cost: 0,
    usage: '', usageEl: null,
    agents: new Set(), // 지금 돌고 있는 서브에이전트의 번호
    waiting: '', // '' | 'approval'(승인·입력 대기) | 'done'(응답 완료, 다음 지시 대기)
    claudeSession: opts.resume || '', resumable: !!opts.resume,
    lastOutput: 0, busy: false, attention: false, exited: false,
    pending: '', sending: false, queued: 0, paused: false,
    focusedAt: 0, shots: [],
  };
  sessions.set(s.key, s);
  ensureGroup(s.group);
  wireTerminal(s);

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
    flush(s); // 띄우는 동안 눌린 키를 보낸다.
  } catch (err) {
    term.write(`\x1b[31m세션을 시작하지 못했습니다: ${err}\x1b[0m\r\n`);
    s.exited = true;
  }
  render();
  saveState();
  return s;
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
  $('empty').hidden = sessions.size > 0;
  requestAnimationFrame(fitVisible);
}

function fitVisible() {
  for (const s of visible) {
    try { s.fitAddon.fit(); } catch { /* 아직 크기를 잴 수 없는 상태 */ }
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

function activate(s) {
  if (!visible.includes(s)) {
    placeInActive(s);
    layout();
  }
  active = s;
  s.attention = false;
  for (const other of sessions.values()) other.pane.classList.toggle('active', other === s);
  renderShots();
  if (!$search.hidden) closeSearch();
  showDiff(s);
  showHttp();
  s.term.focus();
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
  } else if (visible.length > 1) {
    tree = without(tree, s);
    layout();
    if (active === s) activate(visible[0]); else { render(); saveState(); }
  }
}

function closeSession(s) {
  if (s.id != null && !s.exited) invoke('session_close', { id: s.id });
  s.term.dispose();
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

function cycle(step) {
  const all = [...sessions.values()];
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
  active?.term.focus();
}

const byId = (id) => [...sessions.values()].find((s) => s.id === id);

function onExit({ id, code }) {
  const s = byId(id);
  if (!s) return;
  s.exited = true;
  s.agents.clear();
  s.term.write(`\r\n\x1b[90m[세션 종료${code == null ? '' : `: 코드 ${code}`}]\x1b[0m\r\n`);
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
function onHook({ id, event, kind, message, agent }) {
  const s = byId(id);
  if (!s) return;
  if (!s.resumable) { s.resumable = true; saveState(); } // 대화가 생겼으니 다음에 이어갈 수 있다.
  // 서브에이전트: 돌고 있는 것의 번호를 모아 두고 개수를 사이드바에 보여 준다.
  if (event === 'SubagentStart' || event === 'SubagentStop') {
    if (agent) {
      if (event === 'SubagentStart') s.agents.add(agent); else s.agents.delete(agent);
    }
    return render();
  }
  if (event !== 'UserPromptSubmit' && event !== 'Stop' && event !== 'Notification') return;
  if (event === 'UserPromptSubmit') { s.waiting = ''; return render(); }
  if (event === 'Notification' && kind === 'idle_prompt') return; // 완료 때 이미 알렸다.
  s.waiting = event === 'Stop' ? 'done' : 'approval';
  if (event === 'Stop') diffRefreshFor(s);
  render();
  const title = label(s);
  const body = event === 'Stop' ? '요청한 작업이 끝났습니다.' : (message || '입력을 기다리고 있습니다.');
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
    const busy = !s.exited && now - s.lastOutput < BUSY_MS;
    if (busy === s.busy) continue;
    if (!busy && s !== active) s.attention = true;
    s.busy = busy;
    changed = true;
  }
  if (changed) render();
}

const WAITING_TEXT = { approval: '승인·입력 대기', done: '완료 · 다음 지시 대기' };

/** 사이드바의 세션 한 줄. */
function sessionItem(s) {
  const li = make('li', [
    s === active && 'active', visible.includes(s) && 'shown', s.waiting,
    s.exited ? 'exited' : s.busy ? 'busy' : s.attention && 'attention',
  ].filter(Boolean).join(' '));
  li.title = '화면으로 끌어다 놓기: 놓은 위치대로 좌우·상하 분할 · 묶음 머리로 끌기: 그 묶음으로 옮기기 · 오른쪽 클릭: 이름·색·그룹';
  if (s.color) li.style.borderLeftColor = s.color;
  li.onclick = (ev) => (ev.ctrlKey ? toggleShown(s) : activate(s));
  li.onmousedown = (ev) => {
    if (ev.button === 0 && !ev.ctrlKey && !ev.target.closest('button')) drag = { s, x: ev.clientX, y: ev.clientY, active: false };
  };
  li.dataset.key = s.key;
  li.ondblclick = () => rename(s);
  li.oncontextmenu = (ev) => { ev.preventDefault(); sessionMenu(s, ev); };

  const name = make('span', 'name', label(s));
  name.title = s.cwd;
  if (s.color) name.style.color = s.color;
  const close = make('button', 'close', '×');
  close.title = '세션 닫기';
  close.onclick = (ev) => { ev.stopPropagation(); closeSession(s); };
  const context = s.context == null ? null : Math.round(s.context);
  const badge = make('span', 'badge', [
    s.model, s.effort, context == null ? '' : `컨텍스트 ${context}%`, s.cost ? `$${s.cost.toFixed(2)}` : '',
    s.agents.size ? `서브에이전트 ${s.agents.size}` : '',
  ].filter(Boolean).join(' · '));
  if (context >= CONTEXT_DANGER) badge.classList.add('danger');
  else if (context >= CONTEXT_WARN) badge.classList.add('warn');
  if (context >= CONTEXT_WARN) badge.title = '컨텍스트가 거의 찼습니다. /compact 또는 새 세션을 고려하세요.';
  const sub = make('span', 'sub', s.exited ? '종료됨' : WAITING_TEXT[s.waiting] || s.title);
  // 사용량은 3초마다 바뀌므로 목록을 다시 그리지 않고 이 칸만 고친다.
  s.usageEl = make('span', 'usage', s.exited ? '' : s.usage);
  li.append(make('span', 'dot'), name, close, badge, sub, s.usageEl);
  return li;
}

const gigabytes = (bytes) => (bytes / 2 ** 30).toFixed(1);

/** 시스템 전체 사용량은 창 아래에, 세션별 사용량은 사이드바에 보여 준다. */
function onUsage({ cpu, memory_used: used, memory_total: total, sessions: perSession }) {
  const memoryPct = total ? Math.round((used / total) * 100) : 0;
  const $usage = $('status-usage');
  $usage.textContent = `CPU ${Math.round(cpu)}% · 메모리 ${gigabytes(used)}/${gigabytes(total)}GB (${memoryPct}%)`;
  $usage.className = Math.max(cpu, memoryPct) >= 90 ? 'danger' : '';
  for (const item of perSession) {
    const s = byId(item.id);
    if (!s) continue;
    const memory = item.memory >= 2 ** 30 ? `${gigabytes(item.memory)}GB` : `${Math.round(item.memory / 2 ** 20)}MB`;
    s.usage = `CPU ${item.cpu < 10 ? item.cpu.toFixed(1) : Math.round(item.cpu)}% · ${memory}`;
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

function groupMenu(group, ev) {
  const live = group.list.filter((s) => sessions.has(s.key));
  const items = [];
  if (group.logical) {
    items.push({ label: '이 그룹에 새 세션', action: () => newSession(lastProfile, { group: group.name }) });
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
  for (const head of $list.querySelectorAll('li.group.drop')) head.classList.remove('drop');
  const point = document.elementFromPoint(ev.clientX, ev.clientY);
  point?.closest?.('li.group[data-drop]')?.classList.add('drop');
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
  // 터미널 화면에 놓음: 놓은 위치에 따라 그 창을 나누거나 그 자리에 연다.
  const hit = paneZoneAt(ev.clientX, ev.clientY);
  if (hit) dropOnPane(s, hit.target, hit.zone);
  else if (point?.closest?.('#terms')) activate(s);
  render();
});

function render() {
  // 끄는 중에 목록을 다시 그리면 놓을 자리가 사라지므로 미뤄 둔다.
  if (drag?.active && $list.querySelector('li.group.new')) return;

  const groups = new Map();
  // 만들어 둔 그룹은 비어 있어도 위쪽에 보여 준다(대기만 보기에서는 뺀다).
  if (!filterWaiting) {
    for (const name of groupList) groups.set(`g:${name}`, { key: `g:${name}`, logical: true, name, hint: `그룹: ${name}`, list: [] });
  }
  for (const s of sessions.values()) {
    if (filterWaiting && !s.waiting) continue;
    const key = groupKey(s);
    if (!groups.has(key)) {
      groups.set(key, { key, logical: !!s.group, name: s.group || baseName(s.startCwd), hint: s.group ? `그룹: ${s.group}` : s.startCwd, list: [] });
    }
    groups.get(key).list.push(s);
  }
  const rows = [];
  for (const group of groups.values()) {
    const folded = collapsed.has(group.key);
    const head = make('li', [
      'group', group.logical && 'logical', group.list.some((s) => s.attention || s.waiting) && 'attention',
    ].filter(Boolean).join(' '));
    head.title = `${group.hint}\n클릭: 접기/펼치기 · 오른쪽 클릭: 묶음 메뉴 · 세션을 끌어다 놓으면 이 묶음으로`;
    head.dataset.drop = group.logical ? 'logical' : 'folder';
    head.dataset.name = group.name;
    head.append(
      make('span', '', folded ? '▸' : '▾'),
      make('span', 'name', group.logical ? `◆ ${group.name}` : group.name),
    );
    if (group.logical) {
      const add = make('button', 'close', '+');
      add.title = '이 그룹에 새 세션 열기';
      add.onclick = (ev) => { ev.stopPropagation(); newSession(lastProfile, { group: group.name }); };
      head.append(add);
    }
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
  if (filterWaiting && !rows.length) rows.push(make('li', 'group', '대기 중인 세션이 없습니다.'));
  if (drag?.active) {
    const fresh = make('li', 'group new', '+ 새 그룹으로 (여기에 놓기)');
    fresh.dataset.drop = 'new';
    rows.push(fresh);
  }
  $list.replaceChildren(...rows);

  // 창 아래 상태줄에도 지금 세션의 모델·effort를 항상 보여 준다.
  $('status-model').textContent = active?.model
    ? [active.model, active.effort, active.context == null ? '' : `컨텍스트 ${Math.round(active.context)}%`,
      active.agents.size ? `서브에이전트 ${active.agents.size}` : ''].filter(Boolean).join(' · ')
    : '';

  const waitingCount = [...sessions.values()].filter((s) => s.waiting).length;
  const $waiting = $('waiting');
  $waiting.textContent = `대기 ${waitingCount}`;
  $waiting.classList.toggle('has', waitingCount > 0);
  $waiting.classList.toggle('on', filterWaiting);

  // 나란히 볼 때 창마다 붙는 제목 줄
  for (const s of visible) {
    const hide = make('button', 'close', '×');
    hide.title = '나란히 보기에서 빼기 (세션은 유지)';
    hide.onclick = (ev) => { ev.stopPropagation(); toggleShown(s); };
    s.head.replaceChildren(
      make('span', 'title', label(s)),
      make('span', 'model', [s.model, s.effort].filter(Boolean).join(' · ')),
      hide,
    );
  }
}

/** 창 아래에 5시간·주간 사용 한도를 항상 보여 준다. */
function renderLimits() {
  const $limits = $('status-limits');
  const when = (epoch) => {
    if (!epoch) return '';
    const d = new Date(epoch * 1000);
    const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
    const sameDay = d.toDateString() === new Date().toDateString();
    return ` (${sameDay ? '' : `${d.getMonth() + 1}/${d.getDate()} `}${time} 초기화)`;
  };
  const parts = [['five_hour', '5시간'], ['seven_day', '주간']]
    .filter(([key]) => limits?.[key]?.used_percentage != null)
    .map(([key, name]) => ({ pct: limits[key].used_percentage, text: `${name} ${Math.round(limits[key].used_percentage)}%${when(limits[key].resets_at)}` }));
  $limits.textContent = parts.length ? `사용 한도: ${parts.map((p) => p.text).join(' · ')}` : '';
  const worst = Math.max(0, ...parts.map((p) => p.pct));
  $limits.className = worst >= 95 ? 'danger' : worst >= 80 ? 'warn' : '';
}

// ───────────────────────── 세션 꾸미기·내보내기 ─────────────────────────

async function rename(s) {
  const name = await ask('세션 이름 (비우면 기본 이름)', '예: 로그인 수정', s.name);
  if (name == null) return;
  s.name = name;
  render();
  saveState();
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
  showMenu(ev, [
    { label: '이름 바꾸기', action: () => rename(s) },
    { colors: COLORS, action: (color) => { s.color = color; render(); saveState(); } },
    { label: s.group ? `그룹 바꾸기… (지금: ${s.group})` : '그룹 지정…', action: () => askGroup([s], s.group) },
    { label: visible.includes(s) ? '나란히 보기에서 빼기' : '나란히 보기에 넣기', action: () => toggleShown(s) },
    { label: '변경 내용 보기', action: () => { activate(s); toggleDiff(true); } },
    { label: '출력 저장…', action: () => exportText(s) },
    { label: '기억 내보내기·가져오기', action: () => showMemory(s) },
    { label: '세션 닫기', action: () => closeSession(s) },
  ]);
}

// ───────────────────────── 내장 브라우저 ─────────────────────────

/** 브라우저 창을 연다. 실패하면 한 번 더 해 보고, 그래도 안 되면 눈에 띄게 알린다. */
async function openBrowser() {
  const slow = setTimeout(() => notice('브라우저', '창을 여는 데 시간이 걸리고 있습니다…'), 8000);
  try {
    await invoke('browser_open', { url: null });
  } catch {
    try {
      await new Promise((resolve) => setTimeout(resolve, 700));
      await invoke('browser_open', { url: null });
    } catch (err) {
      notice('브라우저를 열지 못했습니다', `${err} (다시 눌러 보세요)`);
    }
  } finally {
    clearTimeout(slow);
  }
}

/**
 * 브라우저 창에서 "Claude에게 넘기기"를 눌렀을 때: 활성 Claude 세션 입력창에 머리말을 넣는다.
 * 사용자가 이어서 할 일을 적고 Enter를 누르면 Claude가 브라우저 도구로 그 페이지를 조작한다.
 */
async function handoffFromBrowser({ url, fresh }) {
  const current = active?.profile.claude && !active.exited
    ? active
    : [...sessions.values()].find((s) => s.profile.claude && !s.exited);
  let target = fresh ? null : current;
  if (!target) {
    // 새 세션으로 넘기기(또는 열린 Claude 세션이 없을 때): 지금 세션과 같은 폴더·그룹에 새로 연다.
    const base = current || active;
    target = await newSession('claude', {
      cwd: base ? (base.project || base.startCwd) : folder, group: base?.group, show: 'split',
    });
    if (!target || target.exited) return notice('브라우저', '새 Claude 세션을 열지 못했습니다.');
    // Claude Code가 입력을 받을 준비가 될 때까지(상태줄이 오면 준비된 것) 기다린다.
    for (let waited = 0; waited < 20000 && !target.model && !target.exited; waited += 300) {
      await new Promise((resolve) => setTimeout(resolve, 300));
    }
  }
  activate(target);
  target.term.paste(`Cypher 브라우저에 열려 있는 페이지(${url})에서 이어서 작업해줘: `);
}

/** 대기 중인 다음 세션으로 간다. 승인 대기가 있으면 그쪽을 먼저 돈다. */
function nextWaiting() {
  const all = [...sessions.values()];
  const approvals = all.filter((s) => s.waiting === 'approval');
  const pool = approvals.length ? approvals : all.filter((s) => s.waiting);
  if (!pool.length) return setStatus('대기 중인 세션이 없습니다.');
  const from = all.indexOf(active);
  const next = pool.find((s) => all.indexOf(s) > from) || pool[0];
  activate(next);
}

// ───────────────────────── 배치 기억 ─────────────────────────

let saveTimer = 0;

function saveState() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(writeState, 300);
}

/** 열린 세션과 배치를 적어 둔다. 다시 시작하면 restore()가 되살린다. */
function writeState() {
  if (closing) return; // 창이 닫히는 중의 종료 알림으로 목록이 비워지지 않게 한다.
  const live = [...sessions.values()].filter((s) => !s.exited && s.id != null);
  const state = {
    sessions: live.map((s) => ({
      profile: s.profile.id,
      cwd: s.project || s.startCwd,
      resume: s.resumable && s.claudeSession ? s.claudeSession : null,
      shown: visible.includes(s),
      name: s.name,
      group: s.group,
      color: s.color,
    })),
    active: live.indexOf(active),
    // 화면 분할 모양. 잎은 세션의 순번({ i }), 가지는 { dir, a, b }.
    layout: (function pack(node) {
      if (!node) return null;
      return node.s ? { i: live.indexOf(node.s) } : { dir: node.dir, ratio: ratioOf(node), a: pack(node.a), b: pack(node.b) };
    })(tree),
  };
  try { localStorage.setItem('cypher.state', JSON.stringify(state)); } catch { /* 저장 공간 문제는 무시 */ }
}

/** 저장해 둔 분할 모양을 되살린다. 없어진 세션의 자리는 짝이 차지한다. */
function unpack(data, list) {
  if (!data) return null;
  if ('i' in data) return list[data.i] ? { s: list[data.i] } : null;
  const a = unpack(data.a, list), b = unpack(data.b, list);
  return a && b ? { dir: data.dir === 'col' ? 'col' : 'row', ratio: data.ratio, a, b } : (a || b);
}

async function restore() {
  if (!config.session.restore) return;
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem('cypher.state') || 'null'); } catch { /* 깨진 기록은 버린다 */ }
  if (!saved?.sessions?.length) return;
  const restored = [];
  for (const item of saved.sessions) {
    restored.push(await newSession(item.profile, {
      cwd: item.cwd, resume: item.resume, show: 'hidden', restoring: true,
      name: item.name, color: item.color, group: item.group || '',
    }));
  }
  // 분할 모양을 되살린다. 저장된 모양이 없으면(예전 기록) 보이던 세션을 고르게 나눈다.
  tree = unpack(saved.layout, restored)
    || balanced(restored.filter((s, index) => s && saved.sessions[index].shown).slice(0, MAX_PANES));
  layout();
  const target = restored[saved.active] || restored.find(Boolean);
  if (target) activate(target);
}

// ───────────────────────── 입력 ─────────────────────────

/** 키 입력을 순서대로 보낸다. 동시 입력이 켜져 있으면 나란히 보이는 모든 세션에 보낸다. */
function send(s, data) {
  const targets = broadcast && visible.includes(s) ? visible.filter((v) => !v.exited) : [s];
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
  // 나란히 보기에서 다른 창을 누르면 그 창이 활성화된다.
  s.pane.addEventListener('mousedown', () => { if (active !== s) activate(s); }, true);

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
  $search.hidden = false;
  $searchInput.focus();
  $searchInput.select();
}

function closeSearch() {
  $search.hidden = true;
  active?.search.clearDecorations();
  active?.term.focus();
}

function find(backward) {
  const query = $searchInput.value;
  if (!query || !active) return;
  const options = { decorations: SEARCH_COLORS };
  const found = backward ? active.search.findPrevious(query, options) : active.search.findNext(query, options);
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
  else if (combo && key === 'o') openBrowser();
  else if (combo && key === 'h') toggleHttp();
  else if (key === 'f1') showHelp();
  else if (combo && key === 'arrowright') focusPane(1);
  else if (combo && key === 'arrowleft') focusPane(-1);
  else if (ev.ctrlKey && key === 'tab') cycle(ev.shiftKey ? -1 : 1);
  // 터미널 밖에서 새로 고침이 눌리면 세션 화면이 날아가므로 막는다.
  else if ((key === 'f5' || (ev.ctrlKey && key === 'r')) && !ev.target.closest?.('.term')) { /* 막기만 한다 */ }
  else handled = false;
  if (handled) { ev.preventDefault(); ev.stopPropagation(); }
}, true);

document.addEventListener('contextmenu', (ev) => ev.preventDefault());

// 자동 검증과 문제 확인용 창구.
window.cypher = {
  sessions, newSession, activate, closeSession, toggleShown, send, setBroadcast, provideLinks, paste, moveToGroup, runMenu,
  onHook, onStatus, nextWaiting, writeState, text: sessionText,
  get visible() { return visible; },
  get active() { return active; },
};

init();
