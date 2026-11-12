// HTTP 항목: .http 파일(IntelliJ 형식)을 바로 쓰고, 저장하고, 보낸다. 사이드바의 항목 하나가 창 하나다.
// 요청 파일이 없으면 예시가 채워진 새 파일로 시작한다. 실행 내용은 요청 파일 옆 history.http에 기록되고,
// Claude 세션도 같은 파일을 도구(http_list / http_run)로 실행할 수 있다.

const NEW_FILE = '__new__';
const METHOD_LINE = /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+(https?:\/\/|\{\{)\S/i;
const URL_LINE = /^(https?:\/\/|\{\{)\S/i;
const VAR_LINE = /^@[\p{L}\p{N}_.-]+\s*=/u;
const AUTOSAVE_MS = 1500; // 입력이 멈추고 이만큼 지나면 저장한다(앱을 끌 때 고친 내용을 잃지 않게).

let api = null; // { invoke, make, ask, setStatus, baseName }

export function initHttp(given) {
  api = given;
}

const dirOf = (path) => path.slice(0, path.lastIndexOf('\\'));

/**
 * 편집기 글에서 "보낼 수 있는 요청"이 시작하는 줄 번호들(0부터). 판정은 앱 쪽 해석기와 같게 한다.
 * 요청 구역(줄 맨 앞의 `###`으로 나뉨)마다, 빈 줄·주석·변수 줄을 건너뛴 첫 줄이
 * 주소(`http://`, `https://`)나 변수(`{{`)로 시작하는 요청 줄이면 그 줄이다.
 */
export function requestLines(text) {
  const found = [];
  let waiting = true; // 이 구역의 요청 줄을 아직 못 만났다.
  text.split('\n').forEach((raw, number) => {
    if (raw.startsWith('###')) { waiting = true; return; }
    const line = raw.trim();
    if (!waiting || !line || line.startsWith('#') || line.startsWith('//') || VAR_LINE.test(line)) return;
    waiting = false;
    if (METHOD_LINE.test(line) || URL_LINE.test(line)) found.push(number);
  });
  return found;
}

/**
 * HTTP 항목의 화면을 host 안에 만든다.
 * item.startCwd: 요청 파일을 찾을 폴더 · saved.file: 지난번에 열어 둔 파일
 * onChange: 제목(파일 이름)이 바뀌었을 때 사이드바를 다시 그리게 알린다.
 */
export function createHttp(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, ask, setStatus, baseName } = api;
  const cwd = item.startCwd;
  let dir = '';        // 새 요청 파일이 만들어질 위치
  let files = [];      // 이 폴더의 요청 파일 [{ path, name, requests }]
  let current = null;  // 편집 중인 파일 { path(저장 전이면 null), name }. 처음 읽기가 끝나기 전에는 null.
  let stored = '';     // 편집기에 불러온(또는 마지막으로 저장한) 내용(편집기가 돌려주는 모양 그대로)
  let crlf = false;    // 파일의 줄 끝이 CRLF였는지(저장할 때 되돌린다)
  let template = '';   // 새 파일에 채워 줄 예시
  let autosave = 0;
  let disposed = false;

  const $title = make('span', 'doc-title', `HTTP — ${baseName(cwd)}`);
  $title.title = `요청 파일을 찾는 폴더: ${cwd}`;
  const $file = make('select', 'http-file');
  $file.title = '요청 파일 고르기 / 새 파일 만들기';
  const $save = make('button', 'http-save', '저장');
  $save.title = '저장 (Ctrl+S). 고친 내용은 입력이 멈추면 자동으로도 저장됩니다';
  const $history = make('button', '', '기록');
  $history.title = '지난 실행의 요청·응답 기록 보기';
  const bar = make('div', 'doc-bar');
  bar.append($title, $file, $save, $history);

  const $list = make('ul', 'side-list http-list');
  const $gutter = make('div', 'http-gutter');
  const $editor = make('textarea', 'http-editor');
  Object.assign($editor, { spellcheck: false, autocomplete: 'off', wrap: 'off' });
  const edit = make('div', 'http-edit');
  edit.append($gutter, $editor);
  const $view = make('div', 'side-view http-view');
  const body = make('div', 'http-body');
  body.append(edit, $view);
  const root = make('div', 'doc http');
  root.append(bar, $list, body);
  host.append(root);

  const dirty = () => !!current && $editor.value !== stored;
  // 저장할 것이 있으면(바뀌었거나 아직 파일이 없으면) 저장 버튼을 강조한다.
  const markDirty = () => $save.classList.toggle('on', dirty() || !current?.path);
  const hint = (text) => $view.replaceChildren(make('p', 'note', text));

  /** 편집기에 글을 넣는다. 줄 끝 형식을 기억하고, 편집기가 돌려주는 모양을 "저장된 내용"으로 삼는다. */
  function fill(text) {
    crlf = text.includes('\r\n');
    $editor.value = text;
    stored = $editor.value; // 편집기는 줄 끝을 \n으로 바꿔 돌려준다. 그것과 비교해야 열자마자 "바뀜"이 되지 않는다.
    renderGutter();
  }

  /** 폴더의 요청 파일 목록을 읽는다. `reset`이면 지난번 파일(없으면 첫 파일, 그것도 없으면 새 파일)을 연다. */
  async function load(reset) {
    const found = await invoke('http_files', { cwd });
    if (disposed) return;
    ({ files, template, dir } = found);
    if (reset) {
      const first = files.find((f) => f.path === saved.file) || files[0];
      current = first ? { path: first.path, name: first.name } : { path: null, name: 'requests.http' };
      await openCurrent();
    }
    renderPicker();
    renderList();
    markDirty();
    onChange();
  }

  async function openCurrent() {
    let text = template;
    if (current.path) {
      try {
        text = await invoke('http_read', { cwd, file: current.path });
      } catch (err) {
        setStatus(String(err));
      }
    } else {
      hint('요청 파일이 아직 없어 예시로 시작합니다. 고친 뒤 ▶(Ctrl+Enter)로 보내거나 Ctrl+S로 저장하면 파일이 만들어집니다.');
    }
    fill(text);
  }

  /**
   * 고치지 않은 상태라면 디스크의 지금 내용과 맞춘다(Claude 세션이나 다른 HTTP 항목, 외부 편집기가 파일을 바꿨을 수 있다).
   * 바뀌어서 다시 읽었으면 참. 편집기에 고친 내용이 있으면 건드리지 않는다.
   */
  async function syncFromDisk() {
    if (!current?.path || dirty()) return false;
    try {
      const text = await invoke('http_read', { cwd, file: current.path });
      if (disposed || dirty() || text.replace(/\r\n/g, '\n') === stored) return false;
      const scroll = $editor.scrollTop;
      fill(text);
      $editor.scrollTop = scroll;
      await load(false);
      return true;
    } catch {
      return false; // 파일이 사라졌으면 편집기 내용을 그대로 둔다(저장하면 다시 만들어진다).
    }
  }

  /** 편집기 왼쪽 여백에, 보낼 수 있는 요청 줄마다 녹색 ▶ 버튼을 놓는다. 누르면 그 요청을 바로 보낸다. */
  function renderGutter() {
    const style = getComputedStyle($editor);
    const lineHeight = parseFloat(style.lineHeight) || 18;
    const top = parseFloat(style.paddingTop) || 0;
    const height = $editor.clientHeight;
    const buttons = [];
    for (const line of requestLines($editor.value)) {
      const y = top + line * lineHeight - $editor.scrollTop;
      if (y < -lineHeight || y > height) continue; // 화면 밖
      const button = make('button', 'run', '▶');
      button.style.top = `${y}px`;
      button.title = '이 요청 보내기 (Ctrl+Enter)';
      button.onclick = () => runLine(line);
      buttons.push(button);
    }
    $gutter.replaceChildren(...buttons);
  }

  function renderPicker() {
    if (!current) return;
    const option = (value, text) => {
      const node = make('option', '', text);
      node.value = value;
      return node;
    };
    const options = files.map((file) => option(file.path, file.name));
    if (!current.path) options.push(option('', `${current.name} (저장 전)`));
    options.push(option(NEW_FILE, '+ 새 파일…'));
    $file.replaceChildren(...options);
    $file.value = current.path || '';
  }

  async function pickFile() {
    if (!current) return;
    const value = $file.value;
    let next;
    if (value === NEW_FILE) {
      const name = await ask('새 요청 파일 이름', '예: 결제 API');
      if (!name) return renderPicker(); // 취소: 고르던 것으로 되돌린다.
      const fileName = name.endsWith('.http') ? name : `${name}.http`;
      // 같은 이름의 파일이 이미 있으면 새로 만들지 않고 그 파일을 연다(예시로 덮어쓰지 않게).
      const existing = files.find((f) => f.name.toLowerCase() === fileName.toLowerCase());
      if (existing) setStatus(`같은 이름의 요청 파일이 이미 있어 그 파일을 엽니다: ${existing.name}`);
      next = existing ? { path: existing.path, name: existing.name } : { path: null, name: fileName };
    } else {
      const file = files.find((f) => f.path === value);
      if (!file) return renderPicker();
      next = { path: file.path, name: file.name };
    }
    // 고친 내용을 저장하지 못했으면 옮겨 가지 않는다(편집기 내용이 사라지지 않게).
    if (!(await saveIfDirty())) return renderPicker();
    current = next;
    await openCurrent();
    renderPicker();
    renderList();
    markDirty();
    onChange();
    $editor.focus();
  }

  /** 편집 중인 파일의 요청 목록. 누르면 그 요청으로 커서를 옮기고 보낸다. */
  function renderList() {
    const requests = files.find((f) => f.path === current?.path)?.requests ?? [];
    const rows = requests.map((request, index) => {
      const row = make('li');
      row.append(make('span', 'run', '▶'), make('span', 'method', request.method), make('span', 'path', request.name));
      row.title = `${request.method} ${request.url}\n클릭: 이 요청 보내기`;
      row.onclick = () => runListed(request, index);
      return row;
    });
    if (!rows.length) rows.push(make('li', 'file', 'Ctrl+Enter: 커서가 있는 요청 보내기 · Ctrl+S: 저장'));
    $list.replaceChildren(...rows);
  }

  /**
   * 목록에서 고른 요청을 보낸다. 목록은 마지막으로 저장한 내용 기준이라, 편집기를 고쳤다면 줄 번호가 다를 수 있다.
   * 먼저 저장해 목록을 새로 만든 뒤, 같은 요청(이름과 메서드가 같은 것)을 다시 찾아 그 줄로 보낸다.
   */
  async function runListed(request, index) {
    if (!current) return;
    if ((!current.path || dirty()) && !(await save())) return;
    await syncFromDisk();
    const now = files.find((f) => f.path === current.path)?.requests ?? [];
    const same = (r) => r.name === request.name && r.method === request.method;
    const found = (now[index] && same(now[index])) ? now[index] : now.find(same);
    if (!found) return hint(`"${request.name}" 요청을 찾지 못했습니다. 편집기에서 요청 줄 왼쪽의 ▶로 보내 주세요.`);
    const offset = $editor.value.split('\n').slice(0, found.line).join('\n').length + (found.line ? 1 : 0);
    $editor.focus();
    $editor.setSelectionRange(offset, offset);
    runLine(found.line, $list.children[now.indexOf(found)]);
  }

  async function save(quiet) {
    if (!current) return false;
    clearTimeout(autosave);
    const shown = $editor.value;
    try {
      const path = await invoke('http_save', {
        cwd, file: current.path, name: current.path ? null : current.name,
        text: crlf ? shown.replace(/\n/g, '\r\n') : shown, // 파일의 줄 끝 형식을 지킨다.
      });
      stored = shown;
      current = { path, name: baseName(path) };
      await load(false);
      if (!quiet) setStatus(`저장했습니다: ${path}`);
      return true;
    } catch (err) {
      setStatus(`저장 실패: ${err}`);
      if (!quiet) hint(`저장 실패: ${err}`);
      load(false).catch(() => {}); // 다른 곳에서 파일이 생겼을 수 있으니 목록을 새로 읽어 둔다.
      return false;
    }
  }

  /** 손댄 내용이 있을 때만 저장한다(예시를 그대로 둔 새 파일은 만들지 않는다). 저장할 것이 없거나 저장했으면 참. */
  async function saveIfDirty() {
    return dirty() ? save() : true;
  }

  /** 그 줄(0부터)이 속한 요청을 보낸다. 보내기 전에 저장한다. */
  async function runLine(line, row) {
    if (!current) return;
    if (!current.path || dirty()) {
      if (!(await save())) return;
    } else if (await syncFromDisk()) {
      // 줄 번호는 화면에 보이던 옛 내용 기준이다. 그대로 보내면 다른 요청이 나갈 수 있다.
      return hint('요청 파일이 다른 곳에서 바뀌어 다시 읽었습니다. 보낼 요청을 확인하고 다시 눌러 주세요.');
    }
    for (const other of $list.children) other.classList.toggle('selected', other === row);
    hint('보내는 중…');
    try {
      const r = await invoke('http_run_at', { file: current.path, line });
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
      $view.replaceChildren(...String(err).split('\n').map((text, index) => make(index ? 'div' : 'p', index ? '' : 'note', text)));
    }
  }

  /** 지난 실행의 요청·응답 기록을 응답 칸에 보여 준다. */
  async function showHistory() {
    const file = `${current?.path ? dirOf(current.path) : dir}\\history.http`;
    try {
      const text = await invoke('http_read', { cwd, file });
      $view.replaceChildren(...text.split('\n').slice(-6000).map((line) => {
        const kind = line.startsWith('###') ? 'hunk' : line.startsWith('<>') ? 'ok' : line.startsWith('#') ? 'meta' : '';
        return make('div', kind, line || ' ');
      }));
      $view.scrollTop = $view.scrollHeight; // 최근 것이 아래에 있다.
    } catch {
      hint('아직 실행 기록이 없습니다. 요청을 보내면 여기에 쌓입니다.');
    }
  }

  $save.onclick = () => save();
  $history.onclick = showHistory;
  $file.onchange = pickFile;
  $editor.addEventListener('input', () => {
    markDirty();
    renderGutter();
    clearTimeout(autosave);
    autosave = setTimeout(() => { if (!disposed && dirty()) save(true); }, AUTOSAVE_MS);
  });
  $editor.addEventListener('scroll', renderGutter);
  $editor.addEventListener('focus', () => { syncFromDisk(); });
  const observer = new ResizeObserver(renderGutter);
  observer.observe($editor);
  $editor.addEventListener('keydown', (ev) => {
    const key = ev.key.toLowerCase();
    if (ev.ctrlKey && !ev.shiftKey && key === 's') { ev.preventDefault(); save(); }
    else if (ev.ctrlKey && key === 'enter') {
      ev.preventDefault();
      runLine($editor.value.slice(0, $editor.selectionStart).split('\n').length - 1);
    } else if (key === 'tab' && !ev.ctrlKey) { ev.preventDefault(); document.execCommand('insertText', false, '  '); }
  });

  const ready = load(true).catch((err) => hint(String(err)));

  return {
    ready,
    focus: () => $editor.focus(),
    insert: (text) => { $editor.focus(); document.execCommand('insertText', false, text); },
    /** 사이드바에 보일 기본 이름. */
    title: () => `HTTP · ${current?.name?.replace(/\.http$/, '') || baseName(cwd)}`,
    /** 다시 시작할 때 되살릴 내용. */
    state: () => ({ file: current?.path || saved.file || null }),
    /** 앱이 꺼질 때: 고친 내용을 저장해 본다(끝까지 기다릴 수는 없다). */
    flush: () => { if (dirty()) save(true); },
    /** 닫기 전에: 손댄 내용을 저장한다. 저장하지 못했으면 버리고 닫을지 묻는다. 거짓이면 닫지 않는다. */
    close: async () => (await saveIfDirty())
      || confirm('고친 내용을 저장하지 못했습니다. 저장하지 않고 닫을까요?\n(닫지 않으려면 "취소"를 누른 뒤 내용을 복사해 두세요)'),
    dispose: () => { disposed = true; clearTimeout(autosave); observer.disconnect(); },
  };
}
