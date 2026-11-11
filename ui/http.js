// HTTP 요청 옆 창: .http 파일(IntelliJ 형식)을 여기서 바로 쓰고, 저장하고, 보낸다.
// 요청 파일이 없으면 예시가 채워진 새 파일로 시작한다. 실행 내용은 요청 파일 옆 history.http에 기록되고,
// Claude 세션도 같은 파일을 도구(http_list / http_run)로 실행할 수 있다.
import { diffOpen, toggleDiff } from './extras.js';

const $ = (id) => document.getElementById(id);
const NEW_FILE = '__new__';

let api = null;      // { invoke, make, ask, setStatus, baseName, getActive, getFolder }
let cwd = '';        // 지금 보고 있는 폴더
let dir = '';        // 새 요청 파일이 만들어질 위치
let files = [];      // 이 폴더의 요청 파일 [{ path, name, requests }]
let current = null;  // 편집 중인 파일 { path(저장 전이면 null), name }
let saved = '';      // 편집기에 불러온(또는 마지막으로 저장한) 내용
let template = '';   // 새 파일에 채워 줄 예시

export function initHttp(given) {
  api = given;
  $('http-close').onclick = () => toggleHttp(false);
  $('http-save').onclick = () => save();
  $('http-history').onclick = showHistory;
  $('http-file').onchange = pickFile;
  const editor = $('http-editor');
  editor.addEventListener('input', () => { markDirty(); renderGutter(); });
  editor.addEventListener('scroll', renderGutter);
  new ResizeObserver(renderGutter).observe(editor);
  editor.addEventListener('keydown', (ev) => {
    const key = ev.key.toLowerCase();
    if (ev.ctrlKey && !ev.shiftKey && key === 's') { ev.preventDefault(); save(); }
    else if (ev.ctrlKey && key === 'enter') { ev.preventDefault(); runAtCursor(); }
    else if (key === 'tab') { ev.preventDefault(); document.execCommand('insertText', false, '  '); }
  });
}

export const httpOpen = () => !$('http').hidden;

const folderOf = (s) => (s ? (s.project || s.startCwd) : api.getFolder());
const dirty = () => $('http-editor').value !== saved;
const dirOf = (path) => path.slice(0, path.lastIndexOf('\\'));

function markDirty() {
  // 저장할 것이 있으면(바뀌었거나 아직 파일이 없으면) 저장 버튼을 강조한다.
  $('http-save').classList.toggle('on', dirty() || !current?.path);
}

export async function toggleHttp(open = !httpOpen()) {
  if (open && diffOpen()) toggleDiff(false); // 옆 창은 한 번에 하나만
  if (!open) await saveIfDirty();
  $('http').hidden = !open;
  $('http-button').classList.toggle('on', open);
  if (open) {
    await load(true);
    $('http-editor').focus();
  } else {
    api.getActive()?.term.focus();
  }
}

/** 다른 폴더의 세션으로 옮겨 가면 그 폴더의 요청 파일로 바꾼다. */
export async function showHttp() {
  if (!httpOpen() || folderOf(api.getActive()) === cwd) return;
  await saveIfDirty();
  await load(true);
}

/** 폴더의 요청 파일 목록을 읽는다. `reset`이면 첫 파일(없으면 새 파일)을 편집기에 연다. */
async function load(reset) {
  cwd = folderOf(api.getActive());
  const found = await api.invoke('http_files', { cwd });
  ({ files, template, dir } = found);
  $('http-title').textContent = `HTTP — ${api.baseName(cwd)}`;
  $('http-title').title = cwd;
  if (reset) {
    current = files[0] ? { path: files[0].path, name: files[0].name } : { path: null, name: 'requests.http' };
    await openCurrent();
  }
  renderPicker();
  renderList();
  markDirty();
}

async function openCurrent() {
  let text = template;
  if (current.path) {
    try {
      text = await api.invoke('http_read', { cwd, file: current.path });
    } catch (err) {
      api.setStatus(String(err));
    }
  } else {
    hint('요청 파일이 아직 없어 예시로 시작합니다. 고친 뒤 Ctrl+Enter로 보내거나 Ctrl+S로 저장하면 파일이 만들어집니다.');
  }
  $('http-editor').value = text;
  saved = text;
  renderGutter();
}

const METHOD_LINE = /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+\S/i;
const URL_LINE = /^(https?:\/\/|\{\{)\S/i;

/**
 * 편집기 글에서 "보낼 수 있는 요청"이 시작하는 줄 번호들(0부터).
 * 요청 구역(`###`으로 나뉨)마다, 빈 줄·주석·변수 줄을 건너뛴 첫 줄이 요청 줄이면 그 줄이다.
 */
function requestLines(text) {
  const found = [];
  let waiting = true; // 이 구역의 요청 줄을 아직 못 만났다.
  text.split('\n').forEach((raw, number) => {
    const line = raw.trim();
    if (line.startsWith('###')) { waiting = true; return; }
    if (!waiting || !line || line.startsWith('#') || line.startsWith('//') || /^@[\w.-]+\s*=/.test(line)) return;
    waiting = false;
    if (METHOD_LINE.test(line) || URL_LINE.test(line)) found.push(number);
  });
  return found;
}

/** 편집기 왼쪽 여백에, 보낼 수 있는 요청 줄마다 녹색 ▶ 버튼을 놓는다. 누르면 그 요청을 바로 보낸다. */
function renderGutter() {
  const editor = $('http-editor');
  const style = getComputedStyle(editor);
  const lineHeight = parseFloat(style.lineHeight) || 18;
  const top = parseFloat(style.paddingTop) || 0;
  const height = editor.clientHeight;
  const buttons = [];
  for (const line of requestLines(editor.value)) {
    const y = top + line * lineHeight - editor.scrollTop;
    if (y < -lineHeight || y > height) continue; // 화면 밖
    const button = api.make('button', 'run', '▶');
    button.style.top = `${y}px`;
    button.title = '이 요청 보내기 (Ctrl+Enter)';
    button.onclick = () => runLine(line);
    buttons.push(button);
  }
  $('http-gutter').replaceChildren(...buttons);
}

function hint(text) {
  $('http-view').replaceChildren(api.make('p', 'note', text));
}

function renderPicker() {
  const select = $('http-file');
  const option = (value, text) => {
    const node = api.make('option', '', text);
    node.value = value;
    return node;
  };
  const options = files.map((file) => option(file.path, file.name));
  if (!current.path) options.push(option('', `${current.name} (저장 전)`));
  options.push(option(NEW_FILE, '+ 새 파일…'));
  select.replaceChildren(...options);
  select.value = current.path || '';
}

async function pickFile() {
  const value = $('http-file').value;
  if (value === NEW_FILE) {
    const name = await api.ask('새 요청 파일 이름', '예: 결제 API');
    if (!name) return renderPicker(); // 취소: 고르던 것으로 되돌린다.
    await saveIfDirty();
    current = { path: null, name: name.endsWith('.http') ? name : `${name}.http` };
  } else {
    await saveIfDirty();
    const file = files.find((f) => f.path === value);
    if (!file) return renderPicker();
    current = { path: file.path, name: file.name };
  }
  await openCurrent();
  renderPicker();
  renderList();
  markDirty();
  $('http-editor').focus();
}

/** 편집 중인 파일의 요청 목록. 누르면 그 요청으로 커서를 옮기고 보낸다. */
function renderList() {
  const { make } = api;
  const requests = files.find((f) => f.path === current.path)?.requests ?? [];
  const rows = requests.map((request) => {
    const row = make('li');
    row.append(make('span', 'run', '▶'), make('span', 'method', request.method), make('span', 'path', request.name));
    row.title = `${request.method} ${request.url}\n클릭: 이 요청 보내기`;
    row.onclick = () => {
      const editor = $('http-editor');
      const offset = editor.value.split('\n').slice(0, request.line).join('\n').length + (request.line ? 1 : 0);
      editor.focus();
      editor.setSelectionRange(offset, offset);
      runLine(request.line, row);
    };
    return row;
  });
  if (!rows.length) rows.push(make('li', 'file', 'Ctrl+Enter: 커서가 있는 요청 보내기 · Ctrl+S: 저장'));
  $('http-list').replaceChildren(...rows);
}

async function save() {
  const text = $('http-editor').value;
  try {
    const path = await api.invoke('http_save', {
      cwd, file: current.path, name: current.path ? null : current.name, text,
    });
    saved = text;
    current = { path, name: api.baseName(path) };
    await load(false);
    api.setStatus(`저장했습니다: ${path}`);
    return true;
  } catch (err) {
    api.setStatus(`저장 실패: ${err}`);
    hint(`저장 실패: ${err}`);
    return false;
  }
}

/** 손댄 내용이 있을 때만 저장한다(예시를 그대로 둔 새 파일은 만들지 않는다). */
async function saveIfDirty() {
  if (current && dirty()) await save();
}

function runAtCursor() {
  const editor = $('http-editor');
  runLine(editor.value.slice(0, editor.selectionStart).split('\n').length - 1);
}

/** 그 줄(0부터)이 속한 요청을 보낸다. 보내기 전에 저장한다. */
async function runLine(line, row) {
  const { invoke, make } = api;
  if ((!current.path || dirty()) && !(await save())) return;
  for (const other of $('http-list').children) other.classList.toggle('selected', other === row);
  hint('보내는 중…');
  try {
    const r = await invoke('http_run_at', { file: current.path, line });
    const $view = $('http-view');
    // 보낸 요청 원문: 빈 줄 앞(요청 줄·헤더)은 흐리게, 본문은 그대로 보여 준다.
    const sent = r.request.split('\n');
    const split = sent.indexOf('');
    $view.replaceChildren(
      make('div', 'hunk', '── 보낸 요청 ──'),
      ...sent.slice(0, 2000).map((text, index) => make('div', index === 0 ? '' : index < split ? 'meta' : '', text || ' ')),
      make('div', '', ' '),
      make('div', 'hunk', '── 응답 ──'),
      make('div', r.code < 400 ? 'ok' : 'fail', `${r.status} · ${r.elapsed_ms}ms`),
      ...r.headers.map(([key, value]) => make('div', 'meta', `${key}: ${value}`)),
      make('div', '', ' '),
      ...r.body.split('\n').slice(0, 6000).map((text) => make('div', '', text || ' ')),
    );
    $view.scrollTop = 0;
  } catch (err) {
    hint(String(err));
  }
}

/** 지난 실행의 요청·응답 기록을 아래 칸에 보여 준다. */
async function showHistory() {
  const { invoke, make } = api;
  const file = `${current?.path ? dirOf(current.path) : dir}\\history.http`;
  try {
    const text = await invoke('http_read', { cwd, file });
    const $view = $('http-view');
    $view.replaceChildren(...text.split('\n').slice(-6000).map((line) => {
      const kind = line.startsWith('###') ? 'hunk' : line.startsWith('<>') ? 'ok' : line.startsWith('#') ? 'meta' : '';
      return make('div', kind, line || ' ');
    }));
    $view.scrollTop = $view.scrollHeight; // 최근 것이 아래에 있다.
  } catch {
    hint('아직 실행 기록이 없습니다. 요청을 보내면 여기에 쌓입니다.');
  }
}
