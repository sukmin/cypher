// 부가 화면: 떠 있는 창(자주 쓰는 지시·기억·도움말), 오른쪽 클릭 메뉴, 변경 내용 옆 창.
// main.js가 initExtras()로 필요한 함수들을 넘겨준다.

import { httpOpen, toggleHttp } from './http.js';

const $ = (id) => document.getElementById(id);
let api = null; // { invoke, make, ask, setStatus, getActive, getFolder, newSession, pasteText, label }

export function initExtras(given) {
  api = given;
  $('popup-close').onclick = () => $('popup').close();
  $('diff-close').onclick = () => toggleDiff(false);
  $('diff-refresh').onclick = () => loadDiff();
  document.addEventListener('mousedown', (ev) => {
    if (!$('menu').hidden && !$('menu').contains(ev.target)) hideMenu();
  }, true);
  window.addEventListener('blur', hideMenu);
}

// ───────────────────────── 떠 있는 창 ─────────────────────────

/** 제목과 내용으로 창을 띄운다. 닫는 함수를 돌려준다. */
export function openPopup(title, nodes) {
  const dialog = $('popup');
  $('popup-title').textContent = title;
  $('popup-body').replaceChildren(...nodes);
  dialog.onclose = () => api.getActive()?.term.focus();
  if (!dialog.open) dialog.showModal();
  return () => dialog.close();
}


// ───────────────────────── 자주 쓰는 지시 ─────────────────────────

const SNIPPET_KEY = 'cypher.snippets';

function loadSnippets() {
  try { return JSON.parse(localStorage.getItem(SNIPPET_KEY) || '[]'); } catch { return []; }
}

function saveSnippets(list) {
  localStorage.setItem(SNIPPET_KEY, JSON.stringify(list));
}

/** 저장해 둔 지시를 보여 준다. 누르면 활성 세션 입력창에 넣는다(보내지는 않는다). */
export function showSnippets() {
  const { make } = api;
  const list = loadSnippets();
  const rows = list.map((text, index) => {
    const row = make('div', 'row');
    const use = make('button', 'row-title', text);
    use.title = '누르면 활성 세션 입력창에 넣습니다';
    use.onclick = () => { close(); api.pasteText(text); };
    const remove = make('button', 'close', '×');
    remove.title = '지우기';
    remove.onclick = () => { list.splice(index, 1); saveSnippets(list); showSnippets(); };
    row.append(use, remove);
    return row;
  });
  const add = make('button', 'row add', '+ 새 지시 저장');
  add.onclick = async () => {
    close();
    const text = await api.ask('저장할 지시', '예: 시험 돌리고 실패한 것만 고쳐줘');
    if (text) { list.push(text); saveSnippets(list); }
    showSnippets();
  };
  const note = make('p', 'note', list.length ? '' : '자주 쓰는 지시를 저장해 두면 클릭 한 번으로 입력합니다.');
  const close = openPopup('자주 쓰는 지시', [...rows, note, add]);
}

// ───────────────────────── 기억 내보내기·가져오기 ─────────────────────────

/**
 * Claude Code가 폴더별로 쌓아 둔 기억 파일을 다른 폴더로 내보내거나, 다른 폴더에서 가져온다.
 * 대상은 활성 Claude 세션의 폴더, 없으면 새 세션 폴더.
 */
export async function showMemory(session) {
  const { invoke, make, setStatus } = api;
  const s = session || api.getActive();
  // 대상은 클릭한(활성) 세션의 폴더. 세션이 없으면 새 세션 폴더.
  const cwd = s ? (s.project || s.startCwd) : api.getFolder();
  let close = () => {};
  const run = async (command, title, done) => {
    const picked = await invoke('pick_folder', { start: null, title });
    if (!picked) return;
    try {
      const args = command === 'memory_export' ? { cwd, dest: picked } : { cwd, source: picked };
      setStatus(done(await invoke(command, args)));
    } catch (err) {
      setStatus(`기억 ${command === 'memory_export' ? '내보내기' : '가져오기'} 실패: ${err}`);
    }
    close();
    showMemory(s);
  };
  try {
    const info = await invoke('memory_info', { cwd });
    const exportButton = make('button', 'row add', '내보내기… (이 폴더의 기억을 고른 폴더로 복사)');
    exportButton.disabled = !info.files;
    exportButton.onclick = () => run('memory_export', '기억을 내보낼 폴더',
      (r) => `기억 파일 ${r.copied}개를 내보냈습니다: ${r.target}`);
    const importButton = make('button', 'row add', '가져오기… (고른 폴더의 기억을 이 폴더로 복사)');
    importButton.onclick = () => run('memory_import', '가져올 기억이 있는 폴더',
      (r) => `기억 파일 ${r.copied}개를 가져왔습니다.${r.backup ? ` 기존 기억은 백업했습니다: ${r.backup}` : ''} 새 세션부터 적용됩니다.`);
    const openButton = make('button', 'row add', '기억 폴더 열기');
    openButton.onclick = () => invoke('memory_open', { cwd }).catch((err) => setStatus(String(err)));
    close = openPopup('Claude 기억 내보내기·가져오기', [
      make('p', 'note', `대상 폴더: ${cwd}`),
      make('p', 'note', `기억 위치: ${info.dir}`),
      make('p', 'note', info.files ? `기억 파일 ${info.files}개` : '아직 쌓인 기억이 없습니다.'),
      exportButton, importButton, openButton,
      make('p', 'note', '가져올 때 기존 기억은 먼저 백업하고, 색인(MEMORY.md)은 합칩니다. 이름이 같은 파일은 가져온 것으로 바뀝니다.'),
    ]);
  } catch (err) {
    setStatus(String(err));
  }
}

const HELP = [
  ['Ctrl+Shift+T', '새 세션 (지금 창 자리에)'],
  ['Ctrl+Shift+D', '새 세션을 나란히 열기'],
  ['Ctrl+Shift+W', '세션 닫기'],
  ['Ctrl+Shift+X', '나란히 보기에서 빼기 (세션은 유지)'],
  ['Ctrl+Tab', '다음 세션 (Shift: 이전)'],
  ['Ctrl+Shift+←/→', '나란히 보이는 창 사이 이동'],
  ['Ctrl+Shift+N', '다음 대기 세션으로 이동'],
  ['Ctrl+Shift+F', '화면에서 찾기'],
  ['Ctrl+Shift+G', '변경 내용 옆 창'],
  ['Ctrl+Shift+P', '자주 쓰는 지시'],
  ['Ctrl+Shift+O', '내장 브라우저 열기'],
  ['Ctrl+Shift+H', 'HTTP 요청 옆 창'],
  ['Ctrl+Shift+B', '동시 입력 (나란히 보이는 모든 세션)'],
  ['Ctrl+V', '붙여넣기 (이미지 포함) · Ctrl+Shift+V: 이미지를 파일 경로로'],
  ['Ctrl+C', '선택 영역이 있으면 복사'],
  ['Shift+Enter', '줄바꿈'],
  ['링크 클릭', '웹 주소는 브라우저, 경로는 탐색기 · Ctrl+클릭: 파일 열기'],
  ['목록에서 Ctrl+클릭', '나란히 보기에 넣기/빼기'],
  ['목록에서 오른쪽 클릭', '이름·색 바꾸기, 출력 저장 등'],
];

export function showHelp() {
  const { make } = api;
  openPopup('단축키', HELP.map(([key, what]) => {
    const row = make('div', 'row help');
    row.append(make('kbd', '', key), make('span', '', what));
    return row;
  }));
}

// ───────────────────────── 오른쪽 클릭 메뉴 ─────────────────────────

/** items: { label, action } 또는 { colors, action(color) } */
export function showMenu(ev, items) {
  const { make } = api;
  const menu = $('menu');
  menu.replaceChildren(...items.map((item) => {
    if (item.colors) {
      const row = make('div', 'swatches');
      for (const color of item.colors) {
        const swatch = make('button', 'swatch');
        swatch.style.background = color || 'transparent';
        swatch.title = color ? '이 색으로 표시' : '색 없음';
        if (!color) swatch.textContent = '×';
        swatch.onclick = () => { hideMenu(); item.action(color); };
        row.append(swatch);
      }
      return row;
    }
    const button = make('button', '', item.label);
    button.onclick = () => { hideMenu(); item.action(); };
    return button;
  }));
  menu.hidden = false;
  const { width, height } = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(ev.clientX, innerWidth - width - 4)}px`;
  menu.style.top = `${Math.min(ev.clientY, innerHeight - height - 4)}px`;
}

function hideMenu() {
  $('menu').hidden = true;
}

// ───────────────────────── 변경 내용 옆 창 ─────────────────────────

let diffSession = null;
let diffRoot = '';
let diffSelected = '';

export const diffOpen = () => !$('diff').hidden;

/** 옆 창을 열거나 닫는다. 열면 활성 세션 폴더의 변경 내용을 보여 준다. */
export function toggleDiff(open = !diffOpen()) {
  if (open && httpOpen()) toggleHttp(false); // 옆 창은 한 번에 하나만
  $('diff').hidden = !open;
  $('changes').classList.toggle('on', open);
  if (open) showDiff(api.getActive());
  else api.getActive()?.term.focus();
}

/** 옆 창이 열려 있으면 이 세션의 변경 내용으로 바꾼다. */
export function showDiff(s) {
  if (!diffOpen() || !s) return;
  if (diffSession !== s) diffSelected = '';
  diffSession = s;
  loadDiff();
}

/** 이 세션의 작업이 끝났을 때 옆 창을 새로 읽는다. */
export function diffRefreshFor(s) {
  if (diffOpen() && s === diffSession) loadDiff();
}

async function loadDiff() {
  const { invoke, make } = api;
  const s = diffSession;
  if (!s) return;
  const $files = $('diff-files'), $view = $('diff-view');
  try {
    const changes = await invoke('git_changes', { cwd: s.cwd });
    if (s !== diffSession) return; // 읽는 사이 다른 세션으로 바뀌었다.
    diffRoot = changes.root;
    $('diff-title').textContent =
      `변경 내용 — ${api.label(s)}${changes.branch ? ` (${changes.branch})` : ''} · ${changes.files.length}개`;
    $files.replaceChildren(...changes.files.map((file) => {
      const row = make('li', file.path === diffSelected ? 'selected' : '');
      row.append(make('span', `status ${file.untracked ? 'new' : ''}`, file.status), make('span', 'path', file.path));
      row.title = file.path;
      row.onclick = () => selectFile(file);
      row.ondblclick = () => invoke('link_open', {
        target: `${diffRoot}/${file.path}`, cwd: diffRoot, openFile: true, line: null, col: null,
      }).catch((err) => api.setStatus(String(err)));
      return row;
    }));
    const current = changes.files.find((f) => f.path === diffSelected) || changes.files[0];
    if (current) selectFile(current);
    else $view.replaceChildren(make('p', 'note', '바뀐 파일이 없습니다.'));
  } catch (err) {
    $('diff-title').textContent = '변경 내용';
    $files.replaceChildren();
    $view.replaceChildren(make('p', 'note', String(err)));
  }
}

async function selectFile(file) {
  const { invoke, make } = api;
  diffSelected = file.path;
  for (const row of $('diff-files').children) row.classList.toggle('selected', row.title === file.path);
  const $view = $('diff-view');
  try {
    const text = await invoke('git_diff', { root: diffRoot, path: file.path, untracked: file.untracked });
    if (diffSelected !== file.path) return;
    const lines = text.split('\n').slice(0, 6000).map((line) => {
      const kind = line.startsWith('@@') ? 'hunk'
        : /^(diff |index |--- |\+\+\+ |new file|deleted file|similarity|rename )/.test(line) ? 'meta'
        : line.startsWith('+') ? 'add' : line.startsWith('-') ? 'del' : '';
      return make('div', kind, line || ' ');
    });
    $view.replaceChildren(...(text.trim() ? lines : [make('p', 'note', '차이가 없습니다(이진 파일이거나 권한만 바뀜).')]));
    $view.scrollTop = 0;
  } catch (err) {
    $view.replaceChildren(make('p', 'note', String(err)));
  }
}
