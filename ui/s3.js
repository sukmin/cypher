// S3 항목: S3 호환 API를 가진 저장소를 Windows 탐색기처럼 본다(버킷 → 폴더 → 객체).
//
// 저장소에 닿는 일은 모두 앱(Rust의 s3 모듈)이 한다. 이 화면은 무엇을 할지 정하고 결과를 보여 준다.
//
// - 접속 정보(저장소 주소, 접근 키, 비밀 키)는 앱이 암호화해 저장한다. 키는 저장할 때 한 번 넘길 뿐, 화면으로 돌아오지 않는다.
// - 접속은 사용자가 [접속]을 눌렀을 때만 한다. Cypher를 다시 켜면 항목은 "접속 안 함" 상태로 되살아나고 아무 요청도 보내지 않는다.
// - 목록은 그 층만, 한 쪽(500개)씩 읽는다. 다음 쪽은 끝까지 내렸을 때나 [더 불러오기]를 눌렀을 때만 읽고,
//   한 번 읽은 폴더는 기억해 두어 뒤로·앞으로·위로 갈 때 다시 읽지 않는다(새로 읽으려면 새로 고침, F5).
//   거르기 칸은 불러온 줄 안에서만 거른다(요청을 보내지 않는다). 서버에서 찾으려면 Enter(그 글로 시작하는 것만 읽는다).
// - 고치거나 지우는 일(삭제, 이름 바꾸기, 옮기기, 덮어쓰기, 글 고쳐 저장)은 확인을 받고 한다. 새로 더하는 일(올리기, 새 폴더,
//   복사해 붙여넣기)은 묻지 않되, 이미 있는 것을 덮어쓰게 되면 그때 묻는다.
// - 주소는 `s3://버킷/폴더/` 모양이다(다른 S3 도구에 그대로 붙여 쓸 수 있다). 주소 줄을 누르면 글로 고칠 수 있다.

const PAGE = 500;          // 한 번에 읽는 목록의 줄 수
const CACHE_MAX = 60;      // 기억해 두는 폴더 수
const JOB_KEEP_MS = 6000;  // 끝난 작업을 작업 줄에 남겨 두는 시간
const ENDED = ['done', 'failed', 'canceled'];

let api = null; // { invoke, listen, make, openPopup, ask, setStatus, showMenu, pickFolder(title) }
/** 접속 번호 → 그 접속으로 도는 작업의 알림을 받을 함수 */
const live = new Map();
/** 복사·잘라내기 해 둔 것: { conn(접속 정보 번호), bucket, base(폴더), items: [{ key, size }], cut } */
let clip = null;

export function initS3(given) {
  api = given;
  api.listen('s3-job', ({ payload }) => live.get(payload.handle)?.(payload));
}

// ───────────────────────── 겹침 물음 ─────────────────────────
//
// 이미 있는 것을 덮어쓰게 된 작업은 멈추고 답을 기다린다. 물음은 떠 있는 창 하나를 같이 쓰므로 줄을 세워 하나씩 묻는다:
// 다른 창(접속 정보 창, 앞선 물음)이 떠 있으면 닫힐 때까지 기다리고, 다른 창이 이 물음을 덮어 치웠으면 다시 묻는다.
// 답 없이 취소로 넘어가는 것은 사용자가 이 물음의 창을 닫았을 때(×, Esc)뿐이다.

/** 기다리는 물음들: { owner(물은 화면), event(작업의 알림), answer(답) → void, done } */
const asks = [];
/** 지금 떠 있는 물음. */
let asking = null;

function pumpAsks() {
  if (asking) return;
  while (asks.length && asks[0].done) asks.shift();
  if (!asks.length) return;
  const dialog = document.getElementById('popup');
  if (dialog.open) { setTimeout(pumpAsks, 300); return; }
  const { make, openPopup } = api;
  const ask = asks.shift();
  const { event } = ask;
  const list = make('div', 's3-conflicts');
  for (const name of event.conflicts) list.append(make('div', '', name));
  if (event.conflict_count > event.conflicts.length) list.append(make('div', 'dim', `… 그 밖에 ${count(event.conflict_count - event.conflicts.length)}개`));
  const buttons = make('div', 'ssh-buttons');
  const over = make('button', 's3-danger', `덮어쓰기 (${count(event.conflict_count)}개)`);
  const skip = make('button', '', '있는 것은 건너뛰기');
  const stop = make('button', '', '취소');
  buttons.append(over, skip, stop);
  const close = openPopup(`${event.title}: 이미 있는 것이 ${count(event.conflict_count)}개 있습니다`, [
    make('p', 'note', `${ask.where ? `${ask.where}\n` : ''}아래 이름은 이미 있습니다. 덮어쓰면 있던 내용은 사라집니다.`),
    list,
    buttons,
  ]);
  asking = ask;
  const finish = (value) => {
    if (ask.done) return;
    ask.done = true;
    if (asking === ask) asking = null;
    ask.answer(value);
    setTimeout(pumpAsks, 0);
  };
  ask.dismiss = () => { ask.done = true; if (asking === ask) { asking = null; if (list.isConnected) close(); } setTimeout(pumpAsks, 0); };
  over.onclick = () => { finish('overwrite'); close(); };
  skip.onclick = () => { finish('skip'); close(); };
  stop.onclick = () => { finish('cancel'); close(); };
  const onClose = () => {
    if (ask.done) return;
    // 닫힘 알림은 늦게 온다: 앞선 창이 닫힌 알림이 이 물음이 뜬 뒤에 올 수 있다. 창이 아직 떠 있고 이 물음이 보이고 있으면
    // 이 물음이 닫힌 것이 아니므로 다음 알림을 기다린다(그러지 않으면 앞 물음에 답하는 순간 이 물음이 취소된다).
    if (dialog.open && list.isConnected) { dialog.addEventListener('close', onClose, { once: true }); return; }
    // 이 물음이 보이던 채로 닫혔으면(×, Esc) 취소다. 다른 창이 이 물음을 덮어 치운 뒤에 닫힌 것이면 다시 묻는다.
    if (list.isConnected) finish('cancel');
    else { asking = null; asks.unshift(ask); setTimeout(pumpAsks, 0); }
  };
  dialog.addEventListener('close', onClose, { once: true });
}

/** 그 화면이 물어 둔 것을 거둔다(접속을 끊거나 항목을 닫을 때: 작업은 앱이 멈춘다). */
function dropAsks(owner) {
  for (const ask of asks) if (ask.owner === owner) ask.done = true;
  if (asking?.owner === owner) asking.dismiss?.();
}

// ───────────────────────── 글·주소 다루기 ─────────────────────────

export function sizeText(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return '';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB', 'PB'];
  let value = bytes / 1024, unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

function dateText(given) {
  if (!given) return '';
  const date = new Date(given);
  if (Number.isNaN(date.getTime())) return given;
  const two = (n) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())} ${two(date.getHours())}:${two(date.getMinutes())}`;
}

const count = (n) => Number(n || 0).toLocaleString('ko-KR');

/** 지금 있는 곳의 주소(`s3://버킷/폴더/`). 버킷 목록은 `s3://`. "그 글로 시작하는 것"만 보는 중이면 끝에 `*`를 붙인다. */
export const addressOf = (at) => `s3://${at.bucket ? `${at.bucket}/${at.prefix || ''}${at.starts ? `${at.starts}*` : ''}` : ''}`;

/** 주소 줄에 적은 글을 버킷과 키로 나눈다(`s3://버킷/키`, `버킷/키`, `/버킷`). 읽을 수 없으면 null. */
export function parseAddress(text) {
  let rest = String(text ?? '').trim();
  if (/^s3:\/\//i.test(rest)) rest = rest.slice(5);
  else if (rest.includes('://')) return null;
  rest = rest.replace(/^\/+/, '');
  const slash = rest.indexOf('/');
  return slash < 0 ? { bucket: rest, key: '' } : { bucket: rest.slice(0, slash), key: rest.slice(slash + 1) };
}

/** 폴더의 위 폴더(`a/b/` → `a/`, `a/` → ``). */
const parentOf = (prefix) => prefix.replace(/[^/]*\/$/, '');
/** 키의 마지막 마디(폴더는 끝의 `/`를 뗀다). */
const leafOf = (key) => key.replace(/\/$/, '').split('/').pop();

const kindText = (row) => {
  if (row.bucket) return '버킷';
  if (row.folder) return '폴더';
  const ext = /\.([^./]+)$/.exec(row.name)?.[1];
  const kind = ext ? `${ext.toUpperCase()} 파일` : '파일';
  return row.class && row.class !== 'STANDARD' ? `${kind} · ${row.class}` : kind;
};

// ───────────────────────── 접속 정보 창 ─────────────────────────

/**
 * 접속 정보를 새로 적거나 고치는 창. 저장하면 `done(저장한 접속 정보, 바로 접속할지)`를 부른다.
 * 고칠 때 접근 키와 비밀 키는 다시 보여 주지 않는다: 비워 두면 저장해 둔 것을 그대로 둔다.
 */
export function showConnForm(fill, done) {
  const { invoke, make, openPopup, setStatus } = api;
  const field = (labelText, placeholder, value = '', title = '', type = 'text') => {
    const row = make('label', 'ssh-field');
    const input = make('input');
    Object.assign(input, { placeholder, value: value ?? '', spellcheck: false, autocomplete: 'off', title, type });
    row.append(make('span', '', labelText), input);
    return { row, input };
  };
  const check = (labelText, on, title) => {
    const row = make('label', 's3-check');
    const input = make('input');
    Object.assign(input, { type: 'checkbox', checked: !!on });
    row.title = title;
    row.append(input, make('span', '', labelText));
    return { row, input };
  };
  const editing = !!fill?.id;
  const name = field('이름', '(선택) 사이드바에 보일 이름 · 예: 사내 저장소', fill?.name);
  const endpoint = field('주소', '전체 주소를 적습니다 · 예: https://kr.ncloudstorage.com', fill?.endpoint, '저장소의 주소(엔드포인트). http://나 https://를 적지 않으면 https로 봅니다.\nNcloud Storage: https://kr.ncloudstorage.com · VPC의 사설망에서는 https://kr-private.ncloudstorage.com\n버킷 이름은 적지 않습니다.');
  const access = field('Access key', editing ? '(그대로 두려면 비움)' : '접근 키', '', '접근 키(Access key). 저장할 때 암호화합니다.', 'password');
  const secret = field('Secret key', editing ? '(그대로 두려면 비움)' : '비밀 키', '', '비밀 키(Secret key). 저장할 때 암호화합니다.', 'password');
  const region = field('지역', '(선택) 비우면 주소에 맞춤: Ncloud Storage는 kr', fill?.region || '', '서명에 넣는 지역 이름. 비워 두면 주소를 보고 정합니다(Ncloud Storage는 kr, 그 밖에는 us-east-1).');
  const start = field('처음 열 곳', '(선택) 예: s3://버킷/폴더/ · 비우면 버킷 목록', fill?.start, '접속하면 처음 보여 줄 곳. 버킷 목록을 볼 권한이 없는 키라면 여기에 버킷을 적어 두세요.');
  const lock = field('잠금 암호', editing ? '(선택) 8자 이상 · 키를 다시 적을 때만 정할 수 있음' : '(선택) 8자 이상 · 정하면 접속할 때마다 묻습니다', '', '키를 한 번 더 잠그는 암호. 어디에도 저장하지 않고 접속할 때마다 묻습니다.\n정해 두면 이 PC와 Windows 계정이 통째로 넘어가도 이 암호 없이는 키를 풀 수 없습니다.', 'password');
  const lockAgain = field('암호 확인', '잠금 암호를 한 번 더', '', '', 'password');
  const pathStyle = check('경로 방식 주소 (서버/버킷/키)', fill ? fill.path_style !== false : true, '켜면 https://서버/버킷/키, 끄면 https://버킷.서버/키로 부릅니다.\nNcloud Storage는 둘 다 됩니다(이름에 점이 든 버킷은 경로 방식만 됩니다). 켜 두면 됩니다.');
  const v2 = check('예전 서명 방식 (V2)', fill?.signature === 'v2', '오래된 저장소만 켭니다. 기본은 AWS Signature V4입니다.');
  const insecure = check('인증서 확인 안 함 (위험)', fill?.insecure, '스스로 서명한 인증서를 쓰는 저장소에만 켭니다. 켜면 중간에서 가로채는 서버를 알아채지 못합니다.\n사내 인증 기관의 인증서가 Windows에 설치돼 있으면 켜지 않아도 됩니다.');
  for (const input of [access.input, secret.input]) {
    // 키를 볼 수 있게: 칸에 들어가 있는 동안에만 글자로 보인다.
    input.addEventListener('focus', () => { input.type = 'text'; });
    input.addEventListener('blur', () => { input.type = 'password'; });
  }
  const problem = make('p', 'form-problem', '');
  const read = () => {
    problem.textContent = '';
    if (!endpoint.input.value.trim()) { problem.textContent = '저장소 주소를 적어 주세요.'; endpoint.input.focus(); return null; }
    if (lock.input.value !== lockAgain.input.value) { problem.textContent = '잠금 암호와 암호 확인이 다릅니다.'; lockAgain.input.focus(); return null; }
    return {
      id: fill?.id || null,
      name: name.input.value, endpoint: endpoint.input.value, region: region.input.value, start: start.input.value,
      path_style: pathStyle.input.checked, signature: v2.input.checked ? 'v2' : 'v4', insecure: insecure.input.checked,
      access: access.input.value, secret: secret.input.value, password: lock.input.value,
    };
  };
  let saving = false;
  const save = async (thenConnect) => {
    const conn = read();
    if (!conn || saving) return;
    saving = true;
    try {
      const info = await invoke('s3_conn_save', { conn });
      // 칸에 남은 키와 암호를 지운다.
      for (const input of [access.input, secret.input, lock.input, lockAgain.input]) input.value = '';
      close();
      setStatus(`S3 접속 정보를 저장했습니다: ${info.name} (키는 암호화해 저장${info.locked ? ' · 잠금 암호로도 잠금' : ''})`);
      done?.(info, thenConnect);
    } catch (err) {
      problem.textContent = String(err);
    } finally {
      saving = false;
    }
  };
  const go = make('button', 'ssh-go', '저장하고 접속');
  go.onclick = () => save(true);
  const only = make('button', '', '저장만');
  only.onclick = () => save(false);
  const buttons = make('div', 'ssh-buttons');
  buttons.append(go, only);
  const checks = make('div', 's3-checks');
  checks.append(pathStyle.row, v2.row, insecure.row);
  const form = make('div', 'ssh-form');
  form.append(name.row, endpoint.row, access.row, secret.row, region.row, start.row, checks, lock.row, lockAgain.row, problem, buttons);
  form.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' && ev.target.tagName === 'INPUT' && ev.target.type !== 'checkbox') { ev.preventDefault(); go.click(); }
  });
  const close = openPopup(editing ? `S3 접속 정보 고치기 — ${fill.name}` : 'S3 접속 정보 더하기', [
    form,
    make('p', 'note', '접근 키와 비밀 키는 암호화해서 이 PC에 저장합니다(%APPDATA%\\cypher\\s3). 파일에는 원문이 적히지 않고, 저장한 뒤에는 이 창에도 다시 보이지 않습니다. '
      + '기본은 Windows 계정으로 잠급니다: 파일만 가져가서는(다른 PC, 다른 계정) 풀 수 없습니다. '
      + '잠금 암호를 정하면 그 암호로 한 번 더 잠급니다: 암호는 저장하지 않고 접속할 때마다 묻으며, 이 PC에서 같은 계정으로 도는 다른 프로그램도, 사용자 폴더를 통째로 가져간 사람도 암호 없이는 풀 수 없습니다(짧은 암호는 대입해 보면 뚫리므로 길게 정하세요). '
      + '다만 키는 요청에 서명할 때 원문이 필요해서 "되돌릴 수 없는" 모양으로는 저장할 수 없고, 접속해 있는 동안에는 Cypher의 메모리에 풀린 채로 있습니다. '
      + '잠금 암호 없이는 Windows 계정이 곧 열쇠입니다: 같은 계정으로 도는 악성 프로그램이나, 사용자 폴더 전체와 Windows 암호를 함께 가진 사람은 풀 수 있습니다.'),
  ]);
  // 저장하지 않고 닫아도(×, Esc) 칸에 적어 둔 키와 암호가 창 안에 남지 않게 지운다.
  // (닫힘 알림은 늦게 오므로, 앞선 창이 닫힌 알림이 이 창이 뜬 뒤에 오면 이 창의 것이 아니다: 다음 알림을 기다린다.)
  const dialog = document.getElementById('popup');
  const wipe = () => {
    if (dialog.open && form.isConnected) { dialog.addEventListener('close', wipe, { once: true }); return; }
    for (const input of [access.input, secret.input, lock.input, lockAgain.input]) input.value = '';
  };
  dialog.addEventListener('close', wipe, { once: true });
  (editing ? name : endpoint).input.focus();
}

// ───────────────────────── 항목 화면 ─────────────────────────

export function createS3(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  /** 고른 접속 정보 { id, name, endpoint, locked, start … }. 아직 고르지 않았으면 null. */
  let conn = typeof saved.conn === 'string' && saved.conn ? { id: saved.conn, name: typeof saved.name === 'string' ? saved.name : '' } : null;
  let known = [];                 // 저장해 둔 접속 정보들
  let knownError = '';
  let handle = null;              // 접속해 있으면 접속 번호
  let connecting = false;
  let gateProblem = '';
  let resume = typeof saved.path === 'string' ? saved.path : ''; // 접속하면 열 곳(지난번에 보던 곳)
  let resumeConn = conn?.id || '';                               // 그 자리가 어느 접속 정보의 것인지(다른 저장소를 고르면 쓰지 않는다)
  let at = { bucket: '', prefix: '', starts: '' };
  let rows = [], next = null, loadedAt = 0, listError = '';
  let busy = false;
  let queued = null;              // 읽는 중에 들어온 읽기: { key(어디), fresh(새로 읽기인지) }
  let nudged = 0;                 // 사용자가 목록을 마지막으로 굴리거나 누른 때(스스로 다음 쪽을 읽지 않으려고 본다)
  const cache = new Map();        // 폴더 → { rows, next, at }
  const back = [], forward = [];
  let selected = new Set(), anchor = null, cursor = null;
  let sort = { by: 'name', dir: 1 };
  let filter = '';
  let shown = null;               // 보고 있는 객체 { bucket, key, name, loading, error, data, editing }
  const jobs = new Map();         // 작업 번호 → { event, after, finished, timer }
  let disposed = false;

  const button = (label, tip, action) => {
    const b = make('button', '', label);
    b.title = tip;
    b.onclick = action;
    return b;
  };

  // ── 화면 뼈대 ──
  const $back = button('←', '뒤로 (Alt+←)', () => step(-1));
  const $forward = button('→', '앞으로 (Alt+→)', () => step(1));
  const $up = button('↑', '위 폴더로 (Backspace)', () => goUp());
  const $refresh = button('⟳', '새로 고침: 이 폴더의 목록을 다시 읽기 (F5)', () => refresh());
  const $crumbs = make('div', 's3-crumbs');
  const $address = make('input', 's3-address');
  Object.assign($address, { spellcheck: false, autocomplete: 'off', hidden: true, placeholder: 's3://버킷/폴더/' });
  const $where = make('div', 's3-where');
  $where.title = '지금 있는 곳 · 누르면 주소를 글로 고칩니다 (Ctrl+L) · 마디를 누르면 그 폴더로';
  $where.append($crumbs, $address);
  const $conn = button('', '접속 정보 · 접속 끊기', (ev) => connMenu(ev));
  $conn.classList.add('s3-conn');
  const nav = make('div', 'doc-bar s3-nav');
  nav.append($back, $forward, $up, $refresh, $where, $conn);

  const $filter = make('input', 'tbl-filter s3-filter');
  Object.assign($filter, { placeholder: '거르기 (불러온 줄에서 · Enter: 이 글로 시작하는 것을 서버에서 찾기)', spellcheck: false, autocomplete: 'off' });
  const $upload = button('올리기', '이 PC의 파일·폴더를 지금 폴더에 올리기 (창에 끌어다 놓아도 됩니다)', (ev) => showMenu(ev, [
    { label: '파일 올리기…', action: () => pickUpload(false) },
    { label: '폴더 올리기… (폴더째)', action: () => pickUpload(true) },
  ]));
  const $mkdir = button('새 폴더', '새 폴더 만들기 (버킷 목록에서는 새 버킷)', () => makeFolder());
  const $download = button('내려받기', '고른 것을 이 PC의 폴더로 내려받기', () => downloadSelected());
  const $copy = button('복사', '고른 것을 복사해 두기 (Ctrl+C)', () => copySelected(false));
  const $cut = button('잘라내기', '고른 것을 옮기려고 잘라 두기 (Ctrl+X)', () => copySelected(true));
  const $paste = button('붙여넣기', '복사·잘라 둔 것을 지금 폴더에 놓기 (Ctrl+V)', () => paste());
  const $rename = button('이름 바꾸기', '고른 것의 이름 바꾸기 (F2)', () => renameSelected());
  const $delete = button('삭제', '고른 것을 지우기 (Delete)', () => removeSelected());
  const tools = make('div', 'doc-bar s3-tools');
  tools.append($upload, $mkdir, $download, $copy, $cut, $paste, $rename, $delete, $filter);

  const $gate = make('div', 's3-gate');
  $gate.tabIndex = -1;
  const $head = make('thead');
  const $body = make('tbody');
  const $table = make('table', 'tbl s3-table');
  $table.append($head, $body);
  const $message = make('div', 's3-message');
  const $more = make('button', 's3-more', '');
  $more.onclick = () => load(true);
  const $list = make('div', 's3-list');
  $list.tabIndex = 0;
  $list.append($table, $message, $more);
  const $preview = make('div', 's3-preview');
  $preview.hidden = true;
  const $main = make('div', 's3-main');
  $main.append($list, $preview);
  const $jobs = make('div', 's3-jobs');
  $jobs.hidden = true;
  const $foot = make('div', 's3-foot');
  const root = make('div', 'doc s3');
  root.append(nav, tools, $gate, $main, $jobs, $foot);
  host.append(root);

  const connected = () => handle != null;
  const same = (a, b) => a.bucket === b.bucket && a.prefix === b.prefix && (a.starts || '') === (b.starts || '');
  const cacheKey = (place) => JSON.stringify([place.bucket, place.prefix, place.starts || '']);
  const rowOf = (key) => rows.find((row) => row.key === key);
  /** 고른 줄들(보이는 것만: 거르기로 가려진 줄은 고른 것으로 치지 않는다). */
  const picked = () => ordered().filter((row) => selected.has(row.key));
  /** 묻거나 고르는 창이 떠 있는 사이에 접속이나 있는 곳이 바뀌지 않았는지. */
  const still = (own, place) => handle === own && same(place, at);
  const moved = () => { setStatus('그 사이 다른 곳으로 옮겨 가서 그만두었습니다. 다시 해 주세요.'); return null; };
  const itemsOf = (list) => list.map((row) => ({ key: row.key, size: row.size || 0 }));

  // ── 접속 ──

  async function loadKnown() {
    try {
      known = await invoke('s3_conns');
      knownError = '';
      // 저장해 둔 것의 이름·주소가 바뀌었으면 따라간다.
      const mine = conn && known.find((other) => other.id === conn.id);
      if (mine && !connected()) conn = mine;
    } catch (err) {
      known = [];
      knownError = String(err);
    }
    if (!disposed) { drawGate(); drawBar(); }
  }

  function drawGate() {
    $gate.hidden = connected();
    $main.hidden = !connected();
    tools.hidden = !connected();
    if (connected()) return;
    const box = make('div', 's3-gate-box');
    const exists = conn && known.some((other) => other.id === conn.id);
    if (conn && exists) {
      box.append(make('div', 'empty-title', `⛁ ${conn.name || 'S3 저장소'}`), make('div', 'empty-hint', conn.endpoint || ''));
      const row = make('div', 'empty-actions');
      let password = null;
      if (conn.locked) {
        password = make('input', 'empty-path s3-password');
        Object.assign(password, { type: 'password', placeholder: '잠금 암호', autocomplete: 'off' });
        password.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); ev.stopPropagation(); go.click(); } });
        row.append(password);
      }
      const go = make('button', 's3-go', connecting ? '접속 중…' : '접속');
      go.disabled = connecting;
      go.title = '이 저장소에 접속합니다. 접속해야 요청을 보냅니다.';
      go.onclick = () => {
        if (conn.locked && !password.value) { password.focus(); return; }
        connect(password ? password.value : null);
      };
      row.append(go);
      box.append(row);
      if (gateProblem) box.append(make('div', 's3-problem', gateProblem));
      const others = make('div', 'empty-actions');
      others.append(
        button('접속 정보 고치기…', '주소·키·옵션 고치기', () => showConnForm(conn, (info, thenConnect) => {
          conn = info;
          onChange();
          loadKnown().then(() => { if (thenConnect && !info.locked) connect(null); });
        })),
        // 고르는 화면은 저장해 둔 접속 정보를 다시 읽어 보여 준다(다른 S3 항목에서 더하거나 지운 것이 보이게. 이 PC의 파일만 읽는다).
        button('다른 접속 정보…', '다른 저장소 고르기', () => { conn = null; gateProblem = ''; drawGate(); drawBar(); onChange(); loadKnown(); }),
      );
      box.append(others, make('div', 'empty-hint s3-quiet', 'Cypher는 스스로 접속하지 않습니다(다시 켠 뒤에도). [접속]을 눌렀을 때만 저장소에 요청을 보냅니다.'));
    } else {
      box.append(make('div', 'empty-title', 'S3 호환 저장소 탐색기'));
      box.append(make('div', 'empty-hint', conn ? '이 항목이 쓰던 접속 정보가 없습니다(지워졌습니다). 다른 접속 정보를 고르세요.' : '접속할 저장소를 고르세요. 누르면 접속합니다.'));
      if (knownError) box.append(make('div', 's3-problem', knownError));
      const list = make('div', 's3-conns');
      for (const other of known) {
        const row = make('div', 'row');
        const use = make('button', 'row-title', `⛁ ${other.name}`);
        use.append(make('span', 'template-summary', `${other.endpoint}${other.locked ? ' · 잠금 암호' : ''}`));
        use.title = `${other.name}\n${other.endpoint}\n클릭: ${other.locked ? '고르기(잠금 암호를 묻습니다)' : '접속'}`;
        use.onclick = () => { conn = other; gateProblem = ''; onChange(); drawBar(); if (other.locked) drawGate(); else connect(null); };
        const edit = make('button', 'close', '✎');
        edit.title = '이 접속 정보 고치기';
        edit.onclick = () => showConnForm(other, (info, thenConnect) => {
          if (thenConnect) { conn = info; gateProblem = ''; onChange(); }
          loadKnown().then(() => { if (thenConnect && !info.locked) connect(null); });
        });
        const remove = make('button', 'close', '×');
        remove.title = '저장해 둔 접속 정보 지우기';
        remove.onclick = async () => {
          if (!confirm(`S3 접속 정보 "${other.name}"을(를) 지울까요?\n저장해 둔 키도 함께 지웁니다(저장소의 내용은 그대로입니다).`)) return;
          await invoke('s3_conn_remove', { id: other.id }).catch((err) => setStatus(String(err)));
          loadKnown();
        };
        row.append(use, edit, remove);
        list.append(row);
      }
      box.append(list);
      const row = make('div', 'empty-actions');
      row.append(button('+ 접속 정보 더하기…', '저장소 주소와 키를 적어 저장하기', () => showConnForm(null, (info, thenConnect) => {
        conn = info;
        gateProblem = '';
        onChange();
        loadKnown().then(() => { if (thenConnect && !info.locked) connect(null); });
      })));
      box.append(row);
    }
    $gate.replaceChildren(box);
  }

  /** 접속한다(사용자가 눌렀을 때만 부른다). 키를 푼 뒤 지난번에 보던 곳(없으면 처음 열 곳, 버킷 목록)을 연다. */
  async function connect(password = null) {
    if (!conn || connected() || connecting) return false;
    connecting = true;
    gateProblem = '';
    drawGate();
    try {
      const result = await invoke('s3_connect', { id: conn.id, password });
      if (disposed) { invoke('s3_disconnect', { handle: result.handle }); return false; }
      handle = result.handle;
      conn = result.info;
      live.set(handle, onJob);
      connecting = false;
      drawGate();
      drawBar();
      onChange();
      // 같은 저장소에 다시 접속하면 보던 곳을, 처음이거나 다른 저장소면 그 접속 정보의 "처음 열 곳"을 연다.
      const first = (resumeConn === conn.id && resume) || conn.start || 's3://';
      resumeConn = conn.id;
      await openAddress(first, { record: false });
      focus();
      return true;
    } catch (err) {
      gateProblem = String(err);
      return false;
    } finally {
      connecting = false;
      if (!disposed) drawGate();
    }
  }

  /** 접속을 끊는다: 앱이 풀어 둔 키를 버리고, 돌던 작업을 멈춘다. 보던 곳은 기억해 둔다. */
  function disconnect({ ask = true } = {}) {
    if (!connected()) return true;
    const running = [...jobs.values()].filter((job) => !job.finished).length;
    if (ask && running && !confirm(`진행 중인 작업 ${running}개를 멈추고 접속을 끊을까요?`)) return false;
    if (ask && !leavePreview()) return false;
    resume = addressOf(at);
    resumeConn = conn.id;
    dropAsks(view);
    invoke('s3_disconnect', { handle }).catch(() => {});
    live.delete(handle);
    handle = null;
    cache.clear();
    back.length = 0;
    forward.length = 0;
    rows = [];
    next = null;
    shown = null;
    for (const job of jobs.values()) clearTimeout(job.timer);
    jobs.clear();
    selected = new Set();
    drawPreview();
    drawJobs();
    drawGate();
    drawBar();
    draw();
    onChange();
    return true;
  }

  function connMenu(ev) {
    if (!connected()) return;
    showMenu(ev, [
      { caption: `${conn.name} · ${conn.endpoint}` },
      { label: '접속 끊기', action: () => disconnect() },
      { label: '이 주소 복사 (s3://…)', action: () => copyText(addressOf(at)) },
    ]);
  }

  const copyText = (text) => invoke('clipboard_write', { text }).then(() => setStatus(`복사했습니다: ${text}`)).catch((err) => setStatus(String(err)));

  // ── 이동 ──

  /** 그곳으로 간다. 읽어 둔 폴더면 다시 읽지 않는다(`fresh`면 다시 읽는다). */
  function go(target, { record = true, fresh = false, select = null } = {}) {
    if (!connected()) return false;
    const place = { bucket: target.bucket || '', prefix: target.bucket ? target.prefix || '' : '', starts: target.bucket ? target.starts || '' : '' };
    if (record && !same(place, at)) {
      back.push(at);
      if (back.length > 100) back.shift();
      forward.length = 0;
    }
    const changed = !same(place, at);
    at = place;
    if (changed || select) {
      selected = new Set(select ? [select] : []);
      cursor = select;
      anchor = select;
    }
    if (changed) {
      filter = '';
      $filter.value = '';
      // 다른 곳의 목록은 처음부터 보인다(앞 폴더의 스크롤 자리가 남아 끝에 닿은 것으로 보이지 않게).
      nudged = 0;
      $list.scrollTop = 0;
    }
    const hit = fresh ? null : cache.get(cacheKey(at));
    listError = '';
    if (hit) {
      ({ rows, next } = hit);
      loadedAt = hit.at;
      draw();
    } else {
      if (changed) { rows = []; next = null; }
      // 읽기를 먼저 시작해 "읽는 중"으로 그린다(고른 줄은 목록이 올 때까지 지키고 있다).
      load(false, fresh);
      draw();
    }
    drawBar();
    onChange();
    return true;
  }

  function step(dir) {
    const from = dir < 0 ? back : forward, to = dir < 0 ? forward : back;
    if (!from.length || !connected()) return;
    to.push(at);
    go(from.pop(), { record: false });
  }

  function goUp() {
    if (!at.bucket) return;
    if (at.starts) return go({ bucket: at.bucket, prefix: at.prefix });
    if (!at.prefix) return go({ bucket: '' }, { select: at.bucket });
    return go({ bucket: at.bucket, prefix: parentOf(at.prefix) }, { select: at.prefix });
  }

  const refresh = () => go(at, { record: false, fresh: true });

  /**
   * 주소 줄에 적은 곳을 연다. `/`로 끝나지 않는 키는 객체일 수 있으니 한 번 물어보고(요청 하나), 객체면 그 폴더를 열어
   * 그 객체를 골라 내용을 보여 준다. 아니면 폴더로 본다.
   */
  async function openAddress(text, opts = {}) {
    const parsed = parseAddress(text);
    if (!parsed) { setStatus('주소는 s3://버킷/폴더/ 모양으로 적습니다.'); return false; }
    if (!parsed.bucket) return go({ bucket: '' }, opts);
    if (!parsed.key || parsed.key.endsWith('/')) return go({ bucket: parsed.bucket, prefix: parsed.key }, opts);
    // 끝이 `*`면 그 글로 시작하는 것만 본다(`s3://버킷/폴더/앞글자*`).
    if (parsed.key.endsWith('*')) {
      const key = parsed.key.slice(0, -1);
      const cut = key.lastIndexOf('/') + 1;
      return go({ bucket: parsed.bucket, prefix: key.slice(0, cut), starts: key.slice(cut) }, opts);
    }
    let isObject = false;
    try { isObject = await invoke('s3_exists', { handle, bucket: parsed.bucket, key: parsed.key }); } catch { /* 물어볼 수 없으면 폴더로 본다 */ }
    if (disposed || !connected()) return false;
    if (!isObject) return go({ bucket: parsed.bucket, prefix: `${parsed.key}/` }, opts);
    const folder = parsed.key.slice(0, parsed.key.lastIndexOf('/') + 1);
    if (!go({ bucket: parsed.bucket, prefix: folder }, { ...opts, select: parsed.key })) return false;
    preview({ key: parsed.key, name: leafOf(parsed.key), bucket: parsed.bucket });
    return true;
  }

  function remember(place, entry) {
    const key = cacheKey(place);
    cache.delete(key);
    cache.set(key, entry);
    while (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value);
  }

  /**
   * 그 버킷에서 그 폴더와 그 아래의 읽어 둔 목록을 버린다. `parents`면 그 위의 폴더들 것도 버린다
   * (새로 생긴 폴더가 위 폴더의 목록에 줄로 보여야 할 때).
   */
  function forget(bucket, prefix = '', parents = true) {
    for (const key of [...cache.keys()]) {
      const [b, p] = JSON.parse(key);
      if (b === bucket && (p.startsWith(prefix) || (parents && prefix.startsWith(p)))) cache.delete(key);
    }
  }

  /**
   * 지금 있는 곳의 목록을 한 쪽 읽는다(`more`면 다음 쪽). 요청은 한 번에 하나만 보낸다: 읽는 중에 다른 곳으로 가면
   * 그 요청이 끝난 뒤에 지금 있는 곳을 읽는다.
   */
  async function load(more, fresh = false) {
    if (!connected()) return;
    if (busy) { if (!more) queued = { key: cacheKey(at), fresh }; return; }
    if (more && !next) return;
    const place = at, own = handle, token = more ? next : null;
    // 다음 쪽은 지금 읽어 둔 목록에 잇는다. 읽는 사이 그 목록이 버려지거나 새로 읽혔으면 이 쪽은 버린다
    // (앞쪽 없이 뒤쪽만 든 목록이 생기지 않게).
    const before = more ? cache.get(cacheKey(place)) : null;
    if (more && !before) return;
    busy = true;
    drawFoot();
    try {
      let page;
      if (!place.bucket) {
        const buckets = await invoke('s3_buckets', { handle: own });
        page = { entries: buckets.map((b) => ({ key: b.name, name: b.name, folder: true, bucket: true, size: 0, modified: b.created, class: '' })), next: null };
      } else {
        page = await invoke('s3_list', { handle: own, bucket: place.bucket, prefix: place.prefix + (place.starts || ''), base: place.prefix, next: token, limit: PAGE });
      }
      if (disposed || handle !== own) return;
      if (more && cache.get(cacheKey(place)) !== before) return;
      const entry = before || { rows: [], next: null, at: 0 };
      if (more) {
        const have = new Set(entry.rows.map((row) => row.key));
        for (const row of page.entries) if (!have.has(row.key)) entry.rows.push(row);
      } else {
        entry.rows = page.entries;
      }
      entry.next = page.next;
      entry.at = Date.now();
      remember(place, entry);
      if (same(place, at)) {
        ({ rows, next } = entry);
        loadedAt = entry.at;
        listError = '';
      }
    } catch (err) {
      if (!disposed && handle === own && same(place, at)) {
        listError = String(err);
        if (!more) { rows = []; next = null; }
      }
    } finally {
      // 읽기가 끝난 상태로 그린다("읽는 중" 표시가 남지 않게). 그 사이 다른 곳으로 갔으면 지금 있는 곳을 이어서 읽는다.
      busy = false;
      const again = queued;
      queued = null;
      // 기다리던 읽기는 지금도 그곳에 있고, 새로 읽으라고 했거나 아직 읽어 둔 것이 없을 때만 한다
      // (그 사이 읽어 둔 폴더로 옮겨 갔으면 다시 읽지 않는다).
      if (again && connected() && !disposed && again.key === cacheKey(at) && (again.fresh || !cache.has(again.key))) load(false, again.fresh);
      if (!disposed) draw();
    }
  }

  // ── 그리기 ──

  function drawBar() {
    const on = connected();
    $back.disabled = !on || !back.length;
    $forward.disabled = !on || !forward.length;
    $up.disabled = !on || !at.bucket;
    $refresh.disabled = !on;
    $conn.textContent = conn ? `⛁ ${conn.name || 'S3'}${on ? '' : ' (접속 안 함)'}` : '⛁ S3';
    $conn.disabled = !on;
    $crumbs.replaceChildren();
    const crumb = (text, place, tip) => {
      const b = make('button', 's3-crumb', text);
      b.title = tip;
      b.onclick = (ev) => { ev.stopPropagation(); go(place); };
      b.disabled = !on;
      return b;
    };
    $crumbs.append(crumb('s3://', { bucket: '' }, '버킷 목록'));
    if (at.bucket) {
      $crumbs.append(crumb(at.bucket, { bucket: at.bucket, prefix: '' }, `s3://${at.bucket}/`));
      let prefix = '';
      for (const part of at.prefix.split('/').filter(Boolean)) {
        prefix += `${part}/`;
        $crumbs.append(make('span', 's3-sep', '›'), crumb(part, { bucket: at.bucket, prefix }, `s3://${at.bucket}/${prefix}`));
      }
      if (at.starts) $crumbs.append(make('span', 's3-sep', '›'), make('span', 's3-starts', `${at.starts}… 로 시작하는 것`));
    }
    if (document.activeElement !== $address) $address.value = addressOf(at);
    $crumbs.scrollLeft = $crumbs.scrollWidth;
  }

  function editAddress() {
    if (!connected()) return;
    $address.value = addressOf(at);
    $crumbs.hidden = true;
    $address.hidden = false;
    $address.focus();
    $address.select();
  }

  function doneAddress() {
    $address.hidden = true;
    $crumbs.hidden = false;
  }

  $where.addEventListener('click', (ev) => { if (ev.target === $where || ev.target === $crumbs || ev.target.classList.contains('s3-sep')) editAddress(); });
  $address.addEventListener('blur', doneAddress);
  $address.addEventListener('keydown', (ev) => {
    ev.stopPropagation();
    if (ev.key === 'Enter') {
      ev.preventDefault();
      const text = $address.value;
      doneAddress();
      $list.focus({ preventScroll: true });
      openAddress(text);
    } else if (ev.key === 'Escape') {
      ev.preventDefault();
      doneAddress();
      $list.focus({ preventScroll: true });
    }
  });

  function ordered() {
    const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
    const list = words.length ? rows.filter((row) => words.every((word) => row.name.toLowerCase().includes(word))) : rows.slice();
    const byName = (a, b) => a.name.localeCompare(b.name, 'ko', { numeric: true });
    const by = sort.by === 'size' ? (a, b) => a.size - b.size
      : sort.by === 'modified' ? (a, b) => String(a.modified || '').localeCompare(String(b.modified || ''))
        : sort.by === 'kind' ? (a, b) => kindText(a).localeCompare(kindText(b), 'ko')
          : byName;
    // 폴더가 먼저, 그 안에서 고른 기준대로.
    return list.sort((a, b) => (b.folder - a.folder) || sort.dir * by(a, b) || byName(a, b));
  }

  function draw() {
    const list = ordered();
    const columns = [['name', at.bucket ? '이름' : '버킷'], ['size', '크기'], ['modified', at.bucket ? '수정한 날짜' : '만든 날짜'], ['kind', '종류']];
    const headRow = make('tr');
    for (const [by, text] of columns) {
      const th = make('th', by === 'size' ? 'num' : '', text);
      if (sort.by === by) th.append(make('span', 'tbl-sort', sort.dir > 0 ? '▲' : '▼'));
      th.onclick = () => { sort = { by, dir: sort.by === by ? -sort.dir : 1 }; draw(); };
      headRow.append(th);
    }
    $head.replaceChildren(headRow);
    // 목록에 없는 줄은 고른 것에서 뺀다. 읽는 중에는 두었다가(가려는 곳에서 고를 줄을 미리 정해 둔다) 목록이 온 뒤에 뺀다.
    const keys = new Set(list.map((row) => row.key));
    if (!busy) for (const key of [...selected]) if (!keys.has(key)) selected.delete(key);
    $body.replaceChildren(...list.map((row) => {
      const tr = make('tr', selected.has(row.key) ? 'sel' : '');
      tr.dataset.key = row.key;
      if (clip?.cut && clip.conn === conn?.id && clip.bucket === at.bucket && clip.items.some((other) => other.key === row.key)) tr.classList.add('cut');
      const name = make('td', 's3-name');
      name.append(make('span', `s3-ico ${row.bucket ? 'bucket' : row.folder ? 'folder' : 'file'}`), make('span', '', row.name));
      name.title = row.bucket ? `s3://${row.key}/` : `s3://${at.bucket}/${row.key}`;
      tr.append(name, make('td', 'num', row.folder ? '' : sizeText(row.size)), make('td', '', dateText(row.modified)), make('td', 's3-kind', kindText(row)));
      return tr;
    }));
    $table.hidden = !list.length;
    const total = rows.length;
    $message.className = `s3-message${listError ? ' fail' : ''}`;
    if (listError) {
      $message.textContent = `${listError}${!at.bucket ? '\n버킷 목록을 볼 권한이 없는 키라면 주소 줄(Ctrl+L)에 s3://버킷이름/ 을 적어 이동하세요. 접속 정보의 "처음 열 곳"에 적어 두면 접속하자마자 그곳을 엽니다.' : ''}`;
    } else if (busy && !total) {
      $message.textContent = '목록을 읽는 중…';
    } else if (!total) {
      $message.textContent = !at.bucket ? '버킷이 없습니다. [새 폴더]로 버킷을 만들 수 있습니다.' : at.starts ? `"${at.starts}"(으)로 시작하는 것이 없습니다.` : '비어 있습니다. 파일을 여기에 끌어다 놓거나 [올리기]로 올리세요.';
    } else if (!list.length) {
      $message.textContent = `불러온 ${count(total)}줄 가운데 "${filter}"이(가) 든 것이 없습니다.${next ? ' 아직 불러오지 않은 줄이 있습니다: Enter를 누르면 이 글로 시작하는 것을 서버에서 찾습니다.' : ''}`;
    } else {
      $message.textContent = '';
    }
    $more.hidden = !next || !!listError;
    $more.textContent = busy ? '불러오는 중…' : `더 불러오기 (다음 ${PAGE}개)`;
    drawFoot();
    drawTools();
  }

  function drawTools() {
    const on = connected();
    const chosen = picked();
    const inBucket = !!at.bucket;
    $upload.disabled = !on || !inBucket;
    $mkdir.disabled = !on;
    $mkdir.textContent = inBucket ? '새 폴더' : '새 버킷';
    $download.disabled = !on || !inBucket || !chosen.length;
    $copy.disabled = $cut.disabled = !on || !inBucket || !chosen.length;
    $paste.disabled = !on || !inBucket || !clip || clip.conn !== conn?.id;
    $rename.disabled = !on || !inBucket || chosen.length !== 1;
    $delete.disabled = !on || !chosen.length;
    for (const tr of $body.children) tr.classList.toggle('sel', selected.has(tr.dataset.key));
  }

  function drawFoot() {
    if (!connected()) { $foot.textContent = conn ? '접속하지 않았습니다.' : ''; return; }
    const parts = [];
    const total = rows.length;
    parts.push(next ? `${count(total)}개 불러옴 (더 있음)` : `${count(total)}개`);
    const chosen = picked();
    if (chosen.length) {
      const bytes = chosen.reduce((sum, row) => sum + (row.folder ? 0 : row.size), 0);
      parts.push(`${count(chosen.length)}개 고름${chosen.some((row) => !row.folder) ? ` · ${sizeText(bytes)}` : ''}`);
    }
    if (filter) parts.push(`거르기: 불러온 줄에서 "${filter}"`);
    if (busy) parts.push('읽는 중…');
    else if (loadedAt) parts.push(`${dateText(loadedAt).slice(11)}에 읽음 (F5: 다시 읽기)`);
    if (clip && clip.conn === conn?.id) parts.push(`${clip.cut ? '잘라 둠' : '복사해 둠'}: ${count(clip.items.length)}개`);
    $foot.textContent = parts.join(' · ');
  }

  // ── 고르기와 키보드 ──

  function setSelection(keys, { keep = false } = {}) {
    if (!keep) selected = new Set();
    for (const key of keys) selected.add(key);
    drawTools();
    drawFoot();
  }

  function reveal(key) {
    const tr = [...$body.children].find((row) => row.dataset.key === key);
    tr?.scrollIntoView({ block: 'nearest' });
  }

  function clickRow(key, ev) {
    const order = ordered().map((row) => row.key);
    if (ev.shiftKey && anchor && order.includes(anchor)) {
      const [from, to] = [order.indexOf(anchor), order.indexOf(key)].sort((a, b) => a - b);
      setSelection(order.slice(from, to + 1), { keep: ev.ctrlKey });
    } else if (ev.ctrlKey) {
      if (selected.has(key)) selected.delete(key); else selected.add(key);
      anchor = key;
      drawTools();
      drawFoot();
    } else {
      anchor = key;
      setSelection([key]);
    }
    cursor = key;
  }

  function moveCursor(delta, extend) {
    const order = ordered().map((row) => row.key);
    if (!order.length) return;
    const from = cursor && order.includes(cursor) ? order.indexOf(cursor) : (delta > 0 ? -1 : order.length);
    const index = Math.max(0, Math.min(order.length - 1, from + delta));
    const key = order[index];
    if (extend && anchor && order.includes(anchor)) {
      const [a, b] = [order.indexOf(anchor), index].sort((x, y) => x - y);
      setSelection(order.slice(a, b + 1));
    } else {
      anchor = key;
      setSelection([key]);
    }
    cursor = key;
    reveal(key);
  }

  /** 줄을 연다: 버킷·폴더는 들어가고, 객체는 내용을 보여 준다. */
  function open(row) {
    if (!row) return;
    if (row.bucket) go({ bucket: row.key, prefix: '' });
    else if (row.folder) go({ bucket: at.bucket, prefix: row.key });
    else preview({ key: row.key, name: row.name, bucket: at.bucket });
  }

  $body.addEventListener('click', (ev) => {
    const tr = ev.target.closest('tr');
    if (tr) clickRow(tr.dataset.key, ev);
  });
  $body.addEventListener('dblclick', (ev) => {
    const tr = ev.target.closest('tr');
    if (tr) open(rowOf(tr.dataset.key));
  });
  $list.addEventListener('click', (ev) => {
    if (!ev.target.closest('tr, button') && !ev.ctrlKey && !ev.shiftKey) setSelection([]);
  });
  $list.addEventListener('contextmenu', (ev) => {
    if (!connected()) return;
    ev.preventDefault();
    const tr = ev.target.closest('tbody tr');
    if (tr && !selected.has(tr.dataset.key)) { anchor = cursor = tr.dataset.key; setSelection([tr.dataset.key]); }
    if (!tr) setSelection([]);
    showMenu(ev, menuItems());
  });
  // 마우스의 뒤로·앞으로 단추
  root.addEventListener('mouseup', (ev) => {
    if (ev.button === 3) { ev.preventDefault(); step(-1); } else if (ev.button === 4) { ev.preventDefault(); step(1); }
  });
  for (const type of ['wheel', 'keydown', 'pointerdown', 'touchstart']) $list.addEventListener(type, () => { nudged = Date.now(); }, { passive: true });
  $list.addEventListener('scroll', () => {
    // 끝까지 내렸을 때만 다음 쪽을 읽는다. 거르는 중에는 줄이 적어 늘 끝에 닿아 있으므로 스스로 읽지 않는다.
    // 사용자가 방금 굴리거나 누른 것이 아니면(줄이 줄어 스크롤 자리가 끝으로 당겨진 것 등) 읽지 않는다.
    if (!next || busy || filter || listError || Date.now() - nudged > 1500) return;
    if ($list.scrollTop + $list.clientHeight >= $list.scrollHeight - 40) load(true);
  });

  function menuItems() {
    const chosen = picked();
    const one = chosen.length === 1 ? chosen[0] : null;
    const inBucket = !!at.bucket;
    const canPaste = inBucket && clip && clip.conn === conn?.id;
    if (!chosen.length) {
      return [
        { caption: addressOf(at) },
        ...(inBucket ? [
          { label: '파일 올리기…', action: () => pickUpload(false) },
          { label: '폴더 올리기…', action: () => pickUpload(true) },
          { label: '새 폴더…', action: () => makeFolder() },
          { label: `붙여넣기${clip?.cut ? ' (옮기기)' : ''}`, action: () => paste(), disabled: !canPaste },
        ] : [{ label: '새 버킷…', action: () => makeFolder() }]),
        { label: '새로 고침 (F5)', action: () => refresh() },
        { label: '이 주소 복사', action: () => copyText(addressOf(at)) },
      ];
    }
    const address = (row) => (row.bucket ? `s3://${row.key}/` : `s3://${at.bucket}/${row.key}`);
    return [
      { caption: one ? one.name : `${count(chosen.length)}개 고름` },
      ...(one ? [{ label: one.folder ? '열기' : '내용 보기', action: () => open(one) }] : []),
      ...(inBucket ? [
        { label: '내려받기…', action: () => downloadSelected() },
        { label: '복사 (Ctrl+C)', action: () => copySelected(false) },
        { label: '잘라내기 (Ctrl+X)', action: () => copySelected(true) },
        ...(one?.folder ? [{ label: '이 폴더 안에 붙여넣기', action: () => paste(one.key), disabled: !canPaste }] : []),
        { label: '이름 바꾸기… (F2)', action: () => renameSelected(), disabled: !one },
      ] : []),
      { label: inBucket ? '삭제… (Delete)' : '버킷 삭제… (비어 있어야 함)', action: () => removeSelected() },
      { label: '주소 복사 (s3://…)', action: () => copyText(chosen.map(address).join('\n')) },
      ...(one && !one.folder ? [{ label: '속성…', action: () => showInfo(one) }] : []),
    ];
  }

  root.addEventListener('keydown', (ev) => {
    if (!connected() || ev.target.closest('input, textarea')) return;
    const key = ev.key;
    const handled = () => { ev.preventDefault(); ev.stopPropagation(); };
    const plain = !ev.ctrlKey && !ev.altKey && !ev.shiftKey && !ev.metaKey;
    // 단추에 초점이 있을 때의 Enter·빈칸은 그 단추를 누르는 것이다.
    if ((key === 'Enter' || key === ' ') && ev.target.closest('button')) return;
    // (새로 고침 F5는 본창이 먼저 받아 이 항목의 refresh를 부른다: main.js)
    if ((ev.ctrlKey && !ev.shiftKey && !ev.altKey && key.toLowerCase() === 'l') || (key === 'F4' && plain)) { handled(); editAddress(); }
    else if (ev.altKey && key === 'ArrowLeft') { handled(); step(-1); }
    else if (ev.altKey && key === 'ArrowRight') { handled(); step(1); }
    else if (ev.altKey && key === 'ArrowUp') { handled(); goUp(); }
    else if (ev.target.closest('.s3-preview')) return;
    else if (key === 'ArrowDown') { handled(); moveCursor(1, ev.shiftKey); }
    else if (key === 'ArrowUp') { handled(); moveCursor(-1, ev.shiftKey); }
    else if (key === 'PageDown') { handled(); moveCursor(15, ev.shiftKey); }
    else if (key === 'PageUp') { handled(); moveCursor(-15, ev.shiftKey); }
    else if (key === 'Home') { handled(); moveCursor(-1e9, ev.shiftKey); }
    else if (key === 'End') { handled(); moveCursor(1e9, ev.shiftKey); }
    else if (key === 'Enter') { handled(); open(rowOf(cursor) || picked()[0]); }
    else if (key === 'Backspace') { handled(); goUp(); }
    else if (key === 'Delete') { handled(); removeSelected(); }
    else if (key === 'F2') { handled(); renameSelected(); }
    else if (key === 'Escape') { if (shown) { handled(); closePreview(); } else if (selected.size) { handled(); setSelection([]); } }
    else if (ev.ctrlKey && !ev.shiftKey && !ev.altKey) {
      const letter = key.toLowerCase();
      if (letter === 'a') { handled(); setSelection(ordered().map((row) => row.key)); }
      // 글자를 골라 둔 것이 있고 목록에 초점이 없으면(보기 창의 글 등) 그 글자를 복사하게 둔다.
      else if (letter === 'c' && (document.activeElement === $list || !String(getSelection()))) { handled(); copySelected(false); }
      else if (letter === 'x') { handled(); copySelected(true); }
      else if (letter === 'v') { handled(); paste(); }
      else if (letter === 'f') { handled(); $filter.focus(); $filter.select(); }
    }
  });

  $filter.addEventListener('input', () => { filter = $filter.value.trim(); draw(); });
  $filter.addEventListener('keydown', (ev) => {
    ev.stopPropagation();
    if (ev.key === 'Escape') { ev.preventDefault(); $filter.value = ''; filter = ''; draw(); $list.focus({ preventScroll: true }); }
    // Enter: 그 글로 시작하는 것만 서버에서 읽는다(요청 하나). 큰 폴더에서 다 불러오지 않고 찾는 길이다.
    if (ev.key === 'Enter' && at.bucket) {
      ev.preventDefault();
      const typed = $filter.value.trim();
      $filter.value = '';
      filter = '';
      // `/`가 들어 있으면 마지막 `/`까지는 폴더, 그 뒤가 "시작하는 글"이다(주소 줄의 `*`와 같은 풀이).
      const cut = typed.lastIndexOf('/') + 1;
      go({ bucket: at.bucket, prefix: at.prefix + typed.slice(0, cut), starts: typed.slice(cut) });
    }
  });

  // ── 작업(올리기·내려받기·지우기·복사·옮기기) ──

  /** 작업을 시작한다. 끝나면 `after(마지막 알림)`을 부른다. */
  async function startJob(command, args, after, where = '') {
    if (!connected()) return null;
    const own = handle;
    let id;
    try {
      id = await invoke(command, { handle: own, ...args });
    } catch (err) {
      setStatus(String(err));
      return null;
    }
    // 기다리는 사이 접속을 끊었으면 그 작업은 앱이 이미 멈췄다(줄에 올리지 않는다).
    if (handle !== own) return null;
    // 알림이 번호보다 먼저 와 있을 수 있다(금방 끝나는 작업).
    const job = jobs.get(id) || { event: null };
    job.where = where;
    job.after = after || (() => {});
    jobs.set(id, job);
    if (job.finished) finish(id, job);
    drawJobs();
    return id;
  }

  function onJob(event) {
    const job = jobs.get(event.job) || {};
    job.event = event;
    jobs.set(event.job, job);
    if (event.state === 'conflict' && !job.asking) askConflict(event, job);
    if (ENDED.includes(event.state)) {
      job.finished = true;
      if (job.after) finish(event.job, job);
    }
    drawJobs();
  }

  function finish(id, job) {
    if (job.closed) return;
    job.closed = true;
    const event = job.event;
    const parts = [];
    if (event.state === 'done') parts.push(`${event.title} 끝: ${count(event.done - event.failed_count - event.skipped)}개`);
    else if (event.state === 'canceled') parts.push(`${event.title} 취소`);
    else parts.push(`${event.title} 실패: ${event.error}`);
    if (event.skipped) parts.push(`건너뜀 ${count(event.skipped)}개`);
    if (event.failed_count) parts.push(`실패 ${count(event.failed_count)}개 (${event.failed[0]})`);
    setStatus(`S3 ${parts.join(' · ')}`);
    try { job.after(event); } catch (err) { console.error(err); }
    // 탈 없이 끝난 작업은 잠시 뒤 줄에서 치운다. 실패가 있으면 사용자가 닫을 때까지 둔다.
    if (event.state !== 'failed' && !event.failed_count) job.timer = setTimeout(() => { jobs.delete(id); drawJobs(); }, JOB_KEEP_MS);
    drawJobs();
  }

  /** 이미 있는 것을 덮어쓰게 됐다: 덮어쓸지, 있는 것은 건너뛸지, 그만둘지 묻는다(물음은 줄을 서서 하나씩 뜬다). */
  function askConflict(event, job) {
    job.asking = true;
    asks.push({
      owner: view,
      event,
      where: job.where || '',
      answer: (value) => {
        job.asking = false;
        invoke('s3_job_answer', { job: event.job, answer: value }).catch(() => {});
      },
    });
    pumpAsks();
  }

  function drawJobs() {
    $jobs.hidden = !jobs.size;
    $jobs.replaceChildren(...[...jobs].filter(([, job]) => job.event).map(([id, job]) => {
      const event = job.event;
      const row = make('div', `s3-job ${event.state}`);
      const ended = ENDED.includes(event.state);
      const ratio = event.bytes_total ? event.bytes / event.bytes_total : event.total ? event.done / event.total : 0;
      const bar = make('progress');
      if (ended) { bar.max = 1; bar.value = event.state === 'done' ? 1 : ratio; } else if (event.total || event.bytes_total) { bar.max = 1; bar.value = Math.min(1, ratio); }
      const state = event.state === 'conflict' ? '확인을 기다리는 중'
        : event.state === 'done' ? `끝${event.failed_count ? ` · 실패 ${count(event.failed_count)}개` : ''}${event.skipped ? ` · 건너뜀 ${count(event.skipped)}개` : ''}`
          : event.state === 'failed' ? `실패: ${event.error}`
            : event.state === 'canceled' ? '취소함'
              : `${count(event.done)}${event.total ? `/${count(event.total)}` : ''}개${event.bytes_total ? ` · ${sizeText(event.bytes)}/${sizeText(event.bytes_total)}` : ''}`;
      const text = make('span', 's3-job-text', `${event.title} · ${state}${!ended && event.current ? ` · ${event.current}` : ''}`);
      text.title = [text.textContent, ...event.failed].join('\n');
      const act = ended
        ? button('닫기', '이 줄 치우기', () => { clearTimeout(job.timer); jobs.delete(id); drawJobs(); })
        : button('취소', '이 작업 멈추기', () => invoke('s3_job_cancel', { job: id }).catch(() => {}));
      row.append(text, bar, act);
      return row;
    }));
  }

  /**
   * 작업으로 저장소가 바뀌었다: 바뀐 폴더들([버킷, 폴더])과 그 아래의 읽어 둔 목록을 버리고, 보고 있던 곳이 그 가운데 있으면
   * 다시 읽는다(요청 하나). 보고 있던 곳이 바뀐 곳이 아니면 아무것도 읽지 않는다.
   */
  function touched(...places) {
    for (const [bucket, prefix] of places) forget(bucket, prefix, false);
    if (connected() && !cache.has(cacheKey(at))) go(at, { record: false, fresh: true });
  }

  /**
   * 보기 창이 보여 주는 객체가 곧 지워지거나 옮겨진다(키들 가운데 그 객체이거나, 그 객체가 든 폴더가 있다).
   * 고치던 글이 있으면 버려도 되는지 묻고(안 된다면 거짓), 보기 창을 닫는다.
   */
  function releasePreview(bucket, keys) {
    if (!shown || shown.bucket !== bucket) return true;
    if (!keys.some((key) => shown.key === key || (key.endsWith('/') && shown.key.startsWith(key)))) return true;
    if (!leavePreview()) return false;
    shown = null;
    drawPreview();
    return true;
  }

  function upload(paths, prefix = at.prefix, bucket = at.bucket) {
    if (!connected() || !bucket) { setStatus('올릴 곳을 먼저 여세요(버킷 안에서 올립니다).'); return null; }
    if (!paths.length) return null;
    return startJob('s3_upload', { bucket, prefix, paths }, (event) => { if (event.changed) touched([bucket, prefix]); }, `올릴 곳: s3://${bucket}/${prefix}`);
  }

  async function pickUpload(folder) {
    if (!connected() || !at.bucket) return;
    // 올릴 곳은 고르기 창을 띄우기 전에 정해 둔다. 고르는 사이 다른 곳으로 가도 처음 그곳에 올리고,
    // 접속이 바뀌었으면(끊었거나 다른 저장소에 붙었으면) 올리지 않는다.
    const own = handle, bucket = at.bucket, prefix = at.prefix;
    const paths = folder
      ? [await api.pickFolder(`S3 저장소에 올릴 폴더 → s3://${bucket}/${prefix}`)].filter(Boolean)
      : await invoke('s3_pick_files', { start: null }).catch(() => []);
    if (paths.length && handle === own) upload(paths, prefix, bucket);
  }

  async function downloadSelected(list = picked()) {
    if (!connected() || !at.bucket || !list.length) return null;
    const own = handle, bucket = at.bucket, base = at.prefix, items = itemsOf(list);
    const dest = await api.pickFolder(`내려받을 폴더 (${count(items.length)}개)`);
    if (!dest || handle !== own) return null;
    return download(items, dest, bucket, base);
  }

  /** 보기 창이 보여 주는 객체를 내려받는다(목록이 어디에 있든 그 객체의 버킷·폴더에서). */
  async function downloadShown() {
    const target = shown;
    if (!target || !connected()) return null;
    const own = handle;
    const dest = await api.pickFolder(`내려받을 폴더 (${target.name})`);
    if (!dest || handle !== own) return null;
    return download([{ key: target.key, size: target.data?.info.size || 0 }], dest, target.bucket, target.key.slice(0, target.key.lastIndexOf('/') + 1));
  }

  const download = (items, dest, bucket = at.bucket, base = at.prefix) => startJob('s3_download', { bucket, base, items, dest }, () => {}, `받을 곳: ${dest}`);

  /** 고른 것을 지운다. 먼저 확인을 받는다. 폴더는 그 아래의 모든 객체를 지운다. */
  async function removeSelected(list = picked(), { confirmed = false } = {}) {
    if (!connected() || !list.length) return null;
    if (!at.bucket) return removeBuckets(list, confirmed);
    const folders = list.filter((row) => row.folder);
    const names = list.slice(0, 8).map((row) => `· ${row.name}${row.folder ? '/' : ''}`).join('\n');
    const rest = list.length > 8 ? `\n… 그 밖에 ${count(list.length - 8)}개` : '';
    const warn = folders.length ? `\n\n폴더 ${count(folders.length)}개는 그 안의 모든 객체까지 지웁니다.` : '';
    const own = handle, bucket = at.bucket, place = at;
    if (!confirmed && !confirm(`${addressOf(at)}\n\n${count(list.length)}개를 지울까요?\n${names}${rest}${warn}\n\n지운 것은 되돌릴 수 없습니다.`)) return null;
    if (!still(own, place)) return moved();
    if (!releasePreview(bucket, list.map((row) => row.key))) return null;
    const keys = new Set(list.map((row) => row.key));
    return startJob('s3_delete', { bucket, items: itemsOf(list) }, (event) => {
      // 지운 폴더 아래의 읽어 둔 목록은 버린다(그 위 폴더들의 것은 둔다).
      for (const row of folders) forget(bucket, row.key, false);
      if (event.state === 'done' && !event.failed_count && cache.has(cacheKey(place))) {
        // 다 지웠으면 다시 읽지 않고 줄만 뺀다(같은 폴더를 "~로 시작하는 것"으로 읽어 둔 목록에서도).
        for (const [key, entry] of cache) {
          const [b, p] = JSON.parse(key);
          if (b === bucket && p === place.prefix) entry.rows = entry.rows.filter((row) => !keys.has(row.key));
        }
        if (at.bucket === bucket && at.prefix === place.prefix) { rows = cache.get(cacheKey(at))?.rows ?? rows.filter((row) => !keys.has(row.key)); draw(); }
        if (shown && shown.bucket === bucket && (keys.has(shown.key) || folders.some((row) => shown.key.startsWith(row.key)))) { shown = null; drawPreview(); }
      } else if (event.changed) {
        touched([bucket, place.prefix]);
      }
    });
  }

  async function removeBuckets(list, confirmed) {
    const own = handle;
    if (!confirmed && !confirm(`버킷 ${count(list.length)}개를 지울까요?\n${list.map((row) => `· ${row.name}`).join('\n')}\n\n비어 있는 버킷만 지워집니다. 지운 것은 되돌릴 수 없습니다.`)) return null;
    for (const row of list) {
      // 지우는 사이 접속이 바뀌었으면(끊고 다른 저장소에 붙었으면) 남은 것은 지우지 않는다.
      if (handle !== own) return null;
      try {
        await invoke('s3_bucket_remove', { handle: own, bucket: row.key });
        setStatus(`버킷을 지웠습니다: ${row.name}`);
      } catch (err) {
        setStatus(`버킷 "${row.name}"을(를) 지우지 못했습니다: ${err}`);
      }
    }
    if (handle !== own) return null;
    cache.delete(cacheKey({ bucket: '', prefix: '', starts: '' }));
    if (!at.bucket) refresh();
    return true;
  }

  /** 새 폴더를 만든다(버킷 목록에서는 새 버킷). 새로 더하는 일이라 따로 확인하지 않는다. */
  async function makeFolder(given = null) {
    if (!connected()) return false;
    const inBucket = !!at.bucket;
    // 만들 곳은 이름을 묻기 전에 정해 둔다(묻는 사이 다른 곳으로 가도 처음 그곳에 만든다).
    const own = handle, place = at.starts ? { bucket: at.bucket, prefix: at.prefix, starts: '' } : at;
    const name = given ?? await api.ask(inBucket ? `새 폴더의 이름 (s3://${place.bucket}/${place.prefix})` : '새 버킷의 이름 (소문자·숫자·-·.)', inBucket ? '새 폴더' : 'my-bucket');
    if (!name || handle !== own) return false;
    if (inBucket && (name.includes('/') || name === '.' || name === '..')) { setStatus('폴더 이름으로 쓸 수 없습니다(`/`, `.`, `..`).'); return false; }
    try {
      if (inBucket) {
        const key = `${place.prefix}${name}/`;
        await invoke('s3_mkdir', { handle: own, bucket: place.bucket, key });
        if (handle !== own) return true;
        const entry = cache.get(cacheKey(place));
        if (entry && !entry.rows.some((row) => row.key === key)) entry.rows.push({ key, name, folder: true, size: 0, modified: '', etag: '', class: '' });
        if (same(place, at)) { if (entry) rows = entry.rows; selected = new Set([key]); cursor = anchor = key; draw(); reveal(key); }
        setStatus(`폴더를 만들었습니다: ${name}`);
      } else {
        await invoke('s3_bucket_create', { handle: own, bucket: name });
        if (handle !== own) return true;
        cache.delete(cacheKey(place));
        if (same(place, at)) go(at, { record: false, fresh: true, select: name });
        setStatus(`버킷을 만들었습니다: ${name}`);
      }
      return true;
    } catch (err) {
      setStatus(`${inBucket ? '폴더' : '버킷'}를 만들지 못했습니다: ${err}`);
      return false;
    }
  }

  /** 이름을 바꾼다(S3에는 이름 바꾸기가 없어 복사한 뒤 원본을 지운다). 확인을 받는다. */
  async function renameSelected(row = picked()[0], given = null, { confirmed = false } = {}) {
    if (!connected() || !at.bucket || !row || row.bucket) return null;
    // 무엇을 어디서 바꾸는지는 묻기 전에 정해 둔다. 묻는 사이 다른 곳으로 갔으면 그만둔다.
    const own = handle, place = at, bucket = at.bucket;
    // 그 줄이 든 폴더(이름에 `/`가 든 줄도 제 폴더 안에서 이름만 바뀐다).
    const folder = row.key.replace(/[^/]*\/?$/, '');
    const old = leafOf(row.key);
    const name = given ?? await api.ask(`"${old}"의 새 이름`, old, old);
    if (!name || name === old) return null;
    if (!still(own, place) || !rowOf(row.key)) return moved();
    if (name.includes('/') || name === '.' || name === '..') { setStatus('이름으로 쓸 수 없습니다(`/`, `.`, `..`).'); return null; }
    const note = row.folder ? '\n\n폴더는 안의 모든 객체를 새 이름 아래로 복사한 뒤 원본을 지웁니다(객체가 많으면 오래 걸립니다).' : '\n\n(복사한 뒤 원본을 지웁니다.)';
    if (!confirmed && !confirm(`s3://${bucket}/${folder}\n\n"${old}"의 이름을 "${name}"(으)로 바꿀까요?${note}`)) return null;
    if (!still(own, place)) return moved();
    if (!releasePreview(bucket, [row.key])) return null;
    const plan = { bucket, base: folder, items: itemsOf([row]), to_bucket: bucket, to_prefix: folder, remove: true, rename: name };
    return startJob('s3_copy', { plan }, (event) => {
      if (event.changed) touched([bucket, folder]);
    }, `s3://${bucket}/${folder}`);
  }

  function copySelected(cut) {
    const list = picked();
    if (!connected() || !at.bucket || !list.length) return;
    clip = { conn: conn.id, bucket: at.bucket, base: at.prefix, items: itemsOf(list), cut };
    setStatus(`${cut ? '잘라 두었습니다' : '복사해 두었습니다'}: ${count(list.length)}개 · 놓을 폴더에서 붙여넣기 (Ctrl+V)`);
    draw();
  }

  /** 복사·잘라 둔 것을 `prefix`(기본은 지금 폴더)에 놓는다. 옮기기(잘라내기)는 원본을 지우므로 확인을 받는다. */
  async function paste(prefix = at.prefix, { confirmed = false } = {}) {
    if (!connected() || !at.bucket) return null;
    if (!clip) { setStatus('복사하거나 잘라 둔 것이 없습니다.'); return null; }
    if (clip.conn !== conn.id) { setStatus('다른 저장소에서 복사한 것은 붙여 넣을 수 없습니다(같은 접속 정보끼리만 됩니다).'); return null; }
    const from = clip, toBucket = at.bucket;
    if (from.cut) {
      if (from.bucket === toBucket && from.base === prefix) { setStatus('같은 폴더로는 옮길 수 없습니다.'); return null; }
      const own = handle, place = at;
      if (!confirmed && !confirm(`${count(from.items.length)}개를 여기로 옮길까요?\n\n에서: s3://${from.bucket}/${from.base}\n으로: s3://${toBucket}/${prefix}\n\n옮기면 원래 자리의 것은 지워집니다.`)) return null;
      if (!still(own, place)) return moved();
      if (!releasePreview(from.bucket, from.items.map((item) => item.key))) return null;
    }
    const plan = { bucket: from.bucket, base: from.base, items: from.items, to_bucket: toBucket, to_prefix: prefix, remove: from.cut, rename: null };
    // 같은 폴더에 복사해 붙이면 "이름 - 복사본"으로 놓는다(하나일 때). 여럿이면 놓을 수 없다고 알린다.
    if (!from.cut && from.bucket === toBucket && from.base === prefix) {
      if (from.items.length !== 1) { setStatus('같은 폴더에 여러 개를 복사해 붙일 수는 없습니다. 다른 폴더에서 붙여 넣으세요.'); return null; }
      const leaf = leafOf(from.items[0].key);
      const dot = from.items[0].key.endsWith('/') ? -1 : leaf.lastIndexOf('.');
      plan.rename = dot > 0 ? `${leaf.slice(0, dot)} - 복사본${leaf.slice(dot)}` : `${leaf} - 복사본`;
    }
    if (from.cut) clip = null;
    return startJob('s3_copy', { plan }, (event) => {
      if (!event.changed) return;
      if (from.cut) touched([from.bucket, from.base], [toBucket, prefix]); else touched([toBucket, prefix]);
    }, `놓을 곳: s3://${toBucket}/${prefix}`);
  }

  /** 창에 끌어다 놓은 파일·폴더를 올린다. 폴더 줄 위에 놓으면 그 폴더 안에 올린다. */
  function dropFiles(paths, target = null) {
    if (!connected()) { setStatus('S3 저장소에 접속한 뒤에 올릴 수 있습니다.'); return null; }
    if (!at.bucket) { setStatus('버킷 안으로 들어간 뒤에 올릴 수 있습니다.'); return null; }
    const tr = target?.closest?.('.s3-table tbody tr');
    const row = tr ? rowOf(tr.dataset.key) : null;
    return upload(paths, row?.folder && !row.bucket ? row.key : at.prefix);
  }

  // ── 객체 하나 보기 ──

  /** 고치던 글이 있으면 버려도 되는지 묻는다. 계속해도 되면 참. */
  function leavePreview() {
    if (!shown?.editing || shown.area.value === shown.data.text) return true;
    return confirm(`"${shown.name}"에 저장하지 않은 고침이 있습니다. 버릴까요?`);
  }

  function closePreview() {
    if (!leavePreview()) return false;
    shown = null;
    drawPreview();
    $list.focus({ preventScroll: true });
    return true;
  }

  /** 객체의 내용을 읽어 보여 준다: 글이면 글로, 그림이면 그림으로, 그 밖에는 정보만. */
  async function preview(target) {
    if (!connected() || !leavePreview()) return;
    const own = { bucket: target.bucket, key: target.key, name: target.name, loading: true, error: '', data: null, editing: false, area: null };
    shown = own;
    drawPreview();
    try {
      own.data = await invoke('s3_preview', { handle, bucket: own.bucket, key: own.key });
    } catch (err) {
      own.error = String(err);
    }
    own.loading = false;
    if (shown === own && !disposed) drawPreview();
  }

  function drawPreview() {
    $preview.hidden = !shown;
    if (!shown) { $preview.replaceChildren(); return; }
    const head = make('div', 's3-preview-head');
    const title = make('b', '', shown.name);
    title.title = `s3://${shown.bucket}/${shown.key}`;
    head.append(title);
    const body = make('div', 's3-preview-body');
    const data = shown.data;
    if (shown.loading) body.append(make('div', 's3-message', '내용을 읽는 중…'));
    else if (shown.error) body.append(make('div', 's3-message fail', shown.error));
    else if (data.kind === 'text') {
      const editable = !data.truncated && !data.lossy;
      if (shown.editing) {
        const area = make('textarea', 's3-text');
        area.spellcheck = false;
        area.value = shown.area ? shown.area.value : data.text;
        shown.area = area;
        body.append(area);
        head.append(
          button('저장', '고친 내용으로 덮어쓰기 (확인을 받습니다 · Ctrl+S)', () => saveText()),
          button('고치기 취소', '고친 것을 버리고 보기로', () => { if (leavePreview()) { shown.editing = false; shown.area = null; drawPreview(); } }),
        );
        area.addEventListener('keydown', (ev) => {
          if (ev.ctrlKey && ev.key.toLowerCase() === 's') { ev.preventDefault(); ev.stopPropagation(); saveText(); }
        });
      } else {
        body.append(make('pre', 's3-text', data.text));
        const edit = button('고치기', editable ? '이 글을 고쳐 저장하기' : data.truncated ? '앞부분만 읽은 글은 고칠 수 없습니다(내려받아 고치세요)' : '읽지 못한 글자가 있어 고칠 수 없습니다', () => { shown.editing = true; drawPreview(); shown.area?.focus(); });
        edit.disabled = !editable;
        head.append(edit, button('글 복사', '보이는 글을 복사', () => invoke('clipboard_write', { text: data.text }).then(() => setStatus('글을 복사했습니다.')).catch((err) => setStatus(String(err)))));
      }
    } else if (data.kind === 'image') {
      const img = make('img', 's3-image');
      img.src = data.image;
      img.alt = shown.name;
      body.append(img);
    } else {
      body.append(make('div', 's3-message', data.truncated && /^image\//.test(data.info.content_type || '') ? '그림이 커서 미리 보지 않습니다. 내려받아 보세요.' : '글이나 그림이 아니라 미리 볼 수 없습니다. 내려받아 여세요.'));
    }
    head.append(
      button('내려받기', '이 객체를 이 PC로 내려받기', () => downloadShown()),
      button('×', '닫기 (Esc)', () => closePreview()),
    );
    const info = make('div', 's3-info');
    if (data) {
      const notes = [sizeText(data.info.size), dateText(data.info.modified), data.info.content_type, data.encoding && `${data.encoding} · ${data.eol}`].filter(Boolean);
      if (data.truncated && data.kind === 'text') notes.push('앞 1MB만 보임');
      if (data.lossy) notes.push('읽지 못한 글자 있음');
      for (const [key, value] of data.info.meta) notes.push(`${key}: ${value}`);
      info.textContent = notes.join(' · ');
      info.title = `ETag: ${data.info.etag}`;
    }
    $preview.replaceChildren(head, info, body);
  }

  /** 고친 글을 그 객체에 덮어쓴다. 확인을 받는다. 원래의 종류와 사용자 메타데이터는 그대로 붙인다. */
  async function saveText({ confirmed = false } = {}) {
    const own = shown;
    if (!own?.editing || !connected()) return false;
    // 앞부분만 읽었거나 읽지 못한 글자가 있는 글은 저장하지 않는다(저장하면 뒷부분이나 그 글자가 사라진다).
    if (own.data.kind !== 'text' || own.data.truncated || own.data.lossy || own.saving) return false;
    const text = own.area.value;
    if (text === own.data.text) { setStatus('고친 것이 없습니다.'); return false; }
    const mine = handle;
    if (!confirmed && !confirm(`s3://${own.bucket}/${own.key}\n\n고친 내용으로 덮어쓸까요? 있던 내용은 사라집니다.\n\n(종류·메타데이터·저장 등급은 그대로 붙입니다. 이 객체에 따로 준 공개 권한(ACL)이 있었다면 저장소의 기본값으로 돌아갈 수 있습니다.)`)) return false;
    if (handle !== mine || shown !== own) return false;
    // 저장하는 동안에는 더 고칠 수 없게 한다(그 사이 친 글자는 저장되지 않고 사라진다).
    own.saving = true;
    own.area.readOnly = true;
    try {
      await invoke('s3_put_text', {
        handle: mine, bucket: own.bucket, key: own.key, text,
        encoding: own.data.encoding, eol: own.data.eol, contentType: own.data.info.content_type || '',
        // 붙어 있던 헤더(사용자 메타데이터, Cache-Control 등)와 저장 등급을 그대로 돌려준다. 받을 것은 앱이 고른다.
        meta: [...own.data.info.meta, ...(own.data.info.class ? [['x-amz-storage-class', own.data.info.class]] : [])],
        // 읽었을 때의 확인 값: 그 사이 다른 사람이 바꿨으면 앱이 쓰지 않고 알린다.
        etag: own.data.info.etag || null,
      });
    } catch (err) {
      setStatus(`저장하지 못했습니다: ${err}`);
      own.saving = false;
      if (own.area) own.area.readOnly = false;
      return false;
    }
    own.saving = false;
    setStatus(`저장했습니다: ${own.name}`);
    if (shown !== own) return true;
    own.editing = false;
    own.area = null;
    await preview(own);
    // 목록의 크기·날짜를 맞춘다(다시 읽지 않고 그 줄만).
    const row = same({ bucket: own.bucket, prefix: own.key.slice(0, own.key.lastIndexOf('/') + 1), starts: at.starts }, at) && rowOf(own.key);
    if (row && shown?.data) { row.size = shown.data.info.size; row.modified = shown.data.info.modified; draw(); }
    return true;
  }

  /** 객체의 속성 창(요청 하나: HEAD). */
  async function showInfo(row) {
    const { openPopup } = api;
    const own = handle, bucket = at.bucket;
    let info;
    try { info = await invoke('s3_head', { handle: own, bucket, key: row.key }); } catch (err) { setStatus(String(err)); return; }
    if (handle !== own) return;
    const line = (name, value) => {
      const node = make('div', 'row help');
      node.append(make('kbd', '', name), make('span', 's3-value', value));
      return node;
    };
    openPopup(`속성 — ${row.name}`, [
      line('주소', `s3://${bucket}/${row.key}`),
      line('크기', `${sizeText(info.size)} (${count(info.size)}바이트)`),
      line('수정한 날짜', dateText(info.modified)),
      line('종류', info.content_type || '(없음)'),
      line('ETag', info.etag || '(없음)'),
      ...(info.class ? [line('저장 등급', info.class)] : []),
      ...info.meta.map(([name, value]) => line(name, value)),
    ]);
  }

  // ── 항목으로서 ──

  function focus() {
    if (connected()) $list.focus({ preventScroll: true });
    else ($gate.querySelector('.s3-password') || $gate).focus({ preventScroll: true });
  }

  const title = () => {
    if (!conn) return 'S3';
    const parsed = !connected() && resumeConn === conn.id ? parseAddress(resume) : null;
    const place = parsed ? { bucket: parsed.bucket, prefix: parsed.key.slice(0, parsed.key.lastIndexOf('/') + 1) } : at;
    const where = place.bucket ? (place.prefix ? leafOf(place.prefix) : place.bucket) : '';
    return `${conn.name || 'S3'}${where ? ` · ${where}` : ''}`;
  };

  drawBar();
  drawGate();
  draw();
  const view = {
    // 저장해 둔 접속 정보의 목록만 읽는다(이 PC의 파일). 저장소에는 아무것도 보내지 않는다.
    ready: loadKnown(),
    focus,
    title,
    /** 다시 만들 때 쓸 것: 접속 정보의 번호와 이름, 보던 곳. 키는 여기에 없다. */
    state: () => ({ conn: conn?.id || '', name: conn?.name || '', path: connected() ? addressOf(at) : (conn && resumeConn === conn.id ? resume : '') }),
    async close() {
      const running = [...jobs.values()].filter((job) => !job.finished).length;
      if (running && !confirm(`S3 "${title()}"에서 진행 중인 작업 ${running}개를 멈추고 닫을까요?`)) return false;
      return leavePreview();
    },
    dispose() {
      disposed = true;
      dropAsks(view);
      for (const job of jobs.values()) clearTimeout(job.timer);
      if (handle != null) { invoke('s3_disconnect', { handle }).catch(() => {}); live.delete(handle); handle = null; }
    },
    conn: () => conn?.id || '',
    /** 창에 끌어다 놓은 파일·폴더를 받는다(drop.js). */
    dropFiles,
    // 아래는 자동 검증·다른 화면에서 쓰는 창구다.
    connected,
    connect,
    disconnect,
    address: () => addressOf(at),
    openAddress,
    go,
    refresh,
    load,
    rows: () => rows.map((row) => ({ ...row })),
    shownRows: () => ordered().map((row) => row.name),
    select: (keys) => { anchor = cursor = keys[0] ?? null; setSelection(keys); },
    selection: () => [...selected],
    setFilter: (text) => { $filter.value = text; filter = text.trim(); draw(); },
    sortBy: (by, dir = 1) => { sort = { by, dir: dir < 0 ? -1 : 1 }; draw(); },
    open: (key) => open(rowOf(key)),
    preview,
    previewState: () => (shown ? { key: shown.key, loading: shown.loading, error: shown.error, editing: shown.editing, kind: shown.data?.kind || '', text: shown.data?.text ?? '', truncated: !!shown.data?.truncated, info: shown.data?.info || null } : null),
    edit: (text) => { if (!shown?.data || shown.data.kind !== 'text' || shown.data.truncated || shown.data.lossy) return false; shown.editing = true; drawPreview(); shown.area.value = text; return true; },
    saveText,
    closePreview,
    upload,
    download,
    remove: (keys, opts) => removeSelected(rows.filter((row) => keys.includes(row.key)), opts),
    rename: (key, name, opts) => renameSelected(rowOf(key), name, opts),
    makeFolder,
    copy: (keys, cut = false) => { setSelection(keys); copySelected(cut); },
    paste,
    jobs: () => [...jobs].map(([id, job]) => ({ id, ...(job.event || {}) })),
    answer: (job, value) => invoke('s3_job_answer', { job, answer: value }),
    busy: () => busy,
    problem: () => listError || gateProblem,
    more: () => !!next,
  };
  return view;
}
