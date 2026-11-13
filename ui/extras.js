// 부가 화면: 떠 있는 창(자주 쓰는 지시·기억·도움말), 오른쪽 클릭 메뉴, 변경 내용 옆 창.
// main.js가 initExtras()로 필요한 함수들을 넘겨준다.

const $ = (id) => document.getElementById(id);
let api = null; // { invoke, make, ask, setStatus, getActive, getFolder, focusActive, newSession, pasteText, label }

export function initExtras(given) {
  api = given;
  $('popup-close').onclick = () => $('popup').close();
  $('diff-close').onclick = () => toggleDiff(false);
  $('diff-refresh').onclick = () => loadDiff();
  document.addEventListener('mousedown', (ev) => {
    if (!$('menu').hidden && !$('menu').contains(ev.target)) hideMenu();
  }, true);
  // 메뉴를 눌러도 초점은 하던 곳(터미널, 그림 등)에 그대로 둔다. 고른 뒤 바로 키보드로 이어서 쓸 수 있다.
  $('menu').addEventListener('mousedown', (ev) => ev.preventDefault());
  // Esc: 메뉴 닫기.
  document.addEventListener('keydown', (ev) => {
    if (ev.key !== 'Escape' || $('menu').hidden) return;
    ev.preventDefault();
    ev.stopPropagation();
    hideMenu();
  }, true);
  window.addEventListener('blur', hideMenu);
}

// ───────────────────────── 떠 있는 창 ─────────────────────────

/** 제목과 내용으로 창을 띄운다. 닫는 함수를 돌려준다. */
export function openPopup(title, nodes) {
  const dialog = $('popup');
  $('popup-title').textContent = title;
  $('popup-body').replaceChildren(...nodes);
  dialog.onclose = () => api.focusActive();
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
  ['Ctrl+Shift+K', '빠른 이동: 항목·명령을 글자로 찾아 가기 (초성도 됩니다: ㄱㅈ → 결제 · 열자마자 Enter: 바로 전에 보던 항목)'],
  ['Ctrl+Shift+R', '가장 나중에 닫은 항목 다시 열기 (목록은 도구 → 닫은 항목 다시 열기)'],
  ['Ctrl+Shift+F', '화면에서 찾기 (치는 대로 찾고 개수 표시 · Alt+C: 대/소문자 구분 · Alt+X: 정규식)'],
  ['Ctrl+Shift+G', '변경 내용 옆 창'],
  ['Ctrl+Shift+P', '자주 쓰는 지시'],
  ['Ctrl+Shift+O', '새 사이트 항목(브라우저): 빈 페이지로 열리고 위의 주소 칸에 주소를 칩니다'],
  ['Ctrl+Shift+H', '새 HTTP 항목'],
  ['Ctrl+Shift+M', '새 메모 항목'],
  ['Ctrl+Shift+E', '프롬프트 편집기로 (세션 하나만 보일 때) · Ctrl+Enter: 입력창에 넣기 · Alt+↑/↓: 보낸 프롬프트 기록'],
  ['메모: Ctrl+F / Ctrl+R', '찾기 / 바꾸기 (Alt+C 대소문자, Alt+O 단어, Alt+X 정규식)'],
  ['메모: Ctrl+S / Ctrl+O', '저장 / 열기 · Ctrl+G: 줄로 이동 · F5: 시간/날짜 · Alt+Z: 줄 바꿈'],
  ['이미지: F / 1 / + / -', '창에 맞춤 / 실제 크기 / 확대 / 축소 · Alt(또는 Ctrl)+휠: 커서 기준 확대·축소 · 끌기: 이동'],
  ['이미지: 휠, ←/→', '같은 묶음의 이전·다음 이미지 · I: 정보 · Ctrl+C: 복사 · Ctrl+S: 저장 · Ctrl+V: 붙여넣기'],
  ['끌어다 놓기', '사이드바에 놓으면 항목으로(이미지·메모·HTTP·사이트) · 터미널에 놓으면 경로·글자 입력'],
  ['사이트(브라우저)', '[캡처]: 화면을 이미지 항목으로(Shift+클릭: 페이지 전체) · [Claude에게]: 같은 그룹의 Claude 세션에 넘기기 · [외부 브라우저]: 기본 브라우저로 열기'],
  ['브레인스토밍', 'Tab: 하위 가지 · Enter: 형제 가지(글 없이 Enter: 빈 가지) · 글자 입력·F2: 고치기 · Delete: 지우기 · 끌기: 옮기기 · Alt+↑/↓: 순서 바꾸기'],
  ['브레인스토밍', 'Ctrl+E: 그림 ↔ 코드 · 오른쪽 클릭: 표식(깃발·별·순위·진행·이모지)·색 · Ctrl+/: 접기 · Ctrl+Z/Y: 되돌리기 · Ctrl+C/V: 가지 복사·붙여넣기 · Ctrl+휠: 확대 · Ctrl+0: 맞춤 · Ctrl+S: 저장'],
  ['사이드바 빈 곳 클릭 뒤 ↑/↓', '항목 사이를 오가기 (Home/End: 처음·끝, Enter: 그 항목으로 초점)'],
  ['설정 → 모양', '테마(어두운·밝은·Windows 고전), 글꼴, 글자 크기'],
  ['Ctrl+Shift+B', '동시 입력 (나란히 보이는 모든 세션)'],
  ['Ctrl+V', '붙여넣기 (이미지 포함) · Ctrl+Shift+V: 이미지를 파일 경로로'],
  ['Ctrl+C', '선택 영역이 있으면 복사'],
  ['Shift+Enter', '줄바꿈'],
  ['링크 클릭', '웹 주소는 브라우저, 경로는 탐색기 · Ctrl+클릭: 파일 열기'],
  ['목록에서 Ctrl+클릭', '나란히 보기에 넣기/빼기'],
  ['목록에서 끌기', '다른 줄 위·아래: 순서 바꾸기 · 묶음 머리: 그 묶음으로 · 화면: 놓은 위치대로 분할'],
  ['목록에서 두 번 누르기', '별칭 붙이기 (세션에는 영향 없음)'],
  ['목록에서 오른쪽 클릭', '별칭·색·그룹, 같은 폴더에 새 세션, 폴더 열기·경로 복사, 종료된 세션 다시 시작 등 · 빈 곳: 새 항목'],
];

/**
 * 브라우저 엔진 정보: 지금 쓰는 엔진과 Chromium 버전을 보여 주고, 최신 안정판과 비교한다.
 * Cypher는 Chromium을 직접 싣지 않으므로(브라우저가 스스로 갱신한다) 갱신이 멈춰 있지 않은지만 살핀다.
 */
export async function showEngine() {
  const { invoke, make } = api;
  const row = (key, value) => {
    const line = make('div', 'row help');
    line.append(make('kbd', '', key), make('span', '', value));
    return line;
  };
  const info = await invoke('browser_engine_info');
  const chrome = info.engine === 'chrome';
  const major = (version) => Number(String(version).split('.')[0]) || 0;
  // 최신 버전은 인터넷에서 읽어야 하므로, 사용자가 단추를 눌렀을 때만 확인한다.
  const latest = row('최신 안정판', '아직 확인하지 않았습니다');
  const verdict = make('p', 'note', '');
  const check = make('button', 'row add', '최신 버전 확인 (인터넷에서 Chrome 안정판 버전을 읽어 옵니다)');
  openPopup('브라우저 엔진 정보', [
    row('엔진', chrome ? 'chrome — 설치된 브라우저를 전용 프로필로 띄워 조작' : 'webview — 사이드바의 사이트 항목(앱에 내장된 웹뷰)'),
    row('브라우저', info.browser),
    row('Chromium 버전', info.version || '알 수 없음'),
    ...(info.path ? [row('실행 파일', info.path)] : []),
    latest, verdict, check,
    make('p', 'note', chrome
      ? '갱신: 이 브라우저가 스스로 갱신합니다. 수동으로 하려면 평소처럼 그 브라우저를 열어 "도움말 → Chrome 정보"(chrome://settings/help, Edge는 edge://settings/help)에서 업데이트한 뒤, Cypher용 브라우저 창을 닫았다 다시 엽니다.'
      : '갱신: WebView2 런타임은 Edge와 함께 Windows가 자동으로 갱신합니다. 수동으로 하려면 Edge를 열어 edge://settings/help에서 업데이트하거나 Microsoft의 "WebView2 런타임(Evergreen)" 설치 프로그램을 실행한 뒤 Cypher를 다시 시작합니다.'),
    make('p', 'note', 'Cypher는 Chromium을 직접 싣지 않아, 엔진 버전은 Cypher 버전과 무관하게 올라갑니다. Cypher는 스스로 버전을 확인하러 나가지 않습니다. 위 단추를 눌렀을 때만 googlechromelabs.github.io에서 최신 버전 번호를 읽습니다(보내는 정보는 없습니다).'),
  ]);
  check.onclick = async () => {
    check.disabled = true;
    latest.lastChild.textContent = '확인 중…';
    try {
      const newest = await invoke('browser_latest_version');
      latest.lastChild.textContent = `${newest} (Chrome 안정판)`;
      const behind = major(newest) - major(info.version);
      verdict.textContent = !info.version ? ''
        : behind <= 0 ? '최신 단계입니다.'
        : behind === 1 ? '한 단계 뒤입니다. 새 안정판이 순서대로 배포되는 동안에는 정상입니다(보통 며칠 안에 따라옵니다).'
        : `${behind}단계 뒤처져 있습니다. 자동 갱신이 멈춘 것일 수 있으니 아래 방법으로 갱신하세요.`;
      verdict.style.color = behind >= 2 ? 'var(--dead)' : '';
    } catch (err) {
      latest.lastChild.textContent = String(err);
    }
    check.disabled = false;
  };
}

export function showHelp() {
  const { make } = api;
  openPopup('단축키', HELP.map(([key, what]) => {
    const row = make('div', 'row help');
    row.append(make('kbd', '', key), make('span', '', what));
    return row;
  }));
}

// ───────────────────────── 오른쪽 클릭 메뉴 ─────────────────────────

/**
 * items: { label, action } · { colors, action(color) } · { caption }(작은 설명 글)
 *      · { icons: [{ node(그림), title, on(켜진 것), action }] }(작은 그림 단추 한 줄)
 */
export function showMenu(ev, items) {
  const { make } = api;
  const menu = $('menu');
  menu.replaceChildren(...items.map((item) => {
    if (item.caption) return make('div', 'menu-caption', item.caption);
    if (item.icons) {
      const row = make('div', 'menu-icons');
      for (const icon of item.icons) {
        const button = make('button', icon.on ? 'on' : '');
        button.title = icon.title || '';
        button.append(icon.node);
        button.onclick = () => { hideMenu(); icon.action(); };
        row.append(button);
      }
      return row;
    }
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
  menu.style.top = `${Math.max(4, Math.min(ev.clientY, innerHeight - height - 4))}px`;
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
  $('diff').hidden = !open;
  $('changes').classList.toggle('on', open);
  if (open) showDiff(api.getActive());
  else api.focusActive();
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
