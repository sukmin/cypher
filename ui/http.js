// HTTP 항목: .http 파일(IntelliJ 형식)을 바로 쓰고, 저장하고, 보낸다. 사이드바의 항목 하나가 창 하나다.
// 요청 파일이 없으면 예시가 채워진 새 파일로 시작한다. 실행 내용은 요청 파일 옆 history.http에 기록되고,
// Claude 세션도 같은 파일을 도구(http_list / http_run)로 실행할 수 있다.

const NEW_FILE = '__new__';
const METHOD_LINE = /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+(https?:\/\/|\{\{)\S/i;
const URL_LINE = /^(https?:\/\/|\{\{)\S/i;
const VAR_LINE = /^@[\p{L}\p{N}_.-]+\s*=/u;
const AUTOSAVE_MS = 1500; // 고치기 시작하고 늦어도 이만큼 지나면 저장한다(앱을 끌 때 고친 내용을 잃지 않게).
const MAX_SENT_LINES = 2000; // 응답 칸에 보여 주는 보낸 요청의 줄 수
const MAX_BODY_LINES = 6000; // 응답 칸에 보여 주는 응답 본문의 줄 수
// 저장하려는 파일이 읽은 뒤 다른 곳에서 바뀌었을 때 앱이 돌려주는 오류(crates/cypher-app/src/http.rs의 CHANGED_ELSEWHERE와 같은 글).
const CHANGED_ELSEWHERE = '파일이 다른 곳에서 바뀌었습니다';

let api = null; // { invoke, make, ask, setStatus, baseName }

export function initHttp(given) {
  api = given;
}

const dirOf = (path) => path.slice(0, path.lastIndexOf('\\'));

/**
 * 편집기 글에서 "보낼 수 있는 요청"이 시작하는 줄 번호들(0부터). 판정은 앱 쪽 해석기와 같게 한다.
 * 요청 구역(줄 맨 앞의 `###`으로 나뉨)마다, 빈 줄·주석·변수 줄을 건너뛴 첫 줄이
 * 주소(`http://`, `https://`)나 변수(`{{`)로 시작하는 요청 줄이면 그 줄이다.
 * 글 맨 앞의 BOM(U+FEFF)은 앱 쪽과 같이 떼고 본다(첫 줄의 `###`을 알아보도록).
 */
export function requestLines(text) {
  const found = [];
  let waiting = true; // 이 구역의 요청 줄을 아직 못 만났다.
  text.replace(/^﻿/, '').split('\n').forEach((raw, number) => {
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
  let stamp = null;    // 불러온(또는 마지막으로 저장한) 때의 내용 지문. 저장할 때 함께 보내, 그 사이 다른 곳에서 바뀐 파일을 덮어쓰지 않게 한다.
  let crlf = false;    // 파일의 줄 끝이 CRLF였는지(저장할 때 되돌린다)
  let template = '';   // 새 파일에 채워 줄 예시
  let unreadable = ''; // 열어 둔 파일을 읽지 못한 까닭. 비어 있지 않으면 편집·저장을 막는다(읽지 못한 파일을 덮어쓰지 않게).
  let conflict = false; // 파일이 다른 곳에서 바뀐 것을 알고 사용자의 선택을 기다리는 중(자동 저장을 멈춘다).
  let autosave = 0;
  let saving = Promise.resolve(); // 저장은 한 번에 하나씩 한다.
  let runs = 0;        // 실행 순번(늦게 온 옛 응답이 새 응답을 덮지 않게 한다)
  const sending = new Set(); // 보내는 중인 요청의 줄 번호들
  let disposed = false;

  const $title = make('span', 'doc-title', `HTTP — ${baseName(cwd)}`);
  $title.title = `요청 파일을 찾는 폴더: ${cwd}`;
  const $file = make('select', 'http-file');
  $file.title = '요청 파일 고르기 / 새 파일 만들기';
  const $save = make('button', 'http-save', '저장');
  $save.title = '저장 (Ctrl+S) · 자동 저장됨';
  const $history = make('button', '', '기록');
  $history.title = '요청·응답 기록 보기';
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

  const dirty = () => !!current && !unreadable && $editor.value !== stored;
  // 저장할 것이 있으면(바뀌었거나 아직 파일이 없으면) 저장 버튼을 강조한다.
  const markDirty = () => $save.classList.toggle('on', !unreadable && (dirty() || !current?.path));
  const hint = (text) => $view.replaceChildren(make('p', 'note', text));
  const stopAutosave = () => { clearTimeout(autosave); autosave = 0; };

  /** 편집기에 글을 넣는다. 줄 끝 형식과 내용 지문을 기억하고, 편집기가 돌려주는 모양을 "저장된 내용"으로 삼는다. */
  function fill(text, given = null) {
    crlf = text.includes('\r\n');
    $editor.value = text;
    stored = $editor.value; // 편집기는 줄 끝을 \n으로 바꿔 돌려준다. 그것과 비교해야 열자마자 "바뀜"이 되지 않는다.
    stamp = given;
    conflict = false;
    renderGutter();
  }

  /** 읽지 못한 파일은 편집기를 잠가 둔다. 까닭이 비면 푼다. */
  function setUnreadable(reason) {
    unreadable = reason;
    $editor.readOnly = !!reason;
    $save.disabled = !!reason;
  }

  const showUnreadable = () => hint(`"${current?.name}" 파일을 읽지 못해 열지 않았습니다: ${unreadable}\n`
    + '파일은 그대로 두었습니다. 이 상태에서는 고치거나 저장할 수 없습니다. 다른 파일을 고르거나, 문제가 풀린 뒤 편집기를 누르면 다시 읽습니다.');

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
    stopAutosave();
    setUnreadable('');
    if (!current.path) {
      hint('요청 파일이 아직 없어 예시로 시작합니다. 고친 뒤 ▶(Ctrl+Enter)로 보내거나 Ctrl+S로 저장하면 파일이 만들어집니다.');
      return fill(template);
    }
    try {
      const read = await invoke('http_read', { cwd, file: current.path });
      fill(read.text, read.stamp);
    } catch (err) {
      // 읽지 못한 파일에 예시를 채워 두면, 한 글자만 고쳐도 자동 저장이 원래 파일을 예시로 덮어쓴다.
      // 편집기를 비우고 잠가 둔 채 까닭을 보여 준다.
      fill('');
      setUnreadable(String(err));
      setStatus(`요청 파일을 읽지 못했습니다: ${err}`);
      showUnreadable();
    }
  }

  /**
   * 고치지 않은 상태라면 디스크의 지금 내용과 맞춘다(Claude 세션이나 다른 HTTP 항목, 외부 편집기가 파일을 바꿨을 수 있다).
   * 바뀌어서 다시 읽었으면 참. 편집기에 고친 내용이 있으면 건드리지 않는다. 읽지 못해 잠가 둔 파일은 읽히면 풀어 준다.
   */
  async function syncFromDisk() {
    if (!current?.path || dirty()) return false;
    const path = current.path;
    try {
      const read = await invoke('http_read', { cwd, file: path });
      if (disposed || current?.path !== path || dirty()) return false;
      if (!unreadable) {
        if (read.stamp === stamp) return false;
        if (read.text.replace(/\r\n?/g, '\n') === stored) {
          // 보이는 글은 같다(줄 끝 형식만 달라진 경우 등). 지문과 줄 끝 형식만 새로 기억한다.
          stamp = read.stamp;
          crlf = read.text.includes('\r\n');
          return false;
        }
      }
      const reopened = !!unreadable;
      const scroll = $editor.scrollTop;
      setUnreadable('');
      fill(read.text, read.stamp);
      $editor.scrollTop = scroll;
      if (reopened) hint('파일을 다시 읽었습니다.');
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
    // 열어 둔 파일이 바깥에서 지워졌거나 이름이 바뀌면 목록에 없다. 편집기 내용은 남아 있고, 저장하면 다시 만들어진다.
    else if (!files.some((f) => f.path === current.path)) options.push(option(current.path, `${current.name} (없어짐 · 저장하면 다시 만듦)`));
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

  /**
   * 저장한다. `force`면 디스크가 바뀌었는지 확인하지 않고 덮어쓴다. 저장했으면 참.
   * 저장은 한 번에 하나씩 한다: 앞선 저장이 돌려준 지문을 다음 저장이 써야, 제 손으로 바꾼 것을 "다른 곳에서 바뀜"으로 보지 않는다.
   */
  function save(quiet, force) {
    const done = saving.then(() => saveNow(quiet, force));
    saving = done.catch(() => {});
    return done;
  }

  async function saveNow(quiet, force) {
    if (!current) return false;
    stopAutosave();
    if (unreadable) {
      if (!quiet) showUnreadable();
      return false;
    }
    const shown = $editor.value;
    try {
      const written = await invoke('http_save', {
        cwd, file: current.path, name: current.path ? null : current.name,
        text: crlf ? shown.replace(/\n/g, '\r\n') : shown, // 파일의 줄 끝 형식을 지킨다.
        stamp: force ? null : stamp,
      });
      stored = shown;
      stamp = written.stamp;
      conflict = false;
      current = { path: written.path, name: baseName(written.path) };
      await load(false);
      if (!quiet) setStatus(`저장했습니다: ${written.path}`);
      return true;
    } catch (err) {
      if (String(err) === CHANGED_ELSEWHERE) {
        // 옛 내용으로 덮어쓰면 다른 곳(Claude 세션 등)에서 더한 요청이 사라진다. 자동 저장을 멈추고 고르게 한다.
        conflict = true;
        setStatus('파일이 다른 곳에서 바뀌어 저장하지 않았습니다');
        askConflict();
        return false;
      }
      setStatus(`저장 실패: ${err}`);
      if (!quiet) hint(`저장 실패: ${err}`);
      load(false).catch(() => {}); // 다른 곳에서 파일이 생겼을 수 있으니 목록을 새로 읽어 둔다.
      return false;
    }
  }

  /** 파일이 다른 곳에서 바뀌어 저장하지 못했을 때: 디스크 내용을 다시 읽을지, 편집기 내용으로 덮어쓸지 고르게 한다. */
  function askConflict() {
    const note = make('p', 'note', '파일이 다른 곳에서 바뀌었습니다(Claude 세션이나 다른 편집기). 편집기의 내용은 저장하지 않았고, 고를 때까지 자동 저장을 멈춥니다. ');
    const $reload = make('button', '', '다시 읽기(내 변경 버림)');
    const $overwrite = make('button', '', '덮어쓰기');
    $reload.onclick = reloadFromDisk;
    $overwrite.onclick = async () => { if (await save(false, true)) hint('편집기의 내용으로 덮어썼습니다.'); };
    note.append($reload, ' ', $overwrite);
    $view.replaceChildren(note);
  }

  /** 편집기에서 고친 내용을 버리고 디스크의 지금 내용을 다시 읽는다. */
  async function reloadFromDisk() {
    if (!current?.path) return;
    try {
      const read = await invoke('http_read', { cwd, file: current.path });
      if (disposed) return;
      stopAutosave();
      fill(read.text, read.stamp);
      await load(false);
      hint('디스크의 내용을 다시 읽었습니다.');
    } catch (err) {
      hint(`다시 읽지 못했습니다: ${err}\n편집기의 내용은 그대로입니다. 저장(Ctrl+S)하면 다시 고를 수 있습니다.`);
    }
  }

  /** 손댄 내용이 있을 때만 저장한다(예시를 그대로 둔 새 파일은 만들지 않는다). 저장할 것이 없거나 저장했으면 참. */
  async function saveIfDirty() {
    return dirty() ? save() : true;
  }

  /** 줄이 `max`보다 많아 잘랐으면, 몇 줄을 줄였는지 알리는 줄. */
  const cutNote = (lines, max, where) => (lines.length > max ? [make('div', 'meta', `… (이하 ${lines.length - max}줄 생략. ${where})`)] : []);

  /** 그 줄(0부터)이 속한 요청을 보낸다. 보내기 전에 저장한다. */
  async function runLine(line, row) {
    // 보내는 중인 요청을 또 누른 것은 무시한다(같은 요청이 두 번 나가지 않게).
    if (!current || sending.has(line)) return;
    if (unreadable) return showUnreadable();
    sending.add(line);
    let turn = 0; // 보내기 직전에 받는 순번
    // 다른 요청을 그 뒤에 보냈거나 항목이 닫혔으면, 늦게 온 이 응답(또는 오류)은 그리지 않는다.
    const stale = () => disposed || (turn !== 0 && turn !== runs);
    try {
      if (!current.path || dirty()) {
        if (!(await save())) return;
      } else if (await syncFromDisk()) {
        // 줄 번호는 화면에 보이던 옛 내용 기준이다. 그대로 보내면 다른 요청이 나갈 수 있다.
        return hint('요청 파일이 다른 곳에서 바뀌어 다시 읽었습니다. 보낼 요청을 확인하고 다시 눌러 주세요.');
      }
      if (disposed) return;
      turn = ++runs;
      for (const other of $list.children) other.classList.toggle('selected', other === row);
      hint('보내는 중…');
      const r = await invoke('http_run_at', { file: current.path, line });
      if (stale()) return;
      // 보낸 요청 원문: 빈 줄 앞(요청 줄·헤더)은 흐리게, 본문은 그대로 보여 준다.
      const sent = r.request.split('\n');
      const split = sent.indexOf('');
      const received = r.body.split('\n');
      $view.replaceChildren(
        make('div', 'hunk', '── 보낸 요청 ──'),
        ...sent.slice(0, MAX_SENT_LINES).map((text, index) => make('div', index === 0 ? '' : index < split ? 'meta' : '', text || ' ')),
        ...cutNote(sent, MAX_SENT_LINES, '요청 본문 전체는 기록 파일 history.http 참고'),
        make('div', '', ' '),
        make('div', 'hunk', '── 응답 ──'),
        make('div', r.code < 400 ? 'ok' : 'fail', `${r.status} · ${r.elapsed_ms}ms`),
        // 리다이렉트를 따라갔으면 응답을 준 최종 주소
        ...(r.final_url ? [make('div', 'meta', `→ ${r.final_url} (리다이렉트를 따라감)`)] : []),
        ...r.headers.map(([key, value]) => make('div', 'meta', `${key}: ${value}`)),
        make('div', '', ' '),
        ...received.slice(0, MAX_BODY_LINES).map((text) => make('div', '', text || ' ')),
        // 기록 파일도 긴 응답은 앞부분만 남긴다. 전체가 거기 있다고 말하지 않는다.
        ...cutNote(received, MAX_BODY_LINES, '기록 파일 history.http에도 긴 응답은 앞부분만 남습니다'),
        ...(r.truncated ? [make('div', 'meta', '… (응답이 5MB를 넘어 앞부분만 받았습니다)')] : []),
      );
      $view.scrollTop = 0;
    } catch (err) {
      if (stale()) return;
      $view.replaceChildren(...String(err).split('\n').map((text, index) => make(index ? 'div' : 'p', index ? '' : 'note', text)));
    } finally {
      sending.delete(line);
    }
  }

  /** 지난 실행의 요청·응답 기록을 응답 칸에 보여 준다. */
  async function showHistory() {
    const file = `${current?.path ? dirOf(current.path) : dir}\\history.http`;
    try {
      const { text } = await invoke('http_read', { cwd, file });
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
    // 걸려 있지 않을 때만 건다. 입력마다 다시 걸면 쉬지 않고 고치는 동안에는 내내 저장되지 않는다.
    // 파일이 다른 곳에서 바뀌어 선택을 기다리는 중에는 걸지 않는다.
    if (autosave || conflict) return;
    autosave = setTimeout(() => {
      autosave = 0;
      if (!disposed && !conflict && dirty()) save(true);
    }, AUTOSAVE_MS);
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
    /** 폴더의 요청 파일 목록을 다시 읽는다(같은 그룹의 Claude 세션이 이 폴더에 새 요청 파일을 만들었을 때). 열어 둔 파일과 고치던 내용은 그대로 둔다. */
    reload: () => load(false).catch((err) => hint(String(err))),
    /** 앱이 꺼질 때: 고친 내용을 저장한다. 돌려주는 약속은 저장이 끝나면 끝난다(고친 것이 없으면 값 없음). */
    flush: () => (dirty() ? save(true) : undefined),
    /** 닫기 전에: 손댄 내용을 저장한다. 저장하지 못했으면 버리고 닫을지 묻는다. 거짓이면 닫지 않는다. */
    close: async () => (await saveIfDirty())
      || confirm('고친 내용을 저장하지 못했습니다. 저장하지 않고 닫을까요?\n(닫지 않으려면 "취소"를 누른 뒤 내용을 복사해 두세요)'),
    dispose: () => { disposed = true; stopAutosave(); observer.disconnect(); },
  };
}
