// 부가 화면: 떠 있는 창(자주 쓰는 지시·기억·도움말), 오른쪽 클릭 메뉴, 변경 내용 옆 창.
// main.js가 initExtras()로 필요한 함수들을 넘겨준다.

const $ = (id) => document.getElementById(id);
let api = null; // { invoke, make, ask, setStatus, getActive, getFolder, focusActive, newSession, pasteText, label }

export function initExtras(given) {
  api = given;
  $('popup-close').onclick = () => $('popup').close();
  $('diff-close').onclick = () => toggleDiff(false);
  $('diff-refresh').onclick = () => loadDiff();
  initDiff();
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
  let list = [];
  try { list = JSON.parse(localStorage.getItem(SNIPPET_KEY) || '[]'); } catch { /* 깨진 기록은 버린다 */ }
  // 목록이 아닌 값이 적혀 있거나 글이 아닌 것이 섞여 있어도 창은 열려야 한다.
  return Array.isArray(list) ? list.filter((text) => typeof text === 'string' && text) : [];
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
  ['Ctrl+Shift+A', '알림 기록: 지나간 완료·승인 대기·Claude가 보낸 알림 모아 보기 (사이드바 아래 [알림 N]의 숫자는 보지 않은 알림)'],
  ['Ctrl+Shift+G', '변경 내용 옆 창 · 줄을 누르거나 끌어 고르고(Shift+클릭: 범위) 한마디를 적어 Enter: Claude 세션 입력창에 넣기 · 파일 오른쪽 클릭: 되돌리기 · [커밋…]: 고른 파일만 커밋'],
  ['Ctrl+Shift+P', '자주 쓰는 지시'],
  ['Ctrl+Shift+O', '새 사이트 항목(브라우저): 빈 페이지로 열리고 위의 주소 칸에 주소를 칩니다'],
  ['Ctrl+Shift+H', '새 HTTP 항목'],
  ['Ctrl+Shift+M', '새 메모 항목'],
  ['Ctrl+Shift+E', '프롬프트 편집기로 (세션 하나만 보일 때) · Ctrl+Enter: 입력창에 넣기 · Alt+↑/↓: 보낸 프롬프트 기록'],
  ['메모: Ctrl+F / Ctrl+R', '찾기 / 바꾸기 (Alt+C 대소문자, Alt+O 단어, Alt+X 정규식)'],
  ['메모: Ctrl+S / Ctrl+O', '저장 / 열기 · Ctrl+G: 줄로 이동 · F5: 시간/날짜 · Alt+Z: 줄 바꿈'],
  ['이미지: F / 1 / + / -', '창에 맞춤 / 실제 크기 / 확대 / 축소 · Alt(또는 Ctrl)+휠: 커서 기준 확대·축소 · 끌기: 이동'],
  ['이미지: 휠, ←/→', '같은 묶음의 이전·다음 이미지 · I: 정보 · Ctrl+C: 복사 · Ctrl+S: 저장 · Ctrl+V: 붙여넣기'],
  ['문서: Ctrl+F / Alt+←', '찾기 / 링크를 따라오기 전 문서로 · 목차 · Ctrl+휠, Ctrl+더하기/빼기/0: 글자 크기 · 파일이 바뀌면 스스로 다시 그림'],
  ['그룹 틀', '도구 → 그룹 틀, 또는 묶음 머리 오른쪽 클릭 → 틀로 저장: 묶음의 구성(세션 폴더·메모·사이트·HTTP·문서)을 저장해 두고 한 번에 펼치기'],
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
// 세션 폴더의 바뀐 파일과 차이를 보여 준다. 차이의 줄을 골라 한마디를 달아 세션 입력창에 넣고,
// 파일의 변경을 되돌리고, 고른 파일만 커밋할 수 있다.

const MAX_DIFF_LINES = 6000;   // 한 파일에서 보여 주는 차이 줄 수
const MAX_QUOTE_LINES = 40;    // 세션에 넣는 글에 싣는 줄 수

let diffSession = null;
let diffRoot = '';
let diffBranch = '';
let diffFiles = [];            // 바뀐 파일 목록 { status, path, untracked, from }
let diffSelected = '';
let diffLines = [];            // 지금 보이는 차이의 줄들(parseDiff가 만든 것)
let diffShown = null;          // 지금 그려 둔 차이 { root, path, text }. 같은 내용이면 다시 그리지 않는다.
let picked = new Set();        // 고른 줄(diffLines에서의 순번)
let pickAnchor = null;         // Shift+클릭으로 범위를 고를 때의 기준 줄
let pickDrag = null;           // 끌어서 고르는 중 { from, base, moved }
let commitDraft = { root: '', text: '' }; // 쓰다 만 커밋 메시지(창을 닫았다 열어도 남는다)

export const diffOpen = () => !$('diff').hidden;

// ── 차이 읽기(화면 없이 도는 계산) ──

/**
 * git이 내놓은 차이 글을 줄 목록으로 바꾼다. 줄마다 종류와 실제 파일에서의 줄 번호를 붙인다.
 *   kind: 'meta'(파일 머리) | 'hunk'(갈래 머리 @@) | 'add' | 'del' | 'ctx'(그대로인 줄) | 'note'(\ No newline …)
 *   oldNo / newNo: 옛 파일·새 파일에서의 줄 번호. 추가된 줄은 newNo만, 지운 줄은 oldNo만 있다(없는 쪽은 null).
 * 갈래 머리(`@@ -a,b +c,d @@`)의 줄 수만큼만 내용으로 읽으므로, `--- `로 시작하는 지운 줄도 파일 머리와 헷갈리지 않는다.
 */
export function parseDiff(text) {
  const raw = String(text).split('\n');
  if (raw.length && raw[raw.length - 1] === '') raw.pop();
  const lines = [];
  let oldNo = 0, newNo = 0;       // 다음에 올 줄의 번호
  let oldLeft = 0, newLeft = 0;   // 이 갈래에 남은 줄 수
  let loose = false;              // 줄 수를 알 수 없는 갈래(충돌 난 파일의 합친 차이 등): 앞 글자로만 가른다.
  for (const each of raw) {
    const line = each.endsWith('\r') ? each.slice(0, -1) : each;
    const push = (kind, o = null, n = null) => lines.push({ kind, text: line, oldNo: o, newNo: n });
    const mark = line[0];
    if (line.startsWith('@@')) {
      const head = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
      loose = !head;
      oldLeft = newLeft = 0;
      if (head) {
        oldNo = Number(head[1]);
        newNo = Number(head[3]);
        oldLeft = head[2] == null ? 1 : Number(head[2]);
        newLeft = head[4] == null ? 1 : Number(head[4]);
      }
      push('hunk');
    } else if (mark === '\\') {
      push('note');
    } else if (loose && (mark === '+' || mark === '-' || mark === ' ')) {
      push(mark === '+' ? 'add' : mark === '-' ? 'del' : 'ctx');
    } else if (mark === ' ' && oldLeft > 0 && newLeft > 0) {
      push('ctx', oldNo++, newNo++);
      oldLeft--; newLeft--;
    } else if (mark === '+' && newLeft > 0) {
      push('add', null, newNo++);
      newLeft--;
    } else if (mark === '-' && oldLeft > 0) {
      push('del', oldNo++, null);
      oldLeft--;
    } else if (line === '' && oldLeft > 0 && newLeft > 0) {
      // 갈래 안의 빈 줄: 앞의 공백 없이 적힌 그대로인 빈 줄이다(git의 diff.suppressBlankEmpty 설정).
      push('ctx', oldNo++, newNo++);
      oldLeft--; newLeft--;
    } else {
      // 갈래 밖의 줄: 파일 머리(diff, index, ---, +++ …)나 덧붙은 안내 글.
      loose = false;
      oldLeft = newLeft = 0;
      push('meta');
    }
  }
  return lines;
}

/**
 * 줄마다 화면에서의 순번(`index`)을 붙인다. 순번이 이어지면 붙어 있는 줄이다.
 * 안내 줄(\ No newline at end of file)은 세지 않는다: 그 줄을 사이에 둔 지운 줄과 추가된 줄은 붙어 있는 것으로 본다.
 */
export function numberLines(lines) {
  let index = -1;
  return lines.map((line) => ({ index: line.kind === 'note' ? index : ++index, ...line }));
}

/** 줄 번호를 말로: "42줄", "42~45줄". */
const lineSpan = (numbers) => (numbers[0] === numbers[numbers.length - 1]
  ? `${numbers[0]}줄` : `${numbers[0]}~${numbers[numbers.length - 1]}줄`);

/**
 * 고른 줄들(순번 `index`가 붙은 줄, 순서대로)을 화면에서 붙어 있는 것끼리 구간으로 묶는다.
 * 구간마다 가리키는 말(label)을 붙인다: 새 파일 기준 "42~45줄", 지운 줄뿐이면 "(지워진 줄, 옛 42~45줄)".
 */
export function segmentsOf(lines) {
  const segments = [];
  for (const line of lines) {
    const last = segments[segments.length - 1];
    if (last && last.lines[last.lines.length - 1].index + 1 === line.index) last.lines.push(line);
    else segments.push({ lines: [line], label: '' });
  }
  for (const segment of segments) {
    const fresh = segment.lines.map((line) => line.newNo).filter((n) => n != null);
    const old = segment.lines.map((line) => line.oldNo).filter((n) => n != null);
    segment.label = fresh.length ? lineSpan(fresh) : old.length ? `(지워진 줄, 옛 ${lineSpan(old)})` : '';
  }
  return segments;
}

/**
 * 고른 줄과 사용자가 적은 한마디를 세션 입력창에 넣을 글로 만든다.
 * 구간마다 "`경로` 42~45줄:" 머리와 차이 그대로의 코드 블록을 두고, 맨 끝에 한마디를 붙인다.
 * 줄이 많으면 앞의 40줄만 싣는다.
 */
export function quoteLines(path, lines, comment = '') {
  // 줄 안에 ```가 있어도 코드 블록이 끊기지 않게, 가장 긴 것보다 하나 긴 울타리를 쓴다.
  const longest = Math.max(0, ...lines.map((line) => Math.max(0, ...(line.text.match(/`+/g) || []).map((run) => run.length))));
  const fence = '`'.repeat(Math.max(3, longest + 1));
  const parts = [];
  let left = MAX_QUOTE_LINES;
  for (const segment of segmentsOf(lines)) {
    if (left <= 0) break;
    const shown = segment.lines.slice(0, left);
    left -= shown.length;
    parts.push(`\`${path}\`${segment.label ? ` ${segment.label}` : ''}:\n${fence}diff\n${shown.map((line) => line.text).join('\n')}\n${fence}`);
  }
  if (lines.length > MAX_QUOTE_LINES) parts.push(`… (이하 ${lines.length - MAX_QUOTE_LINES}줄 생략)`);
  return `${parts.join('\n')}\n${String(comment).trim()}`;
}

// ── 옆 창 ──

/** 옆 창에 단추와 "고른 줄에 한마디 달기" 줄을 끼워 넣고, 줄 고르기를 건다(시작할 때 한 번). */
function initDiff() {
  const { make } = api;
  const $view = $('diff-view');

  const commit = make('button', '', '커밋…');
  Object.assign(commit, { id: 'diff-commit', disabled: true, onclick: showCommit });
  commit.title = '바뀐 파일 가운데 고른 것만 커밋하기 (푸시는 하지 않습니다)';
  $('diff-refresh').before(commit);

  const bar = make('div');
  Object.assign(bar, { id: 'diff-quote', hidden: true });
  const where = make('span', 'where');
  where.id = 'diff-quote-where';
  const text = make('textarea');
  Object.assign(text, {
    id: 'diff-quote-text', rows: 1, spellcheck: false, autocomplete: 'off',
    placeholder: '이 부분을 어떻게 할지 적으세요 (Enter: 세션에 넣기 · Shift+Enter: 줄바꿈)',
  });
  const send = make('button', 'send', '세션에 넣기');
  send.id = 'diff-quote-send';
  send.title = '고른 줄과 적은 글을 세션 입력창에 넣습니다(보내지는 않습니다) · Enter';
  send.onclick = sendQuote;
  const clear = make('button', '', '선택 풀기');
  clear.title = '고른 줄을 풉니다(적은 글은 남습니다) · Esc';
  clear.onclick = () => clearPicked();
  bar.append(where, text, send, clear);
  $view.after(bar);

  const grow = () => { text.style.height = 'auto'; text.style.height = `${Math.min(120, text.scrollHeight + 2)}px`; };
  text.addEventListener('input', grow);
  text.addEventListener('keydown', (ev) => {
    if (ev.isComposing || ev.keyCode === 229) return; // 한글을 조합하는 중의 Enter는 글자 확정이다.
    if (ev.key === 'Enter' && !ev.shiftKey && !ev.ctrlKey && !ev.altKey) { ev.preventDefault(); sendQuote(); }
    else if (ev.key === 'Escape') { ev.preventDefault(); clearPicked(); }
  });

  // 줄 고르기: 누르면 고르고(다시 누르면 풂), Shift+클릭은 기준 줄부터 그 줄까지, 끌면 지나간 줄 모두.
  // 한 줄 안에서 글자를 긁는 것은 글자 선택으로 둔다(복사할 수 있게).
  $view.tabIndex = -1;
  $view.addEventListener('mousedown', (ev) => {
    if (ev.button !== 0) return;
    const index = lineAt(ev.target);
    if (index == null || !pickable(index)) return;
    if (ev.shiftKey) {
      ev.preventDefault(); // 글자 선택이 번지지 않게 한다.
      if (pickAnchor == null || !pickable(pickAnchor)) pickAnchor = index;
      setPicked(withRange(picked, pickAnchor, index));
      return;
    }
    pickDrag = { from: index, base: new Set(picked), moved: false };
  });
  document.addEventListener('mousemove', (ev) => {
    if (!pickDrag) return;
    if (!(ev.buttons & 1)) { // 창 밖에서 단추를 놓았다: 끌기만 그만둔다.
      pickDrag = null;
      $view.classList.remove('picking');
      return;
    }
    const index = lineAt(document.elementFromPoint(ev.clientX, ev.clientY));
    if (index == null || (!pickDrag.moved && index === pickDrag.from)) return;
    if (!pickDrag.moved) {
      pickDrag.moved = true;
      $view.classList.add('picking');
    }
    getSelection()?.removeAllRanges();
    setPicked(withRange(pickDrag.base, pickDrag.from, index));
  });
  document.addEventListener('mouseup', () => endPickDrag());
  $view.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape' && picked.size) { ev.preventDefault(); clearPicked(); }
    // 고른 줄이 있고 긁어 고른 글자가 없으면 Ctrl+C는 고른 줄을 복사한다(앞의 +/- 표시는 뺀다).
    else if (ev.ctrlKey && !ev.shiftKey && ev.key.toLowerCase() === 'c' && picked.size && getSelection()?.isCollapsed) {
      ev.preventDefault();
      const copied = pickedLines().map((line) => line.text.slice(1)).join('\n');
      api.invoke('clipboard_write', { text: copied })
        .then(() => api.setStatus(`고른 ${picked.size}줄을 복사했습니다.`))
        .catch((err) => api.setStatus(String(err)));
    }
  });
}

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
    diffBranch = changes.branch;
    diffFiles = changes.files;
    $('diff-commit').disabled = !changes.files.length;
    $('diff-title').textContent =
      `변경 내용 — ${api.label(s)}${changes.branch ? ` (${changes.branch})` : ''} · ${changes.files.length}개`;
    $files.replaceChildren(...changes.files.map((file) => {
      const row = make('li', file.path === diffSelected ? 'selected' : '');
      row.append(make('span', `status ${file.untracked ? 'new' : ''}`, file.status), make('span', 'path', file.path));
      row.title = `${file.from ? `${file.from} → ` : ''}${file.path}\n클릭: 차이 보기 · 두 번 누르기: 파일 열기 · 오른쪽 클릭: 되돌리기·열기·경로 복사`;
      row.dataset.path = file.path;
      row.onclick = () => selectFile(file);
      row.ondblclick = () => openChanged(diffRoot, file.path, true);
      row.oncontextmenu = (ev) => { ev.preventDefault(); ev.stopPropagation(); fileMenu(file, ev); };
      return row;
    }));
    const current = changes.files.find((f) => f.path === diffSelected) || changes.files[0];
    if (current) selectFile(current);
    else showNoDiff('바뀐 파일이 없습니다.');
  } catch (err) {
    $('diff-title').textContent = '변경 내용';
    diffFiles = [];
    $('diff-commit').disabled = true;
    $files.replaceChildren();
    showNoDiff(String(err));
  }
}

/** 바뀐 파일을 연다(`openFile`이 거짓이면 탐색기에서 보여 주기만 한다). */
function openChanged(root, path, openFile) {
  api.invoke('link_open', { target: `${root}/${path}`, cwd: root, openFile, line: null, col: null })
    .catch((err) => api.setStatus(String(err)));
}

async function selectFile(file) {
  const { invoke } = api;
  diffSelected = file.path;
  for (const row of $('diff-files').children) row.classList.toggle('selected', row.dataset.path === file.path);
  const root = diffRoot;
  try {
    const text = await invoke('git_diff', { root, path: file.path, untracked: file.untracked });
    if (diffSelected !== file.path || diffRoot !== root) return;
    // 달라진 것이 없으면 그대로 둔다(작업이 끝나 새로 읽어도 고른 줄과 보던 자리가 남는다).
    if (diffShown && diffShown.root === root && diffShown.path === file.path && diffShown.text === text) return;
    renderDiff(root, file.path, text);
  } catch (err) {
    if (diffSelected === file.path) showNoDiff(String(err));
  }
}

/** 차이 대신 안내 글을 보여 준다. 고른 줄은 풀린다(적어 둔 한마디는 남는다). */
function showNoDiff(message) {
  diffLines = [];
  diffShown = null;
  clearPicked(false);
  $('diff-view').replaceChildren(api.make('p', 'note', message));
}

/** 차이를 그린다. 줄마다 왼쪽에 옛 파일·새 파일의 줄 번호를 흐리게 보여 준다. */
function renderDiff(root, path, text) {
  const { make } = api;
  const $view = $('diff-view');
  const all = parseDiff(text);
  diffLines = all.slice(0, MAX_DIFF_LINES);
  diffShown = { root, path, text };
  clearPicked(false);
  if (!text.trim()) {
    $view.replaceChildren(make('p', 'note', '차이가 없습니다(이진 파일이거나 권한만 바뀜).'));
    return;
  }
  let widest = 1;
  for (const line of diffLines) widest = Math.max(widest, line.oldNo || 0, line.newNo || 0);
  const width = String(widest).length;
  const number = (n) => String(n ?? '').padStart(width);
  const rows = diffLines.map((line, index) => {
    const row = make('div', line.kind === 'ctx' ? '' : line.kind === 'note' ? 'meta' : line.kind);
    row.dataset.i = index;
    row.append(make('span', 'no', `${number(line.oldNo)} ${number(line.newNo)} `), make('span', 'code', line.text || ' '));
    return row;
  });
  if (all.length > MAX_DIFF_LINES) rows.push(make('div', 'meta', `… (${MAX_DIFF_LINES}줄까지만 보여 줍니다. 전체는 파일을 열어 보세요)`));
  $view.replaceChildren(...rows);
  $view.scrollTop = 0;
}

// ── 줄 고르기와 세션에 넣기 ──

/** 그 요소가 속한 차이 줄의 순번. 차이 줄이 아니면 null. */
function lineAt(target) {
  const row = target?.closest?.('#diff-view > div[data-i]');
  return row ? Number(row.dataset.i) : null;
}

/** 고를 수 있는 줄인지: 추가·삭제·그대로인 줄만(갈래 머리와 파일 머리는 고르지 않는다). */
const pickable = (index) => ['add', 'del', 'ctx'].includes(diffLines[index]?.kind);

/** base에 a~b 사이의 고를 수 있는 줄을 더한 것. */
function withRange(base, a, b) {
  const next = new Set(base);
  for (let index = Math.min(a, b); index <= Math.max(a, b); index++) if (pickable(index)) next.add(index);
  return next;
}

/** 고른 줄들을 순서대로(붙어 있는지 알 수 있게 순번 `index`를 붙여서). */
function pickedLines() {
  const numbered = numberLines(diffLines);
  return [...picked].sort((a, b) => a - b).map((at) => numbered[at]).filter(Boolean);
}

function setPicked(next) {
  picked = next;
  for (const row of $('diff-view').children) {
    if (row.dataset.i != null) row.classList.toggle('picked', picked.has(Number(row.dataset.i)));
  }
  const lines = pickedLines();
  const $bar = $('diff-quote');
  if (!$bar) return;
  const typing = document.activeElement === $('diff-quote-text');
  $bar.hidden = !lines.length;
  if (!lines.length) {
    if (typing) $('diff-view').focus(); // 입력칸이 사라졌으니 초점을 차이 화면에 둔다.
    return;
  }
  const segments = segmentsOf(lines);
  const range = `${segments[0].label}${segments.length > 1 ? ` 외 ${segments.length - 1}곳` : ''}`;
  $('diff-quote-where').textContent = `${diffSelected}${range ? ` ${range}` : ''} · ${lines.length}줄 선택`;
  $('diff-quote-where').title = segments.map((segment) => segment.label).filter(Boolean).join(', ');
}

/** 고른 줄을 모두 푼다. 적어 둔 한마디는 지우지 않는다. */
function clearPicked(refocus = true) {
  pickAnchor = null;
  pickDrag = null;
  $('diff-view').classList.remove('picking');
  const had = picked.size > 0;
  setPicked(new Set());
  if (refocus && had) $('diff-view').focus();
}

/** 끌기를 마친다. 움직이지 않았으면 그냥 누른 것이다: 그 줄을 고르거나 푼다. */
function endPickDrag() {
  if (!pickDrag) return;
  const { from, moved } = pickDrag;
  pickDrag = null;
  $('diff-view').classList.remove('picking');
  pickAnchor = from;
  if (moved) return;
  if (!getSelection()?.isCollapsed) return; // 한 줄 안에서 글자를 긁어 골랐다.
  const next = new Set(picked);
  if (next.has(from)) next.delete(from); else next.add(from);
  setPicked(next);
}

/**
 * 고른 줄을 넣을 수 있는 세션들 { s, label }.
 * 변경 내용을 보고 있는 항목이 돌고 있는 Claude 세션이면 그 세션, 아니면 같은 묶음의 Claude 세션들.
 * (PowerShell 같은 셸에는 넣지 않는다. 여러 줄을 붙여넣으면 줄마다 명령으로 실행된다.)
 */
function quoteTargets() {
  const s = diffSession;
  if (!s) return [];
  if (s.term && !s.exited && s.profile?.claude && s.pane?.isConnected !== false) return [{ s, label: api.label(s) }];
  return api.claudes?.(s) || [];
}

/** [세션에 넣기]: 고른 줄과 한마디를 세션 입력창에 넣는다(보내지는 않는다). 넣을 세션이 여럿이면 고르게 한다. */
function sendQuote() {
  const lines = pickedLines();
  if (!lines.length) return;
  const $text = $('diff-quote-text');
  const targets = quoteTargets();
  if (!targets.length) {
    return api.setStatus('넣을 Claude 세션이 없습니다. 이 묶음에 Claude 세션을 연 뒤 다시 누르세요(고른 줄과 적은 글은 그대로 있습니다).');
  }
  const give = (target) => {
    const now = pickedLines(); // 고르는 사이 차이가 새로 그려졌을 수 있다.
    if (!now.length) return api.setStatus('고른 줄이 풀렸습니다(차이가 바뀌었습니다). 다시 골라 주세요.');
    const text = quoteLines(diffSelected, now, $text.value);
    $text.value = '';
    $text.style.height = 'auto';
    clearPicked(false);
    if (api.pasteTo) api.pasteTo(target.s, text); else api.pasteText(text);
    api.setStatus(`"${target.label}" 입력창에 넣었습니다(보내지는 않았습니다). 확인한 뒤 Enter로 보내세요.`);
  };
  if (targets.length === 1) return give(targets[0]);
  const rect = $('diff-quote-send').getBoundingClientRect();
  showMenu({ clientX: rect.left, clientY: rect.top }, [
    { caption: '어느 Claude 세션에 넣을까요?' },
    ...targets.map((target) => ({ label: target.label, action: () => give(target) })),
  ]);
}

// ── 파일 되돌리기 ──

/** 파일 목록의 줄을 오른쪽 클릭했을 때의 메뉴. */
function fileMenu(file, ev) {
  const root = diffRoot;
  if (diffSelected !== file.path) selectFile(file);
  showMenu(ev, [
    { caption: file.from ? `${file.from} → ${file.path}` : file.path },
    file.untracked
      ? { caption: '새 파일이라 되돌릴 원본이 없습니다' }
      : { label: '이 파일의 변경 되돌리기…', action: () => revertFile(root, file) },
    { label: '파일 열기', action: () => openChanged(root, file.path, true) },
    { label: '탐색기에서 보기', action: () => openChanged(root, file.path, false) },
    {
      label: '경로 복사',
      action: () => {
        const full = `${root}/${file.path}`.replace(/\//g, '\\');
        api.invoke('clipboard_write', { text: full })
          .then(() => api.setStatus(`복사했습니다: ${full}`))
          .catch((err) => api.setStatus(String(err)));
      },
    },
  ]);
}

/** 파일 하나의 변경을 마지막 커밋의 내용으로 되돌린다. 되돌리기 전의 차이는 앱이 파일로 보관한다. */
async function revertFile(root, file) {
  const { invoke, setStatus } = api;
  const what = file.status === '삭제' ? '지운 파일을 되살립니다' : '마지막 커밋의 내용으로 돌아갑니다';
  if (!confirm(`"${file.path}"의 변경을 되돌릴까요?\n${what}. 되돌리기 전의 차이는 파일로 보관해 둡니다.`)) return;
  try {
    const saved = await invoke('git_revert', { root, path: file.path, from: file.from || null, untracked: !!file.untracked });
    setStatus(`되돌렸습니다: ${file.path} (되돌리기 전 차이 보관: ${saved})`);
  } catch (err) {
    setStatus(`되돌리지 못했습니다: ${err}`);
  }
  loadDiff();
}

// ── 커밋 ──

/** [커밋…]: 바뀐 파일 가운데 고른 것만, 적은 메시지로 커밋한다. 푸시는 하지 않는다. */
function showCommit() {
  const { invoke, make, setStatus } = api;
  const root = diffRoot, files = diffFiles.slice();
  if (!files.length) return;
  let busy = false;

  const checks = files.map((file) => {
    const box = make('input');
    Object.assign(box, { type: 'checkbox', checked: true });
    const row = make('label', 'commit-file');
    row.append(box, make('span', `status ${file.untracked ? 'new' : ''}`, file.status),
      make('span', 'path', file.from ? `${file.from} → ${file.path}` : file.path));
    row.title = file.path;
    return { file, box, row };
  });
  const all = make('input');
  Object.assign(all, { type: 'checkbox', checked: true });
  const count = make('span', 'row-note', '');
  const head = make('label', 'commit-file commit-all');
  head.append(all, make('span', 'path', '모두 고르기'), count);
  const list = make('div', 'commit-files');
  list.append(...checks.map((check) => check.row));
  const message = make('textarea', 'commit-message');
  Object.assign(message, {
    rows: 4, spellcheck: false, autocomplete: 'off',
    placeholder: '커밋 메시지 (첫 줄이 제목 · Ctrl+Enter: 커밋)',
    value: commitDraft.root === root ? commitDraft.text : '',
  });
  const problem = make('p', 'note commit-error', '');
  const run = make('button', 'row add', '커밋');

  const chosen = () => checks.filter((check) => check.box.checked).map((check) => check.file.path);
  const sync = () => {
    const n = chosen().length;
    all.checked = n === files.length;
    all.indeterminate = n > 0 && n < files.length;
    count.textContent = `${n}/${files.length}개 고름`;
    run.disabled = busy || !n || !message.value.trim();
    run.textContent = busy ? '커밋하는 중…' : `커밋 (파일 ${n}개)`;
    run.title = !n ? '커밋할 파일을 고르세요' : !message.value.trim() ? '커밋 메시지를 적으세요' : 'Ctrl+Enter';
  };
  // 이 커밋 창이 지금 화면에 떠 있는지. 창을 닫아도 내용은 그대로 붙어 있으므로(다른 창을 띄울 때 바뀐다) 열려 있는지도 본다.
  const showing = () => message.isConnected && $('popup').open;
  const commit = async () => {
    if (run.disabled) return;
    busy = true;
    problem.textContent = '';
    sync();
    try {
      const made = await invoke('git_commit', { root, paths: chosen(), message: message.value.trim() });
      commitDraft = { root: '', text: '' };
      if (showing()) close();
      setStatus(`커밋했습니다: ${made}`);
    } catch (err) {
      busy = false;
      problem.textContent = `커밋하지 못했습니다:\n${err}`;
      sync();
      if (!showing()) setStatus(`커밋하지 못했습니다: ${err}`); // 기다리는 사이 창을 닫았다.
    }
    loadDiff(); // 실패했더라도 색인에 올리는 데까지는 됐을 수 있으니 목록을 맞춘다.
  };

  all.onchange = () => { for (const check of checks) check.box.checked = all.checked; sync(); };
  for (const check of checks) check.box.onchange = sync;
  message.addEventListener('input', () => { commitDraft = { root, text: message.value }; sync(); });
  message.addEventListener('keydown', (ev) => {
    if (ev.ctrlKey && ev.key === 'Enter' && !ev.isComposing) { ev.preventDefault(); commit(); }
  });
  run.onclick = commit;
  sync();

  const close = openPopup('커밋', [
    make('p', 'note', `${diffBranch ? `${diffBranch} 브랜치` : '지금 브랜치'}에 커밋합니다. 고른 파일만 커밋하고, 푸시는 하지 않습니다.`),
    head, list, message, problem, run,
  ]);
  message.focus();
}
