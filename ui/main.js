// Cypher 화면: 세션 목록, 나란히 보는 터미널 창, 링크·붙여넣기·알림.
// 빌드 단계 없이 그대로 실행 파일에 들어간다.
import {
  initExtras, showSnippets, showHelp, showEngine, showUpdate, showAbout, showMemory, showMenu, toggleDiff, showDiff, diffRefreshFor, openPopup,
} from './extras.js';
import { initHttp, createHttp } from './http.js';
import { initMemo, createMemo } from './memo.js';
import { initSite, createSite, syncSites } from './site.js';
import { initImage, createImage } from './image.js';
import { initDrop } from './drop.js';
import { initMindmap, createMindmap } from './mindmap.js';
import { initErd, createErd } from './erd.js';
import { initLook, showLook, setLook, currentLook } from './look.js';
import { initPalette, showPalette } from './palette.js';
import { initClosed, rememberClosed, reopenLast, showClosed, closedCount } from './closed.js';
import { initReader, createReader } from './reader.js';
import { initNotices, recordNotice, markRead, showNotices, unreadCount } from './notices.js';
import { initTemplate, saveTemplate, showTemplates } from './template.js';
import { initTodo, createTodo } from './todo.js';
import { initRun, createRun } from './run.js';
import { initTable, createTable } from './table.js';
import { setPty, windowsPty, matchWidths } from './widths.js';
import { initSsh, showSsh, cleanSpec, specLabel, saveHost, savedHosts } from './ssh.js';
import { initTimer, showTimers, nextTimer, addTimer, removeTimer, timers } from './timer.js';
import { initCode, createCode } from './code.js';
import { initHex, createHex } from './hex.js';
import { initHistory, keepHistory, dropHistory, showHistory } from './history.js';
import { initS3, createS3 } from './s3.js';

const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// 화면 코드에서 난 오류를 모아 둔다(문제 확인용).
window.cypherErrors = [];
window.addEventListener('error', (ev) => window.cypherErrors.push(String(ev.message)));
window.addEventListener('unhandledrejection', (ev) => window.cypherErrors.push(String(ev.reason)));

const $ = (id) => document.getElementById(id);
const $terms = $('terms'), $list = $('sessions');
const $search = $('search'), $searchInput = $('search-input');

const BUSY_MS = 1200;           // 이 시간 안에 출력이 있었으면 "작업 중"으로 본다.
const HELD_STOP_MS = 30000;      // 서브에이전트 때문에 미룬 완료 알림을, 이만큼 조용하면 그냥 알린다.
const INTERRUPT_MS = 4000;       // Esc·Ctrl+C를 누르고 이만큼 지난 뒤에 조용하면 요청이 중단된 것으로 본다.
const INTERRUPT_QUIET_MS = 3000; // 그때 "조용하다"고 보는 시간
const STALLED_MS = 30000;        // 일하는 중이라는데 이만큼 아무 출력이 없으면 완료 신호 없이 끝난 것으로 본다.
const PRESS_HOLD_MS = 1500;      // 사이드바를 누르고 있는 동안 목록 다시 그리기를 미루는 최대 시간
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
  memo: { label: '메모', icon: '✎', hint: '글자 편집기', create: createMemo },
  http: { label: 'HTTP', icon: '⇄', hint: 'HTTP 요청(.http) 쓰고 보내기', create: createHttp },
  site: { label: '사이트', icon: '◎', hint: '브라우저 · 같은 워크스페이스의 Claude가 조작 가능', create: createSite },
  image: { label: '이미지', icon: '▣', hint: '그림 보기(파일·붙여넣기·화면 캡처)', create: createImage },
  mindmap: { label: '마인드맵', icon: '✺', hint: '생각을 가지로 정리 · 그림이나 코드로 작성', create: createMindmap },
  erd: { label: 'ERD', icon: '⊞', hint: '개체 관계도 · erDiagram 글을 그림으로', create: createErd },
  // 문서·명령·표는 볼 파일이나 실행할 명령을 먼저 묻지 않는다. 빈 항목으로 만들고, 대상은 그 항목의 빈 영역에서 정한다
  // (파일을 끌어다 놓기, 고르기, 경로 적기 — empty.js).
  reader: { label: '문서', icon: '❏', hint: 'Markdown·글자 파일 읽기 · 바뀌면 자동 갱신', create: createReader },
  todo: { label: '할 일', icon: '☑', hint: '업무의 체크 목록 · 같은 워크스페이스의 Claude가 채우고 체크', create: createTodo },
  run: { label: '명령', icon: '▷', hint: '자주 돌리는 명령(시험·빌드·개발 서버)을 단추로 두고 실행 · 성공·실패 표시', create: createRun },
  table: { label: '표', icon: '▤', hint: 'CSV·TSV·JSON 파일을 표로 보기 · 정렬·거르기 · 바뀌면 자동 갱신', create: createTable },
  // 코드 보기는 프로젝트 폴더 하나를 통째로 본다(파일마다 항목이 생기지 않는다). 고치지는 않고, 작업한 내용을 읽는 데 쓴다.
  code: { label: '코드', icon: '❮❯', hint: '프로젝트의 코드 보기 · 문법 강조 · 정의로 가기 · 쓰는 곳 찾기 · 바뀐 파일과 줄 표시', create: createCode },
  // 헥사 에디터: 왼쪽 절반은 바이트, 오른쪽 절반은 파일 형식(매직 넘버)·구조·값 해석. 덮어쓰기로 고쳐 저장한다.
  hex: { label: '헥사', icon: '⬡', hint: '바이트 보기·고치기 · 매직 넘버로 형식을 알아보고 구조(머리·덩어리)를 해석', create: createHex },
  // S3 호환 저장소 탐색기: 워크스페이스에 몇 개든 둘 수 있다. 접속 정보(주소·키)는 앱이 암호화해 저장하고,
  // 접속은 사용자가 [접속]을 눌렀을 때만 한다(다시 켠 뒤에는 접속하지 않은 채로 되살아난다).
  s3: { label: 'S3 저장소', icon: '⛁', hint: 'S3 호환 저장소(Ncloud Storage 등)를 탐색기처럼 보기 · 올리기·내려받기·복사·이름 바꾸기·삭제 · 글은 바로 보기', create: createS3 },
};

/**
 * 그 종류의 항목을 새로 만든다. 아무것도 묻지 않고 바로 만든다: 볼 파일이나 실행할 명령이 필요한 종류(문서·명령·표)는
 * 빈 영역으로 뜨고, 사용자가 거기서 파일을 끌어다 놓거나 골라 정한다.
 */
async function createKind(kind, opts = {}) {
  if (!KINDS[kind]) return null;
  const group = groupFor(opts);
  if (!group) return null;
  // 코드 보기는 볼 폴더를 먼저 고른다: 그 폴더 아래의 파일을 색인하므로, 너무 넓은 폴더(사용자 폴더 등)를 열면 느려진다.
  // 고르기 창은 워크스페이스 폴더에서 시작한다. 취소하면 만들지 않는다.
  if (kind === 'code' && opts.saved === undefined) {
    const picked = await invoke('pick_folder', { start: groupFolderOf(group) || startFolderHint(), title: '코드 보기로 열 폴더 (이 폴더 아래의 파일을 색인합니다)' }).catch(() => null);
    // 고르는 사이에 그 워크스페이스를 지우거나 이름을 바꿨으면 만들지 않는다(만들면 옛 이름의 워크스페이스가 되살아난다).
    if (!picked || !groupList.includes(group)) return null;
    return newItem(kind, { ...opts, group, cwd: picked, saved: { root: picked } });
  }
  return newItem(kind, { ...opts, group });
}

/** @type {Map<number, object>} 화면용 번호(key) → 항목(터미널 세션, 메모, HTTP …) */
const sessions = new Map();
let visible = [];               // 나란히 보이는 세션(왼쪽 위부터 순서대로)
let active = null;
let seq = 0;
let config = null;
/** 워크트리 세션을 여는 길(사이드바 단추, 사이드바 메뉴, 빠른 이동의 명령)을 화면에 내놓을지. 잘 쓰이지 않아 숨겨 둔다. 기능(newWorktree)은 그대로 있다. */
const SHOW_WORKTREE = false;
/** 워크트리 세션을 열 수 있는지: 내놓기로 했고 Claude 프로필이 있을 때. */
const offersWorktree = () => SHOW_WORKTREE && config.profiles.some((p) => p.id === 'claude');
let folder = '';
let lastProfile = 'claude';
let broadcast = false;          // 동시 입력: false(꺼짐) | 'all'(나란히 보이는 모든 세션) | 'ssh'(나란히 보이는 SSH 세션끼리)
let closing = false;
let limits = null;              // 계정 사용 한도(어느 Claude 세션이든 마지막으로 알려 준 값)
let filterWaiting = false;      // 대기 중인 세션만 목록에 보이기
let collapsed = new Set();      // 접어 둔 묶음(워크스페이스·폴더)의 열쇠
let drag = null;                // 목록에서 끌고 있는 세션 { s, x, y, active, ghost }
let restoring = false;          // 지난번 항목을 되살리는 중(이 동안에는 상태를 적지 않는다)
let renaming = null;            // 사이드바에서 별칭을 고치고 있는 항목의 번호
let composerFor = null;         // 프롬프트 편집기가 지금 맡고 있는 세션
try { collapsed = new Set(JSON.parse(localStorage.getItem('cypher.collapsed') || '[]')); } catch { /* 무시 */ }
let storeDirs = {};            // 메모·마인드맵·이미지를 두는 폴더(같은 워크스페이스의 Claude 세션에 경로를 알려 줄 때 쓴다)
let groupList = [];            // 만들어 둔 워크스페이스 이름(세션이 없어도 남는다)
try { groupList = JSON.parse(localStorage.getItem('cypher.groups') || '[]'); } catch { /* 무시 */ }
groupList = Array.isArray(groupList) ? groupList.filter((g) => typeof g === 'string' && g) : [];
// 워크스페이스 이름 → 그 워크스페이스가 쓰는 브라우저 프로필의 번호. 워크스페이스 이름을 바꿔도 번호는 그대로라 로그인 상태가 유지된다.
// (워크스페이스 이름이 무엇이든 열쇠로 쓸 수 있게 원형이 없는 객체에 담는다.)
let groupProfiles = Object.create(null);
try { Object.assign(groupProfiles, JSON.parse(localStorage.getItem('cypher.profiles') || '{}')); } catch { /* 무시 */ }
// 워크스페이스 이름 → 그 워크스페이스에서 여는 세션이 시작될 폴더(워크스페이스 폴더). 정해 두지 않은 워크스페이스는 그 워크스페이스에 있는 항목의 폴더나 기본 폴더(사용자 폴더)를 쓴다.
let groupFolders = Object.create(null);
try { Object.assign(groupFolders, JSON.parse(localStorage.getItem('cypher.groupFolders') || '{}')); } catch { /* 무시 */ }
// 브라우저 시작 페이지: 새 사이트 항목과 "chrome" 엔진의 브라우저가 처음 여는 주소. 비어 있으면 빈 화면으로 연다.
// 설정 → "브라우저 시작 페이지…"에서 정하고 이 컴퓨터에 기억한다.
let homePage = '';
try { homePage = localStorage.getItem('cypher.home') || ''; } catch { /* 무시 */ }
if (typeof homePage !== 'string') homePage = '';
// Claude 실행 명령: "+ Claude"로 여는 세션을 띄울 명령(프로그램과 인자). 비어 있으면 프로필의 것(기본은 claude)을 쓴다.
// 설정 → "Claude 실행 명령…"에서 정하고 이 컴퓨터에 기억한다.
let claudeCommand = '';
try { claudeCommand = localStorage.getItem('cypher.claudeCommand') || ''; } catch { /* 무시 */ }
if (typeof claudeCommand !== 'string') claudeCommand = '';
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
  setPty(loaded.pty); // 터미널이 글자 칸 수와 크기 변경을 세션의 가상 터미널에 맞추는 데 쓴다.
  if (loaded.error) setStatus(`설정 파일 오류(기본값으로 동작): ${loaded.error}`);
  // 기본 폴더는 사용자 폴더다. 세션이 시작될 폴더는 워크스페이스마다 정한다(워크스페이스를 만들 때, 또는 워크스페이스 오른쪽 클릭 → 워크스페이스 폴더 정하기).
  folder = await invoke('home_dir');
  storeDirs = await invoke('store_dirs').catch(() => ({}));
  // 테마·글꼴·글자 크기를 적용한다. 바뀔 때마다 열려 있는 터미널과 항목 화면을 맞춘다.
  initLook({ invoke, make, openPopup, config, onChange: applyLook });
  // 화면이 그려질 준비가 됐다. 업데이트 뒤 "다시 시작"으로 뜬 창은 여기까지 숨겨져 있다(하얀 창이 먼저 보이지 않게).
  invoke('app_ready').catch(() => {});

  // 사이드바 맨 위에는 "+ 워크스페이스"만 둔다. 세션과 항목은 워크스페이스 밖(루트)에 만들 수 없고, 워크스페이스 머리의 +로 그 워크스페이스 안에 만든다.
  // (맨 위에 "+ Claude" 같은 단추가 있으면 어느 워크스페이스에 생길지 눈에 보이지 않아 엉뚱한 워크스페이스에 세션이 생긴다.)
  const groupButton = make('button', '', '+ 워크스페이스');
  groupButton.title = '새 워크스페이스 만들기 · 세션과 항목은 워크스페이스 안에서만 만듭니다(워크스페이스 머리의 +)';
  groupButton.onclick = () => newGroup();
  $('new-buttons').append(groupButton);

  initExtras({
    invoke, listen, make, ask, setStatus, newSession, label, runsClaude, restartApp,
    getActive: () => active,
    getFolder: () => folder,
    focusActive: () => focusItem(active),
    pasteText: (text) => pasteInto(active, text),
    // 변경 내용에서 고른 줄을 넘겨받을 수 있는 Claude 세션: 그 항목과 같은 묶음에서 돌고 있는 것.
    claudes: (item) => claudesOf(item).map((s) => ({ s, label: label(s) })),
    pasteTo: (s, text) => { if (!sessions.has(s.key)) return; activate(s); pasteInto(s, text); },
  });
  initHttp({ invoke, make, ask, setStatus, baseName });
  initMemo({
    invoke, make, ask, setStatus, baseName, showMenu,
    openMemo,
  });
  initSite({
    invoke, listen, make, setStatus, showMenu, captured: onCaptured,
    zoom: () => currentLook().zoom,
    // 이 브라우저를 넘겨받을 수 있는 Claude 세션: 같은 이름 붙인 워크스페이스에서 돌고 있는 것.
    claudes: (item) => controllersOf(item).map((s) => ({ s, label: label(s) })),
    // 이 사이트 항목의 워크스페이스가 쓰는 브라우저 프로필. 항목이 다른 워크스페이스로 옮겨지면 그 워크스페이스의 프로필로 다시 연다.
    profile: (item) => ensureProfile(item),
    // 지금 이 브라우저를 조작하는 Claude 세션의 이름(없으면 빈 글).
    controller: (item) => { const by = [...sessions.values()].find((s) => s.browser === item.key && !s.exited && canControl(s, item)); return by ? label(by) : ''; },
    handoff: handoffSite,
  });
  initMindmap({ invoke, make, setStatus, showMenu, baseName, addImages });
  initErd({ invoke, make, setStatus, showMenu, addImages });
  initImage({
    invoke, make, setStatus, showMenu, addImages, toClaude: imageToClaude, openSource: openSourcePage,
    // 같은 묶음의 이미지 항목들(사이드바에 놓인 순서). 이미지 화면에서 이전/다음으로 넘길 때 쓴다.
    siblings: (s) => [...sessions.values()].filter((other) => other.kind === 'image' && groupKey(other) === groupKey(s)),
    show: (s) => activate(s),
  });
  initDrop({
    invoke, listen, setStatus, quote, newItem, addImages, pasteInto, reveal,
    itemOfPane: (pane) => (pane ? [...sessions.values()].find((s) => s.pane === pane) : null),
    itemOfRow: (row) => rowItem(row),
    contextOfHead: (head) => ({ group: head.dataset.group || '', cwd: head.dataset.cwd || folder }),
    screenEmpty: () => !visible.length,
    // 놓은 자리(묶음 머리, 항목의 줄이나 창)의 워크스페이스. 빈 곳에 놓았으면 지금 보는 항목의 워크스페이스로 정한다.
    groupFor: (context) => groupFor({ group: context.near?.group || context.group }),
  });
  initPalette({ make, openPopup, entries: paletteEntries });
  initClosed({
    invoke, make, openPopup, setStatus, notice, baseName,
    revive: (record) => revive(record),
    icon: (record) => (KINDS[record.kind]?.icon || '›'),
    kindLabel: (record) => (KINDS[record.kind]?.label || config.profiles.find((p) => p.id === record.profile)?.label || '세션'),
  });
  initHistory({
    invoke, make, openPopup, ask, setStatus, notice,
    icon: (item) => (KINDS[item.kind]?.icon || '›'),
    kindLabel: (item) => (typeof item.kindLabel === 'string' && item.kindLabel) || KINDS[item.kind]?.label || config.profiles.find((p) => p.id === item.profile)?.label || '세션',
    taken: (name) => groupList.includes(name),
    restore: restoreHistory,
  });
  initReader({
    invoke, make, setStatus, showMenu, baseName, toClaude: imageToClaude,
    openMemo,
  });
  initTodo({ invoke, make, setStatus, showMenu, toClaude: textToClaude });
  initRun({
    invoke, listen, make, setStatus, showMenu, Channel, toClaude: textToClaude,
    look: currentLook,
    scrollback: config.session.scrollback,
    finished: runFinished,
  });
  initTable({ invoke, make, setStatus, showMenu, baseName, toClaude: imageToClaude, openMemo });
  initHex({ invoke, make, setStatus, showMenu, baseName, toClaude: imageToClaude, ask });
  initS3({
    invoke, listen, make, openPopup, ask, setStatus, showMenu,
    pickFolder: (title) => invoke('pick_folder', { start: startFolderHint(), title }).catch(() => null),
  });
  initCode({ invoke, make, setStatus, showMenu, baseName, toClaude: textToClaude });
  initNotices({ make, openPopup, setStatus, jump: jumpToNotice, onChange: renderNoticeButton });
  // SSH 접속 창에서 고르거나 적은 곳으로 세션을 연다.
  initSsh({ invoke, make, openPopup, setStatus, connect: (spec, opts) => newSession(SSH, { ...opts, ssh: spec }) });
  initTemplate({
    invoke, make, openPopup, ask, setStatus, notice, baseName,
    pickFolder: (title) => invoke('pick_folder', { start: startFolderHint(), title }),
    current: currentGroup,
    pack: packGroup,
    open: openTemplate,
  });
  $('notices').onclick = showNotices;
  // 타이머·알람: 맞춰 둔 것이 Cypher가 꺼져 있는 동안 지나갔으면 여기서(켠 직후) 바로 알린다.
  initTimer({ make, openPopup, setStatus, ring: timerRang, onChange: renderTimerButton, wake: (ms) => invoke('wake_after', { ms }) });
  $('timers').onclick = showTimers;
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
      tool('Chrome', 'browser', 'Cypher 전용 프로필의 Chrome·Edge 열기 (Ctrl+Shift+O)', openChrome),
      tool('넘기기', 'handoff', '보고 있는 페이지를 Claude 세션에 넘기기 · 오른쪽 클릭: 새 세션에 넘기기', () => handoffChrome(false), () => handoffChrome(true)),
      tool('캡처', 'capture', '화면을 찍어 이미지 항목으로 넣기 · Shift+클릭: 페이지 전체', (ev) => captureBrowser(ev.shiftKey)),
    );
  }
  // 같은 워크스페이스의 Claude 세션이 Cypher 도구로 물어 오는 것(조작할 브라우저, 워크스페이스의 항목, 마인드맵 쓰기)에 답한다.
  listen('mcp-request', ({ payload }) => onMcpRequest(payload));
  initSidebarKeys();
  listen('menu', ({ payload }) => runMenu(payload));

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
  listen('session-ready', ({ payload }) => onReady(payload));
  // 출력 기록을 적지 못해 멈췄다(드라이브가 빠짐·가득 참 등): 기록 표시를 풀고 알린다.
  listen('session-log-failed', ({ payload }) => {
    const s = [...sessions.values()].find((other) => isTerm(other) && other.id === payload.id);
    // 앞서 끈 기록의 늦은 실패가 지금 기록 중인 다른 파일의 표시를 지우지 않게, 그 파일일 때만 푼다.
    if (s && s.logPath && samePath(s.logPath, payload.path)) {
      s.lastLog = s.logPath;
      s.logPath = '';
      render();
    }
    setStatus(`출력 기록을 멈췄습니다: ${payload.path}에 쓰지 못했습니다 (${payload.error}). 그때까지의 기록은 파일에 남아 있습니다.`);
    if (config.notify.system) invoke('notify', { title: '출력 기록을 멈췄습니다', body: `${payload.path}에 쓰지 못했습니다: ${payload.error}` }).catch(() => {});
  });
  listen('session-status', ({ payload }) => onStatus(payload));
  listen('session-hook', ({ payload }) => onHook(payload));
  listen('session-file', ({ payload }) => onCreatedFile(payload));
  listen('session-edit', ({ payload }) => onEditedFile(payload));
  listen('session-tool', ({ payload }) => onTool(payload));
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
  // 창을 닫으려 하면 앱이 바로 닫지 않고 알려 준다(창이 닫힐 때는 위의 beforeunload가 불리지 않는다).
  // 자동 저장을 기다리던 글(메모·요청 파일·마인드맵)과 배치를 저장한 뒤에 닫게 한다.
  // 다시 시작: 새 프로세스가 막 만들어졌다. 새 실행 파일을 띄우는 데 오래 걸렸으면(Windows의 검사) 그사이 화면이 바뀌었을 수
  // 있으므로, 새 화면이 배치를 읽기 전에 한 번 더 적는다. 그 뒤로는 적지 않는다(새 창이 적는 것을 덮지 않게).
  listen('restart-spawned', () => {
    if (!closing) return;
    closing = false;
    writeState();
    closing = true;
  });
  listen('app-closing', async () => {
    await saveBeforeClose();
    invoke('app_close');
  });

  const auto = await invoke('autostart');
  if (auto) {
    // 바로 시작 모드: 지난 세션은 되살리지 않지만 메모·HTTP 항목은 사이드바에 남긴다.
    // 세션은 워크스페이스 안에서만 열리므로, 워크스페이스 없이 쓰던 것을 모으는 `root` 워크스페이스(없으면 만든다)에 연다.
    await restore({ docsOnly: true });
    await newSession(auto[0], { group: adoptGroup(), cwd: auto[1] });
  } else {
    await restore();
  }
  // 되살릴 항목이 없어도 사이드바는 그린다: 항목 없이 남아 있는 워크스페이스의 머리(세션·항목을 만드는 길은 그 머리의 +뿐이다)와,
  // 워크스페이스가 하나도 없을 때의 안내 줄이 여기서 나온다.
  render();
}

/** 창이 닫히기 전에 쓰던 것을 저장한다: 자동 저장을 기다리던 글(메모·요청 파일·마인드맵)과 배치. 그 뒤로는 상태를 적지 않는다. */
async function saveBeforeClose() {
  const saving = [...sessions.values()].map((s) => { try { return s.view?.flush?.(); } catch { return undefined; } });
  // 저장이 멈춰 있어도 닫히기는 해야 한다(앱도 3초 뒤에는 그냥 닫는다).
  await Promise.race([Promise.allSettled(saving), new Promise((resolve) => setTimeout(resolve, 2500))]);
  writeState();
  closing = true;
}

/**
 * 업데이트를 설치한 뒤 새 버전으로 다시 시작한다(쓰던 것을 저장하고, 새 실행 파일을 띄우고, 새 창이 뜨면 이 창을 닫는다).
 * 새 실행 파일을 처음 띄울 때는 Windows가 그 파일을 검사하느라 오래 걸릴 수 있다. 그동안 기다려 달라는 안내로 창을 덮어 둔다.
 */
async function restartApp() {
  const wait = $('restarting');
  let waiting = true;
  wait.oncancel = (ev) => ev.preventDefault(); // Esc로 닫히지 않게 한다.
  // Esc를 거듭 누르면 막지 못하고 닫히는 수가 있다. 기다리는 동안에는 다시 띄운다(이미 저장을 멈춘 창을 쓰게 두지 않는다).
  wait.onclose = () => { if (waiting) wait.showModal(); };
  if (!wait.open) wait.showModal();
  await saveBeforeClose();
  try {
    await invoke('update_restart');
  } catch (err) {
    closing = false; // 새 버전을 띄우지 못했다: 이 창은 계속 쓴다.
    waiting = false;
    wait.close();
    throw err;
  }
}

/** 창 상단 메뉴(도구·설정·도움말)에서 고른 항목을 실행한다. 항목 번호는 앱 쪽 메뉴 정의와 맞춘다. */
function runMenu(id) {
  const actions = {
    palette: showPalette,
    reopen: showClosed,
    history: showHistory,
    notices: showNotices,
    timers: showTimers,
    templates: showTemplates,
    snippets: showSnippets,
    broadcast: () => setBroadcast(!broadcast),
    'copy-select': () => setCopyOnSelect(!copyOnSelect),
    memory: () => showMemory(),
    look: showLook,
    home: askHomePage,
    'claude-command': askClaudeCommand,
    settings: () => {
      invoke('open_config').catch((err) => setStatus(String(err)));
      setStatus('설정 파일은 저장한 뒤 Cypher를 다시 시작하면 적용됩니다. (테마·글꼴·글자 크기는 설정 → 모양에서 바로 바꿉니다)');
    },
    // Claude Code의 사용자 공통 파일(모든 폴더에 적용되는 설정과 지침)
    'claude-settings': () => openClaudeFile('settings'),
    'claude-memory': () => openClaudeFile('memory'),
    help: showHelp,
    engine: showEngine,
    update: showUpdate,
    about: showAbout,
  };
  actions[id]?.();
}

function openClaudeFile(which) {
  invoke('open_claude_file', { which })
    .then((path) => setStatus(`열었습니다: ${path} (고친 내용은 새로 여는 Claude 세션부터 적용됩니다)`))
    .catch((err) => setStatus(String(err)));
}

/** 지금 보고 있는 항목의 워크스페이스(워크스페이스에 든 항목을 보고 있지 않으면 빈 글). 새 세션은 이 워크스페이스에 들어간다. */
const viewedGroup = () => (active && sessions.has(active.key) && active.group) || '';
/** 그 워크스페이스에 따로 정해 둔 폴더(워크스페이스 폴더). 정해 두지 않았으면 빈 글. */
const groupFolderOf = (name) => (name && typeof groupFolders[name] === 'string' ? groupFolders[name] : '');

/** 폴더를 고르는 창이 처음 보여 줄 곳: 지금 보는 항목의 폴더, 없으면 가장 최근에 세션을 시작한 폴더, 그것도 없으면 기본 폴더. */
const startFolderHint = () => {
  const here = active && sessions.has(active.key) ? active : null;
  return (here && (here.project || here.startCwd)) || recentFolders[0] || folder;
};

/**
 * 새 세션·항목이 들어갈 워크스페이스. 워크스페이스 밖(루트)에는 아무것도 만들 수 없다.
 * 정해 준 워크스페이스 → 지금 보고 있는 항목의 워크스페이스 → 워크스페이스가 하나뿐이면 그 워크스페이스. 정할 수 없으면 까닭을 알리고 빈 글을 돌려준다.
 */
function groupFor(opts) {
  const group = opts.group || (!opts.restoring && viewedGroup()) || (groupList.length === 1 ? groupList[0] : '');
  if (!group) {
    notice('워크스페이스 안에서만 만들 수 있습니다', groupList.length
      ? '어느 워크스페이스에 만들지 알 수 없습니다. 만들 워크스페이스의 머리에 있는 +를 누르거나, 그 워크스페이스의 항목을 보고 있을 때 다시 하세요.'
      : '먼저 사이드바 위의 [+ 워크스페이스]로 워크스페이스를 만드세요. 세션과 항목은 워크스페이스 머리의 +로 그 워크스페이스 안에 만듭니다.');
  }
  return group;
}

/** 워크스페이스 없이 쓰던 항목을 모아 두는 워크스페이스의 이름. */
const ROOT_GROUP = 'root';

/**
 * 워크스페이스 없이 적혀 있던 항목(이전 버전에서 워크스페이스 없이 쓰던 세션·항목, 그때 닫은 항목의 기록)을 넣을 워크스페이스: `root`.
 * 없으면 만든다. 워크스페이스의 폴더는 정하지 않는다(여러 폴더의 항목이 섞여 있고, 항목마다 자기 폴더를 그대로 쓴다).
 */
function adoptGroup() {
  ensureGroup(ROOT_GROUP);
  return ROOT_GROUP;
}

/** 두 경로가 같은 폴더를 가리키는 표기인지(대소문자, 슬래시 방향, 끝의 구분자만 다른 것은 같다고 본다). */
const sameFolder = (a, b) => {
  const plain = (path) => String(path).replace(/\//g, '\\').replace(/(?<=[^:\\])\\+$/, '').toLowerCase();
  return plain(a) === plain(b);
};

/**
 * 워크스페이스 폴더를 정한다. 한 번 정한 폴더는 바꾸거나 풀 수 없다(이미 정해져 있으면 그대로 두고 까닭을 알린다).
 * 워크스페이스를 만들 때 정하고, 그때 정하지 않은 워크스페이스는 나중에 한 번 정할 수 있다.
 * `replaceMissing`: 정해 둔 폴더가 없어진 것을 확인하고 다시 정하는 경우(repointGroupFolder)에만 준다.
 * 정했으면(또는 이미 그 폴더면) true.
 */
function setGroupFolder(name, path, { replaceMissing = false } = {}) {
  if (!name || !path) return false;
  const own = groupFolderOf(name);
  if (own && !replaceMissing) {
    if (sameFolder(own, path)) return true;
    setStatus(`워크스페이스 "${name}"의 폴더(${own})는 바꿀 수 없습니다. 다른 폴더에서 일하려면 그 폴더로 새 워크스페이스를 만드세요.`);
    return false;
  }
  groupFolders[name] = path;
  localStorage.setItem('cypher.groupFolders', JSON.stringify(groupFolders));
  render(); // 묶음 머리의 폴더 줄과 풍선말을 맞춘다.
  setStatus(`워크스페이스 "${name}"의 폴더: ${path} (이 워크스페이스에서 여는 세션은 여기서 시작합니다. 이 폴더는 바꿀 수 없습니다)`);
  return true;
}

/** 폴더를 정하지 않은 워크스페이스의 폴더를 고르게 한다(한 번 정하면 바꿀 수 없다). 이미 정해져 있으면 그 폴더를 탐색기에서 연다. */
async function pickGroupFolder(name) {
  if (groupFolderOf(name)) return openGroupFolder(name);
  const picked = await invoke('pick_folder', { start: startFolderHint(), title: `워크스페이스 "${name}"의 세션을 시작할 폴더 (한 번 정하면 바꿀 수 없습니다)` }).catch(() => null);
  // 고르는 창이 떠 있는 동안에도 앱은 쓸 수 있다. 그사이 그 워크스페이스의 이름이 바뀌거나 지워졌으면 적지 않는다.
  if (picked && groupList.includes(name)) setGroupFolder(name, picked);
}

/** 워크스페이스 폴더가 지금도 있는지. 네트워크 경로(\\서버\…)는 확인하지 않고 있다고 본다(확인하는 명령이 네트워크 경로를 보지 않는다). */
async function groupFolderExists(path) {
  if (/^[\\/]{2}/.test(path)) return true;
  const [exists] = await invoke('dirs_exist', { paths: [path] }).catch(() => [true]);
  return exists !== false;
}

/**
 * 워크스페이스의 폴더를 탐색기에서 연다(폴더 줄의 [폴더] 단추·경로, 워크스페이스 메뉴, 빠른 이동).
 * 폴더를 정하지 않은 워크스페이스면 고르게 하고, 정해 둔 폴더가 없어졌으면 다시 정할지 묻는다.
 */
async function openGroupFolder(name) {
  const own = groupFolderOf(name);
  if (!own) return pickGroupFolder(name);
  if (!(await groupFolderExists(own))) return repointGroupFolder(name);
  invoke('link_open', { target: own, cwd: own, openFile: false, line: null, col: null })
    .then(() => setStatus(`탐색기에서 열었습니다: ${own}`))
    .catch((err) => setStatus(`워크스페이스 "${name}"의 폴더를 열지 못했습니다: ${err}`));
}

/**
 * 정해 둔 폴더가 없어진(지워졌거나 옮겨진) 워크스페이스만 폴더를 다시 정할 수 있다. 그대로 두면 그 워크스페이스의 세션이
 * 계속 기본 폴더에서 시작하기 때문이다. 묻고, 고르는 사이에 폴더가 되살아났으면 바꾸지 않는다.
 */
async function repointGroupFolder(name) {
  const own = groupFolderOf(name);
  if (!confirm(`워크스페이스 "${name}"의 폴더가 없습니다:\n${own}\n\n지워졌거나 옮겨진 것 같습니다. 폴더를 다시 정할까요?\n(폴더가 없어진 워크스페이스만 다시 정할 수 있습니다)`)) return;
  const picked = await invoke('pick_folder', { start: startFolderHint(), title: `워크스페이스 "${name}"의 폴더 다시 정하기 (없어진 폴더: ${own})` }).catch(() => null);
  if (!picked || !groupList.includes(name) || groupFolderOf(name) !== own) return;
  if (await groupFolderExists(own)) return setStatus(`워크스페이스 "${name}"의 폴더가 다시 있어 바꾸지 않았습니다: ${own}`);
  setGroupFolder(name, picked, { replaceMissing: true });
}

/** 폴더를 정하지 않은 워크스페이스의 폴더 줄의 ▾(또는 글): 지금 보는 항목의 폴더와 최근에 세션을 시작한 폴더에서 골라 그 워크스페이스의 폴더로 정한다. */
function groupFolderMenu(ev, name) {
  if (groupFolderOf(name)) return openGroupFolder(name);
  const here = active && sessions.has(active.key) ? (active.project || active.startCwd) : '';
  const seen = new Set();
  const choices = [];
  for (const [path, note] of [[here, '  (지금 항목)'], ...recentFolders.map((path) => [path, ''])]) {
    if (!path || seen.has(path.toLowerCase())) continue;
    seen.add(path.toLowerCase());
    choices.push({ label: `${baseName(path)}  —  ${path}${note}`, action: () => setGroupFolder(name, path) });
  }
  showMenu(ev, [
    { caption: `워크스페이스 "${name}"의 폴더 정하기 (한 번 정하면 바꿀 수 없습니다)` },
    ...choices,
    { label: '다른 폴더 고르기…', action: () => pickGroupFolder(name) },
  ]);
}

/** 세션을 시작한 폴더를 최근 폴더 목록 맨 앞에 올린다. */
function rememberFolder(path) {
  recentFolders = [path, ...recentFolders.filter((other) => other.toLowerCase() !== path.toLowerCase())].slice(0, MAX_FOLDERS);
  localStorage.setItem('cypher.folders', JSON.stringify(recentFolders));
}

const quote = (p) => (p.includes(' ') ? `"${p}"` : p);
const baseName = (p) => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p;
const setStatus = (text) => { $('status-text').textContent = text; };
/** 항목의 원래 이름(종류와 폴더, 메모는 파일 이름이나 첫 줄). */
const baseLabel = (s) => (s.kind === 'term'
  ? `${s.profile.label} · ${s.ssh ? specLabel(s.ssh) : s.worktree || baseName(s.cwd)}`
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
    // 이미 묻고 있는 중이면(이 창이 떠 있어도 창 위쪽의 메뉴는 눌린다) 앞선 물음은 취소로 끝내고 이 창을 넘겨받는다.
    // 그대로 덮어쓰면 앞선 물음은 영영 답을 받지 못한다.
    askDone?.(null);
    askDone = resolve;
    $('ask-title').textContent = title;
    input.value = value;
    input.placeholder = placeholder;
    dialog.returnValue = '';
    dialog.onclose = () => {
      if (askDone !== resolve) return; // 이 물음은 이미 끝났고, 창은 다른 물음의 것이다.
      askDone = null;
      resolve(dialog.returnValue === 'ok' ? input.value.trim() : null);
      focusItem(active);
    };
    if (!dialog.open) dialog.showModal();
    input.focus();
  });
}
/** 지금 떠 있는 한 줄 입력 창의 답을 받을 곳. */
let askDone = null;

// ───────────────────────── 세션 ─────────────────────────

/**
 * 세션을 새로 연다.
 * opts.show: 'replace'(지금 창 자리에. 기본), 'hidden'(목록에만). 화면을 나누는 것은 사용자가 끌어다 놓을 때뿐이다.
 * opts.tabOf: SSH 세션이면 이 세션 바로 옆 탭으로 넣는다(연결 복제).
 * opts.cwd / opts.resume(이어갈 Claude 세션 번호) / opts.worktree(워크트리 이름) / opts.group(워크스페이스)
 */
async function newSession(profileId, opts = {}) {
  const profile = config.profiles.find((p) => p.id === profileId);
  if (!profile) return null;
  // 세션은 워크스페이스 안에서만 연다. 워크스페이스를 정해 주지 않았으면 지금 보고 있는 항목의 워크스페이스에 연다.
  const group = groupFor(opts);
  if (!group) return null;
  // SSH 세션은 접속할 곳이 있어야 한다. 정해 주지 않았으면 접속 창을 띄워 고르게 하고, 거기서 고르면 다시 이리로 온다.
  const sshProfile = isSshProfile(profile);
  const ssh = sshProfile ? cleanSpec(opts.ssh) : null;
  if (sshProfile && !ssh) {
    if (!opts.restoring) showSsh({ ...opts, group, ssh: undefined });
    return null;
  }
  if (!opts.restoring) lastProfile = profileId;
  // 시작 폴더: 정해 주지 않았으면 그 워크스페이스의 폴더(워크스페이스 폴더), 그것도 없으면 기본 폴더(사용자 폴더).
  let start = opts.cwd || folder;
  const own = opts.cwd ? '' : groupFolderOf(group);
  if (own) {
    if (await groupFolderExists(own)) start = own;
    else setStatus(`워크스페이스 "${group}"의 폴더가 없어 기본 폴더(${folder})에서 시작합니다: ${own} (폴더 줄의 [폴더]를 누르면 다시 정할 수 있습니다)`);
  }
  $('empty').hidden = true;
  // 같은 폴더가 표기만 달라 다른 묶음으로 갈리지 않게 역슬래시로 통일한다.
  const cwd = start.replace(/\//g, '\\').replace(/(?<=[^:\\])\\+$/, '');

  const pane = make('div', 'pane hidden');
  const head = make('div', 'pane-head');
  const host = make('div', 'term');
  pane.append(head, host);
  $('parked').append(pane); // 화면에 놓이기 전까지 보관 칸에 둔다.

  const look = currentLook();
  const term = new Terminal({
    fontFamily: look.termFamily, fontSize: ssh ? sshPixels(look) : look.termPixels, theme: look.termTheme, minimumContrastRatio: look.contrast,
    cursorBlink: true, scrollback: config.session.scrollback, allowProposedApi: true, windowsPty: windowsPty(),
    // 매트릭스 테마에서는 터미널 뒤로 흐르는 글자가 비치게 한다. 터미널을 만들 때만 정할 수 있고(나중에 못 바꾼다)
    // 글자가 조금 덜 또렷해지므로, 그 테마로 흐르는 글자를 켜 둔 동안에 만드는 터미널에만 켠다.
    allowTransparency: look.rain,
  });
  const fitAddon = new FitAddon.FitAddon();
  const search = new SearchAddon.SearchAddon();
  term.loadAddon(fitAddon);
  term.loadAddon(search);
  term.loadAddon(new Unicode11Addon.Unicode11Addon());
  term.unicode.activeVersion = '11';
  term.open(host);

  const s = {
    key: ++seq, kind: 'term', id: null, profile, term, fitAddon, search, pane, head, view: null,
    startCwd: cwd, cwd, project: '', worktree: opts.worktree || '',
    name: opts.name || '', color: opts.color || '', mark: freeMark(opts.mark),
    group,
    ssh,               // SSH 세션이면 접속할 곳 { target(주소), port?, name? } — 사용자 이름은 담지 않는다(세션 화면에서 묻는다)
    cloneOf: opts.cloneOf ?? null, // 연결 복제: 이 세션 번호의 연결을 같이 쓴다(처음 띄울 때만)
    logPath: '',       // 출력을 기록하고 있는 파일(기록하지 않으면 빈 글)
    title: '', model: '', effort: '', context: null, cost: 0,
    usage: '', usageEl: null,
    agents: new Map(), // 지금 돌고 있는 서브에이전트: 번호 → 종류
    tools: new Map(),  // 지금 쓰고 있는 도구: 호출 번호 → { tool, detail, full, at, lane, repeat }
    waiting: '', // '' | 'approval'(승인·입력 대기) | 'done'(응답 완료, 다음 지시 대기)
    claudeSession: opts.resume || '', resumable: !!opts.resume,
    lastOutput: 0, busy: false, attention: false, exited: false,
    pending: '', sending: false, queued: 0, paused: false,
    focusedAt: 0, shots: [],
    draft: opts.draft || '', // 프롬프트 편집기에 쓰던 글
    workStart: 0, took: 0,   // 지금 요청을 시작한 시각(일하는 중이 아니면 0)과 지난 요청에 걸린 시간(ms)
    // 쉬는 세션: 다시 시작한 뒤 사이드바에만 되살려 둔 것. 프로세스를 띄우지 않고 있다가 사용자가 누르면 재개한다(wake).
    dormant: !!opts.dormant,
  };
  // SSH는 워크스페이스마다 하나(SSH 방)다: 새 SSH 세션은 그 방의 탭으로 들어간다. 복제(tabOf)는 원래 세션 바로 옆 탭으로,
  // 그 방의 탭을 보고 있었으면 그 옆 탭으로, 아니면 방에서 마지막으로 쓴 창의 맨 끝 탭으로 넣는다(방이 없으면 이 세션으로 방이 생긴다).
  // 되살리는 중에는 넣지 않는다(다시 켜면 SSH는 워크스페이스마다 탭 하나뿐이다. restore).
  let tabAnchor = null, tabBeside = false;
  if (ssh && !opts.restoring) {
    const beside = opts.tabOf && sessions.has(opts.tabOf.key) && opts.tabOf.group === group ? opts.tabOf
      : active?.ssh && sessions.has(active.key) && active.group === group ? active : null;
    tabAnchor = beside || roomFront(group);
    tabBeside = !!beside;
  }
  sessions.set(s.key, s);
  if (ssh) {
    s.tabSet = { list: [s] };
    if (tabAnchor) joinTabs(s, tabAnchor, tabBeside ? undefined : null);
  }
  ensureGroup(s.group);
  if (!opts.restoring) rememberFolder(cwd);
  // 글자의 칸 수를 가상 터미널(conhost)과 같게 센다. 다르면 화면이 한 줄씩 밀린다.
  // Claude Code와 가상 터미널이 서로 다르게 세는 글자가 찍히면 알려 오고, 그때는 응답이 끝난 뒤 화면을 다시 맞춘다(realign).
  matchWidths(term, () => noteSkew(s));
  wirePane(s);
  wireTerminal(s);
  // 찾기 결과의 개수와 지금 몇 번째인지를 찾기 줄에 보여 준다.
  search.onDidChangeResults?.((result) => { if (s === active && !$search.hidden) showSearchCount(result); });
  // 쉬는 세션은 화면에 올리지 않는다. 화면이 비어 있었으면 위에서 숨긴 빈 화면 안내를 되돌린다(배치를 다시 지으면 돌아온다).
  if (s.dormant) { if (!opts.restoring && !visible.length) layout(); render(); return s; }

  // 화면은 나누지 않는다(창 분할은 사용자가 끌어다 놓을 때만): 지금 창 자리에 놓는다. SSH 탭은 SSH 방 안에서 그 묶음의 창에 보이고,
  // 방이 화면에 없으면 방이 지금 창 자리에 놓인다.
  if (opts.show !== 'hidden') { if (s.tabSet) placeTab(s); else placeInActive(s); }
  layout();
  if (opts.show !== 'hidden') activate(s);
  await startProcess(s, opts.worktree);
  return s;
}

/** 세션의 프로세스를 띄워 터미널에 잇는다. Claude 세션은 이어갈 대화가 있으면 그 대화를 잇는다. */
async function startProcess(s, worktree = null) {
  const { term } = s;
  // 재개에 실패한 세션은 같은 터미널로 다시 재개해 본다. 그리기 모듈은 한 번만 붙인다.
  if (!s.webgl) {
    s.webgl = true;
    try {
      const gl = new WebglAddon.WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch { /* WebGL을 못 쓰면 기본 그리기로 동작한다 */ }
  }
  fitVisible(); // 띄우기 전에 실제 크기를 재 둔다.
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
  const size = { cols: term.cols, rows: term.rows };
  // 기본 Claude 프로필은 사용자가 정해 둔 실행 명령으로 띄운다. 그 명령으로 띄우지 못하거나 뜨자마자 끝나면 어디서 고치는지 알려 준다.
  const command = s.profile.id === 'claude' && claudeCommand ? claudeCommand : null;
  s.launch = { command, at: Date.now() };
  // 같은 서버에 이미 접속해 있는 연결을 같이 쓸지는 앱이 정한다: 접속할 곳에는 주소와 포트만 있고 사용자 이름은 세션 화면에서
  // 물어야 알기 때문이다(같은 사용자일 때만 같이 쓴다). 여기서는 연결 복제(cloneOf)만 넘긴다.
  // SSH 세션은 셸이 열렸다는 알림(session-ready)이 올 때까지 로그인 중으로 본다(동시 입력에서 뺀다).
  // 다시 접속하면 다른 사용자로 로그인할 수 있으니 탭의 사용자 이름도 셸이 열릴 때까지 비운다.
  if (s.ssh) Object.assign(s, { sshReady: false, sshUser: '', sshBuiltin: false });
  // 연결은 같은 워크스페이스의 세션끼리만 같이 쓴다: 이 세션이 든 워크스페이스를 함께 넘긴다.
  const scope = s.group || '';
  try {
    s.id = await invoke('session_create', {
      profile: s.profile.id, cwd: s.startCwd, cols: size.cols, rows: size.rows,
      resume: resumeOf(s) || null, worktree: worktree || null, onOutput: channel,
      command, ssh: s.ssh || null, cloneOf: s.cloneOf ?? null, group: scope,
    });
    if (s.ssh) s.scopeSent = scope; // 앱이 알고 있는 이 세션의 워크스페이스(syncSshScopes)
    s.cloneOf = null; // 다시 시작할 때는 새로 접속한다.
    // 번호를 받기 전에 셸이 열렸다(나눠 쓰는 연결은 금방 열린다).
    const early = earlyReady.get(s.id);
    if (early) { earlyReady.delete(s.id); markReady(s, early); }
    // 띄우는 사이에 사용자가 이미 닫았으면, 방금 뜬 프로세스를 끝내고 손을 뗀다.
    if (!sessions.has(s.key)) { invoke('session_close', { id: s.id }); return; }
    // 띄우는 사이에 창 크기가 바뀌었으면(그때는 알릴 프로세스가 없었다) 지금 크기를 알린다.
    s.ptySize = sizeKey(size);
    syncSize(s);
    flush(s); // 띄우는 동안 눌린 키를 보낸다.
    // 뜨자마자 끝난 프로그램은 종료 알림이 번호보다 먼저 와 있을 수 있다.
    if (earlyExits.has(s.id)) {
      const code = earlyExits.get(s.id);
      earlyExits.delete(s.id);
      onExit({ id: s.id, code });
    }
  } catch (err) {
    if (!sessions.has(s.key)) return;
    term.write(`\x1b[31m세션을 시작하지 못했습니다: ${err}\x1b[0m\r\n`);
    if (command) term.write(commandHint(command));
    s.exited = true;
    s.pending = ''; // 띄우는 동안 눌린 키는 버린다(남겨 두면 나중에 다시 띄웠을 때 한꺼번에 들어가 실행된다).
  }
  render();
  saveState();
}

/**
 * 쉬는 세션을 재개한다: 프로세스를 띄우고, Claude 세션은 하던 대화를 잇는다.
 * `chosen`은 그 세션을 고른 때(activate: 누르기, 끌어다 놓기, 북마크·빠른 이동으로 가기)에 참이다. 앱이 다음에 볼 항목을
 * 대신 고르는 길(보던 것을 닫거나 숨긴 뒤. stageFallback)과 지나가기만 하는 길(Ctrl+Tab, 창 사이 이동, 사이드바 방향키)은
 * 쉬는 세션을 고르지 않으므로, 사용자가 고르지 않은 쉬는 세션은 여기로 오지 않는다.
 *
 * SSH 세션은 고른 것만 접속한다. 화면에 올라왔다고, 또는 "모두 재개"로 여럿이 한꺼번에 접속하지 않는다:
 * 한 서버에 여러 접속이 동시에 들어가면 서버가 침입 시도로 보고 막는다. 사용자가 한 번 누르면 접속은 하나만 시작한다.
 */
function wake(s, chosen = false) {
  if (!s?.dormant || !sessions.has(s.key)) return;
  if (s.ssh && !chosen) return;
  // 재개에 실패했던 세션은 사용자가 다시 누를 때만 다시 해 본다(화면 배치가 바뀔 때마다 되풀이하지 않는다).
  if (s.wakeFailed && !chosen) return;
  s.dormant = false;
  s.wakeFailed = false;
  startProcess(s).then(() => {
    // 재개하지 못했으면(폴더가 든 드라이브가 아직 연결되지 않았다 등) 끝난 세션으로 두지 않고 다시 쉬는 세션으로 둔다.
    // 끝난 세션은 저장하지 않으므로, 그대로 앱을 끄면 이어 갈 대화가 목록에서 사라진다. 다시 누르면 다시 재개해 본다.
    if (!sessions.has(s.key) || !s.exited || s.id != null) return;
    s.exited = false;
    s.dormant = true;
    s.wakeFailed = true;
    render();
    saveState();
  });
}

/** 한꺼번에 재개할 수 있는 쉬는 세션들: SSH 세션은 뺀다(SSH는 사용자가 그 SSH를 눌렀을 때만 접속한다. wake). */
const wakeable = (list) => list.filter((s) => s.dormant && !s.ssh && sessions.has(s.key));

/** 그 묶음의 쉬는 세션을 모두 재개한다. SSH 세션은 건드리지 않는다. */
function wakeGroup(group) {
  const resting = wakeable(group.list);
  for (const s of resting) wake(s, true);
  if (resting.length) setStatus(`"${group.name}" 워크스페이스의 세션 ${resting.length}개를 재개했습니다.${group.list.some((s) => s.ssh && s.dormant) ? ' SSH는 사이드바의 SSH 줄을 누르면 접속합니다.' : ''}`);
  // 화면이 비어 있으면 그 가운데 첫 세션을 보여 준다.
  if (resting.length && !visible.length) activate(resting[0]);
  render();
}

/**
 * 터미널이 아닌 항목(메모, HTTP …)을 새로 만든다. 사이드바에 세션처럼 놓이고, 화면에서도 창 하나를 차지한다.
 * opts.show / opts.cwd / opts.group / opts.name / opts.color 는 newSession과 같고,
 * opts.saved 는 그 종류가 되살릴 내용(메모 번호, 열어 둔 파일 등)이다.
 */
async function newItem(kind, opts = {}) {
  const def = KINDS[kind];
  if (!def) return null;
  // 항목은 워크스페이스 안에서만 만든다. 워크스페이스를 정해 주지 않았으면 지금 보고 있는 항목의 워크스페이스에 만든다.
  const group = groupFor(opts);
  if (!group) return null;
  $('empty').hidden = true;
  // 폴더는 지금 보고 있는 항목의 것을 따른다(같은 업무의 요청 파일·메모가 한 폴더에 모이게).
  // 보고 있는 항목이 없으면 그 워크스페이스의 폴더, 그것도 없으면 기본 폴더.
  const base = !opts.restoring && active && sessions.has(active.key) ? (active.project || active.startCwd) : groupFolderOf(group) || folder;
  const cwd = (opts.cwd || base).replace(/\//g, '\\').replace(/(?<=[^:\\])\\+$/, '');

  const pane = make('div', 'pane hidden');
  const head = make('div', 'pane-head');
  const host = make('div', 'doc-host');
  pane.append(head, host);
  $('parked').append(pane);

  const s = {
    key: ++seq, kind, id: null, profile: null, term: null, pane, head, view: null,
    startCwd: cwd, cwd, project: '', worktree: '',
    name: opts.name || '', color: opts.color || '', mark: freeMark(opts.mark),
    group,
    title: '', model: '', effort: '', context: null, cost: 0,
    usage: '', usageEl: null, agents: new Map(), tools: new Map(), waiting: '',
    lastOutput: -Infinity, busy: false, attention: false, exited: false,
    shots: [], draft: '', workStart: 0, took: 0,
  };
  // 제목(파일 이름, 메모 첫 줄)이 바뀌면 사이드바를 다시 그리고 상태를 적어 둔다.
  // 사용자가 새로 만드는 사이트 항목은 시작 페이지로 연다(되살리는 항목, 주소를 받아 만드는 항목, Claude가 만드는 항목은 그대로).
  const fresh = kind === 'site' && opts.saved === undefined && !opts.restoring && homePage ? { url: homePage } : {};
  s.view = def.create(s, host, opts.saved || fresh, () => { if (sessions.has(s.key)) { render(); saveState(); } });
  sessions.set(s.key, s);
  ensureGroup(s.group);
  wirePane(s);

  // 화면은 나누지 않는다(창 분할은 사용자가 끌어다 놓을 때만): 지금 창 자리에 놓는다.
  if (opts.show !== 'hidden') placeInActive(s);
  // 화면에 놓지 않는 항목(Claude가 놓는 항목 등)은 배치가 바뀌지 않는다. 나란히 보기에서는 배치를 다시 짓지 않는다:
  // 다시 지으면 모든 창을 떼었다 붙여, 입력하던 창의 초점과 문서 창의 스크롤 자리가 날아간다.
  // (새 창은 만들 때 이미 보관 칸에 들어가 있다.)
  if (opts.show !== 'hidden' || visible.length < 2) layout();
  if (opts.show !== 'hidden') activate(s);
  await s.view.ready;
  render();
  saveState();
  return s;
}

/** SSH 세션 프로필의 번호(앱의 기본 프로필). */
const SSH = 'ssh';
/** 접속할 곳을 받아 띄우는 SSH 프로필인지. 사용자가 설정 파일에서 같은 번호로 인자까지 적어 둔 프로필은 그 인자대로 띄우는 보통 프로필이다. */
const isSshProfile = (profile) => profile?.id === SSH && !(profile.args || []).length;

/**
 * 같은 서버에 세션을 하나 더 연다(연결 복제). 새 세션은 원래 세션 바로 옆 탭으로 들어가고(사이드바의 SSH 줄은 늘지 않는다) 그 탭이 보인다.
 * 화면은 나누지 않는다(창 분할은 사용자가 탭이나 사이드바의 항목을 끌어다 놓을 때만 한다).
 * 내장 SSH로 접속해 있으면 그 연결에 채널만 하나 더 열어 다시 인증하지 않는다(연결이 끊겼거나 아직 접속 중이면 새로 접속한다).
 */
function cloneSsh(s) {
  if (!s?.ssh) return null;
  const live = s.id != null && !s.exited && !s.dormant;
  return newSession(SSH, { ...besideOf(s), cwd: s.startCwd, ssh: s.ssh, color: s.color, show: 'replace', tabOf: s, cloneOf: live ? s.id : null });
}

// ───────────────────────── SSH 방과 탭 ─────────────────────────
// SSH는 워크스페이스마다 하나다. 사이드바에는 워크스페이스마다 SSH 줄이 하나만 놓이고, 그 워크스페이스의 SSH 세션은 모두 그 안
// (SSH 방)의 탭이 된다. 서버에 더 접속하거나 연결을 복제해도 사이드바에 줄이 늘지 않고 방 안의 탭이 는다.
//
// 방 안은 제 배치를 가진다(rooms: 워크스페이스 이름 → 잎 { s }·가지 { dir, ratio, a, b }). 잎 하나가 창 하나이고, 창마다 탭 묶음 하나가
// 놓인다(s.tabSet = { list: [세션…] }, 탭 순서대로. 잎의 s가 그 창에 지금 보이는 탭). 창 위쪽에는 그 묶음의 탭이 늘어선다.
// 탭을 방 안의 창 가장자리로 끌어다 놓으면 방 안에서 나눠 열고, 다른 창의 탭 줄(또는 가운데)에 놓으면 그 창으로 옮긴다.
// 탭은 방 밖으로 나가지 않는다(창을 옮기는 것은 SSH 안에서만 한다).
//
// 큰 배치(tree)에서는 방 전체가 한 자리다({ room: 워크스페이스 이름 }). 사이드바의 SSH 줄을 누르면 방이 지금 창 자리에 놓이고,
// 그 줄을 화면 가장자리로 끌어다 놓으면 방이 통째로 그쪽에 나뉘어 열린다. 다른 항목을 방의 자리에 열면 방은 통째로 화면에서 빠진다
// (방 안의 배치는 남아 있어, 다시 열면 그대로 돌아온다).
//
// 앱을 다시 켜면 SSH는 워크스페이스마다 탭 하나로 돌아온다([+]로 처음 만든 것처럼. sshDropped). 접속은 사용자가 고른 탭만 한다(wake):
// 여러 탭이 한꺼번에 접속하면 서버가 침입 시도로 본다.

/** SSH 방 안의 배치: 워크스페이스 이름 → 잎 { s }·가지 { dir, ratio, a, b }. 방이 화면에 없어도 남아 있다. */
const rooms = new Map();
/** 지금 화면에 그려진 방의 상자(워크스페이스 이름 → 요소). 다른 항목을 SSH 창 위로 끌 때 방 전체를 한 자리로 보여 주는 데 쓴다. */
const roomBoxes = new Map();

/** 방금 고른 항목(activate가 화면에 놓는 동안만). 쉬는 SSH 탭도 고른 것이면 창에 보이는 탭이 된다(settleRoom). */
let picked = null;

/**
 * 사용자가 고르지 않았는데 앱이 다음에 볼 항목을 대신 골라야 할 때(보던 것을 닫거나 숨긴 뒤) 고를 화면의 항목.
 * 쉬는 세션은 고르지 않는다: 고르면 재개되는데(SSH는 서버에 접속한다), 재개는 사용자가 고른 것만 한다.
 */
const stageFallback = () => visible.find((v) => !v.dormant) || null;

/** 그 워크스페이스의 SSH 세션들(사이드바에 놓인 순서). */
const sshOf =(group) => [...sessions.values()].filter((s) => s.ssh && s.group === group);

/** 그 워크스페이스의 SSH 방이 지금 화면에 있는지. */
const roomShown = (group) => !!findSlot({ room: group });

/** 그 SSH 세션과 같은 탭 묶음에 있는 세션들(탭 순서). 닫은 것, 다른 워크스페이스에 있는 것은 뺀다. */
function tabsOf(s) {
  if (!s?.tabSet) return s ? [s] : [];
  return s.tabSet.list.filter((x) => sessions.has(x.key) && x.group === s.group && x.tabSet === s.tabSet);
}

/** 그 SSH 세션이 지금 보이는 탭으로 놓인 방 안의 잎(방이 화면에 없어도 찾는다). 다른 탭에 가려져 있으면 null. */
const roomLeaf = (s) => (s?.ssh ? findLeaf(s, rooms.get(s.group) || null) : null);

/** 그 SSH 세션의 탭 묶음이 놓인 방 안의 잎(그 묶음에서 지금 보이는 탭의 잎). */
function setLeaf(s) {
  for (const x of tabsOf(s)) {
    const leaf = roomLeaf(x);
    if (leaf) return leaf;
  }
  return null;
}

/**
 * 그 워크스페이스의 SSH 방을 지금 세션에 맞춘다: 방 안의 창은 살아 있는 그 워크스페이스의 SSH 세션만 보이고, 한 탭 묶음은
 * 한 창에만 놓이며, 창이 없는 묶음의 탭은 방의 첫 창 탭 줄 끝에 붙는다(방이 비어 있으면 첫 세션의 창을 만든다).
 * SSH 세션이 없는 워크스페이스의 방은 없앤다. 되살리는 중에는 건드리지 않는다(다 되살린 뒤에 한 번 맞춘다).
 * `front`(배치를 지을 때)이면 쉬는 탭이 보이는 창에 그 묶음의 깨어 있는 탭을 대신 보인다.
 */
function settleRoom(group, front = false) {
  if (restoring) return;
  const all = sshOf(group);
  if (!all.length) { rooms.delete(group); return; }
  const placed = new Set(); // 창을 가진 탭 묶음
  let inner = (function keep(node) {
    if (!node) return null;
    if (node.s) {
      const s = node.s;
      if (!sessions.has(s.key) || !s.ssh || s.group !== group) return null;
      if (placed.has(s.tabSet)) leaveTabs(s); // 한 묶음에서 둘이 보이면 뒤의 것을 떼어 제 창을 준다.
      placed.add(s.tabSet);
      return node;
    }
    const a = keep(node.a), b = keep(node.b);
    return a && b ? Object.assign(node, { a, b }) : (a || b);
  })(rooms.get(group) || null);
  let home = leavesOf(inner)[0] || null;
  for (const s of all) {
    if (placed.has(s.tabSet)) continue;
    if (!home) {
      inner = { s };
      home = s;
      placed.add(s.tabSet);
    } else joinTabs(s, home, null);
  }
  // 창에 보이는 탭이 쉬는 탭인데 그 묶음에 깨어 있는 탭이 있으면, 깨어 있는 탭(접속 중인 것 먼저, 가장 최근에 쓴 것)을 보인다.
  // 쉬는 탭이 앞에 남으면 화면에 빈 창이 보이고, SSH를 열 때 접속해 있는 탭을 두고 그 탭이 새로 접속하게 된다(쉬는 탭만 있던
  // SSH에 다른 워크스페이스의 SSH를 합쳤을 때 생긴다). 사용자가 방금 고른 쉬는 탭(picked)은 그대로 둔다: 곧 접속한다(activate).
  // 배치를 지을 때만 한다(layout): 그 밖의 때에 바꾸면 화면에 놓인 창과 어긋난다.
  if (front) {
    (function raise(node) {
      if (!node) return;
      if (!node.s) { raise(node.a); raise(node.b); return; }
      if (!node.s.dormant || node.s === picked) return;
      const up = tabsOf(node.s).filter((x) => !x.dormant).sort((p, q) => (p.exited - q.exited) || (q.usedAt || 0) - (p.usedAt || 0))[0];
      if (up) node.s = up;
    })(inner);
  }
  rooms.set(group, inner);
}

/**
 * 모든 SSH 방과 큰 배치를 맞춘다(layout이 부른다). 큰 배치에 그냥 놓인 SSH 세션(예전 판이 저장해 둔 배치)은 그 워크스페이스의
 * 방으로 넣고(첫 자리가 방이 되고, 나머지는 방 안의 창이 된다), 없어진 방과 겹친 방의 자리는 뺀다.
 */
function settleRooms() {
  if (restoring) return;
  const mend = (node, fix) => {
    if (!node) return null;
    if (node.room || node.s) return fix(node);
    const a = mend(node.a, fix), b = mend(node.b, fix);
    return a && b ? Object.assign(node, { a, b }) : (a || b);
  };
  const seen = new Set();
  tree = mend(tree, (node) => {
    const group = node.room || (node.s.ssh ? node.s.group : '');
    if (!group) return node;
    if (node.s) {
      const inner = rooms.get(group) || null;
      if (!findLeaf(node.s, inner)) rooms.set(group, inner ? { dir: 'row', a: inner, b: { s: node.s } } : { s: node.s });
    }
    if (seen.has(group)) return null;
    seen.add(group);
    return node.room ? node : { room: group };
  });
  for (const group of new Set([...rooms.keys(), ...[...sessions.values()].filter((s) => s.ssh).map((s) => s.group)])) settleRoom(group, true);
  tree = mend(tree, (node) => (node.room && !rooms.has(node.room) ? null : node));
}

/** 그 방에서 지금 고를 탭: 방 안에서 가장 최근에 쓴 창에 보이는 탭. SSH 세션이 없으면 null. */
function roomFront(group) {
  settleRoom(group);
  const shown = leavesOf(rooms.get(group) || null);
  return shown.reduce((best, s) => (!best || (s.usedAt || 0) > (best.usedAt || 0) ? s : best), null) || sshOf(group)[0] || null;
}

/** 그 워크스페이스의 SSH 탭들을 방 안의 창과 탭 줄에 놓인 순서대로. */
function roomTabs(group) {
  settleRoom(group);
  const ordered = leavesOf(rooms.get(group) || null).flatMap((s) => tabsOf(s));
  return [...new Set([...ordered, ...sshOf(group)])];
}

/** 사이드바의 SSH 줄을 눌렀을 때: 그 워크스페이스의 SSH 방을 화면에 올리고(이미 있으면 그대로) 방에서 쓰던 탭을 고른다. */
function openRoom(group) {
  const here = active?.ssh && active.group === group && sessions.has(active.key) && visible.includes(active);
  const s = here ? active : roomFront(group);
  if (s) activate(s);
}

/** SSH 방을 화면에서 빼거나(세션은 계속 살아 있고 방 안의 배치도 남는다) 화면에 올린다. */
function toggleRoom(group) {
  if (!roomShown(group)) return openRoom(group);
  const mine = active?.ssh && active.group === group;
  tree = withoutSlot(tree, { room: group });
  layout();
  const next = mine ? stageFallback() : null;
  if (next) activate(next); else { render(); saveState(); }
}

/** 분할을 풀고 그 워크스페이스의 SSH 방만 화면에 남긴다(방 안의 배치는 그대로 둔다). */
function showRoomOnly(group) {
  const s = roomFront(group);
  if (!s) return;
  tree = { room: group };
  layout();
  activate(active?.ssh && active.group === group && visible.includes(active) ? active : s);
}

/** 방 안의 창을 s의 창 하나로 모은다: 다른 창의 탭들은 s의 탭 줄 끝에 차례로 붙는다. */
function mergeRoom(s) {
  settleRoom(s.group);
  for (const other of leavesOf(rooms.get(s.group) || null)) {
    if (other.tabSet === s.tabSet) continue;
    for (const tab of tabsOf(other)) joinTabs(tab, s, null);
  }
  rooms.set(s.group, { s });
}

/**
 * 방 안의 창 하나(s가 보이는 창)를 접는다: 그 창의 탭들은 옆 창(앞 창, 없으면 뒤 창)의 탭 줄 끝에 붙는다.
 * 방에 창이 하나뿐이면 접을 곳이 없다(null). 탭을 받은 창에 보이는 세션을 돌려준다.
 */
function foldPane(s) {
  const panes = leavesOf(rooms.get(s.group) || null);
  const at = panes.findIndex((x) => x.tabSet === s.tabSet);
  const next = at < 0 ? null : panes[at - 1] || panes[at + 1] || null;
  if (!next) return null;
  const moving = tabsOf(s);
  const leaf = setLeaf(s);
  const inner = leaf ? without(rooms.get(s.group), leaf.s) : rooms.get(s.group);
  if (inner) rooms.set(s.group, inner);
  for (const tab of moving) joinTabs(tab, next, null);
  return next;
}

/**
 * s를 방에서 뗀다(다른 워크스페이스로 옮기기 직전에 부른다): 보이던 탭이면 그 창을 옆 탭에 넘기고, 혼자 쓰던 창이면 창을 없앤다.
 * 그 창을 넘겨받은 탭을 돌려준다(없으면 null).
 */
function leaveRoom(s) {
  if (!s?.tabSet) return null;
  const heir = handOffTab(s);
  leaveTabs(s);
  if (!heir) takeOut(s);
  return heir;
}

/** s를 탭 묶음에서 떼어 혼자인 묶음으로 둔다(화면의 자리는 건드리지 않는다). */
function leaveTabs(s) {
  if (!s?.tabSet || s.tabSet.list.length < 2) return;
  s.tabSet.list = s.tabSet.list.filter((x) => x !== s);
  s.tabSet = { list: [s] };
}

/**
 * s를 anchor의 탭 묶음에 넣는다(같은 묶음이면 순서만 바꾼다). `before`가 세션이면 그 탭 앞에, null이면 맨 끝에,
 * 주지 않으면 anchor 바로 뒤에 넣는다. 화면의 자리는 건드리지 않는다.
 */
function joinTabs(s, anchor, before) {
  if (!s?.tabSet || !anchor?.tabSet) return;
  if (before === s || (s === anchor && before === undefined)) return; // 제자리에 놓았다.
  if (s.tabSet !== anchor.tabSet) leaveTabs(s);
  const set = anchor.tabSet;
  const list = set.list.filter((x) => x !== s);
  const at = before === undefined ? list.indexOf(anchor) + 1 : before === null ? list.length : list.indexOf(before);
  list.splice(at < 0 ? list.length : at, 0, s);
  set.list = list;
  s.tabSet = set;
}

/**
 * 창에 보이던 탭 s 대신 그 창에 옆 탭(오른쪽, 없으면 왼쪽, 그다음 가까운 탭)을 놓는다(방이 화면에 없어도 한다).
 * 방이 화면에 있을 때는 쉬는 탭에게 넘기지 않는다: 쉬는 탭은 사용자가 고르기 전에는 접속하지 않으므로(wake) 화면에 빈 창으로 남는다.
 * 대신 놓은 세션을 돌려준다. 넘겨받을 탭이 없거나 s가 보이던 탭이 아니면 null(부른 쪽이 그 창을 없앤다. 남은 탭은 방의 다른 창으로 간다).
 */
function handOffTab(s) {
  const leaf = roomLeaf(s);
  if (!leaf) return null;
  const tabs = tabsOf(s);
  const at = tabs.indexOf(s);
  const onStage = roomShown(s.group);
  const fit = (x) => !!x && (!onStage || !x.dormant);
  const heir = [tabs[at + 1], tabs[at - 1]].find(fit) || tabs.slice(at + 1).find(fit) || tabs.slice(0, at).reverse().find(fit) || null;
  if (heir) leaf.s = heir;
  return heir;
}

/**
 * 탭 s를 화면에 올린다: 방 안에서 그 탭의 묶음이 놓인 창에 s를 보이고, 방이 화면에 없으면 방을 지금 창 자리에 놓는다.
 * 방을 놓으면 화면의 창이 한도를 넘을 때는 놓지 않고 거짓을 돌려준다(방 안에서 보일 탭만 바꿔 둔다).
 */
function placeTab(s) {
  settleRoom(s.group);
  const leaf = setLeaf(s);
  if (leaf) leaf.s = s;
  return placeInActive(s);
}

/** 탭의 ×와 탭 메뉴로 SSH 세션을 닫는다. 정말 닫을지 먼저 묻는다. */
function askCloseTab(s) {
  if (!sessions.has(s.key)) return;
  const nick = [s.name, s.ssh.name].find((text) => text && text !== sshServer(s));
  const what = `${nick ? `${nick} · ` : ''}${s.sshUser ? `${s.sshUser}@` : ''}${sshServer(s)}`;
  const live = s.id != null && !s.exited && !s.dormant;
  if (!confirm(`SSH 세션 "${what}"을(를) 닫을까요?${live ? '\n이 세션의 접속을 끊습니다(복제한 다른 세션은 그대로 둡니다).' : ''}`)) return;
  closeSession(s);
}

/** SSH 세션의 탭(창 위쪽 탭 줄에 놓인다): [이름] 사용자@서버 ×. 누르면 그 탭으로, 끌면 옮기거나 나눠 열고, 오른쪽 클릭하면 메뉴. */
function sshTabFor(s) {
  if (s.sshTab) return s.sshTab;
  const tab = make('span', 'ssh-tab');
  const text = make('span', 'ssh-tab-label');
  const close = make('button', 'ssh-tab-close', '×');
  close.title = '이 SSH 세션 닫기 (닫기 전에 묻습니다)';
  close.onmousedown = (ev) => ev.stopPropagation(); // 끌기를 시작하지 않는다.
  close.onclick = (ev) => { ev.stopPropagation(); askCloseTab(s); };
  tab.append(text, close);
  tab.dataset.key = s.key;
  tab.onmousedown = (ev) => {
    if (ev.button !== 0 || ev.target.closest('button')) return;
    cancelDrag();
    drag = { s, tab: true, x: ev.clientX, y: ev.clientY, active: false }; // 탭을 끈다: SSH 방 안에서만 옮긴다.
  };
  tab.onclick = (ev) => {
    if (ev.target.closest('button') || performance.now() - draggedAt < 300) return; // 끌다가 제자리에 놓은 것
    activate(s);
  };
  tab.oncontextmenu = (ev) => { ev.preventDefault(); ev.stopPropagation(); sessionMenu(s, ev); };
  s.sshTab = tab;
  s.sshTabText = text;
  return tab;
}

/** 창 위쪽의 탭 줄을 그 창에 보이는 세션의 묶음에 맞춘다. 같은 탭이면 그대로 둔다(누르는 사이에 바뀌면 그 클릭이 먹히지 않는다). */
function paintTabStrip(s) {
  const strip = s.tabStrip;
  if (!strip) return;
  const tabs = tabsOf(s);
  const wanted = tabs.map(sshTabFor);
  for (const t of tabs) paintSshTab(t);
  const now = [...strip.children];
  if (wanted.length === now.length && wanted.every((el, i) => el === now[i])) return;
  strip.replaceChildren(...wanted);
  revealTab(s);
}

/** 지금 탭이 보이게 탭 줄을 민다(탭이 많아 줄이 넘칠 때). 탭이 바뀔 때와 창 크기가 바뀔 때 부른다. */
function revealTab(s) {
  const strip = s.tabStrip, el = s.sshTab;
  if (!strip || !el || el.parentElement !== strip || !strip.clientWidth) return;
  if (el.offsetLeft < strip.scrollLeft) strip.scrollLeft = el.offsetLeft;
  else if (el.offsetLeft + el.offsetWidth > strip.scrollLeft + strip.clientWidth) strip.scrollLeft = el.offsetLeft + el.offsetWidth - strip.clientWidth;
}

/** 모든 종류의 창에 공통인 동작: 나란히 보기에서 창을 누르면 그 창이 활성화된다. */
function wirePane(s) {
  // 쉬고 있는 세션의 창(재개하지 못해 다시 쉬는 세션 등)은 보고 있던 것이어도 누르면 재개한다.
  s.pane.addEventListener('mousedown', () => { if (active !== s || s.dormant) activate(s); }, true);
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
// 화면 배치는 나무 구조다. 잎은 세션 하나({ s })나 SSH 방 하나({ room: 워크스페이스 이름 }), 가지는 좌우(row)나 상하(col)로 나눈 둘
// ({ dir, a, b }). SSH 방은 그 안에 제 나무(rooms)를 가진다: 큰 배치에서는 한 자리지만 화면에는 방 안의 창이 모두 보인다.
// `visible`은 이 나무의 잎(방은 방 안의 잎)을 화면 순서대로 늘어놓은 목록이고, layout()이 나무에서 다시 만든다.

let tree = null;

const leavesOf = (node) => (!node ? [] : node.room ? leavesOf(rooms.get(node.room) || null) : node.s ? [node.s] : [...leavesOf(node.a), ...leavesOf(node.b)]);

/** 그 세션이 보이는 잎. 큰 배치에서 찾으면 SSH 세션은 방 안의 잎이 나온다(방이 화면에 있을 때). */
function findLeaf(s, node = tree) {
  if (!node) return null;
  if (node.room) return findLeaf(s, rooms.get(node.room) || null);
  if (node.s) return node.s === s ? node : null;
  return findLeaf(s, node.a) || findLeaf(s, node.b);
}

/** 그 세션의 잎을 뺀 나무. 짝이던 쪽이 빈자리를 다 차지한다. 방 안은 들여다보지 않는다(방 안의 창은 takeOut이 뺀다). */
function without(node, s) {
  if (!node) return null;
  if (node.room) return node;
  if (node.s) return node.s === s ? null : node;
  const a = without(node.a, s), b = without(node.b, s);
  return a && b ? { dir: node.dir, ratio: node.ratio, a, b } : (a || b);
}

/** 큰 배치의 한 자리에 놓이는 것: 세션 하나({ s }), SSH 세션이면 그 워크스페이스의 SSH 방({ room }). */
const slotFor = (s) => (s.ssh ? { room: s.group } : { s });
const holds = (node, content) => (content.room ? node.room === content.room : !!content.s && node.s === content.s);

/** 큰 배치에서 그 내용이 놓인 마디(화면에 없으면 null). */
function findSlot(content, node = tree) {
  if (!node) return null;
  if (node.room || node.s) return holds(node, content) ? node : null;
  return findSlot(content, node.a) || findSlot(content, node.b);
}

/** 그 내용의 자리를 뺀 큰 배치. */
function withoutSlot(node, content) {
  if (!node) return null;
  if (node.room || node.s) return holds(node, content) ? null : node;
  const a = withoutSlot(node.a, content), b = withoutSlot(node.b, content);
  return a && b ? { dir: node.dir, ratio: node.ratio, a, b } : (a || b);
}

/** 나무의 마디 하나를 다른 내용으로 바꾼다(그 마디를 가리키는 가지는 그대로 둔다). */
function become(node, content) {
  for (const key of Object.keys(node)) delete node[key];
  Object.assign(node, content);
}

/** 마디 하나(세션 잎이나 SSH 방)의 자리를 둘로 나눠 한쪽(side: left | right | top | bottom)에 내용을 놓는다. */
function splitNode(node, content, side) {
  if (!node) { tree = tree ? { dir: 'row', a: tree, b: content } : content; return; }
  const old = { ...node };
  const first = side === 'left' || side === 'top';
  become(node, { dir: side === 'left' || side === 'right' ? 'row' : 'col', a: first ? content : old, b: first ? old : content });
}

/**
 * s가 놓인 창 자리를 없앤다(세션은 그대로 둔다). SSH 세션이면 방 안에서 그 세션이 보이던 창을 없애고,
 * 그래서 방이 비면 방도 화면에서 뺀다.
 */
function takeOut(s) {
  if (!s.ssh) { tree = without(tree, s); return; }
  for (const [group, inner] of rooms) {
    if (!findLeaf(s, inner)) continue;
    const left = without(inner, s);
    if (left) rooms.set(group, left);
    else { rooms.delete(group); tree = withoutSlot(tree, { room: group }); }
  }
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

/**
 * 지금 창의 자리에 s를 놓는다(있던 것은 화면에서만 빠지고 계속 살아 있다). SSH 세션이면 그 워크스페이스의 SSH 방을 놓고
 * (방이 이미 화면에 있으면 그대로 둔다), 지금 창이 SSH 창이면 그 방 전체의 자리에 놓는다(방은 통째로 화면에서 빠진다).
 */
function placeInActive(s) {
  const content = slotFor(s);
  if (findSlot(content)) return true; // 이미 화면에 있다.
  // 지금 고른 것이 화면에 없으면 마지막 창 자리에 놓는다(화면은 나누지 않는다: 창 분할은 사용자가 끌어다 놓을 때만).
  const shownSlot = (x) => (x && sessions.has(x.key) ? findSlot(slotFor(x)) : null);
  const at = shownSlot(active) || shownSlot(leavesOf(tree).pop());
  // SSH 방은 창이 여럿일 수 있다: 그 자리에 놓았을 때 화면의 창이 한도를 넘으면 놓지 않는다.
  if (leavesOf(tree).length - (at ? panesOf(at) : 0) + panesOf(content) > MAX_PANES) return false;
  if (at) become(at, content); else splitNode(null, content);
  return true;
}

/** 큰 배치의 한 자리가 화면에서 차지하는 창 수: 세션은 하나, SSH 방은 방 안의 창 수. */
const panesOf = (content) => (content.room ? Math.max(1, leavesOf(rooms.get(content.room) || null).length) : 1);

/** 세션 여러 개를 고르게 나눈 배치. */
function balanced(list, dir = 'row') {
  if (list.length < 2) return list.length ? { s: list[0] } : null;
  const mid = Math.ceil(list.length / 2), next = dir === 'row' ? 'col' : 'row';
  return { dir, a: balanced(list.slice(0, mid), next), b: balanced(list.slice(mid), next) };
}

/** 나무대로 창을 배치한다. 화면에 없는 세션의 창은 보이지 않는 보관 칸으로 옮겨 둔다. */
function layout() {
  // 창을 떼었다 붙이면 그 안에 있던 초점이 사라진다. 배치를 다시 지은 뒤에 되돌린다.
  const focused = document.activeElement;
  // 창을 떼었다 붙이면 그 안의 스크롤이 맨 위로 돌아간다. 보던 자리를 스스로 지키는 항목(코드 보기)에 미리 알린다.
  for (const s of visible) { try { s.view?.freeze?.(); } catch { /* 그 화면의 문제로 배치가 멈추지 않게 */ } }
  settleRooms(); // SSH는 워크스페이스마다 방 하나, 한 탭 묶음은 한 창에만 보인다.
  visible = leavesOf(tree);
  for (const s of sessions.values()) {
    const shown = visible.includes(s);
    s.pane.classList.toggle('hidden', !shown);
    if (!shown && s.pane.parentElement !== $('parked')) $('parked').append(s.pane);
  }
  roomBoxes.clear();
  const build = (node) => {
    if (node.room) {
      // SSH 방: 방 안의 창들을 상자 하나에 담는다. 큰 배치에서는 이 상자가 한 칸이다.
      const box = make('div', 'ssh-room');
      const room = rooms.get(node.room);
      if (room) {
        const inner = build(room);
        inner.style.flex = ''; // 방 안의 맨 위 칸은 상자를 다 쓴다(예전 비율이 남아 있지 않게).
        box.append(inner);
      }
      roomBoxes.set(node.room, box);
      return box;
    }
    if (node.s) return node.s.pane;
    const box = make('div', `split split-${node.dir}`);
    const first = build(node.a), second = build(node.b);
    // 둘 사이의 경계선: 끌어서 크기를 조절하고, 두 번 누르면 반반으로 되돌린다.
    const bar = make('div', 'divider');
    bar.title = '끌기: 크기 조절 · 두 번 누르기: 반반';
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
    : groupList.length
      ? '워크스페이스 머리의 +를 눌러 세션이나 메모·HTTP·사이트·문서·할 일 같은 항목을 더하세요.'
      : '왼쪽 위의 [+ 워크스페이스]로 워크스페이스를 만들고, 워크스페이스 머리의 +로 세션이나 항목을 더하세요.';
  syncComposer();
  markBroadcast();
  syncSites(); // 사이트 항목의 웹뷰를 새 창 자리에 맞춘다.
  if (focused && focused !== document.activeElement && focused !== document.body && $terms.contains(focused)) {
    focused.focus({ preventScroll: true });
  }
  requestAnimationFrame(() => {
    fitVisible();
    visible.filter(isTerm).forEach(resyncScroll);
    // 명령 항목처럼 자기 터미널을 가진 항목도 스크롤 막대를 다시 맞춘다.
    for (const s of visible) s.view?.resync?.();
  });
  // 쉬는 세션이 화면에 올라왔으면(누르기, 끌어다 놓기) 재개한다. SSH 세션은 여기서 접속하지 않는다(사용자가 고른 탭만. wake).
  for (const s of visible) wake(s);
  markBroadcast(); // 방금 재개한 세션도 동시 입력을 받는다.
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
  if (fitHeld) return; // 프롬프트 편집기의 높이를 끄는 중: 손이 멈추면 한 번에 맞춘다(settleResize).
  for (const s of visible) {
    try { s.fitAddon?.fit(); } catch { /* 아직 크기를 잴 수 없는 상태 */ }
  }
}

/**
 * 끌어온 것을 창 위에 놓았을 때. 화면을 나누는 것은 이것뿐이다(사용자가 끌어다 놓을 때만).
 * `d`는 끌던 것({ s, tab?, room? }), `hit`은 놓은 자리(dropZoneAt이 준 { target, zone, before }).
 *
 * - SSH 탭(`d.tab`): 그 워크스페이스의 SSH 방 안에서만 옮긴다. 창의 가장자리에 놓으면 방 안에서 그 창을 좌우·상하로 나눠 그쪽에 열고,
 *   탭 줄이나 가운데에 놓으면(`tabs`) 그 창의 탭 묶음에 넣는다(`before` 탭 앞에, null이면 맨 끝, 주지 않으면 지금 탭 바로 뒤).
 *   창에 보이던 탭을 옮기면 원래 창에는 옆 탭이 남고, 혼자 쓰던 창이면 그 창이 없어진다.
 * - 사이드바의 항목(SSH 줄이면 방 통째. `d.room`): 가장자리에 놓으면 그 자리를 나눠 그쪽에 열고, 가운데에 놓으면 그 자리에 연다
 *   (이미 화면에 있던 것이면 둘의 자리를 바꾼다). SSH 창 위에서는 방 전체가 한 자리다.
 */
function dropOnPane(d, hit) {
  const { s } = d;
  let { target } = hit;
  const { zone } = hit;
  if (hit.blocked) return setStatus('SSH 탭은 같은 워크스페이스의 SSH 창 안에서만 옮깁니다. SSH를 통째로 옮기려면 사이드바의 SSH 줄을 끌어다 놓으세요.');
  if (hit.full) return setStatus(`한 화면에 볼 수 있는 창은 ${MAX_PANES}개까지입니다.`);
  if (d.tab) {
    if (zone === 'tabs') {
      // 다른 창에 보이던 탭이면 그 창을 옆 탭에 넘기고(옆 탭이 없으면 그 창을 없애고) 이 창의 묶음으로 옮긴다.
      if (s.tabSet !== target.tabSet && roomLeaf(s) && !handOffTab(s)) takeOut(s);
      joinTabs(s, target, hit.before);
      const leaf = roomLeaf(target);
      if (leaf) leaf.s = s;
    } else {
      if (roomLeaf(s) && tabsOf(s).length > 1) {
        // 보이던 탭을 떼어 나눠 연다: 그 창에는 옆 탭이 남는다. 자기 창의 가장자리에 놓았으면 남은 탭의 창을 나눈다.
        const heir = handOffTab(s);
        // 제 창의 가장자리에 놓았는데 그 창을 넘겨받을 탭이 없으면(옆 탭이 모두 쉬는 중) 나눌 것이 없다.
        if (target === s && !heir) return;
        if (target === s) target = heir;
        leaveTabs(s);
      } else if (!roomLeaf(s)) {
        leaveTabs(s); // 가려져 있던 탭: 묶음에서 떼어 새 창으로 연다.
      }
      if (s === target) return;
      takeOut(s); // 혼자 쓰던 창을 옮기는 것이면 원래 자리를 없앤다.
      splitNode(roomLeaf(target), { s }, zone);
    }
  } else {
    const content = slotFor(s);
    const from = findSlot(content), to = findSlot(slotFor(target));
    if (!to || from === to) return;
    if (zone === 'center') {
      const old = { ...to };
      become(to, content);
      if (from) become(from, old);
    } else {
      tree = withoutSlot(tree, content);
      splitNode(findSlot(slotFor(target)), content, zone);
    }
  }
  layout();
  activate(s);
}

/**
 * 끄는 것(d: { s, tab?, room? })을 그 위치에 놓으면 어떻게 되는지: { target, rect, zone, before, blocked?, full? }.
 * SSH 탭은 같은 워크스페이스의 SSH 창 위에서만 놓인다: 탭 줄에 놓으면 그 탭 묶음의 그 자리(`before` 탭 앞)에, 창 가운데에 놓으면
 * 지금 탭 옆에 넣고, 가장자리에 놓으면 방 안에서 나눠 연다. 다른 곳 위에서는 놓을 수 없다(`blocked`).
 * 사이드바에서 끌어온 것은 SSH 창 위에서 방 전체를 한 자리로 본다.
 */
function dropZoneAt(x, y, d) {
  const s = d?.s;
  if (!s) return null;
  if (d.tab) {
    const head = document.elementFromPoint(x, y)?.closest?.('.pane.ssh > .pane-head');
    const onHead = head && visible.find((v) => v.head === head);
    if (onHead?.tabSet && onHead.group === s.group) {
      const before = tabsOf(onHead).find((t) => {
        const r = t.sshTab?.getBoundingClientRect();
        return r && x < r.left + r.width / 2;
      }) ?? null;
      return { target: onHead, rect: head.getBoundingClientRect(), zone: 'tabs', before };
    }
    const hit = paneZoneAt(x, y);
    if (!hit) return null;
    if (!hit.target.ssh || hit.target.group !== s.group) return { ...roomZone(hit, x, y), zone: 'center', blocked: true };
    if (hit.zone === 'center') return hit.target === s ? null : { ...hit, zone: 'tabs', before: undefined };
    // 가장자리: 창이 하나 늘어나는지(가려져 있던 탭을 꺼내거나, 보이던 탭을 옆 탭들에서 떼어 내거나). 혼자 쓰던 창은 자리만 옮긴다.
    const adds = !roomLeaf(s) || tabsOf(s).length > 1;
    if (!adds && hit.target === s) return null; // 혼자 쓰는 제 창의 가장자리: 옮길 곳이 없다.
    if (adds && visible.length >= MAX_PANES) return { ...hit, full: true };
    return hit;
  }
  let hit = paneZoneAt(x, y);
  if (!hit) return null;
  if (hit.target.ssh) {
    if (d.room && hit.target.group === d.room) return null; // 제 방 위에 놓았다.
    hit = roomZone(hit, x, y);
  }
  // 놓으면 창이 늘어나는데 한도를 넘으면, 놓을 수 없다고 미리 보여 준다(가운데에 놓으면 그 자리에 있던 것은 화면에서 빠진다).
  const adds = (d.room ? roomShown(d.room) : visible.includes(s)) ? 0 : panesOf(slotFor(s));
  const frees = hit.zone === 'center' ? panesOf(slotFor(hit.target)) : 0;
  if (adds && visible.length + adds - frees > MAX_PANES) return { ...hit, full: true };
  return hit;
}

/** 그 위치에 있는 창과, 창 안에서의 구역(left | right | top | bottom | center). */
function paneZoneAt(x, y) {
  const el = document.elementFromPoint(x, y)?.closest?.('.pane');
  const target = el && visible.find((v) => v.pane === el);
  return target ? zoneIn(target, el.getBoundingClientRect(), x, y) : null;
}

/** 네모(rect) 안에서 그 위치가 어느 구역인지: 가장자리에 가까우면 그쪽, 아니면 가운데. */
function zoneIn(target, rect, x, y) {
  const fx = (x - rect.left) / rect.width, fy = (y - rect.top) / rect.height;
  const [nearest, distance] = [['left', fx], ['right', 1 - fx], ['top', fy], ['bottom', 1 - fy]].sort((p, q) => p[1] - q[1])[0];
  return { target, rect, zone: distance < 0.3 ? nearest : 'center' };
}

/** SSH 창 위의 자리를 그 창이 든 SSH 방 전체로 넓힌다(방 밖의 것에게는 방 전체가 한 자리다). */
function roomZone(hit, x, y) {
  const box = hit.target.ssh ? roomBoxes.get(hit.target.group) : null;
  return box?.isConnected ? zoneIn(hit.target, box.getBoundingClientRect(), x, y) : hit;
}

const DROP_TEXT = { left: '왼쪽에 나눠 열기', right: '오른쪽에 나눠 열기', top: '위에 나눠 열기', bottom: '아래에 나눠 열기', center: '이 자리에 열기', tabs: '이 창의 탭으로 넣기' };

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
    tabs: [r.left, r.top, r.width, r.height],
  }[zone];
  Object.assign(hint.style, { left: `${box[0]}px`, top: `${box[1]}px`, width: `${box[2]}px`, height: `${box[3]}px` });
  hint.classList.toggle('blocked', !!(hit.blocked || hit.full));
  hint.textContent = hit.blocked ? 'SSH 탭은 SSH 창 안에서만 옮깁니다'
    : hit.full ? `한 화면에 볼 수 있는 창은 ${MAX_PANES}개까지입니다` : DROP_TEXT[zone];
}

/** 항목을 고른다(화면에 없으면 지금 창 자리에 놓는다). `keepFocus`이면 초점을 옮기지 않는다(사이드바에서 방향키로 오갈 때). */
function activate(s, { keepFocus = false } = {}) {
  if (!s || !sessions.has(s.key)) return; // 기다리는 사이 닫힌 항목(넘기기, 워크스페이스 틀 펼치기)은 화면에 올리지 않는다.
  if (!visible.includes(s)) {
    // 고른 것이 쉬는 SSH 탭이어도 그 창에 보이는 탭으로 둔다(settleRoom은 쉬는 탭을 깨어 있는 탭 뒤로 돌린다). 아래에서 곧 접속한다.
    picked = s;
    try {
      // SSH 탭은 SSH 방 안에서 그 묶음의 창에 보인다(방이 화면에 없으면 방이 지금 창 자리에 놓인다).
      if (!(s.tabSet ? placeTab(s) : placeInActive(s))) {
        // SSH 방의 창이 여럿이라 그 자리에 놓으면 화면의 창이 한도를 넘는다: 놓지 않고 알린다.
        setStatus(`SSH 창 ${panesOf(slotFor(s))}개를 여기에 열면 화면의 창이 ${MAX_PANES}개를 넘습니다. 다른 창을 숨기거나, 사이드바의 SSH 줄을 오른쪽 클릭 → "SSH만 보기"로 여세요.`);
        return;
      }
      layout();
    } finally {
      picked = null;
    }
  }
  // 찾기 줄이 열려 있었으면 보던 세션의 찾기 강조를 지운다(아래 closeSearch는 새로 고른 세션의 것을 지운다).
  if (active !== s && !$search.hidden) active?.search?.clearDecorations();
  active = s;
  wake(s, true); // 쉬고 있던 세션이면 여기서 재개한다.
  s.attention = false;
  s.usedAt = Date.now();
  markRead(s.key); // 이 항목을 열었으니 이 항목이 남긴 알림은 본 것이다.
  for (const other of sessions.values()) other.pane.classList.toggle('active', other === s);
  renderShots();
  if (!$search.hidden) closeSearch();
  showDiff(s);
  syncComposer();
  if (!keepFocus) focusItem(s);
  render();
  saveState();
}

/**
 * 화면에서 빼거나(세션은 계속 살아 있다) 화면에 올린다. 올릴 때는 화면을 나누지 않고 지금 창 자리에 놓는다
 * (창 분할은 사용자가 끌어다 놓을 때만 한다).
 */
function toggleShown(s) {
  if (!visible.includes(s)) {
    activate(s);
  } else if (s.ssh) {
    // SSH 창: 방에 다른 창이 있으면 이 창만 접는다(이 창의 탭들은 옆 창의 탭 줄로 간다). 방에 하나뿐인 창이면 SSH 방을 화면에서 뺀다.
    const next = foldPane(s);
    if (!next) return toggleRoom(s.group);
    layout();
    // 탭을 받은 창에 지금 보이는 탭을 고른다. 쉬는 탭은 고르지 않는다(고르면 접속한다. 접속은 사용자가 고른 탭만).
    const shown = tabsOf(next).find((x) => visible.includes(x) && !x.dormant) || stageFallback();
    if ((active === s || !visible.includes(active)) && shown) activate(shown); else { render(); saveState(); }
  } else {
    // 화면에서만 숨긴다. 마지막 창도 숨길 수 있고, 그때는 화면이 비고 세션은 사이드바에 남는다.
    tree = without(tree, s);
    layout();
    const next = active === s ? stageFallback() : null;
    if (next) activate(next); else { render(); saveState(); }
  }
}

/**
 * 닫은 항목을 다시 열 수 있게 적어 둘 내용. 되살릴 것이 없으면(빈 메모, 빈 페이지, 시작하지 못한 세션) null.
 * 닫기 전에 만들어야 한다(닫은 뒤에는 화면이 제목과 상태를 내놓지 못한다).
 */
function closedRecord(s) {
  if (isTerm(s) && s.id == null && !s.dormant) return null;
  if (s.kind === 'memo' && !s.view.text().trim() && !s.view.state().path) return null;
  if (s.kind === 'image' && s.view.empty()) return null;
  if (s.kind === 'site' && !s.view.url()) return null;
  if (s.kind === 'reader' && !s.view.path()) return null;
  if (s.kind === 'todo' && !s.view.text().trim()) return null;
  if (s.kind === 'run' && !s.view.command()) return null;
  if (s.kind === 'table' && !s.view.path()) return null;
  if (s.kind === 'hex' && !s.view.path()) return null;
  if (s.kind === 's3' && !s.view.conn()) return null;
  try {
    return { ...describe(s), label: label(s) };
  } catch {
    return null;
  }
}

/**
 * 항목을 닫는다. `forget`이면 닫은 항목 목록에 남기지 않는다(다시 시작하려고 닫을 때).
 * `still`이면 화면에 없던 항목을 닫을 때 나란히 보기의 배치를 다시 짓지 않는다(앱이 스스로 걷는 항목 때문에
 * 사용자가 보던 창이 떼였다 붙어 초점과 문서의 스크롤 자리가 날아가지 않게. 숨겨서 만들 때와 같은 까닭이다).
 */
async function closeSession(s, { forget = false, still = false } = {}) {
  // 처음 읽기가 끝난 뒤의 내용으로 기록을 만든다(읽는 중에 닫으면 빈 화면을 보고 "다시 열 것이 없다"고 하게 된다).
  if (s.view) await Promise.resolve(s.view.ready).catch(() => {});
  if (!sessions.has(s.key)) return; // 기다리는 사이 이미 닫혔다.
  const record = forget ? null : closedRecord(s);
  if (s.view) {
    // 메모·HTTP: 닫기 전에 저장할 것을 저장한다. 거짓이면 사용자가 닫기를 취소한 것이다.
    if (!(await s.view.close())) return;
    s.view.dispose();
  } else {
    // 응답 도중에 닫은 세션이 만들던 파일은 놓지 않는다(닫은 뒤에 항목이 생기거나, 지운 워크스페이스가 되살아나지 않게).
    clearTimeout(s.heldFilesTimer);
    s.heldFiles?.clear();
    if (s.id != null && !s.exited) invoke('session_close', { id: s.id });
    s.term.dispose();
  }
  if (!sessions.has(s.key)) return; // 기다리는 사이 이미 닫혔다.
  if (record) {
    rememberClosed(record);
    setStatus(`닫았습니다: ${record.label} · 다시 열기: Ctrl+Shift+R`);
  }
  const shown = visible.includes(s);
  s.pane.remove();
  s.shots.forEach((shot) => URL.revokeObjectURL(shot.url));
  // 창에 보이던 SSH 탭이면 그 창에 옆 탭을 대신 보인다(브라우저의 탭처럼). 그다음 탭 묶음에서 뺀다.
  // 넘겨받을 탭이 없으면(옆 탭이 없거나 모두 쉬는 중) 그 창이 없어지고, 방에 남은 창이 없으면 SSH 방도 화면에서 빠진다
  // (쉬는 탭은 사이드바의 SSH 줄에 남아 있고, 누르면 다시 접속한다).
  const heir = s.tabSet ? handOffTab(s) : null;
  if (s.tabSet) s.tabSet.list = s.tabSet.list.filter((x) => x !== s);
  sessions.delete(s.key);
  takeOut(s);
  visible = leavesOf(tree);
  if (active === s) {
    active = null;
    // 다음에 볼 항목을 고른다. 쉬는 세션은 화면에 있어도 고르지 않는다(고르면 재개된다. 재개는 사용자가 누른 세션만 한다).
    // 닫은 탭의 자리를 받은 옆 탭이 화면에 있으면 그것을, 아니면 화면에 있는 것을, 화면이 비었으면 같은 워크스페이스의 것을 먼저 고른다.
    const awake = [...sessions.values()].filter((other) => !other.dormant);
    const next = (heir && !heir.dormant && visible.includes(heir) ? heir : null) || stageFallback() || awake.filter((other) => other.group === s.group).pop() || awake.pop();
    if (next) activate(next);
  }
  if (!still || shown || visible.length < 2) layout();
  renderShots();
  render();
  saveState();
}

/**
 * 종료된 세션을 같은 자리에서 다시 시작한다. Claude 세션은 하던 대화를 이어 가고, 다른 터미널은 같은 폴더에서 새로 뜬다.
 * 사이드바의 순서, 화면에서의 자리, 별칭·색·워크스페이스는 그대로 둔다.
 */
async function restartSession(s) {
  if (!isTerm(s) || !sessions.has(s.key)) return;
  const all = [...sessions.values()];
  const after = all[all.indexOf(s) + 1];
  const fresh = await revive(describe(s), { show: 'hidden' });
  if (!fresh || !sessions.has(s.key)) return;
  if (after && sessions.has(after.key)) placeInOrder(fresh, after, false);
  // 북마크는 한 숫자에 항목 하나라 새 세션에는 비어 있다. 옛 세션의 것을 넘겨준다.
  if (s.mark) { fresh.mark = s.mark; s.mark = ''; }
  // SSH 탭 묶음에서도 원래 자리(바로 뒤)를 이어받는다. 옛 세션은 아래에서 닫으며 묶음에서 빠진다.
  if (s.tabSet && fresh.tabSet) joinTabs(fresh, s);
  // 옛 세션이 보이던 창을 새 세션이 받는다(SSH는 방이 화면에 없어도 방 안의 그 창을 받는다).
  const shown = visible.includes(s);
  const leaf = s.ssh ? roomLeaf(s) : findLeaf(s);
  if (leaf) leaf.s = fresh;
  const wasActive = active === s;
  await closeSession(s, { forget: true });
  if (wasActive || shown) activate(fresh); else render();
}

// ── 북마크: 항목에 숫자(0~9)를 붙여 두고 Ctrl+숫자로 바로 간다 ──
// 숫자 하나에 항목 하나다. Ctrl+Shift+숫자(또는 오른쪽 클릭 메뉴)로 지금 항목에 붙이고, 같은 숫자를 다시 누르면 푼다.
// 다른 항목이 쓰던 숫자를 붙이면 그 항목에서 옮겨 온다. 다시 시작해도 남고, 닫은 항목을 다시 열면 그 숫자가 비어 있을 때 되돌아온다.

const MARKS = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0']; // 글쇠판에 놓인 차례

/** 그 숫자가 붙은 항목(없으면 undefined). */
const markedItem = (digit) => [...sessions.values()].find((s) => s.mark === digit);
/** 새로 만드는 항목에 줄 북마크: 숫자 한 자리이고 다른 항목이 쓰고 있지 않을 때만. */
const freeMark = (digit) => (typeof digit === 'string' && /^\d$/.test(digit) && !markedItem(digit) ? digit : '');

/** 사이드바 줄과 창 제목 줄에 보이는 북마크 숫자. */
function markBadge(digit) {
  const badge = make('b', 'mark', digit);
  badge.title = `북마크 ${digit}: Ctrl+${digit}로 바로 가기`;
  return badge;
}

/** 오른쪽 클릭 메뉴의 숫자 단추에 붙는 풍선말. */
function markTitle(s, digit) {
  const owner = markedItem(digit);
  if (s.mark === digit) return `북마크 ${digit} 풀기`;
  return owner ? `북마크 ${digit} (지금은 "${label(owner)}"의 것 — 이 항목으로 옮김)` : `북마크 ${digit}로 지정 (Ctrl+Shift+${digit})`;
}

/** 그 항목에 북마크 숫자를 붙인다. 이미 그 숫자면 풀고, 다른 항목이 쓰던 숫자면 옮겨 온다. */
function setMark(s, digit) {
  if (!s || !sessions.has(s.key)) return setStatus('북마크할 항목이 없습니다. 먼저 항목을 고르세요.');
  if (s.mark === digit) {
    s.mark = '';
    setStatus(`북마크 ${digit}을(를) 풀었습니다: ${label(s)}`);
  } else {
    const owner = markedItem(digit);
    if (owner) owner.mark = '';
    s.mark = digit;
    setStatus(`북마크 ${digit}: ${label(s)} — Ctrl+${digit}로 바로 갑니다.${owner ? ` ("${label(owner)}"에서 옮겼습니다)` : ''}`);
  }
  render();
  return saveState();
}

/** 그 숫자의 북마크로 간다: 화면에 없으면 지금 창 자리에 띄우고, 쉬는 세션이면 재개한다. 갔으면 참. */
function jumpToMark(digit) {
  const s = markedItem(digit);
  if (!s) { setStatus(`북마크 ${digit}이(가) 없습니다. 항목을 보면서 Ctrl+Shift+${digit}을(를) 누르면 붙습니다.`); return false; }
  collapsed.delete(groupKey(s)); // 접어 둔 묶음 안에 있으면 펴서 사이드바에서도 보이게 한다.
  localStorage.setItem('cypher.collapsed', JSON.stringify([...collapsed]));
  activate(s);
  rowOf(s)?.scrollIntoView({ block: 'nearest' });
  return true;
}

/** 사이드바에 보이는 차례대로의 항목(접어 둔 묶음 안의 것도 포함). SSH 줄에서는 그 방의 탭을 창과 탭 줄에 놓인 차례로 잇는다. */
const inSidebarOrder = () => groupsInOrder().flatMap((group) => sidebarRows(group.list)
  .flatMap((row) => (row.list ? roomTabs(row.room).filter((s) => row.list.includes(s)) : [row.s])));

function cycle(step) {
  // 쉬는 세션은 건너뛴다(지나가기만 해도 재개되지 않게). 재개하려면 사이드바에서 누른다.
  const all = inSidebarOrder().filter((s) => !s.dormant);
  if (all.length < 2 || !active) return;
  activate(all[(all.indexOf(active) + step + all.length) % all.length]);
}

function focusPane(step) {
  // 쉬는 세션의 창은 건너뛴다(지나가기만 해도 재개되지 않게). 재개하려면 그 창이나 사이드바의 줄을 누른다.
  const panes = visible.filter((v) => !v.dormant || v === active);
  if (panes.length < 2 || !active) return;
  activate(panes[(panes.indexOf(active) + step + panes.length) % panes.length]);
}

/**
 * 동시 입력을 켜거나 끈다. 켜면 한 세션에 친 글쇠가 나란히 보이는 다른 세션에도 함께 들어간다.
 * `on`: 참(나란히 보이는 모든 터미널 세션) | 'ssh'(나란히 보이는 SSH 세션끼리만 — 여러 서버에 같은 명령을 넣을 때) | 거짓(끔)
 */
function setBroadcast(on) {
  broadcast = on === SSH ? SSH : on ? 'all' : false;
  markBroadcast();
  focusItem(active);
}

/**
 * SSH 세션이 아직 로그인 중인지(서버 키를 묻거나 비밀번호를 받는 중). 이런 세션은 동시 입력에서 뺀다:
 * 다른 창에 친 명령이 비밀번호 칸에 들어가거나, 여기에 친 비밀번호가 다른 서버의 셸에 명령으로 찍히지 않게.
 * 앱이 셸을 열면 `session-ready`로 알린다(ssh.exe로 띄운 세션은 띄우자마자 알린다).
 */
const loggingIn = (s) => !!s.ssh && s.sshReady === false;

/** 동시 입력에 넣을 수 있는 SSH 세션: 접속을 마치고 살아 있는 것. */
const castableSsh = (v) => !!v.ssh && v.id != null && !v.exited && !v.dormant && !loggingIn(v);

/** SSH 창 위쪽의 단추(출력 기록·동시 입력)를 지금 상태에 맞춘다. 창이 좁으면 CSS가 짧은 글(.short)이나 기호(.tiny)만 보인다. */
function paintSshButtons(s, targets = castTargets()) {
  // 글 조각(span)은 한 번만 만들고 글만 바꾼다: 다시 그릴 때마다 새로 만들면, 누르고 떼는 사이에 누른 조각이 사라져
  // 그 클릭이 먹히지 않는다(render는 활성 창이 바뀔 때나 400ms마다 상태가 바뀔 때도 돈다).
  const label = (button, ...texts) => {
    if (button.children.length !== 3) button.replaceChildren(make('span', 'full'), make('span', 'short'), make('span', 'tiny'));
    texts.forEach((text, i) => { if (button.children[i].textContent !== text) button.children[i].textContent = text; });
  };
  const why = s.exited ? '끝난 세션입니다' : s.dormant ? '쉬는 세션입니다(눌러 재개한 뒤에 쓸 수 있습니다)' : '';
  if (s.fontLabel) {
    const size = String(sshFontNow());
    if (s.fontLabel.textContent !== size) s.fontLabel.textContent = size;
    s.fontLabel.classList.toggle('own', !!sshFontSize);
    s.fontLabel.title = `SSH 글자 크기 ${size}${sshFontSize ? '' : ' (앱의 터미널 글자 크기를 따름)'}\nSSH 세션에만 적용됩니다 · Ctrl+휠로도 바꿉니다\n누르면 기본 크기(설정 → 모양의 터미널 글자 크기)로 되돌립니다`;
  }
  if (s.logButton) {
    const on = !!s.logPath;
    s.logButton.classList.toggle('on', on);
    if (on) label(s.logButton, '■ 기록 중 · 끄기', '■ 기록 중', '■');
    else label(s.logButton, '● 출력 기록', '● 기록', '●');
    s.logButton.title = on
      ? `출력을 기록하는 중: ${s.logPath}\n누르면 기록을 멈춥니다`
      : why || '이 세션의 콘솔 출력을 파일로 저장하기\n파일을 고르면 그 뒤의 출력을 계속 적고, 다시 누르면 멈춥니다';
    s.logButton.disabled = !!(s.exited || s.dormant) && !on;
  }
  if (s.castButton) {
    // 이 세션이 입력을 받고 있을 때만 켜진 모양으로 보인다(끝난 창이 받는 것처럼 보이지 않게).
    const on = !!broadcast && targets.includes(s);
    const count = targets.length;
    s.castButton.classList.toggle('on', on);
    if (on && broadcast !== SSH) label(s.castButton, `≫ 모든 세션에 동시 입력 중 (${count}) · 끄기`, `≫ 모두 (${count})`, `≫${count}`);
    else if (on) label(s.castButton, `≫ 동시 입력 중 (${count}) · 끄기`, `≫ 동시 (${count})`, `≫${count}`);
    else label(s.castButton, '≫ 동시 입력', '≫ 동시', '≫');
    s.castButton.disabled = !on && (s.exited || s.dormant || s.id == null || loggingIn(s));
    s.castButton.title = on
      ? `동시 입력 끄기 (지금 ${count}개 세션에 함께 입력됩니다)`
      : why || (loggingIn(s)
        ? '로그인을 마친 뒤에 쓸 수 있습니다'
        : 'SSH 동시 입력 켜기: 여기에 치는 글쇠가 나란히 보이는 다른 SSH 창에도 함께 들어갑니다\n나란히 볼 SSH 창은 탭을 SSH 창의 가장자리로 끌어다 놓아 띄웁니다');
  }
}

// ───────────────────────── SSH 글자 크기 ─────────────────────────
// SSH 창 위쪽의 [가−] 크기 [가+]로 SSH 세션의 글자 크기만 따로 바꾼다(Ctrl+휠로도 바꾼다). 모든 SSH 세션이 함께 쓰고
// 이 컴퓨터에 기억한다. 정하지 않았으면 앱의 터미널 글자 크기(설정 → 모양)를 따른다. Claude·셸 세션에는 영향이 없다.

const SSH_FONT_KEY = 'cypher.ssh.fontSize';
const SSH_FONT_MIN = 6, SSH_FONT_MAX = 40;
let sshFontSize = 0; // 0이면 앱의 터미널 글자 크기를 따른다
try {
  const stored = Math.round(Number(localStorage.getItem(SSH_FONT_KEY)));
  if (stored >= SSH_FONT_MIN && stored <= SSH_FONT_MAX) sshFontSize = stored;
} catch { /* 읽지 못하면 앱의 크기를 따른다 */ }

/** SSH 세션의 지금 글자 크기(화면에 보이는 숫자). */
const sshFontNow = (look = currentLook()) => sshFontSize || Math.round(look.termSize);
/** 터미널에 줄 값: 화면 배율이 터미널 글자까지 키우지 않게 그만큼 나눈다(look.js의 termPixels와 같은 셈). */
const sshPixels = (look) => (sshFontSize ? Math.round((sshFontSize / look.zoom) * 100) / 100 : look.termPixels);

/** SSH 세션의 글자 크기를 정한다. `size`가 0이면 앱의 터미널 글자 크기를 따르게 되돌린다. */
function setSshFont(size) {
  const look = currentLook();
  const next = size ? Math.min(SSH_FONT_MAX, Math.max(SSH_FONT_MIN, Math.round(size))) : 0;
  const before = sshFontNow(look);
  // 한도에 걸려 크기가 그대로면 아무것도 바꾸지 않는다(앱의 크기를 따르던 것을 "따로 정한 크기"로 적지 않는다).
  if (next && next === before && next !== Math.round(size)) return setStatus(`SSH 글자 크기는 ${SSH_FONT_MIN}~${SSH_FONT_MAX} 사이로 정합니다 (지금 ${before}).`);
  sshFontSize = next;
  try {
    if (next) localStorage.setItem(SSH_FONT_KEY, String(next)); else localStorage.removeItem(SSH_FONT_KEY);
  } catch { /* 저장 공간 문제는 무시 */ }
  for (const s of sessions.values()) {
    if (!s.ssh || !s.term) continue;
    s.term.options.fontSize = sshPixels(look);
    paintSshButtons(s);
  }
  // 글자 크기가 바뀌면 줄·칸 수가 달라진다: 보이는 창을 다시 맞춰 서버에 새 크기를 알린다.
  let done = false;
  const refit = () => { if (!done) { done = true; fitVisible(); } };
  requestAnimationFrame(refit);
  setTimeout(refit, 120); // 창이 가려져 그리기 신호가 오지 않을 때를 대비한다.
  const now = sshFontNow(look);
  setStatus(!next ? `SSH 글자 크기: 기본(${now}) — 앱의 터미널 글자 크기(설정 → 모양)를 따릅니다.`
    : `SSH 글자 크기: ${now} (SSH 세션에만 적용 · Ctrl+휠로도 바꿉니다)`);
}

/** SSH 세션의 서버: 주소(기본이 아닌 포트를 정했으면 함께). */
const sshServer = (s) => `${s.ssh.target}${s.ssh.port ? `:${s.ssh.port}` : ''}`;

/**
 * SSH 탭 하나를 지금 상태에 맞춘다: [별칭이나 서버에 붙인 이름] 사용자@서버 (로그인 중·끝남) ×.
 * 사용자 이름은 셸이 열린 뒤에야 안다(접속할 때마다 세션 화면에서 묻고, 저장하지 않는다). 그전에는 서버만 보인다.
 * 그 창에 보이고 있는 탭은 `current`(터미널과 이어진 모양)다.
 */
function paintSshTab(s) {
  const tab = s.sshTab;
  if (!tab) return;
  tab.classList.toggle('current', visible.includes(s));
  const server = sshServer(s);
  const nick = [s.name, s.ssh.name].find((text) => text && text !== server) || '';
  const state = s.exited ? '끝남' : s.dormant ? '쉬는 중' : loggingIn(s) ? '로그인 중' : '';
  // 다시 그릴 때마다 조각을 새로 만들지 않는다(바뀐 것이 없으면 그대로 둔다).
  const key = JSON.stringify([nick, s.sshUser, server, state, s.sshBuiltin]);
  if (tab.dataset.paint === key) return;
  tab.dataset.paint = key;
  s.sshTabText.replaceChildren(
    ...(nick ? [make('span', 'ssh-tab-name', nick)] : []),
    ...(s.sshUser ? [make('span', 'ssh-tab-user', s.sshUser), make('span', 'ssh-tab-at', '@')] : []),
    make('span', 'ssh-tab-host', server),
    ...(state ? [make('span', 'ssh-tab-state', state)] : []),
  );
  const account = s.sshUser
    || (state === '로그인 중' ? '(로그인하면 보입니다)'
      : s.sshReady && !s.sshBuiltin ? '(Windows의 ssh.exe가 물어 받은 사용자 이름은 보이지 않습니다)' : '(모름)');
  tab.title = [
    ...(nick ? [nick] : []),
    `계정: ${account}`,
    `서버: ${s.ssh.target}${s.ssh.port ? ` · 포트 ${s.ssh.port}` : ''}`,
    ...(state ? [`상태: ${state}`] : []),
    '누르기: 이 탭 보기 · 끌기: SSH 창의 가장자리에 놓으면 나눠 보기, 다른 SSH 창의 탭 줄에 놓으면 그 창으로 옮기기 (SSH 창 안에서만)',
    '오른쪽 클릭: 별칭·색·세션 복제·닫기 · ×: 닫기',
  ].join('\n');
}

/** SSH 탭 메뉴의 맨 위에 적는 글: 계정@서버와, 복제가 다시 인증하는지. */
function sshTabCaption(s) {
  // 내장 SSH로 로그인을 마친 세션이면 그 연결에 채널만 더 연다. 아니면(끝남·쉬는 중·로그인 중·ssh.exe) 새로 접속한다.
  const reuse = s.id != null && !s.exited && !s.dormant && s.sshReady === true && s.sshBuiltin;
  return `${s.sshUser ? `${s.sshUser}@` : ''}${sshServer(s)} · ${reuse ? '복제는 같은 연결을 써서 다시 인증하지 않습니다' : '복제는 새로 접속해 다시 인증합니다'}`;
}

/**
 * SSH 창의 [동시 입력] 단추. 켜져 있으면 끈다. 나란히 보이는 접속 중인 SSH 세션이 둘 이상이면 켠다.
 * 화면을 스스로 나누지는 않는다(창 분할은 사용자가 끌어다 놓을 때만): 나란히 보이는 SSH 창이 없으면 끌어다 놓는 법을 알린다.
 */
function toggleSshCast(s) {
  if (broadcast) return setBroadcast(false);
  if (!castableSsh(s)) return setStatus(loggingIn(s) ? '로그인을 마친 뒤에 동시 입력을 켤 수 있습니다.' : '접속 중인 SSH 세션에서만 동시 입력을 켤 수 있습니다.');
  if (visible.filter(castableSsh).length < 2) {
    // 화면에 있는 다른 SSH 세션이 로그인 중이면 그것을 알린다(로그인을 마치면 다시 누르면 된다).
    if (visible.some((v) => v !== s && v.ssh && loggingIn(v) && !v.exited)) {
      return setStatus('다른 SSH 세션이 아직 로그인 중입니다. 로그인을 마친 뒤 다시 누르세요(로그인 중인 세션은 동시 입력에 넣지 않습니다).');
    }
    return setStatus('동시 입력은 나란히 보이는 SSH 창끼리 씁니다. SSH 탭을 SSH 창의 가장자리로 끌어다 놓아 나란히 띄운 뒤 누르세요.');
  }
  setBroadcast(SSH);
}

/** 지금 동시 입력을 받는 세션들: 나란히 보이는 살아 있는 터미널 세션(SSH 동시 입력이면 그 가운데 SSH 세션). 로그인 중인 SSH 세션은 뺀다. */
function castTargets() {
  if (!broadcast) return [];
  return visible.filter((v) => isTerm(v) && !v.exited && !v.dormant && !loggingIn(v) && (broadcast !== SSH || v.ssh));
}

/** 동시 입력을 받는 창을 표시하고(제목 줄의 색 줄, SSH 세션의 [동시 입력] 단추) 상태줄에 알린다. */
function markBroadcast() {
  const targets = castTargets();
  $terms.classList.toggle('broadcast', !!broadcast);
  for (const s of sessions.values()) {
    s.pane.classList.toggle('casting', targets.includes(s));
    paintSshButtons(s, targets);
  }
  $('status-mode').textContent = !broadcast ? ''
    : broadcast === SSH ? (targets.length ? `SSH 동시 입력 켜짐: 나란히 보이는 SSH 세션 ${targets.length}개에 함께 입력됩니다` : 'SSH 동시 입력 켜짐: 나란히 보이는 접속 중인 SSH 세션이 없습니다')
    : '동시 입력 켜짐: 나란히 보이는 모든 세션에 입력됩니다';
}

const byId = (id) => [...sessions.values()].find((s) => s.id === id);

const QUICK_EXIT_MS = 10000; // 띄운 지 이 안에 오류로 끝난 세션은 "띄우지 못한 것"으로 보고 실행 명령을 고치는 곳을 알려 준다.
/** 정해 둔 Claude 실행 명령으로 세션을 띄우지 못했을 때 터미널에 적는 안내. */
const commandHint = (command) => `\x1b[33mClaude 실행 명령으로 띄웠습니다: ${command}\r\n이 명령이 맞는지 확인하세요: 창 위쪽 메뉴의 설정 → "Claude 실행 명령…"에서 고치거나, 비우면 기본값(claude)으로 띄웁니다.\x1b[0m\r\n`;

/**
 * 누구의 것인지 아직 모르는 종료 알림(번호 → 종료 코드). 세션을 띄우라는 요청의 답(번호)보다 종료 알림이 먼저 닿으면
 * 그 번호의 세션을 찾지 못한다. 잠깐 받아 두었다가 번호를 받은 쪽이 찾아 쓴다(명령 항목의 종료 알림도 여기를 지나간다).
 */
const earlyExits = new Map();
/** 번호를 받기 전에 온 "셸이 열렸다"(session-ready) 알림: 번호 → { user, builtin }. */
const earlyReady = new Map();

/** SSH 세션의 로그인이 끝나 셸이 열렸다: 동시 입력에 넣을 수 있다. */
function onReady({ id, user, builtin }) {
  const s = byId(id);
  if (!s) {
    earlyReady.set(id, { user, builtin });
    setTimeout(() => earlyReady.delete(id), 10000);
    return;
  }
  markReady(s, { user, builtin });
}

/**
 * 셸이 열린 SSH 세션을 적어 둔다. 로그인한 사용자 이름은 창 위쪽의 탭에 보이기만 하고 저장하지 않는다
 * (사용자 이름은 접속할 때마다 묻는다). ssh.exe가 물어 받은 사용자 이름은 앱이 모른다(`user`가 비어 온다).
 */
function markReady(s, { user, builtin }) {
  s.sshReady = true;
  s.sshUser = user || '';
  s.sshBuiltin = !!builtin; // 내장 SSH면 연결 복제가 다시 인증하지 않는다.
  markBroadcast();
  paintSshTab(s);
}

function onExit({ id, code }) {
  const s = byId(id);
  if (!s) {
    earlyExits.set(id, code);
    setTimeout(() => earlyExits.delete(id), 10000);
    return;
  }
  s.exited = true;
  // 세션이 끝나면 앱이 기록 파일을 닫는다(끝난 줄과 마친 시각을 적는다).
  if (s.logPath) { s.lastLog = s.logPath; s.logPath = ''; }
  s.agents.clear();
  s.tools.clear();
  s.workStart = 0;
  s.interruptAt = 0;
  s.waiting = '';    // 끝난 세션이 "대기"로 남아 대기 수와 다음 대기로 가기(Ctrl+Shift+N)를 막지 않게 한다.
  s.heldStop = false;
  placeHeld(s);      // 끝나기 전에 만든 파일 가운데 최종 파일을 사이드바에 놓는다.
  markBroadcast();   // 끝난 세션은 동시 입력을 받지 않는다.
  s.term.write(`\r\n\x1b[90m[세션 종료${code == null ? '' : `: 코드 ${code}`} · 사이드바에서 오른쪽 클릭 → 다시 시작]\x1b[0m\r\n`);
  // 정해 둔 실행 명령으로 띄운 Claude 세션이 뜨자마자 오류로 끝났으면 그 명령이 문제일 수 있다.
  if (s.launch?.command && code && Date.now() - s.launch.at < QUICK_EXIT_MS) s.term.write(commandHint(s.launch.command));
  render();
  saveState();
}

/** 상태줄 신호로 바뀌는 것 가운데 화면에 보이는 것. 신호가 와도 이것이 그대로면 다시 그리지 않는다. */
const statusShown = (s) => JSON.stringify([s.claudeLive, s.model, s.effort, s.context, s.cost, s.cwd, s.worktree]);
/** 마지막으로 그린 사용 한도(JSON 글). */
let limitsShown = '';

function onStatus({ id, model, effort, context, cwd, project, session, worktree, cost, limits: newLimits }) {
  const s = byId(id);
  if (!s) return;
  const before = statusShown(s);
  // 셸 세션(PowerShell)에서 사용자가 직접 띄운 Claude가 보내온 신호: 이 세션에서 Claude가 돌고 있다고 적어 둔다.
  // 끝난 직후에 뒤늦게 온 상태줄 신호로 되살아나지 않게 잠깐은 받지 않는다.
  if (!s.profile?.claude) {
    if (Date.now() - (s.claudeEndedAt || 0) < 2000) return;
    s.claudeLive = true;
  }
  s.model = model || '';
  s.effort = effort || '';
  s.context = context;
  s.cost = cost || 0;
  // 컨텍스트가 거의 찼을 때 한 번 알린다(/compact 등으로 줄었다가 다시 차면 또 알린다).
  if (context != null && context >= CONTEXT_DANGER && !s.contextAlerted) {
    s.contextAlerted = true;
    const warning = `컨텍스트가 ${Math.round(context)}% 찼습니다. /compact 또는 새 세션을 고려하세요.`;
    logNotice(s, 'context', label(s), warning);
    if (config.notify.in_app) toast(s, label(s), warning);
  } else if (context != null && context < CONTEXT_WARN) {
    s.contextAlerted = false;
  }
  if (newLimits) {
    limits = newLimits;
    const text = JSON.stringify(newLimits);
    if (text !== limitsShown) { limitsShown = text; renderLimits(); }
  }
  if (cwd) s.cwd = cwd; // 상대경로 링크의 기준 폴더도 따라간다.
  if (worktree) s.worktree = worktree;
  if ((project && project !== s.project) || (session && session !== s.claudeSession)) {
    s.project = project || s.project;
    s.claudeSession = session || s.claudeSession;
    saveState();
  }
  // 상태줄 신호는 바뀐 것이 없어도 온다(원래 상태줄이 정해 둔 갱신 주기 등). 그때마다 사이드바 전체를
  // 다시 만들지 않는다.
  if (statusShown(s) !== before) render();
}

/** 알림(Notification) 가운데 알려 주기만 하는 것: 사용자의 입력·승인을 기다린다는 뜻이 아니다. */
const INFO_ONLY = new Set([
  'auth_success', 'elicitation_complete', 'elicitation_response', 'agent_completed',
  'quota_auto_resume_fired', 'quota_auto_resume_stale', 'quota_auto_resume_disabled',
]);

/**
 * 완료 신호 없이 멈춘 요청의 작업 시간 재기를 그만둔다. Claude Code는 사용자가 Esc로 중단했거나
 * API 오류로 끝난 요청에는 완료(Stop) 신호를 보내지 않는다.
 */
function endWork(s) {
  placeHeld(s); // 이 요청에서 만든 파일 가운데 최종 파일을 사이드바에 놓는다.
  s.workStart = 0;
  s.interruptAt = 0;
  s.heldStop = false;
  s.asked = false;
  s.agents.clear(); // 중단되면 돌던 서브에이전트도 함께 끝난다.
  s.tools.clear();
}

/** 셸 세션에서 사용자가 직접 띄운 Claude가 끝났다: 그 줄의 모델·상태 표시를 지우고 평범한 셸로 돌아간다. */
function endClaude(s) {
  s.claudeLive = false;
  s.claudeEndedAt = Date.now();
  // 셸에서 띄운 Claude의 대화는 다시 열 때 잇지 않는다(다시 열면 셸이 뜬다).
  s.claudeSession = '';
  s.resumable = false;
  s.model = '';
  s.effort = '';
  s.context = null;
  s.cost = 0;
  s.waiting = '';
  s.took = 0;
  endWork(s);
  render();
}

/** 서브에이전트 번호를 견줄 수 있게 다듬는다: 신호에 따라 `agent-` 머리가 붙기도 하고 빠지기도 한다. */
const agentKey = (id) => String(id || '').replace(/^agent-/, '');

/**
 * 응답 완료(Stop) 신호에 딸려 온 "아직 돌고 있는 배경 작업"으로 서브에이전트 표시를 맞춘다.
 * 서브에이전트의 종료 신호는 오지 않을 때가 있다(사용자가 멈춘 것, 오류로 끝난 것 등). 그러면 끝난 서브에이전트가
 * 사이드바에 계속 "실행 중"으로 남고, 그 세션의 완료도 "서브에이전트를 기다리는 중"으로 미뤄진다.
 * 완료 신호가 올 때는 본 세션이 기다리며 돌리는 서브에이전트가 없으므로, 그때 돌고 있는 것은 모두 이 목록에 있다.
 * `background`가 없으면(목록을 주지 않는 옛 Claude Code) 아무것도 하지 않는다.
 */
function settleAgents(s, background) {
  if (!background || !Array.isArray(background.agents) || !s.agents.size) return;
  // 워크플로가 도는 동안에는 그 안의 서브에이전트가 목록에 따로 나오지 않을 수 있으므로 지우지 않는다.
  if (background.workflows) return;
  const left = new Set(background.agents.map(agentKey).filter(Boolean));
  const tracked = [...s.agents.keys()];
  // 목록에 없는 것은 끝난 것이다. 번호가 하나도 맞지 않으면 번호 체계가 다른 것이므로 번호로는 가리지 않는다.
  if (!background.agents.length || tracked.some((key) => left.has(key))) {
    for (const key of tracked) if (!left.has(key)) s.agents.delete(key);
  }
  // 번호로 가리지 못했으면 수만 맞춘다: 먼저 시작한 것부터 끝난 것으로 본다.
  for (const key of tracked) {
    if (s.agents.size <= background.agents.length) break;
    s.agents.delete(key);
  }
}

/** Claude Code 훅: 요청 시작, 응답 완료(Stop), 입력·승인 대기(Notification), 서브에이전트 시작·종료, Claude Code 종료(SessionEnd). */
function onHook({ id, event, kind, message, agent, agent_type: agentType, background, force }) {
  const s = byId(id);
  if (!s) return;
  if (event === 'SessionEnd') {
    // Claude 프로필의 세션은 곧 프로세스가 끝나며 onExit가 정리한다.
    // /clear로 대화를 비우거나 /resume으로 다른 대화로 바꾼 것은 끝난 것이 아니다.
    if (!s.profile?.claude && kind !== 'clear' && kind !== 'resume') endClaude(s);
    return;
  }
  if (!s.profile?.claude && !force) s.claudeLive = true;
  if (!s.resumable) { s.resumable = true; saveState(); } // 대화가 생겼으니 다음에 이어갈 수 있다.
  // 서브에이전트: 돌고 있는 것의 번호와 종류를 모아 두고 사이드바에 보여 준다.
  if (event === 'SubagentStart' || event === 'SubagentStop') {
    if (agent) {
      if (event === 'SubagentStart') s.agents.set(agentKey(agent), agentType || ''); else s.agents.delete(agentKey(agent));
    }
    if (s.heldStop) s.heldAt = performance.now(); // 서브에이전트에 움직임이 있으면 조용한 시간을 다시 잰다.
    return render();
  }
  if (event !== 'UserPromptSubmit' && event !== 'Stop' && event !== 'Notification') return;
  if (event === 'UserPromptSubmit') {
    s.waiting = '';
    s.heldStop = false;
    s.workStart = Date.now(); // 이 요청에 걸리는 시간을 잰다.
    s.interruptAt = 0;
    s.asked = false;
    s.took = 0;
    s.tools.clear();          // 앞 요청에서 끝 신호가 빠진 도구 표시가 남지 않게 한다.
    s.toolSig = '';
    placeHeld(s);             // 앞 요청에서 만들고 아직 놓지 않은 파일이 남아 있으면 놓는다(한도는 놓는 묶음마다 따로 센다).
    return render();
  }
  if (event === 'Notification' && kind === 'idle_prompt') {
    // 완료 때 이미 알렸다. 다만 완료 신호 없이 멈춘 요청(중단 등)이 아직 "작업 중"으로 남아 있으면 여기서 멈춘다.
    // 서브에이전트를 기다리며 미뤄 둔 완료는 건드리지 않는다(그쪽은 refreshActivity가 챙긴다).
    if (s.workStart && !s.heldStop) { endWork(s); render(); }
    return;
  }
  // 사용자의 입력을 기다리는 것이 아닌 알림(로그인 성공, 백그라운드 에이전트 완료 등)은 "대기"로 치지 않는다.
  // 종류를 알려 주지 않는 옛 버전의 알림과 모르는 종류의 알림은 지금까지처럼 대기로 친다.
  if (event === 'Notification' && INFO_ONLY.has(kind)) return;
  if (event === 'Notification') s.asked = true; // 답을 기다리는 창이 떠 있다(답한 뒤 다시 일하기 전까지는 조용한 것이 당연하다).
  // 서브에이전트가 아직 돌고 있을 때의 완료 신호는 "잠깐 멈춤"이다(끝나면 Claude가 이어서 답한다).
  // 이때는 끝났다고 알리지 않고, 서브에이전트가 모두 끝난 뒤의 완료 신호를 기다린다.
  // 그 신호가 끝내 오지 않는 경우(서브에이전트 종료 신호가 빠짐 등)는 refreshActivity()가 챙긴다.
  // 그 전에, 완료 신호가 알려 준 "아직 돌고 있는 배경 작업"에 없는 서브에이전트는 끝난 것이므로 표시에서 지운다.
  if (event === 'Stop' && !force) settleAgents(s, background);
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
    s.tools.clear(); // 응답이 끝났으니 쓰고 있는 도구도 없다.
    placeHeld(s);    // 이 요청에서 만든 파일 가운데 최종 파일을 사이드바에 놓는다.
  }
  if (force) s.agents.clear(); // 한참 조용했으니 남아 있던 서브에이전트 표시는 끝난 것으로 본다.
  s.waiting = event === 'Stop' ? 'done' : 'approval';
  if (event === 'Stop') diffRefreshFor(s);
  // 응답에 Claude Code와 가상 터미널이 칸 수를 다르게 세는 글자가 있었으면, 끝난 지금 화면을 다시 맞춘다.
  if (event === 'Stop') realignIfSkewed(s);
  // 큰 창(승인 창 등)이 접히면서 입력창 아래에 빈 공간이 남았으면 메운다.
  if (event === 'Stop') closeGapSoon(s);
  render();
  const title = label(s);
  const body = event === 'Stop'
    ? `요청한 작업이 끝났습니다.${s.took >= 1000 ? ` (${duration(s.took)} 걸림)` : ''}`
    : (message || '입력을 기다리고 있습니다.');
  const focused = document.hasFocus();
  if (!focused && config.notify.system) invoke('notify', { title, body }); // Windows 알림 + 작업 표시줄 깜빡임
  // 알림 기록에는 늘 남긴다. 사용자가 그 세션을 보고 있었으면 이미 본 것으로 적는다.
  const seen = focused && s === active;
  logNotice(s, event === 'Stop' ? 'done' : 'approval', title, body, !seen);
  if (!seen) {
    s.attention = true;
    if (config.notify.in_app) toast(s, title, body);
    render();
  }
}

// ───────────────────────── 지금 하는 일 ─────────────────────────
// Claude 세션이 도구를 쓰기 시작하고 끝낼 때마다 앱이 알려 온다(session-tool). 사이드바의 그 세션 줄에
// "Bash · cargo build"처럼 지금 쓰는 도구를 보여 준다. 설정의 [session] activity로 끈다(끄면 신호가 오지 않는다).

const PARALLEL_MS = 1500;    // 같은 갈래에서 이 안에 시작한 도구들은 함께 도는 것(병렬 호출)으로 본다.
const TOOL_CLOCK_MS = 10000; // 도구 하나가 이만큼 넘게 돌면 그 도구에 걸린 시간도 보인다.
const REPEAT_SHOWN = 3;      // 같은 일을 잇달아 이만큼 하면 몇 번째인지 보인다.
const REPEAT_FREE = new Set(['Edit', 'Write', 'MultiEdit', 'NotebookEdit', 'Read', 'TodoWrite']); // 되풀이해도 맴도는 것으로 치지 않는 도구

let renderTimer = 0;
/** 잦은 신호에 목록을 매번 다시 그리지 않게, 조금 모았다가 한 번 그린다. */
function renderSoon() {
  if (!renderTimer) renderTimer = setTimeout(() => { renderTimer = 0; render(); }, 120);
}

/** phase: 'start'(쓰기 직전) | 'end'(끝남) | 'fail'(실패했거나 중단됨) */
function onTool({ id, phase, tool, detail, full, use_id: useId, agent }) {
  const s = byId(id);
  if (!s || !tool) return;
  const lane = agent || ''; // 갈래: 본 세션('')이나 서브에이전트 하나
  const sig = `${tool}\n${detail || ''}`;
  const key = useId || `${lane}\n${sig}`;
  if (phase === 'start') {
    const now = Date.now();
    // 끝 신호가 오지 않는 도구(승인을 거절한 것 등)가 "하는 중"으로 남지 않게 한다: 한 갈래는 앞의 도구가 끝나야
    // 다음 도구를 쓰므로, 새 도구가 시작되면 그 갈래에서 한참 전에 시작한 것은 끝난 것으로 본다.
    for (const [other, running] of s.tools) if (running.lane === lane && now - running.at > PARALLEL_MS) s.tools.delete(other);
    // 같은 일을 잇달아 하는지 센다. 한 파일을 여러 번 고치거나 나눠 읽는 것은 흔한 일이라 세지 않는다.
    s.toolRepeat = !REPEAT_FREE.has(tool) && s.toolSig === sig ? (s.toolRepeat || 1) + 1 : 1;
    s.toolSig = sig;
    s.tools.set(key, { tool, detail: detail || '', full: full || '', at: now, lane, repeat: s.toolRepeat });
  } else if (!s.tools.delete(key)) {
    return; // 이미 치운 것
  }
  renderSoon();
}

/** 지금 쓰고 있는 도구 가운데 가장 나중에 시작한 것(없으면 null). */
function currentTool(s) {
  let latest = null;
  for (const running of s.tools.values()) if (!latest || running.at >= latest.at) latest = running;
  return latest;
}

/** 사이드바 줄에 보일 글: "Bash · cargo build (1:24) 외 2개". */
function toolText(s, running) {
  const elapsed = Date.now() - running.at;
  return [
    running.detail ? `${running.tool} · ${running.detail}` : running.tool,
    elapsed >= TOOL_CLOCK_MS ? `(${clock(elapsed)})` : '',
    s.tools.size > 1 ? `외 ${s.tools.size - 1}개` : '',
  ].filter(Boolean).join(' ');
}

/** 풍선말: 쓰고 있는 도구 전부를 시작한 순서대로, 줄이지 않은 설명과 함께. */
function toolTip(s) {
  const list = [...s.tools.values()].sort((a, b) => a.at - b.at);
  return ['지금 쓰는 도구', ...list.map((running) => {
    const since = new Date(running.at).toLocaleTimeString('ko-KR', { hour12: false });
    return `${running.tool}${running.full ? ` · ${running.full}` : ''} (${since} 시작${running.lane ? ' · 서브에이전트' : ''})`;
  })].join('\n');
}

// ───────────────────────── 알림 기록 ─────────────────────────
// 떴다가 사라지는 알림을 모아 둔다(notices.js). 사이드바 아래 [알림 N] 단추의 숫자는 아직 보지 않은 알림 수.

/** 그 항목이 속한 묶음의 이름(워크스페이스 이름, 없으면 폴더 이름). */
const groupName = (s) => s.group || baseName(s.startCwd);

/** 알림 기록에 남긴다. `s`는 그 알림이 가리키는 항목(없으면 null). */
function logNotice(s, kind, title, body, unread = true) {
  recordNotice({ kind, title, body, key: s ? s.key : null, group: s ? groupName(s) : '', unread });
}

/** 알림 기록에서 고른 알림의 항목으로 간다. 갔으면 참. */
function jumpToNotice({ key, sameRun, title, group }) {
  let s = sameRun ? sessions.get(key) : null;
  if (!s) {
    // 다시 시작한 뒤에는 항목 번호가 달라진다. 이름과 묶음이 같은 항목이 하나뿐이면 그것으로 간다.
    const same = [...sessions.values()].filter((other) => label(other) === title && groupName(other) === group);
    if (same.length === 1) [s] = same;
  }
  if (!s) return false;
  activate(s);
  return true;
}

function renderNoticeButton() {
  const count = unreadCount();
  const $button = $('notices');
  $button.textContent = `알림 ${count}`;
  $button.classList.toggle('has', count > 0);
}

/** 사이드바 아래 [타이머] 단추: 맞춰 둔 것이 있으면 가장 먼저 울릴 것까지 남은 시간을 보여 준다(1초마다 고친다). */
function renderTimerButton() {
  const next = nextTimer();
  const $button = $('timers');
  const text = next ? `⏰ ${next.left}` : '타이머';
  if ($button.textContent !== text) $button.textContent = text;
  $button.classList.toggle('has', !!next);
  $button.title = next
    ? `타이머·알람 ${next.count}개 · 다음: ${next.label || (next.kind === 'alarm' ? '알람' : '타이머')} (${next.left} 남음) · 누르면 보기·맞추기`
    : '타이머·알람 맞추기 (24시간 이내) · Cypher가 꺼져 있는 동안 지나간 것은 켠 직후에 알려 줍니다';
}

/**
 * 타이머·알람이 울렸다(timer.js): 알림 기록에 남기고, 누를 때까지 남는 알림을 띄우고, 창이 뒤에 있으면 Windows 알림도 띄운다.
 * 놓치면 안 되는 알림이라 스스로 사라지지 않는다.
 */
function timerRang({ title, body }) {
  logNotice(null, 'timer', title, body);
  const box = make('div', 'toast alarm');
  box.append(make('b', '', `⏰ ${title}`), make('span', '', body), make('span', 'alarm-hint', '누르면 닫힙니다'));
  box.onclick = () => box.remove();
  $('toasts').append(box);
  if (!document.hasFocus() && config.notify.system) invoke('notify', { title: `⏰ ${title}`, body });
}

/** 세션과 무관한 알림(오류 등). 상태줄과 달리 다른 표시에 덮이지 않고 누를 때까지 12초간 남는다. */
function notice(title, body) {
  logNotice(null, 'info', title, body);
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
    // 완료 신호 없이 멈춘 요청: Esc·Ctrl+C를 누른 뒤 조용해졌거나, 한참 아무 출력이 없으면 끝난 것으로 본다
    // (일하는 동안에는 Claude Code가 경과 시간을 계속 그리므로 조용할 수 없다). 승인 대기 중에는 건드리지 않는다.
    if (s.workStart && !s.waiting && !s.heldStop) {
      const quiet = now - s.lastOutput;
      const sinceKey = s.interruptAt ? now - s.interruptAt : 0;
      // 승인·질문 창이 떠 있던 요청은 오래 조용해도 끝난 것으로 보지 않는다(Esc로 그만둔 경우만 본다).
      if (sinceKey > INTERRUPT_MS ? quiet > INTERRUPT_QUIET_MS : (!s.interruptAt && !s.agents.size && !s.asked && quiet > STALLED_MS)) {
        endWork(s);
        closeGapSoon(s);
        changed = true;
      } else if (sinceKey > INTERRUPT_MS) {
        s.interruptAt = 0; // 누른 뒤에도 계속 출력한다: 중단이 아니었다.
      }
    }
    // 일하는 중인 세션의 경과 시간은 목록을 다시 그리지 않고 그 칸만 고친다.
    if (s.workStart && s.timeEl?.isConnected) s.timeEl.textContent = clock(Date.now() - s.workStart);
    // 오래 도는 도구에 걸린 시간도 같은 식으로 그 칸만 고친다.
    if (s.toolEl?.isConnected) { const running = currentTool(s); if (running) s.toolEl.textContent = toolText(s, running); }
    if (busy === s.busy) continue;
    // 화면을 다시 맞추느라 나온 출력은 새 소식이 아니다.
    if (!busy && s !== active && now > (s.realignedAt || 0) + REALIGN_QUIET_MS) s.attention = true;
    if (!busy) closeGapSoon(s); // 조용해졌다: 메뉴를 닫은 뒤처럼 아래에 빈 공간이 남았으면 메운다.
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
  // 터미널이 아닌 항목이 내놓는 상태(명령 항목의 실행 중·성공·실패, 다 끝낸 할 일)는 종류 기호와 아랫줄의 색으로 보인다.
  const tone = isTerm(s) ? '' : s.view?.tone?.() || '';
  const li = make('li', [
    s === active && 'active', visible.includes(s) && 'shown', s.waiting, tone && `tone-${tone}`,
    s.exited ? 'exited' : s.dormant ? 'dormant' : s.busy ? 'busy' : s.attention && 'attention',
  ].filter(Boolean).join(' '));
  li.title = '끌기: 화면 가장자리에 놓으면 나눠 보기·순서 바꾸기·워크스페이스 옮기기 · 두 번 누르기: 별칭 · 오른쪽 클릭: 메뉴';
  if (s.color) li.style.borderLeftColor = s.color;
  li.onclick = (ev) => {
    if (ev.target.closest('input')) return; // 별칭을 고치는 중
    // Ctrl+클릭: 화면에 보이는 것은 화면에서 뺀다(나란히 놓는 것은 끌어다 놓기로만 한다).
    if (ev.ctrlKey && visible.includes(s)) toggleShown(s); else activate(s);
  };
  li.onmousedown = (ev) => {
    if (ev.button === 0 && !ev.ctrlKey && !ev.target.closest('button, input')) { cancelDrag(); drag = { s, x: ev.clientX, y: ev.clientY, active: false }; }
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
    if (s.mark) name.append(markBadge(s.mark));
    name.append(make('span', 'alias', label(s)));
    if (s.name) name.append(make('span', 'origin', baseLabel(s)));
    name.title = `${s.name ? `${baseLabel(s)}\n` : ''}${s.cwd}${s.mark ? `\n북마크 ${s.mark}: Ctrl+${s.mark}로 바로 가기` : ''}`;
    if (s.color) name.style.color = s.color;
  }
  const close = make('button', 'close', '×');
  close.title = isTerm(s) ? '세션 닫기' : '항목 닫기';
  close.onclick = (ev) => { ev.stopPropagation(); closeSession(s); };
  const pencil = make('button', 'close', '✎');
  pencil.title = '별칭 붙이기';
  pencil.onclick = (ev) => { ev.stopPropagation(); rename(s); };
  // 화면 표시 토글: 오른쪽 화면에 보이기/숨기기(세션은 계속 실행된다). 항상 보이는 작은 단추.
  const shown = visible.includes(s);
  const eye = make('button', `eye${shown ? ' on' : ''}`, shown ? '◉' : '○');
  eye.title = shown ? '화면에서 숨기기' : '화면에 보이기 (지금 창 자리에)';
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
  if (context >= CONTEXT_WARN) badge.title = '컨텍스트가 거의 참 · /compact나 새 세션 권장';
  // 돌고 있는 서브에이전트: 눈에 띄는 한 줄로 따로 보여 준다.
  const agents = make('span', 'agents-line');
  if (s.agents.size) {
    agents.append(make('i', 'spin'), make('span', '', `서브에이전트 ${s.agents.size}개 실행 중${agentKinds(s) ? ` · ${agentKinds(s)}` : ''}`));
    agents.title = agentSummary(s);
  }
  const sub = make('span', 'sub');
  // 지금 화면(스테이지)에 보이는 항목은 아랫줄 맨 앞에 표를 단다: 입력이 가는 창은 "보는 중", 나란히 보이는 다른 창은 "화면".
  // (이름 옆에 두면 좁은 사이드바에서 이름이 가려진다.)
  if (visible.includes(s)) {
    const chip = make('b', 'on-stage', s === active ? '보는 중' : '화면');
    chip.title = s === active ? '지금 화면에서 보고 있는 항목 (입력이 여기로 갑니다)' : '화면에 나란히 보이는 항목';
    sub.append(chip, ' ');
  }
  if (s.logPath) {
    const rec = make('b', 'recording', '● 기록');
    rec.title = `출력을 파일로 기록하는 중: ${s.logPath}\n오른쪽 클릭 → 출력 기록 끄기`;
    sub.append(rec, ' ');
  }
  // 요청을 처리하는 중이면 경과 시간을 앞에 보여 준다(승인을 기다리는 동안에는 대기 표시가 먼저다).
  s.timeEl = null;
  if (s.workStart && !s.exited && !s.waiting) {
    s.timeEl = make('b', 'elapsed', clock(Date.now() - s.workStart));
    s.timeEl.title = '이 요청의 경과 시간';
    sub.append(s.timeEl);
  }
  // 지금 쓰는 도구: 일하는 동안에는 터미널 제목 대신 보이고, 승인을 기다릴 때는 무엇을 승인하는지 알 수 있게 뒤에 붙는다.
  const running = s.exited || s.waiting === 'done' ? null : currentTool(s);
  const subText = s.exited ? '종료됨 (오른쪽 클릭: 다시 시작)'
    : s.dormant ? (resumeOf(s) ? '쉬는 중 · 누르면 대화를 이어 재개' : s.ssh ? '쉬는 중 · 누르면 다시 접속' : '쉬는 중 · 누르면 재개')
    : s.waiting === 'done' && s.took >= 1000 ? `완료 · ${duration(s.took)} 걸림 · 다음 지시 대기`
    : WAITING_TEXT[s.waiting] || (s.heldStop && s.agents.size ? '서브에이전트가 끝나기를 기다리는 중' : running ? ''
      // 터미널이 아닌 항목은 그 화면이 내놓는 한 줄(할 일의 진행, 명령의 상태)을 보여 준다.
      : isTerm(s) ? s.title : s.view?.sub?.() || '');
  if (subText) sub.append(subText);
  s.toolEl = null;
  if (running) {
    if (subText) sub.append(' · ');
    if (running.repeat >= REPEAT_SHOWN) {
      const again = make('b', 'repeat', `${running.repeat}번째`);
      again.title = '같은 작업을 잇달아 반복 중';
      sub.append(again);
    }
    s.toolEl = make('span', 'tool', toolText(s, running));
    sub.append(s.toolEl);
    sub.title = toolTip(s);
  }
  // 사용량은 3초마다 바뀌므로 목록을 다시 그리지 않고 이 칸만 고친다.
  s.usageEl = make('span', 'usage', s.exited ? '' : s.usage);
  li.append(dot, name, buttons, agents, badge, sub, s.usageEl);
  return li;
}

/** SSH 탭의 짧은 이름: 별칭이나 서버에 붙인 이름, 없으면 사용자@서버. */
const tabName = (s) => [s.name, s.ssh.name].find((text) => text && text !== sshServer(s)) || `${s.sshUser ? `${s.sshUser}@` : ''}${sshServer(s)}`;

/**
 * 사이드바의 SSH 줄. SSH는 워크스페이스마다 하나라서, 그 워크스페이스의 SSH 세션(탭)이 몇 개든 줄은 하나다.
 * 누르면 SSH 방이 지금 창 자리에 열리고, 서버를 더 열면 이 줄이 늘지 않고 방 안의 탭이 는다.
 */
function sshItem(group, given) {
  const list = roomTabs(group).filter((s) => given.includes(s)); // 창과 탭 줄에 놓인 순서대로 적는다.
  const shown = roomShown(group);
  const here = !!active?.ssh && active.group === group && sessions.has(active.key);
  const alive = list.filter((s) => !s.exited && !s.dormant);
  const li = make('li', [
    'ssh-row', here && 'active', shown && 'shown',
    list.every((s) => s.exited) ? 'exited' : !alive.length ? 'dormant' : list.some((s) => s.busy) ? 'busy' : list.some((s) => s.attention) && 'attention',
  ].filter(Boolean).join(' '));
  li.title = 'SSH는 워크스페이스마다 하나입니다(서버를 더 열면 이 안의 탭이 됩니다)\n누르기: SSH 창 열기 · 끌기: 화면 가장자리에 놓으면 나눠 보기·순서 바꾸기·워크스페이스 옮기기 · 오른쪽 클릭: 메뉴\n탭과 창은 SSH 창 안에서 끌어 옮깁니다(창 가장자리에 놓으면 나눠 보기, 다른 창의 탭 줄에 놓으면 그 창으로)';
  li.dataset.key = given[0].key; // 순서를 옮길 때의 기준: 이 워크스페이스의 첫 SSH 세션
  li.dataset.room = group;
  li.onclick = (ev) => {
    if (ev.target.closest('button')) return;
    // Ctrl+클릭: 화면에 보이는 것은 화면에서 뺀다.
    if (ev.ctrlKey && roomShown(group)) toggleRoom(group); else openRoom(group);
  };
  li.onmousedown = (ev) => {
    if (ev.button !== 0 || ev.ctrlKey || ev.target.closest('button')) return;
    const s = roomFront(group);
    if (!s) return;
    cancelDrag();
    drag = { s, room: group, x: ev.clientX, y: ev.clientY, active: false }; // SSH 줄을 끈다: 방을 통째로 옮긴다.
  };
  li.oncontextmenu = (ev) => { ev.preventDefault(); roomMenu(group, ev); };

  const name = make('span', 'name');
  for (const s of list) if (s.mark) name.append(markBadge(s.mark));
  name.append(make('span', 'alias', 'SSH'), make('span', 'origin', `탭 ${list.length}개`));
  name.title = `SSH — 워크스페이스 "${group}"\n${list.map((s) => `· ${tabName(s)}${s.exited ? ' (끝남)' : s.dormant ? ' (쉬는 중)' : loggingIn(s) ? ' (로그인 중)' : ''}`).join('\n')}`;
  const add = make('button', 'close', '+');
  add.title = '서버에 접속… (이 SSH의 새 탭으로)';
  add.onclick = (ev) => { ev.stopPropagation(); addSshTab(group); };
  const eye = make('button', `eye${shown ? ' on' : ''}`, shown ? '◉' : '○');
  eye.title = shown ? 'SSH를 화면에서 숨기기 (세션은 계속 실행)' : 'SSH를 화면에 보이기 (지금 창 자리에)';
  eye.onclick = (ev) => { ev.stopPropagation(); toggleRoom(group); };
  const close = make('button', 'close', '×');
  close.title = 'SSH 세션 모두 닫기 (닫기 전에 묻습니다)';
  close.onclick = (ev) => { ev.stopPropagation(); closeRoom(group); };
  const buttons = make('span', 'row-buttons');
  buttons.append(add, eye, close);

  const dot = make('span', 'dot kind', '⇌');
  dot.title = 'SSH (워크스페이스마다 하나)';
  const sub = make('span', 'sub');
  if (shown) {
    const chip = make('b', 'on-stage', here ? '보는 중' : '화면');
    chip.title = here ? '지금 화면에서 보고 있는 SSH (입력이 여기로 갑니다)' : '화면에 나란히 보이는 SSH';
    sub.append(chip, ' ');
  }
  const logging = list.filter((s) => s.logPath);
  if (logging.length) {
    const rec = make('b', 'recording', '● 기록');
    rec.title = `출력을 파일로 기록하는 중:\n${logging.map((s) => `${tabName(s)} → ${s.logPath}`).join('\n')}`;
    sub.append(rec, ' ');
  }
  const names = list.map(tabName).join(' · ');
  sub.append(list.every((s) => s.exited) ? `종료됨 · ${names}` : !alive.length ? `쉬는 중 · 누르면 다시 접속 · ${names}` : names);
  li.append(dot, name, buttons, sub);
  return li;
}

/** 사이드바에 놓을 줄들: 항목마다 한 줄, SSH 세션은 몇 개든 그 워크스페이스의 SSH 줄 하나(첫 SSH 세션의 자리에 놓인다). */
function sidebarRows(list) {
  const rows = [];
  const done = new Set();
  for (const s of list) {
    if (!s.ssh) rows.push({ s });
    else if (!done.has(s.group)) {
      done.add(s.group);
      rows.push({ room: s.group, list: list.filter((x) => x.ssh && x.group === s.group) });
    }
  }
  return rows;
}

/** 그 항목이 사이드바에서 보이는 줄(SSH 세션이면 그 워크스페이스의 SSH 줄). */
const rowOf = (s) => (s.ssh
  ? [...$list.querySelectorAll('li[data-room]')].find((li) => li.dataset.room === s.group)
  : $list.querySelector(`li[data-key="${s.key}"]`)) || null;

/** 사이드바의 줄이 가리키는 항목. SSH 줄이면 그 방에서 지금 보고 있는(또는 마지막으로 쓴) 탭. */
function rowItem(li) {
  const group = li?.dataset?.room;
  if (group) return active?.ssh && active.group === group && sessions.has(active.key) ? active : roomFront(group);
  return sessions.get(Number(li?.dataset?.key));
}

/** 사이드바 SSH 줄의 [+]와 메뉴: 접속 창을 띄워 그 워크스페이스의 SSH에 탭을 더한다. */
function addSshTab(group) {
  const near = roomFront(group);
  showSsh({ group, ...(near ? { cwd: near.startCwd } : {}), show: 'replace' });
}

/**
 * 여럿을 한꺼번에 닫을 때의 순서: 쉬는 세션부터 닫는다. 보이던 탭을 먼저 닫으면 그 창을 옆의 쉬는 탭이 받아, 곧 닫을 탭이
 * 그사이 화면에 올라왔다 내려간다(쉬는 SSH 탭은 올라와도 접속하지 않지만, 다른 쉬는 세션은 올라오면 재개된다).
 */
const restingFirst = (list) => [...list.filter((s) => s.dormant), ...list.filter((s) => !s.dormant)];

/** 그 워크스페이스의 SSH 세션을 모두 닫는다(정말 닫을지 먼저 묻는다). */
async function closeRoom(group) {
  const list = sshOf(group);
  if (!list.length) return;
  const live = list.filter((s) => s.id != null && !s.exited && !s.dormant).length;
  if (!confirm(`워크스페이스 "${group}"의 SSH 세션 ${list.length}개를 모두 닫을까요?${live ? `\n접속 중인 세션 ${live}개의 접속을 끊습니다.` : ''}`)) return;
  for (const s of restingFirst(list)) await closeSession(s);
}

/** 사이드바 SSH 줄의 오른쪽 클릭 메뉴: 서버 더 열기, 탭으로 가기, 화면에 보이기·숨기기, 모두 닫기. */
function roomMenu(group, ev) {
  const list = roomTabs(group);
  const shown = roomShown(group);
  const panes = leavesOf(rooms.get(group) || null).length;
  // 쉬는 탭을 한꺼번에 다시 접속하는 길은 두지 않는다(동시 접속은 서버가 침입 시도로 본다). 탭을 하나씩 눌러 접속한다.
  showMenu(ev, [
    { caption: `SSH · 워크스페이스 "${group}" · 탭 ${list.length}개${panes > 1 ? ` · 창 ${panes}개` : ''} (SSH는 워크스페이스마다 하나입니다)` },
    { label: '서버에 접속… (새 탭으로)', action: () => addSshTab(group) },
    ...list.map((s) => ({
      label: `탭으로 가기: ${tabName(s)}${s.exited ? ' (끝남)' : s.dormant ? ' (쉬는 중 · 다시 접속)' : loggingIn(s) ? ' (로그인 중)' : ''}`,
      action: () => activate(s),
    })),
    ...(!shown || visible.length > panes ? [{ label: 'SSH만 보기 (나머지는 화면에서 숨기기)', action: () => showRoomOnly(group) }] : []),
    ...(shown ? [{ label: '화면에서 숨기기', action: () => toggleRoom(group) }] : []),
    { caption: '탭 하나를 다루려면 SSH 창의 탭을 오른쪽 클릭하세요 (별칭·색·복제·닫기) · 탭을 끌면 SSH 창 안에서 옮기거나 나눠 봅니다' },
    { label: `SSH 세션 모두 닫기… (${list.length}개)`, action: () => closeRoom(group) },
  ]);
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
      // 스테이지 맨 위 줄과 창 제목은 눌린 클릭이 지나간 뒤에 맞춘다(한가한 세션은 다음 갱신이 오지 않아 옛 이름으로 남는다).
      if (commit) setTimeout(render, 300);
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

// ───────────────────────── 세션 묶음(워크스페이스) ─────────────────────────
// 세션은 기본으로 시작 폴더별로 묶인다. 워크스페이스 이름을 지정한 세션은 폴더와 무관하게 그 워크스페이스로 묶인다.

const groupKey = (s) => (s.group ? `g:${s.group}` : `d:${s.startCwd}`);
function saveGroups() {
  localStorage.setItem('cypher.groups', JSON.stringify(groupList));
}

const saveProfiles = () => localStorage.setItem('cypher.profiles', JSON.stringify(groupProfiles));

/**
 * 그 항목의 워크스페이스가 쓰는 브라우저 프로필의 번호. 브라우저의 로그인 상태는 프로필마다 따로라서, 한 워크스페이스에서 로그인해 둔
 * 계정을 다른 워크스페이스의 브라우저나 Claude 세션이 쓸 수 없다. 워크스페이스가 없는 항목과, 아직 브라우저를 쓴 적이 없는 워크스페이스는 빈 글이다.
 */
function profileOf(item) {
  const id = item?.group ? groupProfiles[item.group] : '';
  return typeof id === 'string' && /^[a-z0-9]{1,40}$/.test(id) ? id : '';
}

/** 브라우저를 띄울 때 쓴다: 그 항목의 워크스페이스가 쓰는 프로필의 번호를 돌려주고, 아직 없으면 만든다. 워크스페이스가 없으면 빈 글(공용 프로필). */
function ensureProfile(item) {
  if (!item?.group) return '';
  if (!profileOf(item)) {
    groupProfiles[item.group] = `p${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    saveProfiles();
  }
  return groupProfiles[item.group];
}

/** 지금 보고 있는 항목(닫힌 것이면 null). */
const viewing = () => (active && sessions.has(active.key) ? active : null);

/** 워크스페이스를 목록에 올린다. 올려 둔 워크스페이스는 세션이 없어도 사이드바에 남는다. */
function ensureGroup(name) {
  if (!name || groupList.includes(name)) return;
  groupList.push(name);
  saveGroups();
}

/**
 * 워크스페이스를 만든다(이미 있으면 그대로 둔다). `path`를 주면 그 워크스페이스의 폴더로 정한다(이미 폴더를 정해 둔 워크스페이스면 바꾸지 않는다).
 * 만든(또는 있던) 워크스페이스의 이름을 돌려준다.
 * 세션과 항목은 워크스페이스 안에서만 만들 수 있으므로, 무엇을 하든 워크스페이스부터 만든다.
 */
function createGroup(name, path = '') {
  const given = String(name ?? '').trim();
  if (!given) return '';
  ensureGroup(given);
  collapsed.delete(`g:${given}`);
  localStorage.setItem('cypher.collapsed', JSON.stringify([...collapsed]));
  if (path) setGroupFolder(given, path); else render();
  return given;
}

/**
 * "+ 워크스페이스" 단추: 이름을 묻고 워크스페이스를 만든 뒤, 그 워크스페이스의 세션이 시작될 폴더를 고르게 한다.
 * 여기서 고른 폴더는 바꿀 수 없다. 취소하면 폴더 없이 만들어지고, 나중에 폴더 줄에서 한 번 정할 수 있다.
 */
async function newGroup() {
  const name = await ask('새 워크스페이스 이름 (세션과 항목은 워크스페이스 안에서만 만듭니다)', '예: 결제 개편');
  if (!name) return '';
  const fresh = !groupList.includes(name);
  createGroup(name);
  if (!fresh) { setStatus(`워크스페이스 "${name}"은(는) 이미 있습니다. 워크스페이스 머리의 +로 세션·항목을 더하세요.`); return name; }
  const picked = await invoke('pick_folder', { start: startFolderHint(), title: `워크스페이스 "${name}"의 세션을 시작할 폴더 (한 번 정하면 바꿀 수 없습니다 · 취소: 나중에 정하기)` }).catch(() => null);
  if (picked && groupList.includes(name)) setGroupFolder(name, picked);
  setStatus(`워크스페이스 "${name}"을(를) 만들었습니다${picked ? ` (폴더: ${picked} · 바꿀 수 없음)` : ' (폴더는 워크스페이스 이름 아래의 폴더 줄에서 한 번 정할 수 있습니다)'}. 워크스페이스 머리의 +로 세션·항목을 더하세요.`);
  return name;
}

/** 세션·항목을 그 워크스페이스로 옮긴다. 워크스페이스 밖으로는 뺄 수 없다(이름이 비면 아무것도 하지 않는다). */
function moveToGroup(list, name) {
  if (!name) return setStatus('항목은 워크스페이스 안에만 둘 수 있습니다. 옮길 워크스페이스의 이름을 적어 주세요.');
  ensureGroup(name);
  // SSH 방과 탭 묶음은 한 워크스페이스 안의 것이다. 한 워크스페이스의 SSH를 통째로, SSH가 없는 워크스페이스로 옮기면
  // (워크스페이스 이름 바꾸기, SSH 줄을 끌어 옮기기) 방 안의 배치를 그대로 가져간다. 그 밖에는 옮기는 탭을 방에서 떼고,
  // 옮겨 간 워크스페이스의 SSH 방에 탭으로 넣는다(SSH는 워크스페이스마다 하나다).
  const moving = new Set(list);
  let taken = sshOf(name).length > 0;
  for (const from of new Set(list.filter((s) => s.ssh && s.group !== name).map((s) => s.group))) {
    const all = sshOf(from);
    if (!taken && all.every((s) => moving.has(s))) {
      const inner = rooms.get(from);
      rooms.delete(from);
      if (inner) rooms.set(name, inner);
      const slot = findSlot({ room: from });
      if (slot) slot.room = name;
      taken = true;
    } else {
      // 통째로 옮기는데 옮겨 갈 워크스페이스에 이미 SSH가 있으면 탭으로 합친다. 보고 있던 SSH의 자리에는 합쳐진 SSH가 열린다.
      const slot = all.every((s) => moving.has(s)) && !roomShown(name) ? findSlot({ room: from }) : null;
      if (slot) slot.room = name;
      for (const s of all) if (moving.has(s)) leaveRoom(s);
    }
  }
  const changed = list.some((s) => s.ssh && s.group !== name);
  for (const s of list) s.group = name;
  if (changed) {
    layout(); // 방을 옮긴 뒤의 세션에 맞춘다.
    // 보고 있던 탭이 옮겨 가며 다른 탭에 가려졌으면: 옮겨 간 SSH가 화면에 있을 때는 그 탭을 다시 보이고, 없으면 화면에 남은 것을 고른다.
    if (active && sessions.has(active.key) && !visible.includes(active)) {
      if (active.ssh && moving.has(active) && roomShown(name)) activate(active);
      else if (stageFallback()) activate(stageFallback()); // 쉬는 세션은 고르지 않는다(고르면 접속한다).
    }
  }
  render();
  return saveState();
}

async function askGroup(list, current = '') {
  const name = await ask('옮길 워크스페이스 이름 (없는 이름이면 새 워크스페이스를 만듭니다)', groupList.length ? `있는 워크스페이스: ${groupList.join(', ')}` : '예: 결제 개편', current);
  if (name) moveToGroup(list, name);
}

/** 그 이름의 워크스페이스에 지금 들어 있는 항목 전부. 사이드바가 "대기만 보기"로 걸러져 있어도, 이름을 묻는 사이 새 항목이 생겼어도 빠뜨리지 않는다. */
const membersOf = (name) => [...sessions.values()].filter((s) => s.group === name);

async function renameGroup(group) {
  const name = await ask('워크스페이스 이름', '', group.name);
  if (!name || name === group.name) return;
  // 같은 이름의 워크스페이스가 이미 있으면 그쪽으로 합쳐진다.
  const merging = groupList.includes(name) || [...sessions.values()].some((s) => s.group === name);
  groupList = [...new Set(groupList.map((g) => (g === group.name ? name : g)))];
  saveGroups();
  // 이름만 바꾼 것이면 브라우저 프로필(로그인 상태)이 새 이름을 따라간다. 그 이름으로 예전에 없앤 워크스페이스의 프로필이
  // 남아 있어도 지금 이 워크스페이스의 것으로 바꾼다. 이미 있는 워크스페이스에 합치는 것이면 옮겨 가는 항목들이 그 워크스페이스의 프로필을 쓴다.
  if (!merging) {
    if (groupProfiles[group.name]) groupProfiles[name] = groupProfiles[group.name];
    else delete groupProfiles[name];
    delete groupProfiles[group.name];
    saveProfiles();
  }
  // 워크스페이스 폴더도 이름을 따라간다. 이미 있는 워크스페이스에 합칠 때는 그 워크스페이스가 정해 둔 폴더가 먼저다.
  const own = groupFolderOf(group.name);
  if (own && !(merging && groupFolderOf(name))) groupFolders[name] = own;
  else if (!merging) delete groupFolders[name];
  delete groupFolders[group.name];
  localStorage.setItem('cypher.groupFolders', JSON.stringify(groupFolders));
  moveToGroup(membersOf(group.name), name);
}

/**
 * 워크스페이스를 없앤다. 항목은 워크스페이스 밖에 둘 수 없으므로, 세션·항목이 남아 있으면 물어보고 모두 닫은 뒤에 없앤다.
 * 닫기를 취소한 항목(저장하지 않은 메모 등)이 남으면 워크스페이스를 그대로 둔다. 없앴으면 참.
 */
async function deleteGroup(group) {
  const members = membersOf(group.name);
  if (members.length) {
    const running = members.filter((s) => isTerm(s) && !s.dormant && !s.exited).length;
    if (!confirm(`워크스페이스 "${group.name}"에 세션·항목이 ${members.length}개 있습니다${running ? ` (실행 중인 세션 ${running}개)` : ''}.\n모두 닫고 워크스페이스를 삭제할까요?\n\n삭제한 워크스페이스의 기록(항목 목록과 메모·할 일의 글)은 3년 동안 보관합니다.`)) return false;
  }
  // 닫기 전에 기록을 적어 둔다(닫은 뒤에는 화면이 글과 상태를 내놓지 못한다). 처음 읽기가 끝난 뒤의 내용으로 만든다.
  await Promise.all(members.map((s) => Promise.resolve(s.view?.ready).catch(() => {})));
  // 항목이 하나도 없는 워크스페이스는 남길 것이 없어 기록하지 않는다(잘못 만들었다 지운 워크스페이스가 3년 동안 목록에 쌓이지 않게).
  let kept = '';
  if (membersOf(group.name).length) {
    try {
      kept = await keepHistory(historyRecord(group.name, membersOf(group.name)));
    } catch (err) {
      if (!confirm(`워크스페이스 "${group.name}"의 기록을 보관하지 못했습니다:\n${err}\n\n기록 없이 삭제할까요?`)) return false;
    }
  }
  if (members.length) {
    for (const s of restingFirst(members)) await closeSession(s);
    // 닫는 사이 취소한 항목이나 새로 생긴 항목이 있으면 워크스페이스를 남긴다(삭제하지 않았으니 기록도 남기지 않는다).
    const left = membersOf(group.name).length;
    if (left) {
      if (kept) dropHistory(kept);
      notice('워크스페이스를 지우지 않았습니다', `워크스페이스 "${group.name}"에 닫지 않은 항목이 ${left}개 남아 있습니다. 그 항목을 닫거나 다른 워크스페이스로 옮긴 뒤에 다시 지우세요.`);
      return false;
    }
  }
  groupList = groupList.filter((g) => g !== group.name);
  saveGroups();
  delete groupFolders[group.name];
  localStorage.setItem('cypher.groupFolders', JSON.stringify(groupFolders));
  rooms.delete(group.name);
  render();
  const where = kept ? ' · 기록은 3년 동안 보관: 도구 → 삭제한 워크스페이스 기록' : '';
  setStatus(members.length
    ? `워크스페이스 "${group.name}"을(를) 지웠습니다 (세션·항목 ${members.length}개를 닫음 · 다시 열기: Ctrl+Shift+R${where}).`
    : `워크스페이스 "${group.name}"을(를) 지웠습니다${where ? ` (${where.slice(3)})` : ''}.`);
  return true;
}

// ── 삭제한 워크스페이스의 기록(history.js): 삭제할 때의 항목과, 글로 보관할 수 있는 내용을 3년 동안 둔다 ──

/**
 * 기록에 담을 항목 하나. 틀에 담는 모양(templateItem)에 이름(label)·종류 이름(kindLabel)을 더한 것이라, 틀을 펼치는 길로 되살린다.
 * - 메모·할 일·마인드맵·ERD는 글째, 명령 항목은 명령째 담는다(`saved.text`, `saved.command`).
 * - 세션은 프로필·폴더와 이어 갈 대화 번호(SSH는 서버 주소·포트·이름)만 담는다.
 * - 사이트·HTTP·문서·표·코드·헥사는 가리키던 주소·경로만 담고, 이미지처럼 담을 것이 없는 산출물은 이름만 남긴다(내용은 보관하지 않는다).
 */
function historyItem(s) {
  const base = {
    kind: s.kind, label: label(s), kindLabel: isTerm(s) ? s.profile.label : KINDS[s.kind].label,
    cwd: s.project || s.startCwd, name: s.name || undefined, color: s.color || undefined,
  };
  if (isTerm(s)) {
    return {
      ...base, profile: s.profile.id, resume: resumeOf(s) || undefined,
      ...(s.ssh ? { ssh: { target: s.ssh.target, name: s.ssh.name, port: s.ssh.port } } : {}),
    };
  }
  try {
    if (s.kind === 'memo') {
      // 파일에 연결된 메모도 지금의 글을 담는다(그 파일이 3년 안에 없어질 수 있다). 연결돼 있던 파일은 경로만 적어 둔다.
      const state = s.view.state();
      return { ...base, saved: { text: s.view.text(), encoding: state.encoding, eol: state.eol }, file: state.path || undefined };
    }
    const item = templateItem(s);
    if (item?.saved) return { ...base, saved: item.saved };
  } catch { /* 상태를 내놓지 못하는 항목은 이름만 남긴다 */ }
  return base;
}

/** 그 워크스페이스의 기록: { name, folder, summary("Claude 2 · 메모 1"), items }. 삭제한 때와 보관 기한은 앱이 적는다. */
function historyRecord(name, members) {
  const counts = new Map();
  const items = members.filter((s) => sessions.has(s.key)).map((s) => {
    const item = historyItem(s);
    counts.set(item.kindLabel, (counts.get(item.kindLabel) || 0) + 1);
    return item;
  });
  return { name, folder: groupFolderOf(name), summary: [...counts].map(([kind, count]) => `${kind} ${count}`).join(' · '), items };
}

/**
 * 기록을 워크스페이스 `name`으로 되살린다(있는 워크스페이스면 거기에 더한다). 틀을 펼치는 길을 쓰되, 세션은 띄우지 않고
 * 쉬는 세션으로 놓는다(누르면 재개하고, Claude 세션은 그때 하던 대화를 잇는다). 새로 만든 워크스페이스면 그때의 폴더가 아직 있을 때 그 폴더를 정한다.
 */
async function restoreHistory(record, name) {
  const items = (Array.isArray(record?.items) ? record.items : []).filter((item) => item && typeof item === 'object');
  // 내용을 보관하지 않은 것(이미지, 파일을 고르지 않았던 문서)은 되살릴 것이 없다.
  const usable = items.filter((item) => !item.kind || item.kind === 'term' || (item.saved && typeof item.saved === 'object'));
  const fresh = !groupList.includes(name);
  const own = typeof record?.folder === 'string' ? record.folder : '';
  createGroup(name);
  if (fresh && own && !/^[\\/]{2}/.test(own) && await groupFolderExists(own) && groupList.includes(name)) setGroupFolder(name, own);
  const result = await openTemplate({ items: usable }, { group: name, resting: true });
  if (items.length > usable.length) result.skipped.push(`내용을 보관하지 않은 항목 ${items.length - usable.length}개(이미지 등)는 되살리지 않았습니다`);
  return result;
}

/** 그 묶음에 새 항목을 넣을 때 쓸 워크스페이스 이름과 폴더. */
function groupContext(group) {
  const first = group.list.find((s) => sessions.has(s.key));
  // 워크스페이스 폴더를 정해 둔 워크스페이스는 그 폴더가 먼저다.
  // 폴더 묶음은 시작 폴더로 묶이므로 그 폴더를 그대로 써야 새 항목이 같은 묶음에 들어간다.
  const own = group.logical ? groupFolderOf(group.name) : '';
  const cwd = own || (!first ? folder : group.logical ? (first.project || first.startCwd) : first.startCwd);
  return { group: group.logical ? group.name : '', cwd };
}

/**
 * 그 묶음에 새 항목을 더하는 메뉴 항목들: 터미널 프로필들과 그 밖의 종류(메모, HTTP …).
 * 폴더는 그 워크스페이스의 폴더, 정해 두지 않았으면 그 묶음에 이미 있는 항목의 것을 따른다(없으면 기본 폴더).
 */
function addItems(group) {
  const opts = groupContext(group);
  // 워크스페이스 폴더를 정해 둔 워크스페이스의 새 세션은 폴더를 넘기지 않는다. newSession이 그 워크스페이스의 폴더를 찾아 쓰고,
  // 그 폴더가 없어졌으면 기본 폴더에서 시작한다.
  const forSession = group.logical && groupFolderOf(group.name) ? { group: opts.group } : opts;
  return [
    { caption: `폴더: ${opts.cwd}${group.logical && groupFolderOf(group.name) ? ' (워크스페이스 폴더)' : ''}` },
    // SSH 세션은 접속할 곳을 고르는 창이 먼저 뜬다. SSH는 워크스페이스마다 하나라서, 이미 있으면 그 SSH의 탭으로 더한다.
    ...config.profiles.map((p) => ({
      label: !isSshProfile(p) ? `+ ${p.label} 세션`
        : group.logical && sshOf(group.name).length ? `+ ${p.label}: 서버에 더 접속… (이 워크스페이스 SSH의 새 탭으로)` : `+ ${p.label} 세션… (서버에 접속 · 워크스페이스마다 하나)`,
      action: () => newSession(p.id, forSession),
    })),
    ...Object.entries(KINDS).map(([kind, def]) => ({ label: kind === 'code' ? `+ ${def.label}… (볼 폴더를 고름)` : `+ ${def.label}`, action: () => createKind(kind, opts) })),
  ];
}

// ── 워크스페이스 틀: 묶음의 항목 구성을 파일로 저장해 두고, 같은 종류의 업무를 시작할 때 한 번에 펼친다(template.js) ──

const MAX_TEMPLATE_ITEMS = 40;  // 틀 하나로 한 번에 펼치는 항목 수

/** 지금 보고 있는 항목이 속한 묶음. */
function currentGroup() {
  if (!active || !sessions.has(active.key)) return null;
  return groupsInOrder().find((group) => group.list.includes(active)) || null;
}

/** 틀에 담을 항목 하나. 담지 않는 것(이미지, 파일을 고르지 않은 문서)은 null. */
function templateItem(s) {
  const base = { kind: s.kind, cwd: s.project || s.startCwd, name: s.name || undefined, color: s.color || undefined };
  // 세션은 프로필과 폴더만 담는다(대화는 담지 않고, 펼칠 때 새로 시작한다).
  // SSH 세션은 접속할 곳(주소·포트·이름)도 담는다. 사용자 이름은 담지 않는다(접속할 때 세션 화면에서 묻는다).
  if (isTerm(s)) return { ...base, profile: s.profile.id, ...(s.ssh ? { ssh: { target: s.ssh.target, name: s.ssh.name, port: s.ssh.port } } : {}) };
  const state = s.view.state();
  switch (s.kind) {
    case 'memo':
      // 파일에 연결돼 있고 고친 것이 없으면 그 파일을, 아니면 지금의 글을 담는다.
      return { ...base, saved: state.path && !state.dirty ? { open: state.path } : { text: s.view.text(), encoding: state.encoding, eol: state.eol } };
    case 'mindmap': return { ...base, saved: { text: s.view.text() } };
    case 'erd': return { ...base, saved: { text: s.view.text() } };
    case 'site': return { ...base, saved: { url: state.url, home: state.home, title: state.title } };
    case 'http': return { ...base, saved: { file: state.file } };
    case 'reader': return state.path ? { ...base, saved: { path: state.path } } : null;
    case 'todo': return { ...base, saved: { text: s.view.text() } };
    case 'run': return state.command ? { ...base, saved: { command: state.command } } : null;
    case 'table': return state.path ? { ...base, saved: { path: state.path } } : null;
    case 'hex': return state.path ? { ...base, saved: { path: state.path } } : null;
    // S3 저장소는 접속 정보의 번호와 이름, 보던 곳만 담는다(키는 화면에 없고, 주소도 담지 않는다).
    case 's3': return state.conn ? { ...base, saved: { conn: state.conn, name: state.name, path: state.path } } : null;
    // 코드 보기는 프로젝트 폴더만 담는다(열어 둔 파일은 담지 않는다).
    case 'code': return { ...base, saved: { root: state.root } };
    default: return null;
  }
}

/** 묶음의 항목을 틀에 담는다: { items, skipped(담지 않은 것의 이름), summary("Claude 2 · 메모 1") } */
function packGroup(group) {
  const items = [], skipped = [], counts = new Map();
  for (const s of group.list.filter((other) => sessions.has(other.key)).slice(0, MAX_TEMPLATE_ITEMS)) {
    const kind = isTerm(s) ? s.profile.label : KINDS[s.kind].label;
    let item = null;
    try { item = templateItem(s); } catch { /* 상태를 내놓지 못하는 항목은 담지 않는다 */ }
    if (!item) { skipped.push(`${label(s)}(${kind})`); continue; }
    items.push(item);
    counts.set(kind, (counts.get(kind) || 0) + 1);
  }
  return { items, skipped, summary: [...counts].map(([kind, count]) => `${kind} ${count}`).join(' · ') };
}

/**
 * 틀 파일에서 읽은 항목의 내용 중 그 종류가 알아듣는 것만 고른다. 틀 파일은 다른 사람에게서 받은 것일 수 있으므로,
 * 저장소 번호(메모 번호 등)처럼 이 PC의 다른 항목을 가리킬 수 있는 값은 받지 않는다.
 */
function templateSaved(kind, saved) {
  const from = saved && typeof saved === 'object' ? saved : {};
  const text = (key) => (typeof from[key] === 'string' ? { [key]: from[key] } : {});
  switch (kind) {
    case 'memo': return typeof from.open === 'string' && from.open ? text('open') : { text: '', ...text('text'), ...text('encoding'), ...text('eol') };
    case 'mindmap': return { ...text('text') };
    case 'erd': return { ...text('text') };
    case 'site': return { ...text('url'), ...text('home'), ...text('title') };
    case 'http': return { ...text('file') };
    case 'reader': return { ...text('path') };
    case 'todo': return { ...text('text') };
    // 명령은 받아 두기만 하고 실행하지 않는다(틀을 펼친 뒤 사용자가 ▶를 눌러야 돈다).
    case 'run': return { ...text('command') };
    case 'table': return { ...text('path') };
    case 'hex': return { ...text('path') };
    // 접속 정보의 번호는 이 PC의 금고에 있는 것만 통한다(없는 번호면 그 항목이 접속 정보를 고르라고 한다).
    // 어느 쪽이든 펼치기만 해서는 접속하지 않는다.
    case 's3': return typeof from.conn === 'string' && /^c[0-9a-f]{1,32}$/.test(from.conn) ? { ...text('conn'), ...text('name'), ...(typeof from.path === 'string' && from.path.startsWith('s3://') ? text('path') : {}) } : null;
    case 'code': return { ...text('root') };
    default: return null;
  }
}

/**
 * 틀을 워크스페이스로 펼친다. `cwd`를 주면 모든 항목을 그 폴더에서 열고, 아니면 저장해 둔 폴더에서 연다
 * (없어진 폴더의 항목은 기본 폴더에서). `cwd`를 줬고 그 워크스페이스에 폴더를 정해 두지 않았으면 그 폴더를 워크스페이스의 폴더로 정한다.
 * 만든 항목 수와 건너뛴 것의 까닭을 돌려준다.
 * `resting`(삭제한 워크스페이스의 기록을 되살릴 때): 세션을 띄우지 않고 쉬는 세션으로 놓고, 적혀 있는 대화 번호를 이어 갈 대화로 둔다.
 */
async function openTemplate(template, { group, cwd = null, resting = false }) {
  const all = (Array.isArray(template?.items) ? template.items : []).filter((item) => item && typeof item === 'object');
  // 기록을 되살릴 때는 수를 줄이지 않는다: 빠진 항목은 다시 되살려도 돌아오지 않는다(기록 하나의 항목 수는 앱이 한도를 둔다).
  const items = resting ? all : all.slice(0, MAX_TEMPLATE_ITEMS);
  const skipped = [];
  if (all.length > items.length) skipped.push(`한 번에 ${MAX_TEMPLATE_ITEMS}개까지만 펼칩니다`);
  const folders = [...new Set(items.map((item) => item.cwd).filter((path) => typeof path === 'string' && path))];
  const exist = cwd ? [] : await invoke('dirs_exist', { paths: folders }).catch(() => folders.map(() => true));
  const missing = new Set(folders.filter((_, index) => exist[index] === false));
  ensureGroup(group);
  if (cwd && !groupFolderOf(group)) setGroupFolder(group, cwd);
  const made = [];
  let moved = 0;
  for (const item of items) {
    const term = !item.kind || item.kind === 'term';
    const saved = term ? null : templateSaved(item.kind, item.saved);
    // 다른 폴더에서 펼치는 코드 보기는 그 폴더를 본다.
    if (saved && item.kind === 'code' && cwd) saved.root = cwd;
    // 틀 파일은 다른 사람에게서 받은 것일 수 있다. SSH 세션은 주소·포트·이름만 받는다(예전 틀의 `user@`은 뗀다).
    const ssh = term && item.profile === SSH ? cleanSpec({ target: item.ssh?.target, name: item.ssh?.name, port: item.ssh?.port }) : null;
    const known = term ? config.profiles.some((p) => p.id === item.profile) && (item.profile !== SSH || !!ssh) : !!KINDS[item.kind] && !!saved;
    const title = typeof item.name === 'string' ? item.name : '';
    if (!known) {
      skipped.push(`${title || item.profile || item.kind}: 이 Cypher에서 펼칠 수 없는 종류입니다`);
      continue;
    }
    const kept = typeof item.cwd === 'string' && item.cwd && !missing.has(item.cwd);
    if (!cwd && !kept) moved += 1;
    try {
      const s = await revive({
        kind: term ? 'term' : item.kind, profile: item.profile, saved, ssh,
        ...(resting && term && typeof item.resume === 'string' && /^[\w-]{1,80}$/.test(item.resume) ? { resume: item.resume } : {}),
        name: title, color: COLORS.includes(item.color) ? item.color : '',
        cwd: cwd || (kept ? item.cwd : folder), group,
        // 틀로 펼친 SSH 세션은 쉬는 세션으로 둔다. 틀은 남에게서 받을 수 있으니, 펼치자마자 그 주소로 접속하지 않고 사용자가 누를 때 접속한다.
      }, { show: 'hidden', ...(ssh || (resting && term) ? { dormant: true } : {}) });
      if (s) made.push(s);
    } catch (err) {
      skipped.push(`${title || item.kind}: ${err}`);
    }
  }
  if (moved) skipped.push(`폴더가 없어진 항목 ${moved}개는 ${folder}에서 열었습니다`);
  reveal(made);
  // 쉬는 세션으로 놓은 것(SSH)은 고르지 않는다: 고르면 바로 접속한다.
  const first = made.find((s) => !s.dormant);
  if (first) activate(first);
  return { made: made.length, skipped };
}

function groupMenu(group, ev) {
  // 사이드바가 "대기만 보기"로 걸러져 있으면 받은 묶음에는 대기 중인 항목만 들어 있다.
  // 묶음 전체를 다루는 일(틀로 저장, 워크스페이스로 만들기)은 걸러지지 않은 묶음으로 하고,
  // 화면에 보이는 것을 다루는 일(나란히 보기, 모두 닫기)은 보이는 항목으로 한다.
  const live = group.list.filter((s) => sessions.has(s.key));
  const whole = groupsInOrder().find((g) => g.key === group.key) ?? group;
  const all = whole.list.filter((s) => sessions.has(s.key));
  const items = [];
  items.push(...addItems(group));
  const resting = wakeable(whole.list).length;
  if (resting) items.push({ label: `쉬는 세션 ${resting}개 모두 재개${whole.list.some((s) => s.ssh && s.dormant) ? ' (SSH는 빼고)' : ''}`, action: () => wakeGroup(whole) });
  if (all.length) items.push({ label: '이 워크스페이스를 틀로 저장… (같은 구성을 다시 펼칠 수 있게)', action: () => saveTemplate(whole) });
  if (group.logical) {
    // 워크스페이스 폴더: 이 워크스페이스에서 여는 세션이 시작될 폴더. 한 번 정하면 바꿀 수 없어, 정해 둔 워크스페이스에서는 탐색기로 열기만 한다.
    const own = groupFolderOf(group.name);
    items.push(own
      ? { label: '워크스페이스 폴더 열기 (탐색기)', action: () => openGroupFolder(group.name) }
      : { label: '워크스페이스 폴더 정하기… (세션이 시작될 폴더 · 한 번 정하면 바꿀 수 없음)', action: () => pickGroupFolder(group.name) });
    items.push({ label: '워크스페이스 이름 바꾸기', action: () => renameGroup(group) });
    // 순서는 머리를 끌어서도 바꾼다. 메뉴로는 한 칸씩 옮긴다.
    const at = groupList.indexOf(group.name);
    if (at > 0) items.push({ label: '위로 옮기기 (머리를 끌어도 됩니다)', action: () => nudgeGroup(group.name, -1) });
    if (at >= 0 && at < groupList.length - 1) items.push({ label: '아래로 옮기기', action: () => nudgeGroup(group.name, 1) });
    items.push({ label: '워크스페이스 삭제', action: () => deleteGroup(whole) }); // 항목이 남아 있으면 물어보고 모두 닫은 뒤 지운다.
  } else {
    items.push({ label: '이 묶음을 워크스페이스로 만들기…', action: () => askGroup(all) });
  }
  // 여러 항목을 나란히 띄우는 메뉴는 두지 않는다: 창 분할은 사용자가 사이드바나 탭에서 끌어다 놓을 때만 한다.
  items.push({
    label: '이 워크스페이스의 세션 모두 닫기',
    action: () => {
      if (confirm(`"${group.name}" 워크스페이스의 세션 ${live.length}개를 모두 닫을까요?`)) restingFirst(live).forEach((s) => closeSession(s));
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
    drag.ghost = make('div', 'drag-ghost', drag.group ? `◆ ${drag.group}` : drag.room ? `⇌ SSH · ${drag.room}` : drag.tab ? tabName(drag.s) : label(drag.s));
    document.body.append(drag.ghost);
    $list.classList.add('dragging');
    if (!drag.group && !drag.tab) render(); // "새 워크스페이스" 놓을 자리를 보여 준다.
  }
  drag.ghost.style.left = `${ev.clientX + 12}px`;
  drag.ghost.style.top = `${ev.clientY + 8}px`;
  for (const row of $list.querySelectorAll('li.drop, li.before, li.after')) row.classList.remove('drop', 'before', 'after');
  const point = document.elementFromPoint(ev.clientX, ev.clientY);
  if (drag.group) {
    // 워크스페이스를 끄는 중: 놓으면 끼어들 자리(다른 워크스페이스의 위나 아래)를 선으로 보여 준다.
    const over = groupUnder(point, ev.clientY);
    if (over && over.name !== drag.group) (over.after ? over.last : over.head).classList.add(over.after ? 'after' : 'before');
    return;
  }
  if (!drag.tab) point?.closest?.('li.group[data-drop]')?.classList.add('drop'); // SSH 탭은 사이드바에 놓이지 않는다.
  // 다른 항목의 줄 위에서는 놓으면 끼어들 자리(그 줄의 위나 아래)를 선으로 보여 준다(SSH 탭은 사이드바의 줄 사이에 끼우지 않는다).
  const over = rowUnder(point, ev.clientY);
  if (over && !drag.tab && !ownRow(over, drag)) over.row.classList.add(over.after ? 'after' : 'before');
  if (drag.tab && point?.closest?.('#sidebar')) return showDropHint(null);
  // 터미널 화면 위에서는 놓으면 창이 어떻게 나뉘는지(SSH 탭 줄이면 그 창의 탭으로 들어가는지) 미리 보여 준다.
  showDropHint(dropZoneAt(ev.clientX, ev.clientY, drag));
});

/** 끌던 것을 제 줄 위에 놓았는지(SSH 줄을 끌면 그 워크스페이스의 SSH 줄이 제 줄이다). */
const ownRow = (over, d) => (d.room ? over.row.dataset.room === d.room : over.target === d.s);

document.addEventListener('mouseup', (ev) => {
  if (!drag) return;
  const dragged = drag;
  const { s, group: draggedGroup, active: moved, ghost } = drag;
  drag = null;
  if (!moved) return; // 그냥 클릭
  ghost.remove();
  $list.classList.remove('dragging');
  showDropHint(null);
  draggedAt = performance.now();
  const point = document.elementFromPoint(ev.clientX, ev.clientY);
  if (draggedGroup) {
    // 워크스페이스를 다른 워크스페이스의 위나 아래에 놓음: 사이드바에서의 순서를 바꾼다.
    const over = groupUnder(point, ev.clientY);
    if (over) moveGroup(draggedGroup, over.name, over.after);
    return render();
  }
  // SSH 탭은 SSH 창 안에서만 옮긴다: 사이드바(워크스페이스 머리, 다른 줄)에 놓으면 아무 일도 하지 않는다.
  // (탭 하나를 다른 워크스페이스로 보내는 것은 탭 오른쪽 클릭 → "워크스페이스 바꾸기…"로 한다.)
  if (dragged.tab && point?.closest?.('#sidebar')) {
    setStatus('SSH 탭은 SSH 창 안에서만 옮깁니다. 다른 워크스페이스로 보내려면 탭을 오른쪽 클릭 → "워크스페이스 바꾸기…"를 쓰세요.');
    return render();
  }
  // 옮길 것: 사이드바의 SSH 줄을 끌었으면 그 워크스페이스의 SSH 세션 전부(방 통째), 아니면 그 항목 하나.
  const moving = dragged.room ? sshOf(dragged.room) : [s];
  const head = point?.closest?.('li.group[data-drop]');
  if (head) {
    // 묶음 머리에 놓음: 그 묶음으로 옮긴다(SSH는 옮겨 간 워크스페이스의 SSH 탭이 된다).
    if (head.dataset.drop === 'new') askGroup(moving).then(render);
    else if (head.dataset.drop === 'logical') moveToGroup(moving, head.dataset.name);
    else render();
    return;
  }
  // 다른 항목의 줄에 놓음: 그 줄의 위나 아래로 순서를 옮긴다. 다른 워크스페이스의 줄이면 그 워크스페이스로도 옮긴다.
  // SSH 탭은 사이드바의 줄 사이에 끼우지 않는다(SSH는 워크스페이스마다 한 줄이고, 그 줄을 끌어 옮긴다).
  const over = rowUnder(point, ev.clientY);
  if (over) {
    if (!ownRow(over, dragged)) {
      const to = over.target.group;
      if (to && to !== s.group) moveToGroup(moving, to); // 워크스페이스 밖으로는 옮기지 않는다.
      // SSH 줄의 뒤에 끼울 때는 그 워크스페이스의 마지막 SSH 세션 뒤에 끼운다(SSH 세션들 사이에 끼어들지 않게).
      const anchor = over.row.dataset.room && over.after ? sshOf(over.row.dataset.room).filter((x) => !moving.includes(x)).pop() || over.target : over.target;
      // 여럿(SSH 세션들)을 옮길 때도 서로의 순서는 지킨다: 뒤에 끼울 때는 거꾸로 하나씩 끼운다.
      for (const item of over.after ? [...moving].reverse() : moving) if (item !== anchor) placeInOrder(item, anchor, over.after);
      saveState();
    }
    return render();
  }
  // 터미널 화면에 놓음: 놓은 위치에 따라 그 창을 나누거나 그 자리에 연다(SSH 탭은 SSH 방 안에서 옮긴다).
  const hit = dropZoneAt(ev.clientX, ev.clientY, dragged);
  if (hit) dropOnPane(dragged, hit);
  else if (!dragged.tab && point?.closest?.('#terms')) activate(s);
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

/** 끌던 것을 그만둔다(Esc, 놓는 신호를 받지 못한 채 다시 눌렀을 때). 따라다니던 표시와 놓을 자리 표시를 치운다. 그만둔 것이 있었으면 참. */
function cancelDrag() {
  if (!drag) return false;
  const moved = drag.active;
  drag.ghost?.remove();
  drag = null;
  if (!moved) return false;
  $list.classList.remove('dragging');
  showDropHint(null);
  draggedAt = performance.now();
  render();
  return true;
}
// 끄는 중에 Esc를 누르면 옮기지 않고 그만둔다.
window.addEventListener('keydown', (ev) => {
  if (ev.key === 'Escape' && drag?.active && cancelDrag()) { ev.preventDefault(); ev.stopPropagation(); }
}, true);

/** 끌기를 막 끝낸 때. 끌다가 처음 누른 줄 위에서 놓으면 클릭도 함께 오는데, 그 클릭으로 묶음이 접히지 않게 한다. */
let draggedAt = -Infinity;

/**
 * 그 자리(point)에 있는 워크스페이스와, 세로 위치로 본 끼어들 자리. 워크스페이스의 머리뿐 아니라 그 아래에 펼쳐진 항목 줄 위도
 * 그 워크스페이스로 친다. 머리부터 마지막 줄까지의 위쪽 절반이면 그 워크스페이스의 앞, 아래쪽 절반이면 뒤다.
 */
function groupUnder(point, y) {
  const row = point?.closest?.('#sessions li');
  if (!row) return null;
  let head = row;
  while (head && !head.classList.contains('group')) head = head.previousElementSibling;
  if (!head || head.dataset.drop !== 'logical' || !groupList.includes(head.dataset.name)) return null;
  let last = head;
  while (last.nextElementSibling && !last.nextElementSibling.classList.contains('group')) last = last.nextElementSibling;
  const top = head.getBoundingClientRect().top, bottom = last.getBoundingClientRect().bottom;
  return { name: head.dataset.name, head, last, after: y > (top + bottom) / 2 };
}

/** 워크스페이스 `name`을 사이드바 순서에서 `target`의 바로 앞(after이면 바로 뒤)으로 옮긴다. 이 순서는 다시 시작해도 유지된다. 옮겼으면 참. */
function moveGroup(name, target, after) {
  if (name === target || !groupList.includes(name) || !groupList.includes(target)) return false;
  const order = groupList.filter((other) => other !== name);
  order.splice(order.indexOf(target) + (after ? 1 : 0), 0, name);
  if (order.every((other, index) => other === groupList[index])) return false; // 제자리
  groupList = order;
  saveGroups();
  render();
  return true;
}

/** 워크스페이스를 사이드바에서 한 칸 위(step -1)나 아래(step 1)로 옮긴다. */
function nudgeGroup(name, step) {
  const at = groupList.indexOf(name), to = at + step;
  if (at < 0 || to < 0 || to >= groupList.length) return false;
  return moveGroup(name, groupList[to], step > 0);
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
 * 사이드바에 보이는 차례대로의 묶음들: 만들어 둔 워크스페이스가 먼저(비어 있어도), 그 뒤로 폴더 묶음이 처음 나온 순서대로.
 * `waitingOnly`이면 대기 중인 세션만 담고 빈 워크스페이스는 뺀다.
 */
function groupsInOrder({ waitingOnly = false } = {}) {
  const groups = new Map();
  if (!waitingOnly) {
    for (const name of groupList) groups.set(`g:${name}`, { key: `g:${name}`, logical: true, name, hint: `워크스페이스: ${name}`, list: [] });
  }
  for (const s of sessions.values()) {
    if (waitingOnly && !s.waiting) continue;
    const key = groupKey(s);
    if (!groups.has(key)) {
      groups.set(key, { key, logical: !!s.group, name: s.group || baseName(s.startCwd), hint: s.group ? `워크스페이스: ${s.group}` : s.startCwd, list: [] });
    }
    groups.get(key).list.push(s);
  }
  return [...groups.values()];
}

// 사이드바의 줄이나 단추를 누르고 있는 동안(누름~뗌) 목록을 다시 그리면 눌린 줄이 새 것으로 바뀌어 그 클릭이 사라진다.
// Claude 세션이 일하는 동안에는 상태줄·훅이 올 때마다 다시 그리므로 드물지 않다. 그동안의 다시 그리기는 뗀 뒤로 미룬다.
let pressedAt = 0;      // 사이드바에서 마우스 단추를 누른 때(누르고 있지 않으면 0)
let renderHeld = false; // 누르고 있어서 미뤄 둔 다시 그리기가 있는지
let heldTimer = 0;      // 뗌을 놓쳤을 때를 대비한 타이머(누름마다 하나)
let heldFor = 0;        // 그 타이머가 지키는 누름의 때
function flushHeldRender() {
  clearTimeout(heldTimer);
  heldTimer = 0;
  pressedAt = 0;
  if (!renderHeld) return;
  renderHeld = false;
  render();
}
$('sidebar').addEventListener('mousedown', () => { pressedAt = performance.now(); }, true);
// 뗀 뒤의 click이 지나가고 나서 그린다.
document.addEventListener('mouseup', () => { if (pressedAt) setTimeout(flushHeldRender); }, true);

function render() {
  // 사이드바를 누르고 있는 동안에는 미룬다(끌기 시작은 스스로 다시 그려야 하므로 예외). 뗌을 놓쳐도 목록이 굳지 않게 시간을 둔다.
  if (pressedAt && !drag?.active) {
    if (performance.now() - pressedAt < PRESS_HOLD_MS) {
      if (heldFor !== pressedAt) {
        // 이번 누름을 위한 타이머를 새로 건다(앞선 누름의 타이머가 이번 누름 도중에 끝나 버리지 않게).
        clearTimeout(heldTimer);
        heldFor = pressedAt;
        heldTimer = setTimeout(() => { if (renderHeld) flushHeldRender(); }, PRESS_HOLD_MS - (performance.now() - pressedAt));
      }
      renderHeld = true;
      return;
    }
    pressedAt = 0;
  }
  renderHeld = false;
  // 끄는 중에 목록을 다시 그리면 놓을 자리가 사라지므로 미뤄 둔다.
  if (drag?.active && $list.querySelector('li.group.new')) return;
  // 별칭을 고치는 중에도 입력칸이 사라지지 않게 미룬다(끝나면 다시 그린다).
  if (renaming != null && $list.querySelector('input.rename')) return;

  const rows = [];
  for (const group of groupsInOrder({ waitingOnly: filterWaiting })) {
    const folded = collapsed.has(group.key);
    const head = make('li', [
      'group', group.logical && 'logical', folded && 'folded', group.list.some((s) => s.attention || s.waiting) && 'attention',
    ].filter(Boolean).join(' '));
    const ownFolder = group.logical ? groupFolderOf(group.name) : '';
    head.title = `${group.hint}\n${group.logical ? `워크스페이스 폴더: ${ownFolder ? `${ownFolder} (바꿀 수 없음)` : `정하지 않음 (기본 폴더 ${folder}에서 시작)`}\n` : ''}클릭: 접기·펼치기 · 오른쪽 클릭: 메뉴${folded ? '(세션·항목 더하기 포함)' : ' · +: 세션·항목 더하기'} · 항목을 끌어다 놓기: 이 워크스페이스에 넣기${group.logical ? ' · 이 머리를 끌기: 워크스페이스 순서 바꾸기' : ''}`;
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
    // 접어 둔 워크스페이스는 이름만 보이는 납작한 한 줄이다(폴더 줄·진행·단추·항목 수는 펼쳤을 때만 보인다).
    // 그 안에 대기 중인 세션이 있는지는 이름 옆의 점으로만 알린다(CSS의 .folded.attention).
    if (!folded) {
      // 묶음의 할 일 진행(그 묶음의 할 일 항목을 모두 합친 것).
      const todo = group.list.reduce((sum, s) => {
        const part = isTerm(s) ? null : s.view?.progress?.();
        return part ? { done: sum.done + part.done, total: sum.total + part.total } : sum;
      }, { done: 0, total: 0 });
      const progress = make('span', todo.total ? `todo-pill${todo.done === todo.total ? ' full' : ''}` : '', todo.total ? `${todo.done}/${todo.total}` : '');
      if (todo.total) progress.title = `이 워크스페이스의 할 일: ${todo.total}개 가운데 ${todo.done}개 끝냄`;
      head.append(progress);
      // 묶음 안에서 돌고 있는 서브에이전트 수.
      const running = group.list.reduce((sum, s) => sum + s.agents.size, 0);
      head.append(running ? agentPill(running, `이 워크스페이스에서 서브에이전트 ${running}개 실행 중`) : make('span'));
      // 쉬는 세션이 있는 묶음: 한 번에 재개하는 단추. SSH 세션은 세지 않는다(SSH는 SSH 줄을 눌러야 접속한다).
      const resting = wakeable(group.list).length;
      const play = make(resting ? 'button' : 'span', resting ? 'close wake' : '', resting ? '▶' : '');
      if (resting) {
        play.title = `쉬는 세션 ${resting}개 재개${group.list.some((s) => s.ssh && s.dormant) ? ' (SSH는 SSH 줄을 눌러 접속)' : ''}`;
        play.onclick = (ev) => { ev.stopPropagation(); wakeGroup(group); };
      }
      head.append(play);
      const add = make('button', 'close add', '+'); // 세션·항목을 만드는 길이므로 마우스를 올리지 않아도 늘 보인다.
      add.title = '이 워크스페이스에 세션·항목 더하기';
      add.onclick = (ev) => { ev.stopPropagation(); showMenu(ev, addItems(group)); };
      head.append(add);
      head.append(make('span', 'count', String(sidebarRows(group.list).length)));
      // 워크스페이스의 폴더 줄: 이름 바로 아래에 워크스페이스 폴더를 그대로 보여 준다.
      // 정해 둔 폴더는 바꿀 수 없어 [폴더] 단추(와 경로)는 그 폴더를 탐색기에서 열기만 한다. 정하지 않은 워크스페이스만 여기서 한 번 고른다.
      if (group.logical) {
        const row = make('div', ownFolder ? 'group-folder' : 'group-folder unset');
        // 오른쪽 정렬 말줄임에서 글자 순서가 뒤집히지 않도록 방향 표시 문자를 붙인다.
        const path = make('span', 'path', ownFolder ? '‎' + ownFolder : '폴더를 정하지 않음');
        path.title = ownFolder
          ? `워크스페이스 "${group.name}"의 폴더: ${ownFolder}\n이 워크스페이스에서 여는 세션은 여기서 시작합니다. 이 폴더는 바꿀 수 없습니다.\n클릭: 탐색기에서 열기`
          : `워크스페이스 "${group.name}"의 폴더를 정하지 않았습니다. 이 워크스페이스의 세션은 이 워크스페이스에 있는 항목의 폴더나 기본 폴더(${folder})에서 시작합니다.\n클릭·▾: 정하기 (한 번 정하면 바꿀 수 없습니다)`;
        row.onclick = (ev) => ev.stopPropagation(); // 폴더 줄을 눌러도 묶음이 접히지 않게
        if (ownFolder) {
          const open = make('button', '', '폴더');
          open.title = `탐색기에서 이 폴더 열기: ${ownFolder}`;
          path.onclick = open.onclick = () => openGroupFolder(group.name);
          row.append(path, open);
        } else {
          const recent = make('button', '', '▾');
          recent.title = '지금 항목의 폴더·최근 폴더에서 골라 이 워크스페이스의 폴더로 정하기 (한 번 정하면 바꿀 수 없습니다)';
          const pick = make('button', '', '폴더…');
          pick.title = '이 워크스페이스의 폴더 정하기 (한 번 정하면 바꿀 수 없습니다)';
          path.onclick = recent.onclick = (ev) => groupFolderMenu(ev, group.name);
          pick.onclick = () => pickGroupFolder(group.name);
          row.append(path, recent, pick);
        }
        head.append(row);
      }
    }
    // 워크스페이스의 머리를 끌면 다른 워크스페이스의 위나 아래로 순서를 옮긴다(단추와 폴더 줄에서는 끌기를 시작하지 않는다).
    if (group.logical && !filterWaiting) {
      head.onmousedown = (ev) => {
        if (ev.button === 0 && !ev.ctrlKey && !ev.target.closest('button, input, .group-folder')) { cancelDrag(); drag = { group: group.name, x: ev.clientX, y: ev.clientY, active: false }; }
      };
    }
    head.onclick = () => {
      if (performance.now() - draggedAt < 300) return; // 끌다가 제자리에 놓은 것
      if (folded) collapsed.delete(group.key); else collapsed.add(group.key);
      localStorage.setItem('cypher.collapsed', JSON.stringify([...collapsed]));
      render();
    };
    head.oncontextmenu = (ev) => { ev.preventDefault(); groupMenu(group, ev); };
    rows.push(head);
    // SSH 세션은 몇 개든 그 워크스페이스의 SSH 줄 하나로 놓인다(SSH는 워크스페이스마다 하나).
    if (!folded) rows.push(...sidebarRows(group.list).map((row) => (row.list ? sshItem(row.room, row.list) : sessionItem(row.s))));
  }
  if (renaming != null && !sessions.has(renaming)) renaming = null;
  if (filterWaiting && !rows.length) rows.push(make('li', 'group', '대기 중인 세션이 없습니다.'));
  // 워크스페이스가 하나도 없으면 무엇부터 해야 하는지 알려 준다(세션과 항목은 워크스페이스 안에서만 만든다).
  if (!filterWaiting && !rows.length) {
    const hint = make('li', 'group new', '워크스페이스가 없습니다 — 여기를 누르거나 위의 [+ 워크스페이스]로 워크스페이스를 만드세요');
    hint.title = '세션과 항목은 워크스페이스 안에서만 만들 수 있습니다';
    hint.onclick = () => newGroup();
    rows.push(hint);
  }
  if (drag?.active && !drag.group && !drag.tab) {
    const fresh = make('li', 'group new', '+ 새 워크스페이스로 (여기에 놓기)');
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

  syncSites(); // 워크스페이스가 바뀐 사이트 항목은 새 워크스페이스의 브라우저 프로필로 다시 연다.
  const waitingCount = [...sessions.values()].filter((s) => s.waiting).length;
  const $waiting = $('waiting');
  $waiting.textContent = `대기 ${waitingCount}`;
  $waiting.classList.toggle('has', waitingCount > 0);
  $waiting.classList.toggle('on', filterWaiting);

  // 화면을 나눠 볼 때 창마다 붙는 제목 줄: 이 창만 남기기, 이 창 숨기기
  for (const s of visible) {
    // 단추는 한 번만 만든다. 활성이 아닌 창을 누르면 그 창이 활성화되며 다시 그리는데(mousedown),
    // 그때마다 단추를 새로 만들면 방금 눌린 단추가 사라져 그 클릭이 먹히지 않는다.
    if (!s.headInfo) {
      // 나눠 볼 때만 쓰는 단추(한 창만 볼 때는 숨긴다)
      const only = make('button', 'pane-button split-only', '이 창만');
      only.title = '이 창만 보기 (제목 줄 두 번 누르기)';
      only.onclick = (ev) => { ev.stopPropagation(); showOnly(s); };
      const hide = make('button', 'pane-button split-only', '숨기기');
      hide.title = '이 창 숨기기 (세션은 계속 실행)';
      hide.onclick = (ev) => { ev.stopPropagation(); toggleShown(s); };
      s.headInfo = make('span', 'pane-info');
      s.head.replaceChildren(s.headInfo, only, hide);
      if (s.ssh) {
        only.title = '이 창만 보기: SSH의 다른 창은 이 창의 탭으로 모으고 나머지 항목은 숨깁니다 (제목 줄 두 번 누르기)';
        hide.title = '이 창 접기: 탭들은 옆 창의 탭 줄로 갑니다. SSH에 창이 하나뿐이면 SSH를 화면에서 숨깁니다 (세션은 계속 실행)';
        // SSH 세션은 한 창만 볼 때도 제목 줄을 보이고, 거기에 출력 기록·동시 입력 단추를 눈에 띄게 둔다.
        s.pane.classList.add('ssh');
        // 맨 왼쪽의 탭 줄: 이 창 자리의 SSH 탭들(계정@서버 ×). 탭이 넘치면 가로로 밀린다(휠로도 민다).
        s.tabStrip = make('div', 'ssh-tabs');
        s.tabStrip.addEventListener('wheel', (ev) => {
          if (!ev.deltaY || s.tabStrip.scrollWidth <= s.tabStrip.clientWidth) return;
          s.tabStrip.scrollLeft += ev.deltaY;
          ev.preventDefault();
        }, { passive: false });
        s.head.prepend(s.tabStrip);
        // 창이 좁아지거나 넓어져도 지금 탭이 탭 줄 밖으로 밀려나지 않게 한다.
        new ResizeObserver(() => revealTab(s)).observe(s.tabStrip);
        // 탭과 단추 사이의 빈 곳(나눠 볼 때는 그 사이에 워크스페이스 이름이 온다)
        s.head.insertBefore(make('span', 'pane-fill'), only);
        // 글자 크기: SSH 세션에만 적용된다(모든 SSH 세션이 함께 쓴다). [가−] 크기 [가+], 크기를 누르면 기본으로.
        const fontBox = make('span', 'pane-font');
        const fontButton = (text, tip, change) => {
          const b = make('button', '', text);
          b.title = tip;
          b.onclick = (ev) => { ev.stopPropagation(); change(); focusItem(s); };
          return b;
        };
        s.fontLabel = fontButton('', '', () => setSshFont(0));
        s.fontLabel.className = 'pane-font-size';
        fontBox.append(
          fontButton('가−', 'SSH 글자 작게 (Ctrl+휠 아래로) · SSH 세션에만 적용', () => setSshFont(sshFontNow() - 1)),
          s.fontLabel,
          fontButton('가+', 'SSH 글자 크게 (Ctrl+휠 위로) · SSH 세션에만 적용', () => setSshFont(sshFontNow() + 1)),
        );
        // 단추를 눌러도 입력 초점이 터미널에서 떠나지 않게 한다(누른 뒤 바로 이어서 칠 수 있게).
        fontBox.addEventListener('mousedown', (ev) => ev.preventDefault());
        s.head.insertBefore(fontBox, only);
        // 출력 기록: 누르면 기록할 파일을 고르고 시작한다. 기록 중에 누르면 멈춘다.
        s.logButton = make('button', 'pane-tool rec');
        s.logButton.onclick = (ev) => { ev.stopPropagation(); toggleLog(s); };
        // 동시 입력: 나란히 보이는 SSH 창끼리 같은 글쇠를 넣는다(화면을 스스로 나누지는 않는다).
        s.castButton = make('button', 'pane-tool cast');
        s.castButton.onclick = (ev) => { ev.stopPropagation(); toggleSshCast(s); };
        s.head.insertBefore(s.logButton, only);
        s.head.insertBefore(s.castButton, only);
        requestAnimationFrame(fitVisible); // 제목 줄이 생긴 만큼 터미널 크기를 다시 맞춘다.
      }
      // 단추를 빠르게 두 번 누른 것은 "이 창만 보기"로 치지 않는다.
      s.head.ondblclick = (ev) => { if (!ev.target.closest('button, .ssh-tabs')) showOnly(s); }; // 탭을 두 번 누른 것은 탭 고르기다.
    }
    paintSshButtons(s);
    paintTabStrip(s);
    s.headInfo.replaceChildren(
      // 나란히 볼 때는 창마다 그 세션의 워크스페이스를 적는다(서로 다른 워크스페이스의 세션을 나란히 두고 헷갈리지 않게).
      ...(isTerm(s) && s.group ? [make('span', 'pane-group', `◆ ${s.group}`)] : []),
      ...(s.mark ? [markBadge(s.mark)] : []),
      // SSH 세션은 이름을 탭에 적는다.
      ...(s.tabStrip ? [] : [
        make('span', 'title', label(s)),
        ...(s.agents.size ? [agentPill(s.agents.size, agentSummary(s))] : []),
        make('span', 'model', [s.model, s.effort].filter(Boolean).join(' · ')),
      ]),
    );
  }
  syncComposer();
  syncSshScopes();
}

/**
 * SSH 세션이 든 워크스페이스가 바뀌었으면(다른 워크스페이스로 옮김, 워크스페이스 이름 바꿈) 앱에 알린다.
 * SSH 연결은 같은 워크스페이스의 세션끼리만 같이 쓰므로, 옮겨 간 세션의 연결에 그 뒤로는 새 워크스페이스의 세션만 붙는다
 * (이미 같이 쓰고 있던 세션은 끊지 않는다).
 */
function syncSshScopes() {
  for (const s of sessions.values()) {
    if (!s.ssh || s.id == null || s.exited || s.scopeSent === undefined || s.scopeSent === (s.group || '')) continue;
    s.scopeSent = s.group || '';
    invoke('session_scope', { id: s.id, group: s.scopeSent }).catch(() => {});
  }
}

/**
 * 스테이지 맨 위에 지금 보고 있는 항목을 알려 준다: 워크스페이스(또는 폴더 묶음) › 별칭·이름 · 원래 이름 · 폴더.
 * 터미널 세션(Claude·셸·SSH)을 보고 있을 때는 워크스페이스 이름을 크게 적는다: 여러 워크스페이스의 세션을 오가다
 * 엉뚱한 세션에 입력하는 일이 없게, 지금 입력이 어느 업무로 가는지 한눈에 보이게 한다.
 */
function renderWhere() {
  const $where = $('where');
  const s = active && sessions.has(active.key) ? active : null;
  $where.hidden = !s;
  // 화면에 터미널 세션이 보이는 동안에는 줄의 높이를 그대로 둔다. 나란히 볼 때 고른 창에 따라 높이가 바뀌면 그때마다
  // 터미널의 줄 수가 바뀌어 Claude Code가 화면을 다시 그린다.
  $where.classList.toggle('big', isTerm(s) || visible.some(isTerm));
  if (!s) return;
  const name = make('span', 'where-name', label(s));
  if (s.color) name.style.color = s.color;
  const parts = [
    make('span', 'where-group', s.group ? `◆ ${s.group}` : baseName(s.startCwd)),
    make('span', 'where-sep', '›'),
  ];
  if (!isTerm(s)) parts.push(make('span', 'where-kind', `${KINDS[s.kind].icon} ${KINDS[s.kind].label}`));
  if (s.mark) parts.push(markBadge(s.mark));
  parts.push(name);
  if (s.name) parts.push(make('span', 'where-origin', baseLabel(s)));
  parts.push(make('span', 'where-path', `‎${s.cwd}`));
  $where.replaceChildren(...parts);
  $where.title = `${s.group ? `워크스페이스: ${s.group}` : `폴더 묶음: ${s.startCwd}`}\n${s.name ? `별칭: ${s.name}\n` : ''}${baseLabel(s)}\n${s.cwd}`;
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
    // 쉬는 세션은 건너뛴다(Ctrl+Tab처럼): 방향키로 지나가기만 해도 재개되면 안 된다. 재개하려면 그 세션을 누른다.
    const rows = [...$list.querySelectorAll('li[data-key]')].map(rowItem).filter((s) => s && !s.dormant);
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
    rowOf(next)?.scrollIntoView({ block: 'nearest' });
  });
}

/** 테마·글꼴·글자 크기가 바뀌면 열려 있는 터미널과 항목 화면을 맞춘다. */
function applyLook(look) {
  for (const s of sessions.values()) {
    if (!s.term) continue;
    Object.assign(s.term.options, {
      theme: look.termTheme, fontFamily: look.termFamily, fontSize: s.ssh ? sshPixels(look) : look.termPixels, minimumContrastRatio: look.contrast,
    });
    if (s.ssh) paintSshButtons(s); // 따로 정하지 않은 SSH 글자 크기는 앱의 크기를 따른다: 위쪽의 숫자도 맞춘다
  }
  window.dispatchEvent(new Event('cypher-look')); // 마인드맵 그림처럼 색을 직접 칠하는 화면이 다시 그린다.
  requestAnimationFrame(() => { fitVisible(); syncSites(); });
}

/** 분할을 풀고 이 세션 하나만 화면에 남긴다. 다른 세션은 닫지 않는다. */
function showOnly(s) {
  // SSH 창이면 그 워크스페이스의 SSH 방만 남기고, 방 안의 다른 창은 이 창의 탭으로 모은다(탭은 닫지 않는다).
  if (s.ssh) mergeRoom(s);
  tree = slotFor(s);
  layout();
  activate(s);
}

/** 사이드바의 빈 곳을 오른쪽 클릭했을 때의 메뉴: 새 워크스페이스, 보기 방식. 세션과 항목은 워크스페이스 머리의 +(또는 워크스페이스 오른쪽 클릭)로 만든다. */
function sidebarMenu(ev) {
  const items = [
    { caption: '세션과 항목은 워크스페이스 안에서만 만듭니다 (워크스페이스 머리의 +)' },
    { label: '새 워크스페이스 만들기…', action: () => newGroup() },
  ];
  items.push({ label: '빠른 이동·명령 찾기…  (Ctrl+Shift+K)', action: showPalette });
  if (closedCount()) items.push({ label: '닫은 항목 다시 열기…  (Ctrl+Shift+R)', action: showClosed });
  items.push({ label: '삭제한 워크스페이스 기록…  (3년 보관)', action: showHistory });
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
      label: allFolded ? '워크스페이스 모두 펼치기' : '워크스페이스 모두 접기',
      action: () => {
        collapsed = allFolded ? new Set() : new Set(keys);
        localStorage.setItem('cypher.collapsed', JSON.stringify([...collapsed]));
        render();
      },
    });
  }
  showMenu(ev, items);
}

/**
 * 빠른 이동 창에 올릴 것: 사이드바의 항목(최근에 본 순서, 지금 보고 있는 것은 맨 뒤)과 앱의 명령.
 * 아무것도 치지 않고 Enter를 누르면 바로 전에 보던 항목으로 간다. 쉬는 세션은 지금 보는 항목보다 뒤에, 쉬는 SSH 세션은 명령보다도 뒤에 둔다.
 */
function paletteEntries() {
  const items = [...sessions.values()]
    // 쉬는 세션은 지금 보는 항목보다도 뒤로 돌린다: 창을 열자마자 Enter를 눌렀을 때, 본 적 없는 쉬는 세션이 맨 앞에 있어
    // 재개되지 않게 한다. 쉬는 세션은 설명에 "쉬는 중"으로 보이고, 골라서 Enter를 누르면 재개한다.
    .sort((a, b) => !!a.dormant - !!b.dormant || (a === active) - (b === active) || (b.usedAt || 0) - (a.usedAt || 0))
    .map((s) => {
      const state = !isTerm(s) ? '' : s.exited ? '종료됨' : s.dormant ? (s.ssh ? '쉬는 중 · 고르면 접속' : '쉬는 중 · 고르면 재개')
        : WAITING_TEXT[s.waiting] || (s.busy || s.workStart ? '작업 중' : '');
      return {
        icon: isTerm(s) ? '›' : KINDS[s.kind].icon,
        title: label(s),
        note: [s.mark ? `북마크 ${s.mark}` : '', s.group ? `◆ ${s.group}` : baseName(s.startCwd), s.name ? baseLabel(s) : '', state, s === active ? '지금 보는 항목' : ''].filter(Boolean).join(' · '),
        keywords: [s.cwd, s.title, isTerm(s) ? '세션 터미널' : KINDS[s.kind].label, s.model].filter(Boolean).join(' '),
        tone: s.waiting === 'approval' ? 'alert' : s.waiting === 'done' ? 'done' : '',
        action: () => activate(s),
        restingSsh: !!s.ssh && !!s.dormant,
      };
    });
  const command = (title, action, note = '명령') => ({ icon: '⌘', title, note, action });
  // 새로 만드는 것은 지금 보고 있는 항목의 워크스페이스에 들어간다(워크스페이스가 하나뿐이면 그 워크스페이스). 어느 워크스페이스인지 설명에 적는다.
  const into = viewedGroup() || (groupList.length === 1 ? groupList[0] : '');
  const where = into ? `명령 · 워크스페이스 "${into}"에 만듦` : `명령 · ${groupList.length ? '워크스페이스의 항목을 보고 있을 때만(또는 워크스페이스 머리의 +)' : '먼저 워크스페이스를 만들어야 함'}`;
  const commands = [
    ...config.profiles.map((p) => command(`새 ${p.label} 세션`, () => newSession(p.id), into ? `${where} · 폴더: ${groupFolderOf(into) || folder}` : where)),
    ...(offersWorktree() ? [command('새 워크트리 세션…', () => newWorktree('replace'), where)] : []),
    ...Object.entries(KINDS).map(([kind, def]) => command(`새 ${def.label}`, () => createKind(kind), where)),
    command('새 워크스페이스 만들기…', newGroup),
    command('닫은 항목 다시 열기…', showClosed, '명령 · Ctrl+Shift+R: 가장 나중에 닫은 것'),
    command('삭제한 워크스페이스 기록…', showHistory, '명령 · 3년 보관 · 메모·할 일의 글 보기 · 워크스페이스로 되살리기'),
    command('알림 기록', showNotices, `명령 · Ctrl+Shift+A${unreadCount() ? ` · 새 알림 ${unreadCount()}개` : ''}`),
    command('타이머·알람 맞추기…', showTimers, `명령 · 24시간 이내${nextTimer() ? ` · 다음 알림까지 ${nextTimer().left}` : ''}`),
    command('워크스페이스 틀: 저장해 둔 구성 펼치기…', showTemplates),
    ...(currentGroup() ? [command('지금 워크스페이스를 틀로 저장…', () => saveTemplate(currentGroup()), `명령 · ${currentGroup().name}`)] : []),
    command('다음 대기 세션으로', nextWaiting, '명령 · Ctrl+Shift+N'),
    command(filterWaiting ? '모든 세션 보기' : '대기 중인 세션만 보기', () => { filterWaiting = !filterWaiting; render(); }),
    command('변경 내용 옆 창 열기/닫기', () => toggleDiff(), '명령 · Ctrl+Shift+G'),
    command('화면에서 찾기', openSearch, '명령 · Ctrl+Shift+F'),
    command('자주 쓰는 지시', showSnippets, '명령 · Ctrl+Shift+P'),
    command(broadcast ? '동시 입력 끄기' : '동시 입력 켜기', () => setBroadcast(!broadcast), '명령 · Ctrl+Shift+B'),
    command(broadcast === SSH ? 'SSH 동시 입력 끄기' : 'SSH 동시 입력 켜기', () => setBroadcast(broadcast === SSH ? false : SSH), '명령 · 나란히 보이는 SSH 세션끼리만 함께 입력'),
    ...(active?.ssh ? [command(`이 SSH 연결 복제 (${specLabel(active.ssh)})`, () => cloneSsh(active), '명령 · 바로 옆 탭으로 · 다시 인증하지 않음')] : []),
    ...(isTerm(active) && (active.logPath || (!runsClaude(active) && !active.exited && !active.dormant))
      ? [command(active.logPath ? '출력 기록 끄기' : '출력을 파일로 기록…', () => toggleLog(active), active.logPath ? `명령 · 기록 중: ${active.logPath}` : '명령 · 이 세션의 출력을 끌 때까지 파일에 적음')]
      : []),
    command('프롬프트 편집기로 가기', focusComposer, '명령 · Ctrl+Shift+E'),
    ...(visible.length > 1 && active ? [command('분할 풀기 (지금 창만 보기)', () => showOnly(active))] : []),
    command('기억 내보내기·가져오기', () => showMemory()),
    command('모양: 테마·글꼴·글자 크기', showLook, '설정'),
    command(`끌어서 고른 글 바로 복사 ${copyOnSelect ? '끄기' : '켜기'}`, () => setCopyOnSelect(!copyOnSelect), `설정 · 지금: ${copyOnSelect ? '켬' : '끔'} · 터미널(Claude·셸·SSH)`),
    ...(active?.ssh ? [
      command('SSH 글자 크게', () => setSshFont(sshFontNow() + 1), `명령 · 지금 ${sshFontNow()} · SSH 세션에만 적용 · Ctrl+휠`),
      command('SSH 글자 작게', () => setSshFont(sshFontNow() - 1), `명령 · 지금 ${sshFontNow()} · SSH 세션에만 적용 · Ctrl+휠`),
      command('SSH 글자 크기를 기본으로', () => setSshFont(0), '명령 · 앱의 터미널 글자 크기(설정 → 모양)를 따름'),
    ] : []),
    command('브라우저 시작 페이지 정하기…', askHomePage, `설정 · 지금: ${homePage || '빈 화면'}`),
    command('Claude 실행 명령 정하기…', askClaudeCommand, `설정 · 지금: ${claudeCommand || '기본값(claude)'}`),
    command('Cypher 설정 파일 열기', () => runMenu('settings'), '설정'),
    command('Claude 공통 설정 열기 (settings.json)', () => runMenu('claude-settings'), '설정'),
    command('Claude 공통 지침 열기 (CLAUDE.md)', () => runMenu('claude-memory'), '설정'),
    command('단축키 보기', showHelp, '도움말 · F1'),
    command('Cypher 업데이트…', showUpdate, '도움말 · 단추를 눌렀을 때만 확인'),
    command('Cypher 정보', showAbout, '도움말 · 버전·GitHub 주소'),
    ...(viewedGroup() ? [groupFolderOf(viewedGroup())
      ? command(`워크스페이스 "${viewedGroup()}"의 폴더 열기 (탐색기)`, () => openGroupFolder(viewedGroup()), `폴더 · ${groupFolderOf(viewedGroup())}`)
      : command(`워크스페이스 "${viewedGroup()}"의 폴더 정하기…`, () => pickGroupFolder(viewedGroup()), '폴더 · 지금: 정하지 않음 · 한 번 정하면 바꿀 수 없음')] : []),
  ];
  // 쉬는 SSH 세션은 명령보다도 뒤에 둔다: 아무것도 치지 않았을 때의 첫 줄(열자마자 Enter)이 되면 본 적 없는 서버에 접속한다.
  // 이름을 쳐서 찾거나 아래로 내려 고르면 접속한다(접속은 사용자가 고른 것만).
  return [...items.filter((item) => !item.restingSsh), ...commands, ...items.filter((item) => item.restingSsh)];
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
  if (s.ssh) {
    // SSH 탭은 사이드바에 제 줄이 없다(SSH는 워크스페이스마다 한 줄). 별칭을 물어 탭에 적는다.
    return ask('SSH 탭의 별칭 (비우면 원래 이름으로)', `${s.sshUser ? `${s.sshUser}@` : ''}${sshServer(s)}`, s.name).then((name) => {
      if (name === null || !sessions.has(s.key)) return;
      s.name = name;
      render();
      saveState();
    });
  }
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
  for (let i = 0; i < buffer.length; i++) {
    const line = buffer.getLine(i);
    const words = line.translateToString(true);
    // 터미널 너비를 넘어 접힌 줄은 앞 줄에 이어 붙인다(글을 골라 복사할 때와 같다).
    if (line.isWrapped && lines.length) lines[lines.length - 1] += words; else lines.push(words);
  }
  return lines.join('\n').trimEnd();
}

// ── 출력 기록: 세션 출력을 사용자가 고른 글자 파일에 계속 적는다(색·커서 이동은 빼고 글자만) ──

/** 기록 파일의 기본 이름: `<세션 이름>-YYYYMMDD-HHMMSS.log`. */
function logFileName(s) {
  const d = new Date();
  const two = (n) => String(n).padStart(2, '0');
  const stamp = `${d.getFullYear()}${two(d.getMonth() + 1)}${two(d.getDate())}-${two(d.getHours())}${two(d.getMinutes())}${two(d.getSeconds())}`;
  const name = label(s).replace(/[\\/:*?"<>|\s]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 60) || 'session';
  return `${name}-${stamp}.log`;
}

/** 기록할 파일을 고르고 기록을 시작한다. 지난번에 고른 폴더에서 시작한다. `given`(시험용)이면 묻지 않고 그 파일에 기록한다. */
async function startLog(s, given = null) {
  if (!isTerm(s) || s.id == null || s.exited || s.dormant) return setStatus('접속해 있는 세션에서만 기록할 수 있습니다.');
  let start = '';
  try { start = localStorage.getItem('cypher.logDir') || ''; } catch { /* 없으면 세션 폴더 */ }
  const path = given || await invoke('session_log_pick', { name: logFileName(s), start: start || s.startCwd }).catch(() => null);
  if (!path || !sessions.has(s.key) || s.exited) return;
  try {
    s.logPath = await invoke('session_log_start', { id: s.id, path, label: label(s) });
    try { localStorage.setItem('cypher.logDir', path.replace(/[\\/][^\\/]*$/, '')); } catch { /* 저장 공간 문제는 무시 */ }
    setStatus(`출력을 기록합니다: ${s.logPath} (끌 때: ${s.ssh ? '창 위쪽의 [■ 기록 중 · 끄기]나 ' : ''}세션을 오른쪽 클릭 → 출력 기록 끄기)`);
  } catch (err) {
    setStatus(String(err));
  }
  render();
}

async function stopLog(s) {
  if (!s.logPath) return;
  const path = s.id != null ? await invoke('session_log_stop', { id: s.id }).catch(() => null) : null;
  s.lastLog = path || s.logPath;
  s.logPath = '';
  setStatus(`출력 기록을 마쳤습니다: ${s.lastLog} (오른쪽 클릭 → 기록 파일을 문서 항목으로 보기)`);
  render();
}

const toggleLog = (s) => (s.logPath ? stopLog(s) : startLog(s));

/** 기록 파일을 문서 항목으로 연다(이미 그 파일을 보는 항목이 있으면 그것을 보여 준다). 기록 중이면 바뀌는 대로 따라 그린다. */
async function showLog(s) {
  const path = s.logPath || s.lastLog;
  if (!path) return;
  const shown = [...sessions.values()].find((other) => other.kind === 'reader' && samePath(other.view.path(), path));
  if (shown) return activate(shown);
  const doc = await newItem('reader', { ...besideOf(s), saved: { path }, show: 'replace' });
  if (doc) activate(doc);
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
  // SSH 세션의 메뉴는 SSH 창의 탭을 오른쪽 클릭하면 뜬다(사이드바에는 워크스페이스마다 SSH 줄 하나뿐이다).
  const panes = s.ssh ? leavesOf(rooms.get(s.group) || null).length : 0;
  showMenu(ev, [
    ...(s.ssh ? [{ caption: sshTabCaption(s) }] : []),
    { label: s.name ? `별칭 바꾸기… (지금: ${s.name})` : '별칭 붙이기…', action: () => rename(s) },
    ...(s.name ? [{ label: '별칭 지우기 (원래 이름으로)', action: () => { s.name = ''; render(); saveState(); } }] : []),
    { colors: COLORS, action: (color) => { s.color = color; render(); saveState(); } },
    { caption: s.mark ? `북마크 ${s.mark} · Ctrl+${s.mark}로 바로 가기 (같은 숫자를 다시 누르면 풂)` : '북마크: 숫자를 고르면 Ctrl+숫자로 바로 갑니다' },
    { icons: MARKS.map((digit) => ({ node: make('b', '', digit), title: markTitle(s, digit), on: s.mark === digit, action: () => setMark(s, digit) })) },
    { label: s.group ? `워크스페이스 바꾸기… (지금: ${s.group}${s.ssh ? ' · 옮겨 간 워크스페이스의 SSH 탭이 됩니다' : ''})` : '워크스페이스 지정…', action: () => askGroup([s], s.group) },
    ...(s.ssh ? [
      // SSH 창: 방 안의 다른 창은 이 창의 탭으로 모으고, 다른 항목은 화면에서 숨긴다.
      ...(visible.length > 1 || !visible.includes(s) ? [{ label: panes > 1 ? '이 창만 보기 (SSH의 다른 창은 이 창의 탭으로, 나머지는 숨기기)' : '이것만 보기 (나머지는 화면에서 숨기기)', action: () => showOnly(s) }] : []),
      ...(visible.includes(s) ? [{ label: panes > 1 ? '이 창 접기 (탭들은 옆 창의 탭 줄로)' : 'SSH를 화면에서 숨기기', action: () => toggleShown(s) }] : []),
    ] : [
      ...(visible.length > 1 || !visible.includes(s) ? [{ label: '이것만 보기 (나머지는 화면에서 숨기기)', action: () => showOnly(s) }] : []),
      // 화면에 올리는 것은 누르기(지금 창 자리), 나란히 놓는 것은 끌어다 놓기로만 한다.
      ...(visible.includes(s) ? [{ label: '화면에서 숨기기', action: () => toggleShown(s) }] : []),
    ]),
    ...(term ? [
      ...(s.dormant ? [{ label: resumeOf(s) ? '재개 (대화 이어 가기)' : '재개', action: () => activate(s) }] : []),
      ...(s.exited ? [{
        label: resumeOf(s) ? '다시 시작 (대화 이어 가기)' : s.ssh ? '다시 접속' : '다시 시작 (같은 폴더에서)',
        action: () => restartSession(s),
      }] : []),
      ...(s.ssh ? [
        { label: `세션 복제 (바로 옆 탭으로 · ${specLabel(s.ssh)})`, action: () => cloneSsh(s) },
        // 쉬는 탭에서 고른 것이면 그 탭을 고르지 않는다(고르면 그 탭도 접속을 시작한다). 새 탭은 그 탭 바로 옆에 놓는다.
        { label: '다른 서버에 접속… (이 창의 새 탭으로)', action: () => { if (!s.dormant) activate(s); showSsh({ ...besideOf(s), cwd: s.startCwd, show: 'replace', tabOf: s }); } },
        ...(savedHosts().some((host) => host.target === s.ssh.target && (host.port || 0) === (s.ssh.port || 0)) ? [] : [{
          label: '이 서버를 저장해 두기 (접속 창에서 한 번에 접속)',
          action: () => { if (saveHost({ ...s.ssh, name: s.ssh.name || s.name })) setStatus(`SSH 서버를 저장했습니다: ${specLabel(s.ssh)}`); },
        }]),
        ...(visible.includes(s) && visible.filter((v) => v.ssh).length > 1 ? [{
          label: broadcast === SSH ? 'SSH 동시 입력 끄기' : 'SSH 동시 입력 켜기 (나란히 보이는 SSH 세션에 함께 입력)',
          action: () => setBroadcast(broadcast === SSH ? false : SSH),
        }] : []),
      ] : [{
        label: `같은 폴더에 새 ${s.profile.label} 세션`,
        action: () => newSession(s.profile.id, { ...besideOf(s), show: 'replace' }),
      }]),
      { label: '변경 내용 보기', action: () => { activate(s); toggleDiff(true); } },
      ...(s.exited || s.dormant ? [] : [{ label: '화면 다시 맞추기 (글자가 어긋나 보일 때)', action: () => realign(s) }]),
      { label: '출력 저장… (지금까지 화면에 남은 것)', action: () => exportText(s) },
      // 기록 중이면 Claude 세션이 됐어도(셸에서 claude를 띄움) 끌 수 있어야 한다. 새로 켜는 것은 Claude 세션이 아닐 때만.
      ...(s.logPath
        ? [{ label: `출력 기록 끄기 (${baseName(s.logPath)})`, action: () => stopLog(s) }]
        : runsClaude(s) ? [] : [{ label: '출력을 파일로 기록… (파일을 고르면 그 뒤의 출력을 끌 때까지 적음)', action: () => startLog(s), disabled: s.exited || s.dormant }]),
      ...(s.logPath || s.lastLog ? [{ label: `기록 파일을 문서 항목으로 보기 (${baseName(s.logPath || s.lastLog)})`, action: () => showLog(s) }] : []),
      { label: '기억 내보내기·가져오기', action: () => showMemory(s) },
    ] : []),
    { label: '폴더를 탐색기에서 열기', action: () => openFolder(s) },
    { label: '폴더 경로 복사', action: () => invoke('clipboard_write', { text: s.cwd }).then(() => setStatus(`복사했습니다: ${s.cwd}`)).catch((err) => setStatus(String(err))) },
    // SSH 탭은 닫기 전에 묻는다(탭의 ×와 같다).
    { label: s.ssh ? '세션 닫기…' : term ? '세션 닫기' : `${KINDS[s.kind].label} 닫기`, action: () => (s.ssh ? askCloseTab(s) : closeSession(s)) },
    ...(s.ssh ? [{ caption: '나란히 보려면 탭을 SSH 창의 가장자리로 끌어다 놓으세요 (탭은 SSH 창 안에서만 옮깁니다)' }] : []),
  ]);
}

/** 항목의 폴더를 탐색기에서 연다. */
function openFolder(s) {
  invoke('link_open', { target: s.cwd, cwd: s.cwd, openFile: false, line: null, col: null }).catch((err) => setStatus(String(err)));
}

// ───────────────────────── 브라우저(사이트 항목)와 Claude 세션 ─────────────────────────
// 브라우저는 사이드바의 사이트 항목이다. 같은 워크스페이스의 Claude 세션이 Cypher 도구로 그 브라우저를 조작한다.
// 어느 브라우저를 조작할지: 사용자가 넘겨준 것 → 같은 워크스페이스에서 마지막으로 본 것 → 없으면 새로 만든다.
//
// Claude 세션이 Cypher 도구로 다룰 수 있는 범위:
// - 그 세션과 같은 "이름 붙인 워크스페이스"에 있는 항목만 다룬다. 다른 워크스페이스의 항목은 번호를 알아도 다룰 수 없다.
// - 워크스페이스 없이 폴더 묶음에 있는 것끼리는 서로 다룰 수 없다(같은 폴더에서 열었을 뿐 한 업무라는 보장이 없다).
//   워크스페이스 없는 세션은 자기 묶음에 새 항목을 놓는 것(메모·이미지·문서·마인드맵)만 할 수 있다.
// - 터미널 세션(다른 Claude 세션 포함)은 어떤 경우에도 다룰 수 없다.

const chromeEngine = () => config.browser.engine?.toLowerCase() === 'chrome';
/** 그 세션에서 Claude Code가 돌고 있는지: Claude 프로필로 연 세션이거나, 셸 세션에서 사용자가 직접 띄운 Claude가 신호를 보내오는 중이다. */
const runsClaude = (s) => !!s?.profile?.claude || !!s?.claudeLive;
const aliveClaude = (s) => runsClaude(s) && !s.exited && !s.dormant;
/** 다시 열 때 이어 갈 Claude 대화의 번호. Claude 프로필로 연 세션만 대화를 이어 연다(셸에서 직접 띄운 Claude의 대화는 잇지 않는다). */
const resumeOf = (s) => (s.profile?.claude && s.resumable && s.claudeSession ? s.claudeSession : '');
/** 그 항목과 같은 묶음에서 돌고 있는 Claude 세션들(사용자가 직접 글을 넣어 줄 대상을 고를 때 쓴다). */
const claudesOf = (item) => [...sessions.values()].filter((s) => aliveClaude(s) && groupKey(s) === groupKey(item));
/**
 * Claude 세션 s가 Cypher 도구로 그 항목을 다룰 수 있는지: 둘이 같은 이름 붙인 워크스페이스에 있고,
 * 대상이 터미널 세션이 아니어야 한다. 워크스페이스가 없는 것끼리는 다룰 수 없다.
 */
const canControl = (s, target) => !!s?.group && !!target && target !== s && target.group === s.group && !isTerm(target);
/** 파일을 메모로 연다. 그 파일에 연결된 메모가 이미 있으면 새로 만들지 않고 그 항목으로 간다(둘이 되면 서로 덮어쓴다). */
function openMemo(path) {
  const open = [...sessions.values()].find((s) => s.kind === 'memo' && samePath(s.view?.file?.() || '', path));
  if (open) return activate(open);
  return newItem('memo', { saved: { open: path }, show: 'replace' });
}

/** 그 항목을 넘겨받을 수 있는 Claude 세션들: 같은 이름 붙인 워크스페이스에서 돌고 있는 것. */
const controllersOf = (item) => [...sessions.values()].filter((s) => aliveClaude(s) && canControl(s, item));
/** 그 세션이 다룰 수 있는 사이트 항목들. */
const sitesOf = (s) => [...sessions.values()].filter((other) => other.kind === 'site' && canControl(s, other));
/** 워크스페이스에 든 세션만 할 수 있는 일 앞에서 부른다. 워크스페이스가 없으면 까닭을 돌려주며 그만둔다. */
function requireGroup(s) {
  if (!s.group) throw new Error(NO_GROUP);
}
/** 워크스페이스가 없는 세션이 항목을 다루려 할 때 Claude에게 돌려주는 말. */
const NO_GROUP = '이 세션은 이름 붙인 워크스페이스에 속해 있지 않습니다. Cypher 도구는 같은 워크스페이스에 있는 항목만 다룰 수 있고, 워크스페이스 없이 있는 것끼리는 서로 다룰 수 없습니다. 사용자에게 이 세션과 다룰 항목을 한 워크스페이스에 넣어 달라고 하세요(사이드바에서 오른쪽 클릭 → 워크스페이스 지정).';
/** 그 항목과 같은 묶음에 새 항목을 만들 때 쓸 워크스페이스 이름과 폴더. */
const besideOf = (s) => ({ group: s.group, cwd: s.group ? (s.project || s.startCwd) : s.startCwd });

/** 새로 연 Claude 세션이 입력을 받을 준비가 될 때까지(상태줄이 오면 준비된 것) 기다린다. */
async function untilReady(s) {
  for (let waited = 0; waited < 20000 && !s.model && !s.exited && sessions.has(s.key); waited += 300) {
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
}

/**
 * 사이트 항목의 [Claude에게]: 그 브라우저를 Claude 세션에 넘긴다. 세션이 없으면(target이 null) 같은 묶음에 새로 연다.
 * 넘겨받은 세션의 브라우저 도구는 이 사이트를 조작하고, 입력창에는 이어서 할 일을 적을 머리말이 들어간다.
 */
async function handoffSite(item, target) {
  if (!sessions.has(item.key)) return;
  // 같은 워크스페이스 안에서만 넘길 수 있다. 워크스페이스가 없는 사이트 항목은 어느 세션도 조작할 수 없다.
  if (!item.group) return notice('브라우저', `"${label(item)}"은(는) 워크스페이스에 속해 있지 않아 Claude 세션에 넘길 수 없습니다. 먼저 워크스페이스에 넣으세요(사이드바에서 오른쪽 클릭 → 워크스페이스 지정).`);
  if (target && !(aliveClaude(target) && canControl(target, item))) {
    return notice('브라우저', `"${label(target)}"은(는) 이 브라우저와 같은 워크스페이스의 Claude 세션이 아니라 넘길 수 없습니다.`);
  }
  if (!target) {
    target = await newSession('claude', { ...besideOf(item), show: 'replace' });
    if (target) await untilReady(target);
    if (!target || target.exited || !sessions.has(target.key)) return notice('브라우저', '새 Claude 세션을 열지 못했습니다.');
  }
  // 기다리는 사이에 사이트 항목이 닫혔거나 어느 한쪽이 다른 워크스페이스로 옮겨졌으면 넘기지 않는다.
  if (!sessions.has(item.key) || !canControl(target, item)) return;
  // 한 브라우저는 한 세션이 맡는다(넘겨주면 앞서 맡던 세션에서는 떼어 낸다).
  for (const other of sessions.values()) if (other.browser === item.key) other.browser = null;
  target.browser = item.key;
  // 화면은 나누지 않는다(창 분할은 사용자가 끌어다 놓을 때만): 세션을 지금 창 자리에 보인다. 브라우저는 사이드바에 있다.
  activate(target);
  const url = item.view.url();
  target.term.paste(`Cypher 브라우저 "${label(item)}"${url ? `(${url})` : ''}에서 이어서 작업해줘: `);
  syncSites();
}

/** 그 세션이 지금 조작하는 브라우저: 넘겨받은 것, 없으면 같은 묶음에서 마지막으로 본 것. */
function browserOf(s) {
  const bound = s.browser != null ? sessions.get(s.browser) : null;
  // 넘겨받은 뒤에 어느 한쪽이 다른 워크스페이스로 옮겨졌으면 더는 조작하지 못한다.
  if (bound?.kind === 'site' && canControl(s, bound)) return bound;
  return sitesOf(s).sort((a, b) => (b.usedAt || 0) - (a.usedAt || 0))[0] || null;
}

const siteLine = (site) => `#${site.key} "${label(site)}" — ${site.view.url() || '빈 페이지'}`;

/** 브라우저 도구가 조작할 사이트를 정한다. `create`이면(주소로 이동하라는 요청) 묶음에 브라우저가 없을 때 새로 만든다. */
async function browserTarget(s, create) {
  requireGroup(s);
  let site = browserOf(s);
  const created = !site;
  if (!site) {
    if (!create) throw new Error('이 세션의 워크스페이스에 브라우저(사이트 항목)가 없습니다. browser_navigate로 주소를 열면 새 브라우저가 만들어집니다.');
    site = await newItem('site', { ...besideOf(s), saved: {}, show: 'hidden' });
    // 화면은 나누지 않는다(창 분할은 사용자가 끌어다 놓을 때만): 사이드바에 놓아 두고, 사용자가 끌어다 나란히 볼 수 있다.
    reveal([site]);
  }
  if (s.browser !== site.key) {
    for (const other of sessions.values()) if (other.browser === site.key) other.browser = null;
    s.browser = site.key;
    syncSites();
  }
  const others = sitesOf(s).filter((other) => other !== site);
  // 이 브라우저가 써야 하는 프로필을 앱에 지금 알려 둔다(방금 워크스페이스를 옮겼다면 앱은 아직 옛 프로필로 알고 있을 수 있다).
  const profile = ensureProfile(site);
  await site.view.assign(profile);
  return {
    key: site.key,
    url: site.view.url() || 'about:blank',
    profile,
    // 도구의 답 끝에 덧붙일 말: 새로 만들었거나, 브라우저가 여러 개라 어느 것을 조작했는지 알려야 할 때만.
    note: created ? `이 워크스페이스에 새 브라우저(사이트 항목 #${site.key})를 만들어 열었습니다.`
      : others.length ? `조작한 브라우저: 사이트 항목 #${site.key}. 같은 워크스페이스에 다른 브라우저가 ${others.length}개 더 있습니다(browser_tabs로 보고 browser_select로 바꿉니다).` : '',
  };
}

function browserTabs(s) {
  if (!s.group) return NO_GROUP;
  const sites = sitesOf(s);
  if (!sites.length) return '이 세션의 워크스페이스에 브라우저(사이트 항목)가 없습니다. browser_navigate로 주소를 열면 새로 만들어집니다.';
  const current = browserOf(s);
  return `이 세션의 워크스페이스에 있는 브라우저(사이트 항목):\n${sites.map((site) => `${site === current ? '●' : ' '} ${siteLine(site)}${site === current ? '  ← 지금 조작 대상' : ''}`).join('\n')}`;
}

function browserSelect(s, id) {
  const site = itemInGroup(s, id, 'site');
  for (const other of sessions.values()) if (other.browser === site.key) other.browser = null;
  s.browser = site.key;
  syncSites();
  return `조작 대상을 바꿨습니다: ${siteLine(site)}`;
}

/** 같은 묶음의 항목을 Claude 세션이 알 수 있게 글로 적는다: 종류, 이름, 파일 경로나 주소. */
function groupItemsText(s) {
  // 워크스페이스가 없는 세션에는 같은 폴더 묶음의 다른 항목을 알려 주지 않는다(다룰 수 없는 것들이다).
  if (!s.group) {
    return `${NO_GROUP}\n지금은 폴더 묶음 ${s.startCwd}에 있습니다. 워크스페이스가 없어도 새 항목을 놓는 것은 됩니다: memo_write(새 메모), image_add, doc_show, table_show, mindmap_write(새 마인드맵), erd_write(새 ERD), todo_write(새 할 일). 놓은 항목은 이 세션의 폴더 묶음에 들어갑니다.`;
  }
  const dir = (name) => storeDirs[name] || `%APPDATA%\\cypher\\${name}`;
  const lines = [...sessions.values()].filter((other) => other.group === s.group).map((other) => {
    const head = `#${other.key}`;
    const name = `"${label(other)}"`;
    if (other === s) return `- ${head} 이 세션 ${name} — 폴더: ${other.cwd}`;
    if (isTerm(other)) {
      const state = other.exited ? '종료됨' : WAITING_TEXT[other.waiting] || (other.busy ? '작업 중' : '대기');
      return `- ${head} ${other.profile.label} 세션 ${name} — 폴더: ${other.cwd} · ${state} (다른 세션은 Cypher 도구로 다룰 수 없음)`;
    }
    const state = other.view.state();
    if (other.kind === 'memo') {
      const file = state.path && !state.dirty ? state.path : `${dir('memos')}\\${state.memo}.txt`;
      return `- ${head} 메모 ${name} — 파일: ${file}${state.path && state.dirty ? ` (저장하지 않은 고침이 있음. 원래 파일: ${state.path})` : ''} (덧붙이거나 바꿀 때는 memo_write에 id ${other.key})`;
    }
    if (other.kind === 'reader') return `- ${head} 문서 ${name} — 파일: ${other.view.path() || '(아직 파일 없음)'} (서식 있게 보여 주는 화면. 파일을 고치면 스스로 다시 그린다)`;
    if (other.kind === 'http') return `- ${head} HTTP 요청 ${name} — 요청 폴더: ${other.startCwd}${state.file ? ` · 열린 파일: ${state.file}` : ''} (http_list / http_run으로 다룬다)`;
    if (other.kind === 'site') return `- ${head} 브라우저(사이트) ${name} — 주소: ${other.view.url() || '빈 페이지'} (browser_* 도구로 조작)`;
    if (other.kind === 'image') return `- ${head} 이미지 ${name} — 파일: ${other.view.path() || '(아직 그림 없음)'}${state.source ? ` · 출처: ${state.source}` : ''}`;
    if (other.kind === 'mindmap') return `- ${head} 마인드맵 ${name} — 개요 글 파일: ${dir('mindmaps')}\\${other.view.store()}.md (고칠 때는 mindmap_write에 id ${other.key})`;
    if (other.kind === 'erd') return `- ${head} ERD ${name} — erDiagram 글 파일: ${dir('erds')}\\${other.view.store()}.mmd (고칠 때는 erd_write에 id ${other.key})`;
    if (other.kind === 'todo') {
      const { done, total } = other.view.progress();
      return `- ${head} 할 일 ${name} — 끝낸 일 ${done}/${total} · 체크 목록 파일: ${dir('todos')}\\${other.view.store()}.md (더하거나 바꿀 때는 todo_write, 체크할 때는 todo_check에 id ${other.key})`;
    }
    if (other.kind === 'run') return `- ${head} 명령 ${name} — 명령: ${other.view.command() || '(없음)'} · 폴더: ${other.cwd} · 상태: ${other.view.stateText()} (출력은 run_output에 id ${other.key}. 실행·멈춤은 사용자가 한다)`;
    if (other.kind === 'table') return `- ${head} 표 ${name} — 파일: ${other.view.path() || '(아직 파일 없음)'} (표로 보여 주는 화면. 파일을 고치면 스스로 다시 읽는다)`;
    if (other.kind === 's3') return `- ${head} S3 저장소 탐색기 ${name} — ${other.view.connected() ? `보고 있는 곳: ${other.view.address()}` : '접속하지 않음'} (사용자가 직접 다루는 화면. Cypher 도구로는 다룰 수 없고, 접속·올리기·삭제는 사용자가 한다)`;
    if (other.kind === 'hex') return `- ${head} 헥사 에디터 ${name} — 파일: ${other.view.path() || '(아직 파일 없음)'} (바이트를 보고 고치는 화면. 사용자가 저장하기 전에는 파일이 바뀌지 않는다)`;
    return `- ${head} ${KINDS[other.kind]?.label || other.kind} ${name}`;
  });
  return `워크스페이스 "${s.group}"의 Cypher 항목 ${lines.length}개(이 워크스페이스 밖의 항목은 다룰 수 없음):\n${lines.join('\n')}`;
}

/** 마인드맵 항목을 개요 글로 만들거나 고친다. */
async function mindmapWrite(s, { item, title, text }) {
  const name = typeof title === 'string' ? title.trim().slice(0, 120) : ''; // 글자가 아닌 값이 별칭이 되지 않게 한다.
  if (item != null) {
    let target = itemInGroup(s, item, 'mindmap');
    await target.view.ready;
    target = itemInGroup(s, item, 'mindmap'); // 기다리는 사이에 닫혔거나 다른 워크스페이스로 옮겨졌으면 여기서 거절된다.
    if (target.view.setText(text) === false) {
      throw new Error(`마인드맵 #${target.key}은(는) 사용자가 코드 화면에서 고치는 중이라 바꾸지 않았습니다(오류가 있어 아직 반영되지 않은 글이 있습니다). 사용자에게 알리고, 사용자가 고친 뒤에 다시 시도하세요.`);
    }
    if (name) target.name = name;
    reveal([target]);
    saveState();
    return `마인드맵 #${target.key} "${label(target)}"의 내용을 바꿨습니다.`;
  }
  const made = await newItem('mindmap', { ...besideOf(s), saved: { text }, name, show: 'hidden' });
  reveal([made]);
  return `마인드맵 항목 #${made.key} "${label(made)}"을 만들었습니다(${placedIn(s)}). ${s.group ? `고칠 때는 mindmap_write의 id에 ${made.key}번을 주세요.` : LATER_NEEDS_GROUP}`;
}

/** ERD 항목을 erDiagram 글로 만들거나 고친다. 글에 읽지 못한 줄이 있으면 줄 번호와 고치는 법을 함께 알려 준다. */
async function erdWrite(s, { item, title, text }) {
  const name = typeof title === 'string' ? title.trim().slice(0, 120) : ''; // 글자가 아닌 값이 별칭이 되지 않게 한다.
  const report = (target) => {
    const problems = target.view.problems();
    const { entities, relations } = target.view.summary();
    return problems.length
      ? ` 개체 ${entities}개·관계 ${relations}개를 그렸지만 읽지 못한 줄이 ${problems.length}개 있습니다. 고쳐서 다시 쓰세요:\n${problems.slice(0, 20).map((p) => `- ${p.line}번째 줄: ${p.message} (${p.fix})`).join('\n')}`
      : ` 개체 ${entities}개·관계 ${relations}개를 그렸습니다.`;
  };
  if (item != null) {
    let target = itemInGroup(s, item, 'erd');
    await target.view.ready;
    target = itemInGroup(s, item, 'erd'); // 기다리는 사이에 닫혔거나 다른 워크스페이스로 옮겨졌으면 여기서 거절된다.
    target.view.setText(text);
    if (name) target.name = name;
    reveal([target]);
    saveState();
    return `ERD #${target.key} "${label(target)}"의 내용을 바꿨습니다.${report(target)}`;
  }
  const made = await newItem('erd', { ...besideOf(s), saved: { text }, name, show: 'hidden' });
  reveal([made]);
  return `ERD 항목 #${made.key} "${label(made)}"을 만들었습니다(${placedIn(s)}).${report(made)} ${s.group ? `고칠 때는 erd_write의 id에 ${made.key}번을 주세요.` : LATER_NEEDS_GROUP}`;
}

const ANNOUNCE_QUIET_MS = 30000; // 같은 항목의 같은 알림을 다시 띄우지 않는 시간
let announced = { key: null, title: '', at: 0 };

/** Claude 세션이 사이드바에 놓은(또는 고친) 항목을 사용자에게 알린다. 누르면 그 항목을 연다. */
function announce(item, by, what) {
  reveal([item]);
  const title = `${label(by)}: ${what}`;
  const body = `${label(item)} — ${whereOf(item)} (누르면 엽니다)`;
  // 같은 항목에 같은 일을 잇달아 하면(메모에 여러 번 덧붙이기 등) 알림은 처음 한 번만 띄운다.
  const now = Date.now();
  const again = announced.key === item.key && announced.title === title && now - announced.at < ANNOUNCE_QUIET_MS;
  announced = { key: item.key, title, at: now };
  if (again) return;
  const seen = visible.includes(item) && document.hasFocus();
  logNotice(item, 'item', title, body, !seen);
  if (!seen && config.notify.in_app) toast(item, title, body);
}

/** 그 번호의 항목이 도구를 부른 세션과 같은 묶음에 있는 그 종류의 항목인지 확인하고 돌려준다. */
function itemInGroup(s, id, kind) {
  requireGroup(s);
  const target = sessions.get(Number(id));
  if (target?.kind !== kind || !canControl(s, target)) {
    throw new Error(`#${id}은(는) 이 세션의 워크스페이스("${s.group}")에 있는 ${KINDS[kind].label} 항목이 아닙니다. 같은 워크스페이스의 항목만 다룰 수 있습니다. cypher_items로 번호를 확인하세요.`);
  }
  return target;
}

/** 새 항목이 놓인 곳을 Claude에게 알려 주는 말. */
const placedIn = (s) => (s.group ? '사이드바의 같은 워크스페이스' : '이 세션의 폴더 묶음');
const LATER_NEEDS_GROUP = '이 세션이 워크스페이스에 속해 있지 않아, 놓은 뒤에는 Cypher 도구로 고칠 수 없습니다(워크스페이스에 넣으면 됩니다).';

/** 메모 항목을 만들거나, 있는 메모에 덧붙이거나 통째로 바꾼다. */
async function memoWrite(s, { item, title, text, replace }) {
  if (item != null) {
    let target = itemInGroup(s, item, 'memo');
    await target.view.ready;
    target = itemInGroup(s, item, 'memo'); // 기다리는 사이 닫히거나 다른 워크스페이스로 옮겨졌으면 쓰지 않는다.
    const kept = await target.view.write(text, !!replace);
    if (title) target.name = String(title).slice(0, 120);
    saveState();
    announce(target, s, replace ? '메모를 바꿨습니다' : '메모에 덧붙였습니다');
    if (kept) setStatus(`"${label(target)}" 메모를 바꾸기 전의 글은 30일 동안 보관합니다: ${kept}`);
    return `메모 #${target.key} "${label(target)}"${replace ? '의 내용을 바꿨습니다' : '의 끝에 덧붙였습니다'}.`;
  }
  const made = await newItem('memo', { ...besideOf(s), saved: { text }, name: title ? String(title).slice(0, 120) : '', show: 'hidden' });
  announce(made, s, '메모를 만들었습니다');
  return `메모 항목 #${made.key} "${label(made)}"을 만들었습니다(${placedIn(s)}). ${s.group ? `덧붙이거나 바꿀 때는 memo_write에 id ${made.key}를 주세요.` : LATER_NEEDS_GROUP}`;
}

/** 저장소에 넣어 둔 그림을 그 세션의 묶음에 이미지 항목으로 놓는다. */
async function imageAdd(s, { stored, title }) {
  if (!stored?.id) throw new Error('그림을 저장소에 넣지 못했습니다.');
  if (title) stored.name = String(title).slice(0, 120);
  const [made] = await addImages([stored], { ...besideOf(s) });
  if (!made) throw new Error('이미지 항목을 만들지 못했습니다.');
  announce(made, s, '이미지를 넣었습니다');
  return `이미지 항목 #${made.key} "${label(made)}"을 넣었습니다(${placedIn(s)}).`;
}

const samePath = (a, b) => pathKey(a) === pathKey(b);

/**
 * 파일을 그 세션의 묶음에 그 파일을 보여 주는 항목(문서: 'reader', 표: 'table')으로 놓는다.
 * 이미 그 파일을 보여 주는 같은 종류의 항목이 있으면 그것을 새로 읽는다. `noun`은 Claude와 사용자에게 보일 종류 이름.
 */
async function showFile(s, { path, title }, kind, noun) {
  let doc = [...sessions.values()].find((other) => other.kind === kind && groupKey(other) === groupKey(s) && samePath(other.view.path(), path));
  const fresh = !doc;
  if (!doc) {
    doc = await newItem(kind, { ...besideOf(s), saved: { path }, name: title ? String(title).slice(0, 120) : '', show: 'hidden' });
  } else if (canControl(s, doc)) {
    await doc.view.reload();
    if (title) { doc.name = String(title).slice(0, 120); saveState(); }
  } else {
    // 워크스페이스 없는 묶음에 이미 있는 항목은 건드리지 않는다(파일이 바뀌면 그 화면이 스스로 다시 그린다).
    return `이 파일을 보여 주는 ${noun} 항목 #${doc.key} "${label(doc)}"이 이미 있습니다. ${LATER_NEEDS_GROUP}`;
  }
  announce(doc, s, fresh ? `${noun}를 놓았습니다` : `${noun}를 새로 읽었습니다`);
  return fresh
    ? `${noun} 항목 #${doc.key} "${label(doc)}"을 놓았습니다(${placedIn(s)}). 파일이 바뀌면 화면이 스스로 다시 그립니다.`
    : `이 파일을 보여 주는 ${noun} 항목 #${doc.key} "${label(doc)}"이 이미 있어 새로 읽었습니다.`;
}

/** 할 일 항목을 만들거나, 있는 목록에 덧붙이거나 통째로 바꾼다. 답에는 할 일마다 번호를 붙인 지금의 목록을 싣는다. */
async function todoWrite(s, { item, title, text, replace }) {
  const name = typeof title === 'string' ? title.trim().slice(0, 120) : ''; // 글자가 아닌 값이 별칭이 되지 않게 한다.
  const listing = (target) => {
    const { done, total } = target.view.progress();
    return `지금의 목록(끝낸 일 ${done}/${total}, 앞의 숫자가 할 일의 번호):\n${target.view.listing()}`;
  };
  if (item != null) {
    let target = itemInGroup(s, item, 'todo');
    await target.view.ready;
    target = itemInGroup(s, item, 'todo'); // 기다리는 사이에 닫혔거나 다른 워크스페이스로 옮겨졌으면 여기서 거절된다.
    if (replace) target.view.setText(text); else target.view.append(text);
    if (name) target.name = name;
    saveState();
    announce(target, s, replace ? '할 일을 바꿨습니다' : '할 일을 더했습니다');
    return `할 일 #${target.key} "${label(target)}"${replace ? '의 내용을 바꿨습니다' : '의 끝에 덧붙였습니다'}. ${listing(target)}`;
  }
  const made = await newItem('todo', { ...besideOf(s), saved: { text }, name, show: 'hidden' });
  announce(made, s, '할 일을 만들었습니다');
  return `할 일 항목 #${made.key} "${label(made)}"을 만들었습니다(${placedIn(s)}). ${s.group ? `일을 끝내면 todo_check에 id ${made.key}와 그 할 일의 번호나 글을 주어 체크하세요.` : LATER_NEEDS_GROUP} ${listing(made)}`;
}

/** 할 일 항목에서 할 일을 체크하거나 푼다. 진행은 사이드바(그 줄과 묶음 머리)에 바로 보이므로 따로 알리지 않는다. */
async function todoCheck(s, { item, items, done }) {
  let target = itemInGroup(s, item, 'todo');
  await target.view.ready;
  target = itemInGroup(s, item, 'todo'); // 기다리는 사이에 닫혔거나 다른 워크스페이스로 옮겨졌으면 여기서 거절된다.
  const { changed, already, problems } = target.view.check(items, done !== false);
  const { done: finished, total } = target.view.progress();
  const listing = `지금의 목록(끝낸 일 ${finished}/${total}):\n${target.view.listing()}`;
  // 하나도 찾지 못했으면 실패로 알린다(번호나 글을 고쳐 다시 부를 수 있게 목록을 함께 준다).
  if (!changed && !already) throw new Error(`할 일을 찾지 못했습니다: ${problems.join(' / ')}\n${listing}`);
  if (changed) saveState();
  const parts = [
    changed ? `${changed}개를 ${done !== false ? '끝냄으로 체크했습니다' : '체크를 풀었습니다'}` : '',
    already ? `${already}개는 이미 그 상태였습니다` : '',
    problems.length ? `찾지 못한 것: ${problems.join(' / ')}` : '',
  ].filter(Boolean);
  return `할 일 #${target.key} "${label(target)}": ${parts.join(' · ')}. ${listing}`;
}

/** 명령 항목의 상태와 출력의 끝부분. 읽기만 한다(실행·멈춤은 사용자가 한다). */
function runOutput(s, { item, lines }) {
  const target = itemInGroup(s, item, 'run');
  const body = target.view.output(Math.min(400, Math.max(1, Number(lines) || 80)));
  return `명령 #${target.key} "${label(target)}" — 명령: ${target.view.command() || '(없음)'} · 폴더: ${target.cwd} · 상태: ${target.view.stateText()}\n출력의 끝부분:\n${body || '(출력 없음)'}`;
}

const RUN_NOTICE_MS = 10000; // 이보다 금방 끝난 성공한 명령은 알리지 않는다(그 줄의 표시로 충분하다).

/** 명령 항목의 명령이 끝났다: 보고 있지 않았으면 그 줄에 표시하고, 실패했거나 오래 걸린 것은 알린다. */
function runFinished(item, { ok, took, text }) {
  if (!sessions.has(item.key)) return;
  const seen = visible.includes(item) && document.hasFocus();
  if (!visible.includes(item)) item.attention = true;
  render();
  if (ok && took < RUN_NOTICE_MS) return;
  const title = `${label(item)}: ${ok ? '명령이 끝났습니다' : '명령이 실패했습니다'}`;
  const body = `${text} — ${whereOf(item)} (누르면 엽니다)`;
  logNotice(item, 'item', title, body, !seen);
  if (seen) return;
  if (!document.hasFocus() && config.notify.system) invoke('notify', { title, body });
  if (config.notify.in_app) toast(item, title, body);
}

/** Claude 세션이 보낸 알림: 앱 안 알림과(창이 뒤에 있으면) Windows 알림으로 보이고 알림 기록에 남긴다. */
function claudeNotice(s, { title, message }) {
  const head = typeof title === 'string' && title.trim() ? title.trim().slice(0, 80) : label(s);
  const body = String(message ?? '').slice(0, 500);
  const focused = document.hasFocus();
  if (!focused && config.notify.system) invoke('notify', { title: head, body });
  logNotice(s, 'claude', head, body, !(focused && s === active));
  if (config.notify.in_app) toast(s, head, body);
  if (s !== active) { s.attention = true; render(); }
  return '사용자에게 알렸습니다.';
}

/**
 * 그 세션이 HTTP 도구(http_list, http_run)로 다룰 수 있는 폴더: 같은 워크스페이스에 있는 HTTP 항목의 요청 폴더와
 * 그 세션을 시작한 폴더. 워크스페이스가 없는 세션은 HTTP 도구를 쓸 수 없다(같은 폴더의 워크스페이스 없는 HTTP 항목이 보는 요청을
 * 실행하고 기록을 남기게 되기 때문이다). 폴더는 Cypher가 정한 시작 폴더만 쓴다(세션이 알려 온 폴더는 믿지 않는다).
 */
function httpFolders(s) {
  requireGroup(s);
  const shared = [...sessions.values()].filter((other) => other.kind === 'http' && canControl(s, other)).map((other) => other.startCwd);
  return [...new Set([s.startCwd, ...shared])];
}

// ───────────────────────── Claude 세션이 만든 파일 ─────────────────────────
// Claude 세션이 Write 도구로 새 파일을 만들면 앱이 알려 온다(session-file). 그 파일을 종류에 맞는 항목으로
// 그 세션의 묶음에 놓는다: 문서(.md, .txt) → 문서 항목, 그림 → 이미지 항목, 요청 파일(.http) → HTTP 항목,
// 표 파일(.csv, .tsv) → 표 항목, HTML 파일(.html, .htm) → 사이트 항목(그 파일을 브라우저로 연다).
// 보던 화면은 바꾸지 않고 사이드바에만 놓으며, 알림으로 알려 준다. 설정의 [session] show_created로 끄거나 넓힌다.
//
// 만들자마자 놓지 않고 그 요청의 응답이 끝날 때 놓는다(placeHeld). 일하는 도중에 만들었다가 지운 파일, 잠깐 쓰는 파일
// (tmp_…, ….bak), 같은 요청에서 만든 문서에 들어간 그림 같은 "중간 파일"은 놓지 않고 최종 파일만 놓는다.

const HELD_TRIES = 5;            // 빈 채로 남은 새 파일을 다시 살펴보는 횟수(응답이 끝날 때마다 한 번)
const CREATED_SETTLE_MS = 4000;  // 요청 중이 아닐 때(신호 없이 만든 파일) 이만큼 더 만들지 않으면 놓는다

const MAX_PLACED = 8;           // 요청 하나에서 사이드바에 놓는 파일 수
/** 드라이브 글자만 남은 폴더(`C:`)에는 구분자를 붙인다(붙이지 않으면 "그 드라이브의 지금 폴더"를 뜻하게 된다). */
const withRootSlash = (dir) => (/^[A-Za-z]:$/.test(dir) ? `${dir}\\` : dir);
/** 파일이 든 폴더. */
const dirName = (path) => withRootSlash(path.replace(/[\\/][^\\/]*$/, ''));
/** 요청 파일을 보여 줄 HTTP 항목의 폴더: 파일이 `<폴더>\http`나 `<폴더>\.cypher\http`에 있으면 그 `<폴더>`. */
const httpHome = (path) => { const dir = dirName(path); const m = /^(.+?)[\\/](?:\.cypher[\\/]http|http)$/i.exec(dir); return m ? withRootSlash(m[1]) : dir; };
/** 이 컴퓨터의 파일을 사이트 항목으로 열 때의 주소(`file:///C:/…`). 이름의 빈칸·한글·# 같은 글자는 주소에 맞게 바꾼다. */
const fileUrl = (path) => `file:///${path.replace(/\\/g, '/').split('/').map((part, i) => (i === 0 ? part : encodeURIComponent(part))).join('/')}`;
/** 두 주소가 같은 파일을 가리키는지(주소에 맞게 바꾼 글자와 대/소문자 차이, 뒤에 붙은 `#자리`·`?값`은 무시). */
const sameFileUrl = (a, b) => {
  const bare = (url) => url.replace(/[?#].*$/, '');
  // 글자를 모두 풀어서 견준다. 경로로 만든 주소(fileUrl)는 `&`·`,`·`+` 같은 글자도 %로 바꿔 적지만 웹뷰가 알려 주는
  // 주소는 그 글자를 그대로 둔다. decodeURI는 이런 글자의 %표기를 풀지 않아 같은 파일을 다르다고 본다.
  try { return decodeURIComponent(bare(a)).toLowerCase() === decodeURIComponent(bare(b)).toLowerCase(); } catch { return bare(a) === bare(b); }
};
/** 경로를 견줄 때 쓰는 모양(구분자와 대/소문자를 맞춘 것). */
const pathKey = (path) => path.replace(/\//g, '\\').toLowerCase();

/** 그 항목이 그 파일을 보여 주고 있는지: 문서·표가 읽는 파일, 메모에 연결된 파일, HTTP 항목이 열어 둔 요청 파일, 사이트가 연 파일, 이미지를 가져온 파일. */
function showsFile(other, path) {
  switch (other.kind) {
    case 'reader': case 'table': case 'hex': return samePath(other.view.path(), path);
    case 'memo': return samePath(other.view.state().path || '', path);
    case 'http': return samePath(other.view.state().file || '', path);
    case 'site': return sameFileUrl(other.view.url() || '', fileUrl(path));
    case 'image': return samePath(other.view.state().source || '', path);
    case 'code': return other.view.shows(path);
    // 마인드맵·ERD·할 일은 저장소의 파일을 본다(Claude 세션에 그 경로를 알려 주므로 도구 대신 파일을 직접 고칠 수 있다).
    // 다시 읽게 하지 않으면 화면이 옛 내용으로 남아, 다음에 화면에서 고칠 때 Claude가 고친 것을 덮어쓴다.
    case 'mindmap': return !!storeDirs.mindmaps && samePath(`${storeDirs.mindmaps}\\${other.view.store()}.md`, path);
    case 'erd': return !!storeDirs.erds && samePath(`${storeDirs.erds}\\${other.view.store()}.mmd`, path);
    case 'todo': return !!storeDirs.todos && samePath(`${storeDirs.todos}\\${other.view.store()}.md`, path);
    default: return false;
  }
}

/**
 * Claude 세션이 파일을 썼다: 그 세션과 같은 묶음(워크스페이스)에서 그 파일을 보여 주는 항목이 바로 다시 읽게 한다.
 * 문서·표·메모는 화면에 보이는 동안 스스로도 살피지만 그보다 먼저 맞추고, 스스로 살피지 않는 사이트(HTML 파일)와
 * 이미지는 여기서만 새 내용이 된다. 고치던 글이 있는 메모·HTTP 항목은 덮어쓰지 않는다(그 화면이 알아서 가린다).
 */
function refreshShown(s, path) {
  for (const other of sessions.values()) {
    if (other === s || groupKey(other) !== groupKey(s)) continue;
    // 코드 보기는 열어 둔 파일이 아니어도 프로젝트의 파일이 바뀐 것을 알아야 한다(나무와 바뀐 파일 목록).
    try { other.view?.fileChanged?.(path); } catch { /* 그 화면의 문제로 다른 항목까지 멈추지 않게 */ }
    if (!showsFile(other, path)) continue;
    Promise.resolve(other.view.refresh?.()).catch(() => {});
  }
}

/** 앱이 알려 온 "Claude 세션이 고쳐 쓴 파일": 그 파일을 보여 주는 항목을 다시 읽게 한다. */
function onEditedFile({ id, path }) {
  const s = byId(id);
  if (s && path) refreshShown(s, path);
}

/**
 * 앱이 알려 온 "Claude 세션이 새로 만든 파일". 바로 놓지 않고 적어 두었다가 그 요청의 응답이 끝날 때 최종 파일만 놓는다(placeHeld).
 */
function onCreatedFile({ id, path, kind }) {
  const s = byId(id);
  if (!s || !path) return;
  // 지웠다가 다시 만든 파일처럼, 이미 그 파일을 보여 주는 항목이 있으면 그 항목은 새 내용을 읽는다.
  refreshShown(s, path);
  s.heldFiles ??= new Map(); // 경로(견주는 꼴) → { path, kind }. 만든 차례대로.
  s.heldFiles.set(pathKey(path), { path, kind });
  // 요청 중이 아닌데 온 파일(요청 시작 신호를 받지 못한 경우)은 잠깐 기다렸다가 놓는다. 요청 중이면 응답이 끝날 때 놓는다.
  clearTimeout(s.heldFilesTimer);
  s.heldFilesTimer = setTimeout(() => { if (!s.workStart) placeHeld(s); }, CREATED_SETTLE_MS);
}

/**
 * 적어 둔 "새로 만든 파일" 가운데 최종 파일을 그 세션의 묶음에 놓는다. 응답이 끝났을 때(완료 신호, 중단, 세션 종료) 부른다.
 * 그사이 지운 파일·잠깐 쓰는 파일·문서에 들어간 그림은 앱이 가려 준다(created_final).
 */
function placeHeld(s) {
  if (!s) return Promise.resolve();
  clearTimeout(s.heldFilesTimer);
  // 지금까지 적어 둔 것만 한 묶음으로 놓는다(차례를 기다리는 사이 다음 요청이 만든 파일이 이 묶음의 한도에 세어지지 않게).
  const held = [...(s.heldFiles?.values() || [])];
  s.heldFiles?.clear();
  if (!held.length && !s.autoPlaced?.length) return s.placing || Promise.resolve();
  // 잇달아 불러도 차례대로 놓는다(앞의 것을 놓는 중이면 그 뒤에 잇는다). 놓기 전에, 앞서 놓아 준 파일 가운데
  // 그사이 지워진 것의 항목을 먼저 걷는다.
  // (걷는 일이 실패해도 이번에 만든 파일은 놓는다.)
  s.placing = (s.placing || Promise.resolve())
    .then(() => sweepPlaced(s).catch(() => {}))
    .then(() => (held.length ? placeHeldNow(s, held) : undefined))
    .catch(() => {});
  return s.placing;
}

/**
 * 이 세션이 만든 파일이라 놓아 준 항목 가운데, 그 파일이 그사이 지워진 것을 사이드바에서 걷는다(응답이 끝날 때마다 본다).
 * 앞선 응답에서 만든 작업 메모·초안을 뒤 응답에서 지우면 그 항목은 열어도 볼 것이 없는 중간 파일의 흔적이다.
 *
 * 걷는 것은 그 파일을 읽어 보여 주는 항목(문서, 표, HTML을 연 사이트)뿐이다. 이미지 항목은 그림을 Cypher의 저장소로
 * 가져와 두므로 원본이 없어져도 볼 수 있어 그대로 둔다. 사용자가 손댄 흔적이 있는 항목도 두고(이름·색·북마크를 붙였다,
 * 다른 워크스페이스로 옮겼다, 지금 화면에 띄워 보고 있거나 마지막에 보던 것이다 — 다음에 다시 본다), 다른 것을 보게 된 항목
 * (사이트가 다른 주소로 갔다)은 잊는다. 화면에 없는 항목만 걷으므로 보던 창의 배치는 건드리지 않는다.
 */
async function sweepPlaced(s) {
  if (!s.autoPlaced?.length) return;
  const alive = () => s.autoPlaced.filter(({ item, path }) => sessions.has(item.key) && showsFile(item, path));
  s.autoPlaced = alive();
  const list = [...s.autoPlaced];
  if (!list.length) return;
  let gone;
  try { gone = await invoke('created_gone', { paths: list.map((entry) => entry.path) }); } catch { return; }
  const removed = [];
  for (const [index, { item, path }] of list.entries()) {
    if (!gone[index] || !sessions.has(item.key) || !showsFile(item, path)) continue;
    // (화면을 비워 두어도 마지막에 보던 항목은 활성으로 남아 있다: 그것을 닫으면 다른 항목이 대신 화면에 뜬다.)
    if (item.name || item.color || item.mark || groupKey(item) !== groupKey(s) || visible.includes(item) || item === active) continue;
    const name = label(item);
    // 다시 열 것이 없으므로 닫은 항목 목록에는 남기지 않는다.
    await closeSession(item, { forget: true, still: true });
    if (sessions.has(item.key)) continue; // 닫히지 않았다.
    removed.push(name);
    // 같은 이름으로 다시 만들면 다시 놓을 수 있게 한다.
    s.placedFiles?.delete(pathKey(path));
  }
  s.autoPlaced = alive();
  if (!removed.length || !sessions.has(s.key)) return;
  const body = `만든 뒤에 지워진 파일의 항목 ${removed.length}개를 사이드바에서 걷었습니다: ${removed.slice(0, 5).join(', ')}${removed.length > 5 ? ' …' : ''}`;
  logNotice(s, 'item', label(s), body, false);
  setStatus(body);
}

async function placeHeldNow(s, held) {
  // 가려 주지 못하면(오류) 지금까지처럼 모두 놓는다. 앱은 중간 파일에 더해 git이 무시하는 파일과 소스 폴더의 파일도 가린다
  // (모든 글자 파일을 놓는 설정이면 소스 폴더의 파일은 가리지 않는다).
  const keepSources = (config.session.show_created || 'known') === 'all';
  const keep = await invoke('created_final', { paths: held.map((file) => file.path), base: s.startCwd || null, keepSources }).catch(() => held.map(() => 'final'));
  if (!sessions.has(s.key)) return; // 기다리는 사이 세션을 닫았다.
  const batch = { placed: 0, unplaced: 0 }; // 이번에(응답 하나에서) 놓은 수와 넘쳐서 놓지 않은 수
  for (const [index, file] of held.entries()) {
    const { path, kind } = file;
    // 아직 빈 파일은 다음 응답이 끝날 때 다시 본다(빈 파일로 만들어 두고 다음 요청에서 채우는 경우). 몇 번 보고도 비어 있으면 잊는다.
    if (keep[index] === 'later') {
      file.tries = (file.tries || 0) + 1;
      if (file.tries < HELD_TRIES && !s.heldFiles.has(pathKey(path))) s.heldFiles.set(pathKey(path), file);
      continue;
    }
    if (keep[index] !== 'final' || !shouldPlace(s, path, kind, batch)) continue;
    try {
      if (!sessions.has(s.key)) return;
      const item = await placeCreated(s, path, kind);
      // 그 파일을 읽어 보여 주는 항목은 적어 둔다: 파일이 나중에 지워지면 걷는다(sweepPlaced).
      if (item && ['reader', 'table', 'site'].includes(item.kind)) (s.autoPlaced ??= []).push({ item, path });
      if (item && sessions.has(s.key)) announce(item, s, '파일을 만들었습니다');
    } catch (err) {
      setStatus(`만든 파일을 사이드바에 놓지 못했습니다(${baseName(path)}): ${err}`);
    }
  }
}

/** 그 파일을 사이드바에 놓을지: 설정, 이미 놓았거나 보이는 파일, 한 요청의 한도를 본다. 놓기로 했으면 센다. */
function shouldPlace(s, path, kind, batch) {
  const mode = config.session.show_created || 'known';
  if (mode === 'off' || (kind === 'other' && mode !== 'all')) return false;
  // 같은 파일은 그 세션에서 한 번만 놓는다(지웠다가 다시 만들어도).
  s.placedFiles ??= new Set();
  if (s.placedFiles.has(pathKey(path))) return false;
  s.placedFiles.add(pathKey(path));
  // 그 묶음에 이미 이 파일을 보여 주는 항목이 있으면 더 놓지 않는다(사이트 항목은 HTML 파일일 때만 친다).
  // 코드 보기가 탭으로 열어 둔 것은 "보여 주는 항목"으로 치지 않는다(코드 보기는 프로젝트의 어느 파일이든 열어 둘 수 있다).
  const shows = (other) => other.kind !== 'code' && (other.kind !== 'site' || kind === 'html') && showsFile(other, path);
  if ([...sessions.values()].some((other) => groupKey(other) === groupKey(s) && shows(other))) return false;
  // 한 요청에서 파일을 많이 만들면 사이드바가 넘치므로 몇 개까지만 놓고, 넘친 것은 한 번만 알린다.
  if (batch.placed >= MAX_PLACED) {
    batch.unplaced += 1;
    if (batch.unplaced === 1) {
      const body = `이 요청에서 만든 파일이 많아 ${MAX_PLACED}개까지만 사이드바에 놓았습니다.`;
      logNotice(s, 'item', label(s), body);
      if (config.notify.in_app) toast(s, label(s), body);
    }
    return false;
  }
  batch.placed += 1;
  return true;
}

/** 종류에 맞는 항목으로 놓는다: 그림 → 이미지 항목, 표 파일 → 표 항목, HTML 파일 → 사이트 항목, 요청 파일 → HTTP 항목, 그 밖의 글 → 문서 항목. 놓은(또는 다시 읽게 한) 항목을 돌려준다. */
async function placeCreated(s, path, kind) {
  if (kind === 'image') {
    const [image] = await addImages([await invoke('image_import', { path })], { ...besideOf(s), source: path });
    return image;
  }
  if (kind === 'table') return newItem('table', { ...besideOf(s), saved: { path }, show: 'hidden' });
  // HTML 파일은 그 파일을 여는 사이트 항목으로 놓는다(웹뷰는 사용자가 그 항목을 처음 볼 때 뜬다).
  if (kind === 'html') return newItem('site', { ...besideOf(s), saved: { url: fileUrl(path), title: baseName(path) }, show: 'hidden' });
  // 요청 파일은 그 폴더를 보는 HTTP 항목으로 놓는다. 워크스페이스가 없는 세션은 자기 폴더 묶음에만 놓을 수 있으므로,
  // 요청 파일이 다른 폴더의 것이면(남의 폴더 묶음에 끼어들게 된다) 문서 항목으로 보여 준다.
  const home = kind === 'http' ? httpHome(path) : '';
  if (kind !== 'http' || !(s.group || samePath(home, s.startCwd))) {
    return newItem('reader', { ...besideOf(s), saved: { path }, show: 'hidden' });
  }
  // HTTP 항목은 폴더 단위로 요청 파일을 찾는다. 그 폴더를 보는 HTTP 항목이 이 묶음에 이미 있으면 새로 만들지 않고
  // 파일 목록만 다시 읽게 한다.
  const there = [...sessions.values()].find((other) => other.kind === 'http' && groupKey(other) === groupKey(s)
    && (samePath(other.startCwd, home) || samePath(other.startCwd, dirName(path))));
  if (there) {
    await there.view.reload?.();
    return there;
  }
  // 워크스페이스 없는 세션은 시작 폴더의 요청만 여기로 온다. 폴더는 세션의 것을 그대로 써야 같은 폴더 묶음에 들어간다
  // (파일 경로에 적힌 대/소문자가 달라도 다른 묶음이 생기지 않게).
  return newItem('http', { group: s.group, cwd: s.group ? home : s.startCwd, saved: { file: path }, show: 'hidden' });
}

/** Claude 세션의 Cypher 도구가 물어 온 것에 답한다. `req.session`은 도구를 부른 세션의 번호. */
async function mcpAction(req) {
  // 어느 세션이 불렀는지 모르면 답하지 않는다(지금 보고 있는 세션으로 치면 다른 워크스페이스의 항목을 다루게 될 수 있다).
  const s = req.session != null ? byId(req.session) : null;
  if (!s || !aliveClaude(s)) throw new Error('이 도구를 부른 Claude 세션을 Cypher에서 찾지 못했습니다.');
  switch (req.action) {
    case 'browser_target': return browserTarget(s, !!req.create);
    case 'browser_tabs': return { text: browserTabs(s) };
    case 'browser_select': return { text: browserSelect(s, req.site) };
    case 'items': return { text: groupItemsText(s) };
    case 'mindmap_write': return { text: await mindmapWrite(s, req) };
    case 'erd_write': return { text: await erdWrite(s, req) };
    case 'memo_write': return { text: await memoWrite(s, req) };
    case 'image_add': return { text: await imageAdd(s, req) };
    case 'doc_show': return { text: await showFile(s, req, 'reader', '문서') };
    case 'table_show': return { text: await showFile(s, req, 'table', '표') };
    case 'todo_write': return { text: await todoWrite(s, req) };
    case 'todo_check': return { text: await todoCheck(s, req) };
    case 'run_output': return { text: runOutput(s, req) };
    case 'notify': return { text: claudeNotice(s, req) };
    case 'http_scope': return { folders: httpFolders(s) };
    // "chrome" 엔진의 브라우저는 워크스페이스마다 따로 뜬다. 워크스페이스에 든 세션에만, 그 워크스페이스의 브라우저만 맡긴다.
    case 'chrome_scope': requireGroup(s); return { profile: ensureProfile(s) };
    default: throw new Error(`모르는 요청입니다: ${req.action}`);
  }
}

async function onMcpRequest(req) {
  // 화면이 한참 멈춰 있다가(확인 창이 떠 있었다 등) 뒤늦게 받은 요청: 앱은 이미 "응답하지 않습니다"로 답했고
  // Claude는 실패로 알고 있다. 지금 실행하면 다시 시도한 것과 겹쳐 두 번 들어간다.
  if (req.deadline && Date.now() > req.deadline) return;
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

/**
 * 적어 넣은 시작 페이지를 주소로 다듬는다: 앞에 `https://`가 없으면 붙인다. 비우면 빈 글(빈 화면), 주소가 아니면 null.
 */
function homeAddress(text) {
  const given = text.trim();
  if (!given) return '';
  if (/\s/.test(given)) return null;
  try {
    const url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(given) ? given : `https://${given}`);
    const named = url.hostname.includes('.') || url.hostname === 'localhost';
    return /^https?:$/.test(url.protocol) && named ? url.href : null;
  } catch {
    return null;
  }
}

/** 설정 → "브라우저 시작 페이지…": 새 사이트 항목과 "chrome" 엔진의 브라우저가 처음 여는 주소를 정한다. */
async function askHomePage() {
  const given = await ask('브라우저 시작 페이지 (새 사이트 항목이 처음 여는 주소 · 비우면 빈 화면)', '예: https://www.google.com', homePage);
  if (given == null) return;
  const address = homeAddress(given);
  if (address == null) return notice('브라우저 시작 페이지', `주소가 아닙니다: ${given} (예: https://www.google.com). 바꾸지 않았습니다.`);
  homePage = address;
  try { localStorage.setItem('cypher.home', homePage); } catch { /* 저장 공간 문제는 무시 */ }
  setStatus(homePage ? `브라우저 시작 페이지: ${homePage} — 이제부터 만드는 사이트 항목이 이 주소로 열립니다.` : '브라우저 시작 페이지를 비웠습니다. 새 사이트 항목은 빈 화면으로 열립니다.');
}

/**
 * 설정 → "Claude 실행 명령…": "+ Claude"로 여는 세션을 띄울 명령을 정한다(별칭·감싼 스크립트·늘 붙이는 인자 등).
 * Cypher는 이 명령 뒤에 자기 인자(--settings, --mcp-config, --resume)를 붙여 띄운다. 비우면 프로필의 명령(기본은 claude)이다.
 * 프로그램은 PATH에서 찾고(실행 파일, 배치 파일, PowerShell 스크립트), 파일로 찾지 못하면 PowerShell의 명령(프로필에 만들어 둔 별칭·함수)으로
 * 실행해 본다. 경로의 `%USERPROFILE%`·`~`를 풀고, 빈칸이 든 경로를 따옴표 없이 적어도 실제 파일까지를 프로그램으로 본다(cypher-pty, profile.rs).
 */
async function askClaudeCommand() {
  const given = await ask('Claude 실행 명령 ("+ Claude"로 여는 세션을 이 명령으로 띄웁니다 · 비우면 기본값 claude)', '예: claude --dangerously-skip-permissions', claudeCommand);
  if (given != null) setClaudeCommand(given);
}

/** Claude 실행 명령을 정한다(빈 글이면 기본 명령). 따옴표가 닫히지 않은 명령은 받지 않는다. 정했으면 참. */
function setClaudeCommand(given) {
  const command = String(given ?? '').trim();
  // 빈칸이 든 경로는 큰따옴표로 감싼다. 따옴표가 짝이 맞지 않으면 세션을 띄울 때 읽지 못한다.
  if ((command.match(/"/g) || []).length % 2) { notice('Claude 실행 명령', `큰따옴표가 닫히지 않았습니다: ${command}. 바꾸지 않았습니다.`); return false; }
  claudeCommand = command;
  try { localStorage.setItem('cypher.claudeCommand', claudeCommand); } catch { /* 저장 공간 문제는 무시 */ }
  setStatus(claudeCommand
    ? `Claude 실행 명령: ${claudeCommand} — 이제부터 여는 Claude 세션에 적용됩니다(열려 있는 세션은 그대로). 프로그램·배치 파일(.cmd)·PowerShell 스크립트(.ps1)와 PowerShell 프로필의 별칭·함수를 쓸 수 있습니다.`
    : 'Claude 실행 명령을 비웠습니다. 기본 명령(claude)으로 띄웁니다.');
  return true;
}

/** 지금 보고 있는 항목의 워크스페이스가 쓰는 Chrome을 연다. 워크스페이스가 없으면 공용 프로필의 Chrome(Claude 세션은 조작할 수 없다). */
async function openChrome() {
  const group = viewing()?.group || '';
  try {
    const profile = ensureProfile(viewing());
    // 처음 띄울 때만 시작 페이지로 연다(이미 떠 있는 브라우저는 보던 페이지를 그대로 둔다).
    const open = await invoke('browser_where', { profile }).catch(() => null);
    await invoke('browser_open', { url: open ? null : (homePage || null), profile });
    setStatus(group ? `워크스페이스 "${group}"의 Chrome을 열었습니다. 이 워크스페이스의 Claude 세션만 조작할 수 있습니다.`
      : '워크스페이스 없는 Chrome을 열었습니다. Claude 세션에 맡기려면 워크스페이스에 든 항목을 보고 있을 때 여세요.');
  } catch (err) {
    notice('브라우저를 열지 못했습니다', String(err));
  }
}

/** Chrome에 지금 열려 있는 페이지를 Claude 세션에 넘긴다. `fresh`이면 새 Claude 세션을 연다. */
async function handoffChrome(fresh) {
  // 넘기는 것은 지금 보고 있는 항목의 워크스페이스가 쓰는 Chrome이고, 넘겨받는 것은 그 워크스페이스의 Claude 세션이다.
  const from = viewing();
  if (!from?.group) return notice('브라우저', 'Chrome은 워크스페이스마다 따로 뜨고 그 워크스페이스의 Claude 세션만 조작합니다. 워크스페이스에 든 항목을 보고 있을 때 넘기세요.');
  // 프로필이 아직 없는 워크스페이스는 Chrome을 띄운 적이 없다.
  const profile = profileOf(from);
  const url = profile ? await invoke('browser_where', { profile }).catch(() => null) : null;
  if (!url) return notice('브라우저', `워크스페이스 "${from.group}"의 Chrome이 열려 있지 않습니다. 먼저 [Chrome]을 눌러 여세요.`);
  let target = fresh ? null : (aliveClaude(from) ? from : [...sessions.values()].find((s) => aliveClaude(s) && s.group === from.group));
  if (!target) {
    target = await newSession('claude', { ...besideOf(from), show: 'replace' });
    if (target) await untilReady(target);
    if (!target || target.exited || !sessions.has(target.key)) return notice('브라우저', '새 Claude 세션을 열지 못했습니다.');
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
      // 곁에 둘 항목이 있으면 그 항목과 같은 묶음에 넣는다(워크스페이스 없는 항목은 시작 폴더로 묶인다).
      group: group ?? (base ? besideOf(base).group : undefined),
      cwd: cwd ?? (base ? besideOf(base).cwd : undefined),
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
  rowOf(fresh[fresh.length - 1])?.scrollIntoView({ block: 'nearest' });
}

/**
 * 화면을 찍은 그림을 이미지 항목으로 넣는다. `near`(찍은 사이트 항목, 또는 "chrome" 엔진이면 찍기를 누를 때 보고 있던 항목)의
 * 묶음에 넣는다. 찍는 사이에 다른 항목으로 옮겨 가도 찍은 브라우저의 워크스페이스에 들어간다.
 */
async function onCaptured(captured, near) {
  const [s] = await addImages([captured.image], { near, source: captured.source, shot: true, full: captured.full });
  if (!s) return;
  toast(s, captured.full ? '페이지 전체 캡처' : '화면 캡처',
    `${label(s)} — ${whereOf(s)}에 이미지 항목으로 넣었습니다.${captured.cut ? ' 페이지가 너무 길어 아래쪽은 잘렸습니다.' : ''} (누르면 엽니다)`);
}

/** 그 항목이 놓인 곳을 말로: 워크스페이스 "결제 개편" 또는 "ultron" 묶음. */
const whereOf = (s) => (s.group ? `워크스페이스 "${s.group}"` : `"${baseName(s.startCwd)}" 묶음`);

/** 본창에서 브라우저 화면을 찍어, 찍기를 누를 때 보고 있던 항목의 묶음에 이미지 항목으로 넣는다. */
async function captureBrowser(full) {
  // 찍는 것은 지금 보고 있는 항목의 워크스페이스가 쓰는 Chrome이다(워크스페이스가 없으면 공용 프로필의 Chrome).
  const from = viewing();
  if (from?.group && !profileOf(from)) return notice('캡처하지 못했습니다', `워크스페이스 "${from.group}"의 Chrome이 열려 있지 않습니다.`);
  setStatus('브라우저 화면을 찍는 중…');
  try {
    const captured = await invoke('browser_capture', { full, profile: profileOf(from) });
    setStatus('');
    await onCaptured(captured, from);
  } catch (err) {
    setStatus('');
    notice('캡처하지 못했습니다', String(err));
  }
}

/**
 * 캡처한 그림의 원본 페이지를 연다: 그 이미지 항목과 같은 묶음에 사이트 항목으로 연다
 * ("chrome" 엔진이면 그 워크스페이스의 Chrome으로). 그래야 그 워크스페이스의 로그인 상태로 열린다.
 */
async function openSourcePage(item, url) {
  try {
    if (chromeEngine()) await invoke('browser_open', { url, profile: ensureProfile(item) });
    else await newItem('site', { ...besideOf(item), saved: { url }, show: 'replace' });
  } catch (err) {
    setStatus(String(err));
  }
}

/** 글을 Claude 세션 입력창에 넣는다(보내지는 않는다). 같은 묶음의 세션을 먼저 고르고, 여럿이면 고르게 한다. */
function textToClaude(item, text, ev) {
  if (!text) return undefined;
  const claudes = [...sessions.values()].filter(aliveClaude);
  if (!claudes.length) return setStatus('열려 있는 Claude 세션이 없습니다. 먼저 Claude 세션을 여세요.');
  const give = (target) => {
    activate(target);
    target.term.paste(text);
  };
  const same = claudes.filter((s) => groupKey(s) === groupKey(item));
  const pool = same.length ? same : claudes;
  if (pool.length === 1) return give(pool[0]);
  return showMenu(ev, pool.map((s) => ({ label: `${label(s)}에 넣기`, action: () => give(s) })));
}

/** 파일(이미지·문서·표)의 경로를 Claude 세션 입력창에 넣는다. */
function imageToClaude(item, path, ev) {
  if (!path) return setStatus('아직 파일이 없습니다.');
  return textToClaude(item, `${quote(path)} `, ev);
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
 * SSH 세션에는 두지 않는다(원격 셸에는 긴 프롬프트를 써 넣을 일이 없고, 그만큼 터미널을 넓게 쓴다).
 * 세션이 종료돼도 편집기는 남긴다(쓰던 글을 복사해 갈 수 있게). 보내기만 막는다.
 */
const composerTarget = () => (visible.length === 1 && isTerm(visible[0]) && !visible[0].ssh ? visible[0] : null);

/** 프롬프트 편집기를 접거나 편다. 단추 글자와 저장값을 함께 맞춘다. */
function setComposerFolded(folded) {
  if ($('composer').classList.contains('folded') === folded) return;
  // 접거나 펴면 세션 화면의 줄 수가 바뀐다. 높이를 끌 때처럼 한 번에 맞추고 Claude 세션은 화면을 다시 맞춘다.
  resizeComposer(() => {
    $('composer').classList.toggle('folded', folded);
    $('composer-fold').textContent = folded ? '프롬프트 편집기 ▴' : '프롬프트 편집기 ▾';
    try { localStorage.setItem('cypher.composer.folded', folded ? '1' : ''); } catch { /* 저장 공간 문제는 무시 */ }
  });
}

/**
 * 프롬프트 편집기의 크기를 바꾸는 일(`change`)을 하고, 그 뒤에 세션 화면을 한 번에 새 크기에 맞춘다(settleResize).
 * 바꾸는 동안에는 터미널 크기를 묶어 두고, 무슨 일이 있어도 묶인 채로 남지 않게 한다.
 */
function resizeComposer(change) {
  fitHeld = true;
  try {
    change();
  } finally {
    let done = false;
    const settle = () => { if (!done) { done = true; settleResize(); } };
    requestAnimationFrame(settle);
    setTimeout(settle, 120); // 창이 가려져 그리기 신호가 오지 않을 때를 대비한다.
  }
}

function initComposer() {
  const $text = $('composer-text'), $box = $('composer');
  const setHeight = (height) => {
    const clamped = Math.min(Math.max(56, Math.round(height)), Math.max(120, innerHeight - 260));
    $box.style.setProperty('--composer', `${clamped}px`);
    return clamped;
  };
  setHeight(Number(localStorage.getItem('cypher.composer.height')) || COMPOSER_HEIGHT);
  // 처음에는 저장해 둔 대로만 차린다(맞출 세션이 아직 없다).
  $box.classList.toggle('folded', !!localStorage.getItem('cypher.composer.folded'));
  $('composer-fold').textContent = $box.classList.contains('folded') ? '프롬프트 편집기 ▴' : '프롬프트 편집기 ▾';
  $('composer-fold').onclick = () => {
    setComposerFolded(!$box.classList.contains('folded'));
    if (!$box.classList.contains('folded')) $text.focus(); else focusItem(active);
  };
  // "바로 전송"은 기본으로 켜져 있다: 한 번도 건드리지 않았으면(저장값 없음) 켜고, 사용자가 일부러 끈 것('0', 예전 판의 '')은 끈 채로 둔다.
  const sendNow = localStorage.getItem('cypher.composer.enter');
  $('composer-enter').checked = sendNow == null || sendNow === '1';
  $('composer-enter').onchange = (ev) => localStorage.setItem('cypher.composer.enter', ev.target.checked ? '1' : '0');
  $('composer-send').onclick = () => sendPrompt();
  $('composer-history').onclick = promptMenu;
  $text.addEventListener('input', () => {
    if (!recalling) promptAt = -1; // 직접 고치기 시작하면 기록을 넘기던 자리를 잊는다.
    if (composerFor) { composerFor.draft = $text.value; saveState(); }
  });
  // 클립보드에 그림만 있을 때의 붙여넣기(Ctrl+V): 터미널에 붙여넣을 때와 같게 한다. 글자가 있으면 평소대로 글자를 넣는다.
  let plainPaste = false; // Ctrl+Shift+V로 붙여넣는 중인지(그림을 파일 경로로 넣는다)
  $text.addEventListener('paste', (ev) => {
    const items = [...(ev.clipboardData?.items || [])];
    if (items.some((item) => item.type === 'text/plain') || !items.some((item) => item.type.startsWith('image/'))) return;
    ev.preventDefault();
    pasteImageInComposer(plainPaste);
  });
  $text.addEventListener('keydown', (ev) => {
    if (ev.ctrlKey && ev.key.toLowerCase() === 'v') plainPaste = ev.shiftKey;
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
    // 끄는 동안에는 세션 화면의 줄 수를 그대로 두고, 손이 멈추면(그리고 놓으면) 새 높이에 맞춰 화면을 다시 맞춘다.
    const move = (e) => {
      if (e.buttons === 0) return stop(); // 놓는 신호를 받지 못했다(창 밖에서 놓음): 여기서 끝낸다
      settleResizeSoon();
      height = setHeight(bottom - e.clientY - 4);
    };
    const stop = () => {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', stop);
      window.removeEventListener('blur', stop);
      document.body.classList.remove('resizing-col');
      try { if (height) localStorage.setItem('cypher.composer.height', String(height)); } catch { /* 저장 공간 문제는 무시 */ }
      settleResize();
    };
    // 끄는 중에 창이 초점을 잃어 놓는 신호를 받지 못해도 크기가 묶인 채로 남지 않게 한다.
    window.addEventListener('blur', stop);
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', stop);
  };
  grip.ondblclick = () => resizeComposer(() => {
    const height = setHeight(COMPOSER_HEIGHT);
    try { localStorage.setItem('cypher.composer.height', String(height)); } catch { /* 저장 공간 문제는 무시 */ }
  });
}

/**
 * 프롬프트 편집기에서 클립보드의 그림을 붙여넣는다. 터미널에 붙여넣을 때(paste)와 같다.
 * - Claude 세션: 그 세션에 Alt+V 신호를 보내 Claude Code가 클립보드의 그림을 입력창에 붙이게 한다([Image #n]).
 *   편집기의 글은 [프롬프트]를 누를 때 그 뒤에 들어간다.
 * - 그 밖의 세션, 또는 Ctrl+Shift+V: PNG 파일로 저장하고 그 경로를 편집기의 커서 자리에 넣는다.
 */
async function pasteImageInComposer(forceFile) {
  const s = composerFor, $text = $('composer-text');
  if (!s) return;
  try {
    const clip = await invoke('clipboard_read');
    if (clip.kind === 'empty') return;
    const asText = () => { $text.focus(); document.execCommand('insertText', false, clip.text); };
    if (clip.kind !== 'image') return asText();
    if (runsClaude(s) && !forceFile && !s.exited) {
      send(s, '\x1bv');
      setStatus('그림을 세션 입력창에 붙였습니다. 편집기에 쓴 글은 [프롬프트]를 누르면 그 뒤에 들어갑니다. (Ctrl+Shift+V: 파일 경로로 넣기)');
    } else {
      asText();
    }
    addShot(s, clip);
  } catch (err) {
    setStatus(`붙여넣기 실패: ${err}`);
  }
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
    ? '종료된 세션 · 쓰던 글은 복사해 다른 세션에 쓸 수 있음'
    : '세션 입력창에 넣기 (Ctrl+Enter)';
  if (target) {
    $text.placeholder = `"${label(target)}"에 보낼 긴 프롬프트를 여기에 씁니다. [프롬프트] 또는 Ctrl+Enter를 누르면 세션 입력창에 들어갑니다. (Ctrl+Shift+E: 여기로 오기)`;
  }
}

function focusComposer() {
  if (!composerTarget()) {
    return setStatus(visible.length === 1 && visible[0].ssh
      ? 'SSH 세션에는 프롬프트 편집기가 없습니다.'
      : '프롬프트 편집기는 화면에 세션 하나만 보일 때 쓸 수 있습니다.');
  }
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
  // 재개하지 못해 쉬고 있는 세션에는 받을 프로세스가 없다(보내면 글만 사라진다).
  if (s.dormant) return setStatus('이 세션은 재개하지 못해 쉬고 있습니다. 사이드바에서 세션을 눌러 재개한 뒤 보내세요. 쓰던 글은 편집기에 그대로 있습니다.');
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
  // 쉬는 세션(아직 재개하지 않은 것)도 적어 둔다. 빠뜨리면 다음에 켰을 때 목록에서 사라진다.
  // 띄우는 중인 세션(번호를 아직 받지 못했다)도 적는다. 재개하는 짧은 사이에 적은 기록에서 빠지면 안 된다.
  const live = [...sessions.values()].filter((s) => (isTerm(s) ? s.dormant || !s.exited : !!s.view));
  const pack = (node) => {
    if (!node) return null;
    if (node.room) return { room: node.room };
    return node.s ? { i: live.indexOf(node.s) } : { dir: node.dir, ratio: ratioOf(node), a: pack(node.a), b: pack(node.b) };
  };
  const state = {
    // 되살리지 못한 항목의 기록은 그대로 다시 적는다(순번이 배치와 어긋나지 않게 맨 뒤에 둔다).
    // 항목 하나의 화면이 상태를 내놓다 실패해도 저장 전체가 멈추지 않게, 그 항목은 지난번에 적은 내용을 그대로 쓴다.
    sessions: [...live.map((s) => {
      try { s.lastRecord = describe(s); } catch (err) { console.error('항목의 상태를 읽지 못했습니다', s.kind, err); }
      return s.lastRecord ? { ...s.lastRecord, shown: visible.includes(s) } : null;
    }), ...unrestored],
    active: live.indexOf(active),
    // 화면 분할 모양. 잎은 세션의 순번({ i })이나 SSH 방({ room: 워크스페이스 이름 }), 가지는 { dir, a, b }.
    layout: pack(tree),
    // SSH 방 안의 배치(워크스페이스마다 하나). 다시 켜면 SSH는 탭 하나로 돌아오므로 배치를 되살리지는 않고,
    // 어느 탭을 남길지 고르는 데만 쓴다(sshDropped: 창에 보이던 탭이 먼저다).
    rooms: [...rooms].map(([group, inner]) => ({ group, layout: pack(inner) })),
  };
  try { localStorage.setItem('cypher.state', JSON.stringify(state)); } catch { /* 저장 공간 문제는 무시 */ }
}

/**
 * 항목을 다시 만들 수 있게 적어 둘 내용: 종류, 폴더, 별칭·워크스페이스·색, 그리고 세션이면 프로필과 이어갈 대화 번호,
 * 그 밖의 항목이면 그 화면이 내놓은 상태. 배치 기억(writeState)과 닫은 항목 목록이 함께 쓴다.
 */
function describe(s) {
  return {
    kind: s.kind,
    cwd: s.project || s.startCwd,
    name: s.name,
    group: s.group,
    color: s.color,
    mark: s.mark || undefined, // 북마크 숫자(Ctrl+숫자로 바로 가기)
    ...(isTerm(s) ? {
      profile: s.profile.id,
      resume: resumeOf(s) || null,
      draft: s.draft || undefined,
      ssh: s.ssh || undefined, // SSH 세션이 접속할 곳
    } : { saved: s.view.state() }),
  };
}

/** describe()로 적어 둔 내용으로 항목을 다시 만든다. `opts`는 newSession·newItem에 그대로 더해진다(show, restoring 등). */
function revive(item, opts = {}) {
  // 워크스페이스 없이 적혀 있던 것(이전 버전에서 워크스페이스 없이 쓰던 것, 그때 닫은 항목)은 `root` 워크스페이스에 넣는다. 워크스페이스 밖에는 아무것도 둘 수 없다.
  const common = { cwd: item.cwd, name: item.name, color: item.color, mark: item.mark, ...opts, group: opts.group || item.group || adoptGroup() };
  // 종류 표시가 없는 예전 기록은 터미널 세션이다.
  const term = !item.kind || item.kind === 'term';
  return term
    ? newSession(item.profile, { ...common, resume: item.resume, draft: item.draft, ssh: item.ssh })
    : newItem(item.kind, { ...common, saved: item.saved });
}

/**
 * 저장해 둔 분할 모양을 되살린다. 없어진 세션의 자리는 짝이 차지한다.
 * SSH 방의 자리({ room })는 되살리지 않는다: 다시 켠 뒤의 SSH는 쉬고 있고, 화면에 올리는 것은 사용자가 SSH 줄을 누를 때다.
 */
function unpack(data, list) {
  if (!data || typeof data !== 'object') return null;
  if (typeof data.room === 'string') return null;
  if ('i' in data) return list[data.i] ? { s: list[data.i] } : null;
  const a = unpack(data.a, list), b = unpack(data.b, list);
  return a && b ? { dir: data.dir === 'col' ? 'col' : 'row', ratio: data.ratio, a, b } : (a || b);
}

/**
 * 다시 켰을 때 되살리지 않을 SSH 세션들(저장 기록의 순번): 워크스페이스마다 하나만 남기고 나머지는 뺀다.
 *
 * 탭을 모두 되살리면 사용자가 SSH를 다시 열 때 여러 탭이 한꺼번에 서버에 접속하게 되고, 서버는 동시에 들어오는 접속을 침입 시도로
 * 보고 막는다. 그래서 다시 켠 뒤의 SSH는 워크스페이스 머리의 [+]로 처음 만든 것과 같게 둔다: 탭 하나, 창 하나, 접속은 사용자가
 * 누를 때 한 번(사용자 이름부터 묻는다). 서버가 더 필요하면 접속한 뒤 탭을 복제하거나(같은 연결을 써서 다시 인증하지 않는다)
 * [+]로 더 연다. 남기는 탭은 끌 때 보고 있던 탭, 아니면 그 SSH 창에 보이던 첫 탭, 그것도 없으면 첫 탭이다.
 */
function sshDropped(saved) {
  const isSsh = (item) => !!item && typeof item === 'object' && (!item.kind || item.kind === 'term')
    && isSshProfile(config.profiles.find((p) => p.id === item.profile)) && !!cleanSpec(item.ssh);
  const leaves = (node) => (!node || typeof node !== 'object' ? [] : Number.isInteger(node.i) ? [node.i] : [...leaves(node.a), ...leaves(node.b)]);
  const front = new Map(); // 워크스페이스 → 그 SSH 창들에 보이던 탭의 순번
  for (const room of Array.isArray(saved.rooms) ? saved.rooms : []) {
    if (room && typeof room.group === 'string') front.set(room.group, leaves(room.layout));
  }
  const byGroup = new Map(); // 워크스페이스 → 그 워크스페이스의 SSH 세션 순번들
  saved.sessions.forEach((item, index) => {
    if (!isSsh(item)) return;
    const group = String(item.group || ROOT_GROUP); // revive와 같게 센다: 워크스페이스 없이 적힌 것은 root로 간다.
    if (!byGroup.has(group)) byGroup.set(group, []);
    byGroup.get(group).push(index);
  });
  const dropped = new Set();
  for (const [group, list] of byGroup) {
    const keep = list.includes(saved.active) ? saved.active : (front.get(group) || []).find((i) => list.includes(i)) ?? list[0];
    for (const index of list) if (index !== keep) dropped.add(index);
  }
  return dropped;
}

/**
 * 지난번 항목과 배치를 되살린다.
 * `docsOnly`이면(세션 복원을 꺼 두었거나 바로 시작 모드) 터미널 세션은 되살리지 않고 메모·HTTP 항목만
 * 사이드바에 되살린다. 메모는 사용자가 쓴 글이라, 세션 복원 설정과 무관하게 목록에서 사라지면 안 된다.
 */
/** 되살리지 못한 항목의 저장 기록. 다음 저장 때 그대로 다시 적는다. */
let unrestored = [];

async function restore({ docsOnly = false } = {}) {
  docsOnly = docsOnly || !config.session.restore;
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem('cypher.state') || 'null'); } catch { /* 깨진 기록은 버린다 */ }
  if (!saved?.sessions?.length) return;
  const restored = [];
  unrestored = [];
  // SSH는 워크스페이스마다 탭 하나만 되살린다(sshDropped). 뺀 탭은 되살리지 못한 것이 아니므로 기록에도 남기지 않는다.
  const dropped = docsOnly ? new Set() : sshDropped(saved);
  // 되살리는 동안에는 상태를 적지 않는다. 도중에 적으면 아직 되살리지 않은 뒤쪽 항목이 기록에서 빠져,
  // 그때 앱이 꺼지면 그 항목(메모 포함)이 목록에서 사라진다.
  restoring = true;
  try {
    for (const [index, item] of saved.sessions.entries()) {
      if (dropped.has(index)) { restored.push(null); continue; }
      // 항목 하나가 되살아나지 못해도(깨진 기록 포함) 뒤의 항목은 되살린다. 여기서 멈추면 뒤의 항목들이 다음 저장 때
      // 기록에서 사라진다. 되살리지 못한 항목의 기록은 남겨 두었다가 다음에 켤 때 다시 되살려 본다.
      try {
        const term = !item.kind || item.kind === 'term';
        // 터미널 세션은 프로세스를 띄우지 않고 사이드바에만 되살려 둔다(쉬는 세션). 사용자가 누른 것만 재개한다.
        // 설정의 [session] restore_on_click = false이면 예전처럼 모두 바로 띄운다. SSH 세션은 그 설정과 무관하게 늘 쉬는 세션으로
        // 둔다: 켜자마자 여러 워크스페이스의 SSH가 한꺼번에 접속하면 안 된다(접속은 사용자가 SSH 줄을 누를 때).
        const dormant = term && (config.session.restore_on_click !== false || !!item.ssh);
        const skip = term && docsOnly;
        const s = skip ? null : await revive(item, { show: 'hidden', restoring: true, dormant });
        restored.push(s);
        // 오류 없이 되살아나지 못한 항목도 기록을 남겨 둔다: 설정 파일에 오류가 있어 기본값으로 뜬 동안에는 사용자가 만든
        // 프로필이 없어 그 세션을 만들지 못하는데, 그대로 저장하면 설정을 고친 뒤에도 돌아오지 않는다(별칭·이어 갈 대화까지).
        // 프로필을 정말 지운 경우에 끝없이 남지 않도록 몇 번 해 보고 그만둔다.
        const missed = (item?.missed || 0) + 1;
        if (!s && !skip && item && typeof item === 'object' && missed <= 5) unrestored.push({ ...item, shown: false, missed });
      } catch (err) {
        console.error('항목을 되살리지 못했습니다', item, err);
        restored.push(null);
        if (item && typeof item === 'object') unrestored.push({ ...item, shown: false });
      }
    }
  } finally {
    restoring = false;
  }
  if (docsOnly) { render(); saveState(); return; } // 메모·HTTP는 사이드바에만 되살린다.
  // SSH 방은 저장해 둔 배치로 되돌리지 않는다: 워크스페이스마다 탭 하나뿐이라, 방은 그 탭의 창 하나로 새로 생긴다(settleRoom).
  rooms.clear();
  // 분할 모양을 되살린다. 저장된 모양이 없으면(예전 기록) 보이던 세션을 고르게 나눈다.
  // 되살리는 도중에 사용자가 닫은 항목은 배치에서 뺀다(순번은 저장 때와 맞아야 하므로 자리는 비워 둔다).
  // 쉬는 세션은 화면에 올리지 않는다(올리면 재개된다). SSH는 늘 쉬고 있으므로 SSH 방도 화면에 올리지 않는다.
  const alive = restored.map((s) => (s && sessions.has(s.key) && !s.dormant ? s : null));
  tree = unpack(saved.layout, alive)
    || balanced(alive.filter((s, index) => s && saved.sessions[index].shown).slice(0, MAX_PANES));
  layout();
  // 끌 때 보던 것이 세션이면 지금은 쉬고 있어 고를 수 없다. 그때는 되살린 배치에 있는 것을 고른다
  // (화면에 없던 항목을 골라 배치에 끼워 넣지 않는다).
  const target = alive[saved.active] || leavesOf(tree)[0] || alive.find(Boolean);
  if (target) activate(target); else render();
  const resting = wakeable(restored.filter(Boolean)).length; // ▶로 한꺼번에 재개할 수 있는 것(SSH가 아닌 쉬는 세션)
  const sshResting = restored.some((s) => s?.ssh && s.dormant && sessions.has(s.key));
  const sshNote =dropped.size ? ` SSH는 워크스페이스마다 탭 하나로 되살렸습니다(다른 탭 ${dropped.size}개는 닫음 · 접속한 뒤 탭을 복제하거나 +로 더 엽니다).` : '';
  if (unrestored.length) setStatus(`지난 항목 ${unrestored.length}개를 되살리지 못했습니다. 다음에 켤 때 다시 되살려 봅니다.${sshNote}`);
  else if (resting) setStatus(`지난 세션 ${resting}개는 쉬고 있습니다. 사이드바에서 세션을 누르거나 워크스페이스 머리의 ▶를 누르면 재개합니다${sshResting ? '(SSH는 SSH 줄을 누를 때만 접속)' : ''}.${sshNote}`);
  else if (sshResting) setStatus(`SSH는 쉬고 있습니다. 사이드바의 SSH 줄을 누르면 접속합니다.${sshNote}`);
}

// ───────────────────────── 입력 ─────────────────────────

/** 키 입력을 순서대로 보낸다. 동시 입력이 켜져 있으면 나란히 보이는 모든 세션에 보낸다. */
function send(s, data) {
  const casting = castTargets();
  const targets = casting.includes(s) ? casting : [s];
  const interrupts = data === '\x1b' || data === '\x03'; // Esc, Ctrl+C
  for (const t of targets) {
    // 프로세스가 없는 세션(재개에 실패해 다시 쉬고 있거나 시작하지 못한 것)에 친 키는 받지 않는다.
    // 쌓아 두면 나중에 재개했을 때 예전에 친 글이 Enter까지 한꺼번에 들어간다.
    if (t.id == null && (t.dormant || t.exited)) continue;
    // 일하는 중에 누른 Esc·Ctrl+C는 요청을 중단시킬 수 있다. 조금 뒤 조용해졌는지 보고 작업 시간을 멈춘다(refreshActivity).
    if (interrupts && t.workStart) t.interruptAt = performance.now();
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

// ───────────────────────── 화면 다시 맞추기 ─────────────────────────
// Claude Code는 화면 아래쪽(쓰고 있는 응답, 입력창)을 "몇 줄 위로 올라가 다시 그리는" 식으로 고친다. 그런데 국기 🇰🇷·키캡 1️⃣·
// 이어 붙인 이모지·결합 글자는 Claude Code와 Windows의 가상 터미널(conhost)이 칸 수를 다르게 세서, 그런 글자가 든 줄이
// 가상 터미널에서 한 줄 더 차지하면 Claude Code가 줄 수를 잘못 알게 되고 구분선·입력창이 겹쳐 남는다. 화면 쪽에서는 이 차이를
// 없앨 수 없으므로, 그런 글자가 찍힌 세션은 응답이 끝났을 때 Claude Code가 화면을 처음부터 다시 그리게 한다.
// 다시 그리게 하는 방법은 창 크기 변경이다: 줄 수를 한 줄 줄였다가 되돌리면 Claude Code가 화면을 지우고 전부 다시 그린다.

const REALIGN_HOLD_MS = 450;   // 한 줄 줄인 채로 두는 시간(프로그램이 크기 변경을 알아채고 다시 그릴 틈)
const REALIGN_QUIET_MS = 2500; // 다시 맞춘 뒤 이 시간 동안 찍힌 글자는 다시 그리느라 나온 것이므로 세지 않는다

/**
 * 어긋날 만한 글자가 이 세션에 찍혔다. 적어만 두고, 다시 맞추는 것은 응답이 끝났을 때 한다(realignIfSkewed).
 * Claude가 도는지는 여기서 가리지 않는다: 대화를 이어 열 때는 Claude가 뜬 것을 알기도 전에 지난 대화가 찍힌다.
 * 셸만 도는 세션은 완료 신호가 오지 않으므로 다시 맞추는 일이 없다.
 */
function noteSkew(s) {
  if (s.skewed || performance.now() < (s.realignedAt || 0) + REALIGN_QUIET_MS) return;
  s.skewed = true;
}

/** 어긋날 만한 글자가 찍힌 세션이면 화면을 다시 맞춘다. 응답이 끝났을 때 부른다. */
function realignIfSkewed(s) {
  if (!s?.skewed || s.exited) return;
  // 지난 출력을 올려 보고 있으면 건드리지 않는다(다시 그리면 보던 자리를 잃는다). 다음 응답이 끝날 때 다시 본다.
  const buffer = s.term.buffer.active;
  if (buffer.viewportY < buffer.baseY) return;
  realign(s);
}

/** 다시 맞추기 전의 화면: 화면의 윗줄 글자와 그 줄에 꽂은 표시, 내용의 끝을 보고 있었는지, 보던 자리, 아래에 남은 빈 줄 수. */
function screenBefore(s) {
  const buffer = s.term.buffer.active;
  return {
    top: topLines(s.term),
    mark: buffer.type === 'normal' ? s.term.registerMarker(-buffer.cursorY) : null, // 지금 화면의 맨 윗줄
    atEnd: seesEnd(s.term),
    viewY: buffer.viewportY,
    gap: gapBelow(s.term),
  };
}

// 프롬프트 편집기의 위쪽 테두리를 끌어 높이를 바꾸면 그 위의 세션 화면도 줄 수가 바뀐다. 끄는 내내 터미널 크기를 따라 바꾸면
// Claude Code가 그때마다 화면을 다시 그려 지난 출력에 같은 줄이 겹쳐 쌓이고, 다 끌고 나면 화면이 잘리거나 아래가 빈 채로 남는다.
// 그래서 끄는 동안에는 터미널 크기를 그대로 두고(fitHeld), 손이 멈추면 한 번에 새 크기로 맞춘 뒤 Claude 세션은 화면을 다시
// 맞춘다(realign): Claude Code가 새 줄 수에 맞게 처음부터 다시 그리고, 겹친 줄은 지운다.

const RESIZE_SETTLE_MS = 220;  // 끌던 손이 이만큼 멈추면 화면을 새 크기에 맞춘다
const RESIZE_BOUNCE_MS = 300;  // 새 크기를 알린 뒤 이만큼 있다가 다시 그리게 한다(프로그램이 새 크기를 먼저 알아채게)
const RESIZE_RETRY_MS = 250;   // 다시 맞추는 중이면 이만큼 뒤에 다시 본다
const RESIZE_TRIES = 24;       // 그렇게 기다려 보는 횟수(넘으면 그냥 맞춘다)
let fitHeld = false;           // 참인 동안에는 터미널 크기를 창에 맞추지 않는다
let settleTimer = 0;
let settleTries = 0;

/** 끄는 중: 손이 멈추면 맞추게 걸어 둔다. */
function settleResizeSoon() {
  fitHeld = true;
  clearTimeout(settleTimer);
  settleTries = 0;
  settleTimer = setTimeout(() => settleResize(true), RESIZE_SETTLE_MS);
}

/**
 * 화면에 보이는 터미널을 지금 창 크기에 맞추고, 줄·칸 수가 바뀐 Claude 세션은 화면을 다시 맞춘다.
 * `still`이면 아직 끄는 중이다(다음에 손이 멈출 때까지 다시 크기를 묶어 둔다).
 *
 * - 줄 수만 줄었으면 Claude Code가 스스로 화면을 다시 그린다. 그 뒤의 정리(겹친 줄 지우기, 보던 자리)만 한다.
 * - 줄 수가 늘었거나 칸 수가 바뀌었으면 새 크기를 알린 뒤 조금 있다가 한 줄 줄였다 되돌린다(realign). 바로 이어서 줄이면
 *   프로그램은 두 번의 변경을 "조금 늘어난 것" 한 번으로 보고 다시 그리지 않아, 늘어난 만큼 아래가 빈 채로 남는다.
 * - 앞선 다시 맞추기가 아직 끝나지 않았으면 끝난 뒤에 한다. 겹쳐서 하면 어느 줄이 겹친 줄인지 가려낼 수 없다.
 */
function settleResize(still = false) {
  clearTimeout(settleTimer);
  settleTimer = 0;
  const claudes = visible.filter((s) => isTerm(s) && aliveClaude(s) && s.id != null);
  if (claudes.some((s) => s.realigning || s.settling) && settleTries < RESIZE_TRIES) {
    settleTries += 1;
    fitHeld = true;
    settleTimer = setTimeout(() => settleResize(still), RESIZE_RETRY_MS);
    return;
  }
  settleTries = 0;
  fitHeld = false;
  // 크기를 바꾸기 전의 화면을 적어 둔다(다시 그린 뒤 겹친 줄을 가려내는 데 쓴다).
  const watched = claudes
    .filter((s) => !s.realigning && !s.settling)
    .map((s) => ({ s, cols: s.term.cols, rows: s.term.rows, before: screenBefore(s) }));
  fitVisible();
  for (const { s, cols, rows, before } of watched) {
    if (s.term.cols === cols && s.term.rows === rows) { before.mark?.dispose(); continue; } // 크기가 그대로다
    s.settling = true;
    s.skewed = false;
    s.realignedAt = performance.now();
    if (s.term.cols === cols && s.term.rows < rows) { settleRepaint(s, before); continue; }
    setTimeout(() => {
      s.settling = false;
      if (!sessions.has(s.key)) { before.mark?.dispose(); return; }
      realign(s, before);
    }, RESIZE_BOUNCE_MS);
  }
  fitHeld = still;
}

/**
 * 세션의 화면을 다시 맞춘다: 터미널을 한 줄 줄였다가 되돌려, 세션의 프로그램이 화면을 처음부터 다시 그리게 한다.
 * 다시 그리지 않는 프로그램(셸)이라도 가상 터미널이 자기 화면을 통째로 다시 보내므로 화면 쪽이 어긋난 것은 바로잡힌다.
 */
function realign(s, before = null) {
  const { cols, rows } = s?.term || {};
  if (!s?.term || s.id == null || s.exited || s.realigning || s.settling || rows < 3) { before?.mark?.dispose(); return; }
  s.realigning = true;
  s.settling = true;
  s.skewed = false;
  s.realignedAt = performance.now() + REALIGN_HOLD_MS; // 되돌린 뒤까지 조용한 시간으로 친다
  // 다시 그린 뒤에 겹친 줄을 지우고 보던 자리를 되찾을 수 있게, 지금 화면의 윗줄과 보던 자리를 적어 둔다(settleRepaint).
  // 크기를 바꾼 뒤에 다시 맞추는 경우(settleResize)에는 크기를 바꾸기 전에 적어 둔 것을 받는다.
  before ??= screenBefore(s);
  s.term.resize(cols, rows - 1);
  setTimeout(() => {
    s.realigning = false;
    if (!sessions.has(s.key)) { s.settling = false; before.mark?.dispose(); return; }
    try {
      // 그 사이에 창 크기가 바뀌었으면(터미널이 이미 새 크기다) 그대로 두고, 아니면 원래 줄 수로 되돌린다.
      if (s.term.cols === cols && s.term.rows === rows - 1) {
        // 크기를 묶어 둔 동안(프롬프트 편집기의 높이를 바꾸는 중)에는 원래 줄 수로만 되돌린다. 여기서 새 높이에 맞춰 버리면
        // 뒤따르는 settleResize가 "크기가 그대로"로 보고 넘어가, 새 높이로 다시 그린 것의 겹친 줄을 지우지 못한다.
        if (visible.includes(s) && !fitHeld) { try { s.fitAddon.fit(); } catch { s.term.resize(cols, rows); } } else s.term.resize(cols, rows);
      }
    } finally {
      s.realignedAt = performance.now();
      s.skewed = false;
      settleRepaint(s, before);
    }
  }, REALIGN_HOLD_MS);
}

// ───────────────────────── 아래에 남은 빈 공간 메우기 ─────────────────────────
// Claude Code는 화면 아래쪽의 움직이는 부분(승인 창, 메뉴, 진행 목록)이 길어지면 그만큼 지난 출력을 위로 밀어 올리고, 그 부분이
// 접히면 그 줄들을 지우기만 한다. 터미널은 위로 올라간 줄을 도로 끌어내릴 수 없으므로 입력창 아래가 다음 출력이 채울 때까지
// 빈 채로 남는다(창이 클수록 크다). 그때 화면이 맨 아래를 따라가지 않고 있었으면 입력창은 바닥에 보이는데 스크롤은 그 빈 줄만큼
// 더 내려간다.
// Claude Code는 터미널의 줄 수가 줄면 화면을 맨 위부터 대화의 끝부분으로 다시 채워 그린다. 그래서 쉬고 있는 Claude 세션의
// 아래에 큰 빈 공간이 남아 있으면 화면을 다시 맞춘다(realign). 이때 위로 올라가 있던 줄이 화면에 한 번 더 그려지므로,
// 지난 출력의 끝에 겹쳐 남은 줄은 지운다(dropRepeated).

const GAP_MIN_ROWS = 10;     // 빈 줄이 이만큼은 돼야 메운다(자동 완성 목록을 닫은 정도의 작은 틈으로 화면을 다시 그리지 않는다)
const GAP_SHARE = 0.2;       // 그리고 화면 줄 수의 이 비율은 돼야 한다
const GAP_SETTLE_MS = 800;   // 메울지 살피기 전에 기다리는 시간
const REPAINT_MIN_MS = 500;  // 줄 수를 되돌린 뒤 프로그램이 다시 그릴 때까지 적어도 이만큼 기다린다
const REPAINT_QUIET_MS = 300; // 그 뒤 이만큼 출력이 없으면 다 그린 것으로 본다
const REPAINT_WAIT_MS = 3000; // 그래도 이만큼 지나면 더 기다리지 않는다
const TOP_LINES = 3;         // 화면이 얼마나 내려왔는지 알아볼 때 견주는 윗줄 수
const REPEAT_EXTRA = 6;      // 겹친 덩이를 견줄 때, 다시 그린 쪽에만 있어도 되는 줄 수(줄 수가 줄 때 지난 출력에서 빠졌던 줄)

const lineText = (buffer, y) => buffer.getLine(y)?.translateToString(true) ?? '';

/** 화면(스크롤 맨 아래의 한 화면)에서 커서와 마지막 글자 아래에 남은 빈 줄 수. */
function gapBelow(term) {
  const buffer = term.buffer.active;
  const bottom = buffer.baseY + term.rows - 1;
  let last = bottom;
  while (last > buffer.baseY + buffer.cursorY && !lineText(buffer, last)) last--;
  return bottom - last;
}

const gapLeast = (term) => Math.max(GAP_MIN_ROWS, Math.floor(term.rows * GAP_SHARE));

/** 내용의 끝(커서가 있는 줄)이 지금 보이는가. 맨 아래를 보고 있거나, 아래 빈 공간만큼 위를 보고 있을 때다. */
function seesEnd(term) {
  const buffer = term.buffer.active;
  return buffer.viewportY + term.rows > buffer.baseY + buffer.cursorY;
}

/** 지금 화면의 윗줄 몇 개의 글자. */
function topLines(term) {
  const buffer = term.buffer.active;
  const lines = [];
  for (let i = 0; i < Math.min(TOP_LINES, term.rows); i++) lines.push(lineText(buffer, buffer.baseY + i));
  return lines;
}

/** 조금 뒤에 이 세션의 아래 빈 공간을 메울지 살핀다. 여러 번 불러도 한 번만 살핀다. */
function closeGapSoon(s) {
  if (s.gapTimer || !isTerm(s) || !aliveClaude(s)) return;
  s.gapTimer = setTimeout(() => { s.gapTimer = 0; closeGap(s); }, GAP_SETTLE_MS);
}

/**
 * 쉬고 있는 Claude 세션의 아래에 큰 빈 공간이 남아 있으면 화면을 다시 맞춰 메운다.
 * 일하는 중이거나 승인을 기다리는 중이면(곧 출력이 채우거나 창이 떠 있다), 지난 출력을 올려 보고 있으면, 글자를 골라 둔
 * 상태면 건드리지 않는다. 메워지지 않는 빈 공간(대화가 한 화면보다 짧은 경우)은 한 번만 해 보고 그만둔다.
 */
function closeGap(s) {
  if (!sessions.has(s.key) || !aliveClaude(s) || s.id == null || s.realigning || s.settling) return;
  if (s.workStart || s.heldStop || s.waiting === 'approval') return;
  if (performance.now() - s.lastOutput < BUSY_MS) return; // 조용해지면 다시 살핀다(refreshActivity)
  const buffer = s.term.buffer.active;
  if (buffer.type !== 'normal' || buffer.baseY === 0 || s.term.hasSelection()) return;
  const gap = gapBelow(s.term);
  if (gap < gapLeast(s.term)) { forgetStuck(s); return; }
  if (stillStuck(s, gap)) return;                // 이미 해 봤는데 메워지지 않은 빈 공간이다
  if (!seesEnd(s.term)) return;                  // 지난 출력을 올려 보고 있다. 내려오면 다시 살핀다
  realign(s);
}

/**
 * 다시 맞춘 뒤에도 남은 빈 공간은 메울 수 없는 것이다(대화를 비운 직후처럼 대화가 한 화면보다 짧을 때). 적어 두고,
 * 빈 공간이 더 커지거나 화면의 줄이 위로 더 올라가기 전에는(stillStuck) 다시 해 보지 않는다.
 */
function noteStuck(s) {
  forgetStuck(s);
  const buffer = s.term.buffer.active;
  const gap = gapBelow(s.term);
  if (buffer.type !== 'normal' || gap < gapLeast(s.term)) return;
  const mark = s.term.registerMarker(0); // 커서가 있는 줄. 줄이 위로 올라가면 화면에서의 자리(row)가 줄어든다.
  s.gapStuck = { gap, mark, row: mark ? mark.line - buffer.baseY : 0 };
}

function forgetStuck(s) {
  s.gapStuck?.mark?.dispose();
  s.gapStuck = null;
}

/** 해 봤는데 메워지지 않은 그 빈 공간 그대로인가. 그 뒤로 새 출력이 줄을 위로 밀어 올렸으면 메울 수 있게 됐을 수 있다. */
function stillStuck(s, gap) {
  const stuck = s.gapStuck;
  if (!stuck || gap > stuck.gap || !stuck.mark || stuck.mark.isDisposed) return false;
  return stuck.mark.line - s.term.buffer.active.baseY >= stuck.row;
}

/**
 * 화면을 다시 맞춘 뒤의 정리: 프로그램이 다 그리기를 기다렸다가, 지난 출력의 끝에 겹쳐 남은 줄을 지우고,
 * 내용의 끝을 보고 있었으면 다시 맨 아래를 보게 한다.
 */
function settleRepaint(s, before) {
  const started = performance.now();
  const finish = () => {
    s.settling = false;
    try {
      if (!sessions.has(s.key)) return;
      const buffer = s.term.buffer.active;
      // 기다리는 사이에 사용자가 스크롤을 움직였으면 보는 자리를 건드리지 않는다.
      const stayed = buffer.viewportY === buffer.baseY || Math.abs(buffer.viewportY - before.viewY) <= 1;
      // 겹친 줄은 화면을 다시 채워 그리는 프로그램(Claude Code)에서만 생긴다. 셸의 화면은 그대로 둔다.
      if (runsClaude(s)) dropRepeated(s, before);
      if (before.atEnd && stayed) s.term.scrollToBottom();
      noteStuck(s);
    } finally {
      before.mark?.dispose();
    }
  };
  const wait = () => {
    const now = performance.now();
    if (sessions.has(s.key) && now - started < REPAINT_WAIT_MS && now - s.lastOutput < REPAINT_QUIET_MS) return void setTimeout(wait, 100);
    finish();
  };
  setTimeout(wait, REPAINT_MIN_MS);
}

/**
 * 화면을 다시 그리게 한 뒤, 지난 출력(스크롤 위쪽)에 화면 윗부분과 똑같이 겹쳐 남은 줄을 지운다.
 *
 * 프로그램이 화면을 맨 위부터 다시 채워 그리면 내용이 아래로 내려오면서, 이미 위로 올라가 있던 줄이 화면 위쪽에 한 번 더
 * 그려진다. 몇 줄이 겹쳤는지는 다시 그리기 전 화면의 윗줄(before.top)이 그 자리(before.mark)에서 몇 줄 아래로 내려와 있는지로
 * 안다. 겹친 두 덩이는 다시 그릴 때 화면이 시작되던 줄을 사이에 두고 위아래로 붙어 있으므로, 그 줄 수만큼의 위아래 덩이가
 * 글자까지 똑같은 곳을 찾아 위쪽(지난 출력 쪽) 덩이를 지운다. 그런 곳이 없으면 아무것도 지우지 않는다.
 * 자리를 마커로 잡으므로 다시 그리는 사이에 새 출력이 와서 줄이 더 올라가도 맞는다. 지운 줄 수를 돌려준다.
 */
function dropRepeated(s, before) {
  const { term } = s;
  const buffer = term.buffer.active;
  if (buffer.type !== 'normal' || !before.mark || before.mark.isDisposed || !before.top.some(Boolean)) return 0;
  // 글자를 골라 둔 동안에는 지우지 않는다(xterm.js가 고른 자리를 따라 옮겨 주지 않는다).
  if (term.hasSelection()) return 0;
  const from = before.mark.line; // 다시 그리기 전에 화면 맨 윗줄이던 자리
  const base = buffer.baseY;
  const most = Math.min(term.rows, from);
  const topAt = (y) => before.top.every((text, i) => lineText(buffer, y + i) === text);
  // 윗줄이 내려온 줄 수: 빈 공간이 줄어든 만큼을 먼저 본다(똑같은 줄이 되풀이되는 출력에서는 가까운 데부터 찾으면 0줄로 잡힌다).
  // 그사이 줄이 위로 올라갔으면(새 출력이 왔다) 빈 공간이 줄어든 까닭이 그것일 수 있으니 이 짐작은 쓰지 않는다.
  let moved = 0;
  const filled = base === from ? before.gap - gapBelow(term) : 0;
  if (filled > 0 && filled <= most && topAt(from + filled)) moved = filled;
  else for (let d = 0; d <= most; d++) if (topAt(from + d)) { moved = d; break; }
  if (moved <= 0) return 0;
  const same = (a, b) => {
    for (let i = 0; i < moved; i++) if (lineText(buffer, a + i) !== lineText(buffer, b + i)) return false;
    return true;
  };
  let edge = -1; // 겹친 두 덩이 사이: 다시 그릴 때 화면이 시작되던 줄
  let count = moved; // 지울 줄 수
  for (let y = from; y <= Math.min(from + moved, base) && edge < 0; y++) {
    if (y - moved >= 0 && same(y - moved, y)) edge = y;
  }
  // 똑같은 덩이가 없으면: 다시 그린 쪽에만 있는 줄이 몇 줄 끼어 있는 경우를 본다. 프로그램은 줄 수가 줄 때마다 화면의 끝부분만
  // 다시 그려서 그때 화면 맨 위에 있던 줄이 지난 출력에서 빠지는데, 나중에 넓어져 다시 그리면 그 줄이 되살아난다. 그러면 위쪽 덩이는
  // 아래쪽 덩이에서 그 몇 줄이 빠진 모양이다. 아래에서부터 차례로 맞춰 보아, 아래쪽에만 있는 줄이 몇 줄(REPEAT_EXTRA) 이하이고
  // 위쪽의 줄이 모두 차례대로 맞으면 위쪽의 맞은 줄들을 지운다(지우는 줄은 모두 아래쪽에 같은 차례로 그대로 있다).
  for (let y = from; y <= Math.min(from + moved, base) && edge < 0; y++) {
    let upper = y - 1, lower = y + moved - 1, extra = 0;
    while (lower >= y && upper >= 0 && extra <= REPEAT_EXTRA) {
      if (lineText(buffer, upper) === lineText(buffer, lower)) upper -= 1; else extra += 1;
      lower -= 1;
    }
    const matched = y - 1 - upper;
    // 맞은 줄이 적으면(빈 줄 하나가 우연히 맞은 것일 수 있다) 겹친 것으로 보지 않는다: 세 줄 이상, 끼어든 줄보다 많이,
    // 그리고 글자가 있는 줄이 하나는 맞아야 한다.
    let written = 0;
    for (let i = upper + 1; i < y; i++) if (lineText(buffer, i).trim()) written += 1;
    if (lower < y && extra > 0 && extra <= REPEAT_EXTRA && matched === moved - extra && matched >= Math.max(3, extra) && written > 0) { edge = y; count = matched; }
  }
  if (edge < 0) return 0;
  // xterm.js에는 지난 출력의 가운데 줄을 지우는 공개 기능이 없어 안쪽 버퍼를 직접 고친다(같이 넣어 둔 판에 맞춘 것).
  const inner = term._core?.buffers?.normal;
  if (!inner?.lines?.splice || inner.ybase !== base) return 0;
  const start = edge - count;
  const shift = (y) => (y >= edge ? y - count : Math.min(y, start)); // 지운 줄 아래는 그만큼 올라오고, 지운 줄 안은 그 자리로
  inner.lines.splice(start, count);
  inner.ybase -= count;
  inner.ydisp = shift(inner.ydisp);
  inner.savedY = shift(inner.savedY);
  term._core.viewport?.syncScrollArea(true);
  term.refresh(0, term.rows - 1);
  return count;
}

const sizeKey = ({ cols, rows }) => `${cols}x${rows}`;

/**
 * 터미널의 지금 크기를 세션의 가상 터미널에 알린다. 둘의 크기가 다르면 화면이 어긋난다(가상 터미널은 화면을
 * 줄바꿈으로 이어 그려 보내므로, 줄 수가 다르면 그만큼 밀려 찍힌다).
 * 한 번에 하나씩 보내고, 보내는 사이에 또 바뀌었으면 마지막 크기를 이어서 보낸다(늦게 보낸 것이 먼저 닿아 옛 크기로 남지 않게).
 * `again`이면 이미 알린 크기라도 다시 알린다.
 */
function syncSize(s, again = false) {
  if (!s.term || s.id == null || s.exited) return;
  if (again) s.ptySize = '';
  if (s.sizing) return;
  const size = { cols: s.term.cols, rows: s.term.rows };
  if (s.ptySize === sizeKey(size)) return;
  const id = s.id;
  s.sizing = true;
  invoke('session_resize', { id, ...size }).then(
    () => { s.sizing = false; if (s.id === id) { s.ptySize = sizeKey(size); syncSize(s); } },
    () => { s.sizing = false; }, // 끝난 세션 등. 다음 크기 변경 때 다시 알린다.
  );
}

/**
 * 앱이 직접 만드는 키 신호(Shift+Enter의 줄바꿈 등)를 보낸다. 조합 중이던 글자(한글의 마지막 글자)가 있으면 그 글자부터 보낸다.
 *
 * 한글을 조합하다가 다른 키를 누르면 입력기는 "글자 확정 → 그 키" 순서로 알려 주는데, xterm.js는 확정된 글자를 바로 보내지 않고
 * 한 박자 뒤(setTimeout 0)에 보낸다. 그 사이에 여기서 신호를 먼저 보내면 순서가 뒤집혀, "안녕" 뒤에 Shift+Enter를 누를 때
 * 줄바꿈이 먼저 가고 "녕"이 다음 줄에 찍힌다. xterm.js가 스스로 다루는 키(Enter 등)는 보내기 전에 조합을 마무리하므로,
 * 여기서도 같은 일을 먼저 한다.
 */
function sendAfterComposition(s, data) {
  // xterm.js에 조합을 지금 마무리하게 하는 공개 기능이 없어 안쪽 것을 직접 부른다(같이 넣어 둔 판에 맞춘 것).
  const helper = s.term?._core?._compositionHelper;
  if (typeof helper?._finalizeComposition !== 'function') {
    // 안쪽 구조가 달라졌으면 한 박자 미뤄 보낸다(xterm.js가 먼저 걸어 둔 "확정 글자 보내기" 뒤에 나간다).
    setTimeout(() => send(s, data), 0);
    return;
  }
  if (helper._isComposing || helper._isSendingComposition) helper._finalizeComposition(false);
  send(s, data);
}

function wireTerminal(s) {
  const { term } = s;
  term.onData((data) => send(s, data));
  term.onResize(() => syncSize(s));
  // 이 터미널로 돌아올 때 크기를 한 번 더 알린다. 세션 안의 프로그램이 콘솔 크기를 바꿔 놓았어도(`mode con …`) 제 크기로 돌아오고,
  // 가상 터미널이 화면을 통째로 다시 그려 보내므로 어긋나 있던 화면도 바로잡힌다.
  term.textarea?.addEventListener('focus', () => syncSize(s, true));
  term.onTitleChange((title) => { s.title = title; render(); });
  term.onBell(() => { if (s !== active) { s.attention = true; render(); } });
  term.textarea?.addEventListener('focus', () => { s.focusedAt = performance.now(); });
  // 지난 출력을 보다가 내용의 끝으로 내려왔을 때, 그 아래에 빈 공간이 남아 있으면 메운다.
  s.pane.querySelector('.xterm-viewport')?.addEventListener('scroll', () => closeGapSoon(s), { passive: true });

  // 셸이 알려 주는 현재 폴더(OSC 9;9 경로, OSC 7 file://…)를 따라간다.
  term.parser.registerOscHandler(9, (data) => {
    if (!data.startsWith('9;')) return false;
    s.cwd = data.slice(2).replace(/^"|"$/g, '');
    // 셸이 프롬프트를 다시 그렸다: 이 셸에서 돌던 Claude는 끝났다. 끝 신호 없이 끝난 경우(강제 종료, 충돌)에도
    // 그 줄이 "Claude 실행 중"으로 남지 않게 여기서 정리한다.
    if (s.claudeLive) endClaude(s); else render();
    return true;
  });
  term.parser.registerOscHandler(7, (data) => {
    const m = /^file:\/\/[^/]*\/(.+)$/.exec(data);
    if (m) {
      let dir = m[1];
      // 인코딩하지 않은 경로(이름에 %가 든 폴더)는 그대로 쓴다. 여기서 예외가 나면 터미널의 출력 처리가 그 자리에서 멈춘다.
      try { dir = decodeURIComponent(dir); } catch { /* 그대로 */ }
      s.cwd = dir.replace(/\//g, '\\');
      render();
    }
    return true;
  });

  term.attachCustomKeyEventHandler((ev) => {
    const key = ev.key.toLowerCase();
    // Shift+Enter는 줄바꿈으로 전달한다(Claude Code 여러 줄 입력).
    if (key === 'enter' && ev.shiftKey && !ev.ctrlKey && !ev.altKey) {
      if (ev.type === 'keydown') sendAfterComposition(s, '\x1b\r');
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

  // 끌어서 고르면 바로 복사한다(설정 → "끌어서 고른 글 바로 복사"로 끈다). 마우스를 누른 뒤 고른 것이 바뀌었을 때만 복사한다:
  // 스크롤 막대를 끌거나 창을 누르기만 한 것으로 예전에 고른 글이 다시 클립보드를 덮지 않게.
  // Claude Code가 마우스를 받는 모드에서는 Claude Code가 스스로 고르고 복사한다(터미널의 고르기는 Shift+끌기).
  term.onSelectionChange(() => { if (picking?.s === s) picking.changed = true; });
  s.pane.addEventListener('mousedown', (ev) => {
    if (ev.button === 0 && ev.target.closest('.term')) picking = { s, changed: false, before: term.getSelection() };
    // Claude Code가 마우스를 받고 있으면(전체 화면 모드) 오른쪽 클릭을 넘겨주지 않는다. 넘겨주면 Claude Code도 클립보드를
    // 붙여넣어(Windows에서 오른쪽 클릭 = 붙여넣기), 아래의 붙여넣기와 겹쳐 같은 글이 두 번 들어간다.
    if (ev.button === 2 && swallowsRightClick(s, ev)) ev.stopPropagation();
  }, true);
  s.pane.addEventListener('mouseup', (ev) => { if (ev.button === 2 && swallowsRightClick(s, ev)) ev.stopPropagation(); }, true);
  // 오른쪽 단추를 누른 채 움직이는 신호도 넘기지 않는다(모든 움직임을 받는 모드).
  s.pane.addEventListener('mousemove', (ev) => { if (ev.buttons === 2 && swallowsRightClick(s, ev)) ev.stopPropagation(); }, true);

  // SSH 세션: Ctrl+휠로 글자 크기를 바꾼다(SSH 세션에만 적용).
  if (s.ssh) {
    s.pane.addEventListener('wheel', (ev) => {
      if (!ev.ctrlKey || !ev.deltaY || !ev.target.closest('.term')) return;
      ev.preventDefault();
      ev.stopPropagation();
      setSshFont(sshFontNow() + (ev.deltaY < 0 ? 1 : -1));
    }, { capture: true, passive: false });
  }

  // 오른쪽 클릭: 선택 영역이 있으면 복사, 없으면 붙여넣기. xterm.js에 닿기 전에 받아 그쪽의 처리를 건너뛴다(아래 설명).
  s.pane.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    // 터미널 영역에서만 한다. 나란히 보기의 창 제목 줄을 오른쪽 클릭했다고 클립보드를 붙여넣으면 안 된다(SSH 탭의 메뉴는 그대로 뜬다).
    if (!ev.target.closest('.term')) return;
    ev.stopPropagation();
    term.focus();
    if (term.hasSelection()) copy(s); else paste(s, false);
    // xterm.js는 오른쪽 클릭 때 숨은 입력칸에 고른 글을 채우고 통째로 골라 둔다(브라우저 메뉴의 "복사"용인데, 그 메뉴는 쓰지 않는다).
    // 그대로 두면 바로 뒤에 치는 한글이 그 글을 덮어쓰면서 조합 자리가 어긋나, 첫 글자가 사라지고 다음 글자의 첫 자음이 먼저 나간다
    // ("좋습니다" → "ㅅ습니다"). 다른 창에 다녀오면(입력칸이 비워진다) 풀리던 문제다. 그래서 이 신호를 xterm.js에 넘기지 않고
    // (글자를 조합하는 중에 오른쪽 클릭해도 입력칸을 건드리지 않는다), 혹시 남아 있는 글도 여기서 비운다.
    clearHiddenInput(term);
  }, true);

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

/** 터미널의 숨은 입력칸(xterm.js가 키 입력을 받는 textarea)을 비운다. 글자를 조합하는 중에는 건드리지 않는다. */
function clearHiddenInput(term) {
  const input = term.textarea;
  const helper = term._core?._compositionHelper;
  if (!input || !input.value || helper?._isComposing || helper?._isSendingComposition) return;
  input.value = '';
}

/**
 * 이 세션에서 오른쪽 클릭을 세션 안의 프로그램에 넘기지 않을지: 이 컴퓨터에서 도는 Claude Code가 마우스를 받고 있을 때.
 * (Shift를 누른 클릭은 원래 프로그램에 가지 않는다. 다른 프로그램 — vim·tmux의 마우스 모드 등 — 에는 지금까지처럼 넘긴다.)
 */
const swallowsRightClick = (s, ev) => !ev.shiftKey && !!ev.target.closest?.('.term') && runsClaude(s) && !s.ssh
  && s.term.modes.mouseTrackingMode !== 'none';

// 끌어서 고른 글 바로 복사: 기본으로 켜 둔다. 사용자가 끈 것('0')만 끈 채로 둔다.
let copyOnSelect = localStorage.getItem('cypher.copyOnSelect') !== '0';
/** 터미널에서 마우스 왼쪽 단추를 누르고 있는 동안: { s, changed(그사이 고른 것이 바뀌었는지), before(누르기 전에 고른 글) } */
let picking = null;

function setCopyOnSelect(on) {
  copyOnSelect = !!on;
  try { localStorage.setItem('cypher.copyOnSelect', copyOnSelect ? '1' : '0'); } catch { /* 저장 공간 문제는 무시 */ }
  setStatus(copyOnSelect
    ? '끌어서 고른 글 바로 복사: 켬 — 터미널(Claude·셸·SSH)에서 끌어 고르면 바로 클립보드에 복사합니다.'
    : '끌어서 고른 글 바로 복사: 끔 — 고른 뒤 Ctrl+C나 오른쪽 클릭으로 복사합니다.');
}

/** 마우스를 놓았다: 터미널에서 끌어(또는 두 번·세 번 눌러) 고른 글이 있으면 클립보드에 복사한다. 고른 표시는 그대로 둔다. */
function copyPicked() {
  const held = picking;
  if (!held) return;
  // xterm.js가 놓는 신호를 받아 고른 범위를 마무리한 뒤에(고른 것이 바뀌었다는 알림도 그때 온다) 읽는다.
  setTimeout(() => {
    if (picking === held) picking = null;
    if (!copyOnSelect || !sessions.has(held.s.key)) return;
    const text = held.s.term.getSelection();
    // 누르기 전과 고른 것이 그대로면(스크롤 막대를 끌었다 등) 복사하지 않는다.
    if (!text || (!held.changed && text === held.before)) return;
    invoke('clipboard_write', { text })
      .then(() => setStatus(`복사했습니다 (${text.length.toLocaleString('ko-KR')}자) · 붙여넣기: Ctrl+V나 오른쪽 클릭`))
      .catch((err) => setStatus(`복사 실패: ${err}`));
  }, 0);
}
window.addEventListener('mousedown', () => { picking = null; }, true); // 놓는 신호를 받지 못한 앞의 누름은 잊는다(터미널을 누른 것이면 바로 뒤에 다시 적는다)
window.addEventListener('mouseup', (ev) => { if (ev.button === 0) copyPicked(); }, true);
window.addEventListener('blur', () => { picking = null; });

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
    if (runsClaude(s) && !forceFile) send(s, '\x1bv');
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
  const key = ev.key.toLowerCase();
  if ($('ask').open || $('popup').open) {
    // 창이 열려 있는 동안에는 앱 단축키를 듣지 않는다. 새로 고침만은 막는다(본창이 새로 고쳐지면 세션 화면이 날아간다).
    if (key === 'f5' || (ev.ctrlKey && key === 'r')) ev.preventDefault();
    return;
  }
  const combo = ev.ctrlKey && ev.shiftKey && !ev.altKey;
  // 북마크: Ctrl+Shift+숫자로 지금 항목에 숫자를 붙이고, Ctrl+숫자로 그 항목에 간다.
  // Shift를 누르면 글쇠가 내는 글자가 바뀌므로(1 → !) 글쇠의 자리로 본다.
  // 숫자판은 NumLock이 꺼져 있으면 Home·End·방향 글쇠다(글쇠의 자리는 그대로 Numpad7·Numpad1…). 그때는 숫자로 보지 않는다.
  const digit = /^Digit(\d)$/.exec(ev.code)?.[1] ?? (/^\d$/.test(ev.key) ? /^Numpad(\d)$/.exec(ev.code)?.[1] : undefined);
  let handled = true;
  if (digit && combo) setMark(active, digit);
  // 그 숫자의 북마크가 없으면 가로채지 않는다(Ctrl+0은 문서·ERD·마인드맵·메모에서 기본 크기·전체 맞춤이기도 하다).
  else if (digit && ev.ctrlKey && !ev.shiftKey && !ev.altKey && markedItem(digit)) jumpToMark(digit);
  // 마지막에 연 것이 SSH 세션이고 지금 SSH 세션을 보고 있으면 그 연결을 복제한다(아니면 접속 창이 뜬다).
  // Ctrl+Shift+T·D: 새 세션을 지금 창 자리에 연다(화면은 나누지 않는다). SSH 세션을 보고 있으면 그 연결을 옆 탭으로 복제한다.
  else if (combo && (key === 't' || key === 'd')) { if (lastProfile === SSH && active?.ssh) cloneSsh(active); else newSession(lastProfile); }
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
  else if (combo && key === 'a') showNotices();
  else if (key === 'f1') showHelp();
  // 창을 나눠 보고 있을 때만 가로챈다. 하나만 보고 있으면 옮길 창이 없으니 낱말 단위 선택(편집기·터미널)으로 넘긴다.
  else if (combo && key === 'arrowright' && visible.length > 1) focusPane(1);
  else if (combo && key === 'arrowleft' && visible.length > 1) focusPane(-1);
  else if (ev.ctrlKey && key === 'tab') cycle(ev.shiftKey ? -1 : 1);
  // 터미널 밖에서 새로 고침이 눌리면 세션 화면이 날아가므로 막는다.
  // (메모에서는 F5가 시간/날짜, Ctrl+R이 바꾸기라서 메모가 직접 처리한다.)
  else if ((key === 'f5' || (ev.ctrlKey && key === 'r')) && !ev.target.closest?.('.term, .memo')) {
    // 본창을 새로 읽는 것은 막는다. S3 저장소 창에서 누른 F5는 그 폴더의 목록을 다시 읽으라는 것이다.
    const pane = ev.target.closest?.('.pane');
    const item = pane && key === 'f5' ? [...sessions.values()].find((s) => s.pane === pane) : null;
    if (item?.kind === 's3') item.view.refresh();
  }
  else handled = false;
  if (handled) { ev.preventDefault(); ev.stopPropagation(); }
}, true);

document.addEventListener('contextmenu', (ev) => ev.preventDefault());

// 자동 검증과 문제 확인용 창구.
window.cypher = {
  sessions, newSession, newItem, activate, closeSession, toggleShown, send, setBroadcast, provideLinks, paste, moveToGroup, runMenu, showOnly,
  onHook, onStatus, nextWaiting, writeState, rename, sendPrompt, text: sessionText, addImages, onCaptured, captureBrowser,
  setLook, currentLook, mcpAction, handoffSite, onMcpRequest, setMark, jumpToMark, markedItem, textToClaude, runFinished,
  showPalette, paletteEntries, showClosed, reopenLast, restartSession, placeInOrder, inSidebarOrder, cycle, find, recallPrompt, createGroup, newGroup, adoptGroup, groupFor,
  showTimers, addTimer, removeTimer, timers, nextTimer, onCreatedFile, placeHeld, cloneSsh, showSsh, castTargets, saveHost, savedHosts, onEditedFile, onTool, realign, settleResize, settleResizeSoon, moveGroup, nudgeGroup, closeGap, gapBelow, wake, wakeGroup, restore, runsClaude, askClaudeCommand, setClaudeCommand, setGroupFolder, groupFolderOf, openGroupFolder, groupContext, canControl, profileOf, ensureProfile, renameGroup, deleteGroup, createKind, showNotices, unreadCount, logNotice, notice, showTemplates, saveTemplate, currentGroup, packGroup, openTemplate, groupsInOrder, toggleDiff,
  startLog, stopLog, showLog, setSshFont, sshFontNow, setCopyOnSelect, syncSshScopes, clearHiddenInput,
  rooms, sshOf, roomShown, roomFront, openRoom, toggleRoom, showRoomOnly, closeRoom, addSshTab, tabsOf, sidebarRows,
  showHistory, historyRecord, restoreHistory,
  get tree() { return tree; },
  get copyOnSelect() { return copyOnSelect; },
  get visible() { return visible; },
  get broadcast() { return broadcast; },
  get active() { return active; },
};

init();
