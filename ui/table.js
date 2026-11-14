// 표 항목: CSV·TSV·JSON 파일을 표로 보고 정렬하고 거른다(고치지는 않는다).
//
// 파일은 문서 항목과 같은 명령으로 읽고(reader_read), 화면에 보이는 동안 바뀌었는지 살펴(reader_stamp)
// 바뀌면 정렬·거르기·보던 자리를 지킨 채 다시 읽는다. 그래서 Claude 세션이 뽑아 쓰는 결과 파일을 옆에 띄워 두고 볼 수 있다.
//
// - CSV·TSV: 구분자(쉼표, 탭, 쌍반점, 세로줄)는 첫 줄에서 알아본다. 큰따옴표로 감싼 값 안의 구분자·줄 바꿈·"" 를 읽는다.
// - JSON: 객체의 배열(열 = 키), 배열의 배열, 값의 배열. 객체 하나면 그 안의 첫 배열을 쓰고, 없으면 키·값 두 열로 보인다.
// - .jsonl·.ndjson: 한 줄에 JSON 하나.

let api = null; // { invoke, make, setStatus, showMenu, baseName, toClaude(item, path, ev), openMemo(path) }

export function initTable(given) {
  api = given;
}

// ───────────────────────── 파일 읽기 ─────────────────────────

const MAX_ROWS = 100000;      // 이보다 많은 줄은 앞부분만 보여 준다.
const MAX_COLUMNS = 200;      // 이보다 많은 열은 앞부분만 보여 준다.
const CHUNK = 200;            // 한 번에 그리는 줄 수(스크롤하면 이어서 그린다)
const MAX_CELL = 300;         // 칸에 보이는 글자 수(전체는 고르면 아래에 보인다)
const FILTER_MS = 150;        // 거르기 글을 치는 동안 기다리는 시간
const CHECK_MS = 1500;        // 파일이 바뀌었는지 살피는 간격
const COPY_ROWS = 5000;       // 한 번에 복사하는 줄 수
const MARKDOWN_ROWS = 200;    // Markdown 표로 복사하는 줄 수

const extOf = (path) => (/\.([^.\\/]+)$/.exec(path)?.[1] || '').toLowerCase();

/** 구분자로 나눈 글을 줄과 칸으로 읽는다. 큰따옴표로 감싼 값은 구분자·줄 바꿈을 품을 수 있고, ""는 따옴표 하나다. 빈 줄은 뺀다. */
export function parseDelimited(text, delimiter = ',') {
  const rows = [];
  const total = text.length;
  if (!total) return rows;
  let row = [];
  let at = 0;
  const push = () => { if (row.length > 1 || row[0] !== '') rows.push(row); row = []; };
  for (;;) {
    let value = '';
    if (text[at] === '"') {
      let from = at + 1;
      for (;;) {
        const quote = text.indexOf('"', from);
        if (quote < 0) { value += text.slice(from); from = total; break; } // 닫히지 않은 따옴표: 끝까지가 값이다.
        value += text.slice(from, quote);
        if (text[quote + 1] === '"') { value += '"'; from = quote + 2; } else { from = quote + 1; break; }
      }
      at = from;
    }
    // 따옴표 없는 값(또는 닫는 따옴표 뒤에 붙은 글자)을 구분자나 줄 끝까지 읽는다.
    let end = at;
    while (end < total && text[end] !== delimiter && text[end] !== '\n') end += 1;
    value += text.slice(at, end);
    at = end;
    row.push(value);
    if (at >= total) { push(); break; }
    if (text[at] === delimiter) { at += 1; continue; }
    push();
    at += 1;
    if (at >= total) break;
  }
  return rows;
}

/** 첫 줄(따옴표 밖)에 가장 많이 나오는 구분자. 없으면 쉼표. */
export function detectDelimiter(text) {
  const line = text.slice(0, 20000).split('\n').find((row) => row.trim()) || '';
  const counts = { ',': 0, '\t': 0, ';': 0, '|': 0 };
  let quoted = false;
  for (const ch of line) {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch in counts) counts[ch] += 1;
  }
  const [best, times] = Object.entries(counts).sort((a, b) => b[1] - a[1])[0];
  return times ? best : ',';
}

/** JSON 값 하나를 칸에 보일 글로. */
function cellText(value) {
  if (value == null) return '';
  if (typeof value === 'string') return value;
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

/** JSON 값을 표로. → { columns(없으면 null: 첫 줄을 제목으로 쓸 수 있다), rows, note } */
export function tableFromJson(value) {
  let list = value, note = '';
  if (!Array.isArray(value)) {
    if (value && typeof value === 'object') {
      const found = Object.entries(value).find(([, inner]) => Array.isArray(inner) && inner.length);
      if (!found) return { columns: ['키', '값'], rows: Object.entries(value).map(([key, inner]) => [key, cellText(inner)]), note: '객체 하나라 키와 값으로 보여 줍니다' };
      [note, list] = [`"${found[0]}" 배열`, found[1]];
    } else {
      return { columns: ['값'], rows: [[cellText(value)]], note: '' };
    }
  }
  if (list.length && list.every(Array.isArray)) return { columns: null, rows: list.map((row) => row.map(cellText)), note };
  const columns = [], index = new Map();
  const plain = '값'; // 객체가 아닌 값이 섞여 있으면 이 열에 넣는다.
  const columnOf = (name) => {
    if (!index.has(name)) { index.set(name, columns.length); columns.push(name); }
    return index.get(name);
  };
  const rows = list.map((entry) => {
    const row = [];
    if (entry && typeof entry === 'object' && !Array.isArray(entry)) {
      for (const [key, inner] of Object.entries(entry)) row[columnOf(key)] = cellText(inner);
    } else {
      row[columnOf(plain)] = cellText(entry);
    }
    return row;
  });
  return { columns, rows, note };
}

/**
 * 파일의 글을 표로 읽는다. `header`: 첫 줄을 제목으로 쓸지(구분자 글과 배열의 배열에만 뜻이 있다).
 * → { columns, rows(칸 수를 맞춘 글자 배열들), numeric(열마다 숫자 열인지), note, hasHeaderChoice }
 */
export function readTable(text, path, header = true) {
  const ext = extOf(path);
  let columns = null, rows = [], note = '', hasHeaderChoice = true;
  if (ext === 'json' || ext === 'jsonl' || ext === 'ndjson') {
    let value;
    if (ext === 'json') {
      value = JSON.parse(text);
    } else {
      value = [];
      text.split('\n').forEach((line, at) => {
        if (!line.trim()) return;
        try { value.push(JSON.parse(line)); } catch (err) { throw new Error(`${at + 1}번째 줄을 JSON으로 읽지 못했습니다: ${err.message}`); }
      });
    }
    ({ columns, rows, note } = tableFromJson(value));
    hasHeaderChoice = columns == null;
  } else {
    const delimiter = ext === 'tsv' ? '\t' : detectDelimiter(text);
    rows = parseDelimited(text, delimiter);
  }
  if (columns == null) {
    if (header && rows.length) columns = rows.shift().map((name) => name.trim());
    else columns = [];
  }
  const width = Math.min(MAX_COLUMNS, rows.reduce((max, row) => Math.max(max, row.length), columns.length));
  const notes = note ? [note] : [];
  if (rows.some((row) => row.length > MAX_COLUMNS) || columns.length > MAX_COLUMNS) notes.push(`열이 많아 앞의 ${MAX_COLUMNS}개만 보여 줍니다`);
  if (rows.length > MAX_ROWS) {
    notes.push(`줄이 많아 앞의 ${MAX_ROWS.toLocaleString('ko-KR')}줄만 보여 줍니다(전체 ${rows.length.toLocaleString('ko-KR')}줄)`);
    rows = rows.slice(0, MAX_ROWS);
  }
  columns = Array.from({ length: width }, (_, at) => columns[at] || `열 ${at + 1}`);
  rows = rows.map((row) => Array.from({ length: width }, (_, at) => row[at] ?? ''));
  // 숫자 열: 빈 칸이 아닌 값이 모두 숫자(쉼표 자릿수 포함)인 열. 오른쪽으로 붙이고 크기로 정렬한다.
  const sample = rows.slice(0, 2000);
  const numeric = columns.map((_, at) => {
    const filled = sample.map((row) => row[at]).filter((value) => value !== '');
    return filled.length > 0 && filled.every((value) => numberOf(value) != null);
  });
  return { columns, rows, numeric, note: notes.join(' · '), hasHeaderChoice };
}

/** 칸의 글을 숫자로(쉼표 자릿수·끝의 %를 받아 준다). 숫자가 아니면 null. */
function numberOf(value) {
  const text = value.trim().replace(/,/g, '').replace(/%$/, '');
  if (!text || !/^[-+]?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?$/i.test(text)) return null;
  return Number(text);
}

// ───────────────────────── 화면 ─────────────────────────

/**
 * 표 항목의 화면을 host 안에 만든다.
 * saved: { path(보던 파일), header(첫 줄을 제목으로), filter(거르기 글), sort: { name(열 이름), dir(1 | -1) } }
 */
export function createTable(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  let path = typeof saved.path === 'string' ? saved.path : '';
  let header = saved.header !== false;
  let filter = typeof saved.filter === 'string' ? saved.filter : '';
  let sort = saved.sort && typeof saved.sort.name === 'string' ? { name: saved.sort.name, dir: saved.sort.dir === -1 ? -1 : 1 } : null;
  let raw = null;              // 마지막으로 읽은 글(첫 줄을 제목으로 쓸지 바꿀 때 다시 읽지 않으려고 둔다)
  let table = { columns: [], rows: [], numeric: [], note: '', hasHeaderChoice: true };
  let order = [];              // 거르고 정렬한 뒤의 줄 번호들
  let drawn = 0;               // 그려 둔 줄 수
  let picked = null;           // 고른 칸 { row(원래 줄 번호), col }
  let seen;                    // 마지막으로 읽어 본(읽으려 한) 파일의 고친 시각
  let loaded = false, shown = false, checking = false, loading = 0, loadSeq = 0, disposed = false;
  let filterTimer = 0;

  const button = (label, tip, action) => {
    const b = make('button', '', label);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const $title = make('span', 'doc-title');
  const $note = make('span', 'reader-note');
  const $info = make('span', 'tbl-info');
  const $filter = make('input', 'tbl-filter');
  Object.assign($filter, { value: filter, placeholder: '거르기 (낱말을 빈칸으로 나눠 적으면 모두 든 줄만 · Ctrl+F)', spellcheck: false, autocomplete: 'off' });
  const $header = button('첫 줄은 제목', '첫 줄을 열 이름으로 쓸지', () => { header = !header; rebuild(); onChange(); });
  const tools = [
    button('새로 고침', '다시 읽기 (바뀌면 자동 갱신)', () => reload()),
    $header,
    $filter,
    $info,
    button('복사', '보이는 줄을 복사', (ev) => showMenu(ev, [
      { caption: `거르고 정렬한 ${order.length.toLocaleString('ko-KR')}줄` },
      { label: `탭으로 나눈 글로 복사 (${COPY_ROWS.toLocaleString('ko-KR')}줄까지 · 스프레드시트에 붙여넣기)`, action: () => copyRows('tsv') },
      { label: `Markdown 표로 복사 (${MARKDOWN_ROWS}줄까지)`, action: () => copyRows('markdown') },
    ])),
    button('Claude에 넣기', '파일 경로를 Claude 세션 입력창에 넣기', (ev) => api.toClaude?.(item, path, ev)),
    button('메모로 편집', '메모 항목으로 열어 고치기', () => api.openMemo?.(path)),
    button('탐색기에서 보기', '탐색기에서 보기', () => invoke('link_open', { target: path, cwd: item.cwd || '', openFile: false, line: null, col: null }).catch((err) => setStatus(String(err)))),
  ];
  const bar = make('div', 'doc-bar');
  bar.append($title, $note, button('파일 열기', 'CSV·TSV·JSON 파일 열기 (Ctrl+O)', () => pick()), ...tools);

  const $head = make('thead');
  const $body = make('tbody');
  const $table = make('table', 'tbl');
  $table.append($head, $body);
  const $empty = make('div', 'reader-empty',
    '[파일 열기]로 CSV·TSV·JSON 파일을 고르면 표로 보여 줍니다. 열 이름을 누르면 정렬하고, 거르기 칸에 낱말을 적으면 그 낱말이 든 줄만 남깁니다. 파일이 바뀌면 표도 따라 바뀝니다.');
  const $scroll = make('div', 'tbl-scroll');
  $scroll.tabIndex = 0;
  $scroll.append($empty, $table);
  const $detail = make('div', 'tbl-detail');
  $detail.hidden = true;
  const root = make('div', 'doc tabular');
  root.append(bar, $scroll, $detail);
  host.append(root);

  const title = () => (path ? api.baseName(path) : '표');
  const count = (n) => n.toLocaleString('ko-KR');
  const note = (text, fail = false) => {
    $note.textContent = text;
    $note.title = text;
    $note.classList.toggle('fail', fail);
  };

  function refreshBar() {
    $title.textContent = title();
    $title.title = path || '표';
    for (const tool of tools) tool.hidden = !path;
    $header.hidden = !path || !table.hasHeaderChoice;
    $header.classList.toggle('on', header);
    $empty.hidden = !!path;
    $table.hidden = !path;
    const all = table.rows.length;
    $info.textContent = !loaded ? '' : order.length === all ? `${count(all)}줄 · ${table.columns.length}열` : `${count(order.length)} / ${count(all)}줄 · ${table.columns.length}열`;
  }

  // ── 거르기·정렬·그리기 ──

  /** 거르기와 정렬을 적용해 보일 줄의 차례를 정한다. */
  function arrange() {
    const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
    order = [];
    for (let at = 0; at < table.rows.length; at += 1) {
      if (words.length) {
        const joined = table.rows[at].join('\u0001').toLowerCase();
        if (!words.every((word) => joined.includes(word))) continue;
      }
      order.push(at);
    }
    const col = sort ? table.columns.indexOf(sort.name) : -1;
    if (col >= 0) {
      const { dir } = sort;
      const numeric = table.numeric[col];
      const collator = new Intl.Collator('ko', { numeric: true, sensitivity: 'base' });
      order.sort((a, b) => {
        const x = table.rows[a][col], y = table.rows[b][col];
        // 빈 칸은 어느 쪽으로 정렬하든 맨 뒤에 둔다.
        if (x === '' || y === '') return (x === '') - (y === '') || a - b;
        const diff = numeric ? numberOf(x) - numberOf(y) : collator.compare(x, y);
        return diff * dir || a - b;
      });
    }
  }

  function drawHead() {
    const row = make('tr');
    const corner = make('th', 'tbl-no', '#');
    corner.title = '파일에서의 줄 차례';
    row.append(corner);
    table.columns.forEach((name, col) => {
      const cell = make('th', table.numeric[col] ? 'num' : '');
      cell.dataset.col = col;
      cell.title = `${name}\n누르기: 오름차순 → 내림차순 → 원래 차례`;
      cell.append(make('span', '', name));
      if (sort?.name === name) cell.append(make('b', 'tbl-sort', sort.dir > 0 ? '▲' : '▼'));
      row.append(cell);
    });
    $head.replaceChildren(row);
  }

  /** 다음 줄들을 이어서 그린다. */
  function drawMore() {
    const upto = Math.min(order.length, drawn + CHUNK);
    const rows = [];
    for (; drawn < upto; drawn += 1) {
      const at = order[drawn];
      const row = make('tr');
      row.dataset.row = at;
      row.append(make('td', 'tbl-no', String(at + 1)));
      table.rows[at].forEach((value, col) => {
        const cell = make('td', table.numeric[col] ? 'num' : '', value.length > MAX_CELL ? `${value.slice(0, MAX_CELL)}…` : value);
        if (picked && picked.row === at && picked.col === col) cell.classList.add('sel');
        row.append(cell);
      });
      rows.push(row);
    }
    $body.append(...rows);
  }

  /** 표를 처음부터 다시 그린다. `scroll`: 그린 뒤 옮길 자리. */
  function draw(scroll = 0) {
    arrange();
    drawHead();
    $body.replaceChildren();
    drawn = 0;
    drawMore();
    // 보던 자리까지는 이어서 그려 둔다(화면에 없어 높이를 잴 수 없을 때는 하지 않는다).
    while ($scroll.clientHeight > 0 && drawn < order.length && $table.offsetHeight < scroll + $scroll.clientHeight) drawMore();
    $scroll.scrollTop = scroll;
    if (picked && !order.includes(picked.row)) choose(null);
    refreshBar();
  }

  /** 읽어 둔 글로 표를 다시 만든다(첫 줄을 제목으로 쓸지 바꿨을 때, 파일을 다시 읽었을 때). */
  function rebuild(scroll = 0) {
    if (raw == null) return;
    try {
      table = readTable(raw, path, header);
      loaded = true;
      note(table.note);
    } catch (err) {
      table = { columns: [], rows: [], numeric: [], note: '', hasHeaderChoice: false };
      loaded = true;
      note(`표로 읽지 못했습니다: ${err.message || err}`, true);
    }
    draw(scroll);
  }

  /** 칸을 고른다(null이면 고른 것을 푼다). 고른 칸의 전체 값을 아래에 보여 준다. */
  function choose(next) {
    picked = next;
    for (const cell of $body.querySelectorAll('td.sel')) cell.classList.remove('sel');
    if (!next) { $detail.hidden = true; return; }
    const row = $body.querySelector(`tr[data-row="${next.row}"]`);
    const cell = row?.children[next.col + 1];
    cell?.classList.add('sel');
    cell?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    $detail.replaceChildren(make('b', '', `${table.columns[next.col]} · ${next.row + 1}번째 줄`), make('span', '', table.rows[next.row][next.col] || '(빈 칸)'));
    $detail.hidden = false;
  }

  function copyText(text, what = '복사했습니다.') {
    if (text == null) return;
    invoke('clipboard_write', { text }).then(() => setStatus(what)).catch((err) => setStatus(String(err)));
  }

  function copyRows(kind) {
    if (!order.length) return setStatus('복사할 줄이 없습니다.');
    const limit = kind === 'markdown' ? MARKDOWN_ROWS : COPY_ROWS;
    const rows = order.slice(0, limit).map((at) => table.rows[at]);
    const flat = (value) => value.replace(/[\t\r\n]+/g, ' ');
    const text = kind === 'markdown'
      ? [`| ${table.columns.map((name) => flat(name).replace(/\|/g, '\\|')).join(' | ')} |`,
        `| ${table.columns.map((_, col) => (table.numeric[col] ? '---:' : '---')).join(' | ')} |`,
        ...rows.map((row) => `| ${row.map((value) => flat(value).replace(/\|/g, '\\|')).join(' | ')} |`)].join('\n')
      : [table.columns.map(flat).join('\t'), ...rows.map((row) => row.map(flat).join('\t'))].join('\n');
    return copyText(`${text}\n`, `${count(rows.length)}줄을 복사했습니다.${order.length > limit ? ` (전체 ${count(order.length)}줄 가운데 앞부분)` : ''}`);
  }

  function setFilter(text) {
    filter = text;
    if ($filter.value !== text) $filter.value = text;
    draw();
    onChange();
  }

  // ── 파일 ──

  /** 파일을 읽어 그린다. 성공하면 참. `keep`: 보던 자리를 지킨다(바뀐 파일을 다시 읽을 때). */
  async function load(target, { keep = false } = {}) {
    const seq = ++loadSeq;
    let file;
    loading += 1;
    try {
      file = await invoke('reader_read', { path: target });
    } catch (err) {
      loading -= 1;
      if (disposed || seq !== loadSeq) return false;
      if (target !== path) { setStatus(`표를 열지 못했습니다: ${err}`); return false; }
      note(loaded ? `파일을 읽지 못했습니다(마지막으로 읽은 내용입니다): ${err}` : `파일을 읽지 못했습니다: ${err}`, true);
      refreshBar();
      return false;
    }
    loading -= 1;
    if (disposed || seq !== loadSeq) return false;
    const moved = file.path !== path;
    path = file.path;
    seen = file.modified;
    raw = file.text;
    if (moved) { picked = null; $detail.hidden = true; }
    rebuild(keep && !moved ? $scroll.scrollTop : 0);
    if (moved) onChange();
    return true;
  }

  /** 파일이 바뀌었는지 보고, 바뀌었으면 보던 자리를 지킨 채 다시 읽는다. */
  async function check() {
    if (disposed || !path || checking || loading) return;
    checking = true;
    const watched = path;
    try {
      const stamp = await invoke('reader_stamp', { path: watched });
      if (disposed || watched !== path || loading || stamp === seen) return;
      seen = stamp;
      if (stamp == null) {
        note(loaded ? '파일이 없어졌습니다. 마지막으로 읽은 내용을 보여 주고 있습니다.' : '파일이 없습니다. 생기면 바로 보여 줍니다.', true);
      } else if (await load(watched, { keep: true }) && !$note.classList.contains('fail')) {
        const d = new Date();
        const two = (n) => String(n).padStart(2, '0');
        note([`${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())} 바뀐 내용 반영`, table.note].filter(Boolean).join(' · '));
      }
    } catch { /* 다음 번에 다시 본다 */ } finally {
      checking = false;
    }
  }

  async function reload() {
    if (!path) return;
    if (await load(path, { keep: true }) && !$note.classList.contains('fail')) note(['다시 읽었습니다', table.note].filter(Boolean).join(' · '));
  }

  async function pick() {
    const start = path ? path.replace(/[\\/][^\\/]*$/, '') : item.cwd || null;
    const chosen = await invoke('table_pick', { start });
    if (chosen && await load(chosen)) focus();
  }

  // ── 마우스·키보드 ──

  $head.addEventListener('click', (ev) => {
    const cell = ev.target.closest('th[data-col]');
    if (!cell) return;
    const name = table.columns[Number(cell.dataset.col)];
    sort = sort?.name !== name ? { name, dir: 1 } : sort.dir > 0 ? { name, dir: -1 } : null;
    draw();
    onChange();
  });
  $body.addEventListener('click', (ev) => {
    const cell = ev.target.closest('td');
    const row = cell?.parentElement;
    if (!row || getSelection()?.toString()) return; // 글을 고르려고 끈 것이다.
    const col = cell.cellIndex - 1;
    choose({ row: Number(row.dataset.row), col: Math.max(0, col) });
  });
  $scroll.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    const cell = ev.target.closest('td');
    const row = cell?.parentElement;
    if (!row || !path) return;
    const at = Number(row.dataset.row), col = Math.max(0, cell.cellIndex - 1);
    choose({ row: at, col });
    const value = table.rows[at][col];
    showMenu(ev, [
      { caption: `${table.columns[col]} · ${at + 1}번째 줄` },
      { label: '칸 복사  (Ctrl+C)', action: () => copyText(value) },
      { label: '줄 복사 (탭으로 나눈 글)', action: () => copyText(table.rows[at].join('\t')) },
      ...(value && !/\s/.test(value) ? [{ label: `이 값으로 거르기: ${value.length > 30 ? `${value.slice(0, 30)}…` : value}`, action: () => setFilter(value) }] : []),
      ...(filter ? [{ label: '거르기 풀기', action: () => setFilter('') }] : []),
      { label: '파일 경로 복사', action: () => copyText(path) },
    ]);
  });
  $scroll.addEventListener('scroll', () => {
    if (drawn < order.length && $scroll.scrollTop + $scroll.clientHeight * 2 > $scroll.scrollHeight) drawMore();
  });
  $filter.addEventListener('input', () => {
    clearTimeout(filterTimer);
    filterTimer = setTimeout(() => setFilter($filter.value), FILTER_MS);
  });
  $filter.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape') { ev.preventDefault(); ev.stopPropagation(); clearTimeout(filterTimer); if ($filter.value) setFilter(''); else focus(); }
    else if (ev.key === 'Enter') { ev.preventDefault(); clearTimeout(filterTimer); setFilter($filter.value); focus(); }
  });
  root.addEventListener('keydown', (ev) => {
    const key = ev.key.toLowerCase();
    const plain = ev.ctrlKey && !ev.altKey && !ev.shiftKey;
    let handled = true;
    if (plain && key === 'f') { if (path) { $filter.focus(); $filter.select(); } }
    else if (plain && key === 'o') pick();
    else if (plain && key === 'c' && ev.target !== $filter && picked && !getSelection()?.toString()) copyText(table.rows[picked.row][picked.col]);
    else if (ev.target === $scroll && picked && ['arrowup', 'arrowdown', 'arrowleft', 'arrowright'].includes(key) && !ev.ctrlKey && !ev.altKey) {
      // 방향키: 고른 칸을 옮긴다(아래로 가다 그려 둔 끝에 닿으면 이어서 그린다).
      const index = order.indexOf(picked.row);
      const next = { ...picked };
      if (key === 'arrowleft') next.col = Math.max(0, picked.col - 1);
      else if (key === 'arrowright') next.col = Math.min(table.columns.length - 1, picked.col + 1);
      else {
        const moved = Math.min(order.length - 1, Math.max(0, index + (key === 'arrowdown' ? 1 : -1)));
        while (moved >= drawn) drawMore();
        next.row = order[moved];
      }
      choose(next);
    } else if (key === 'escape' && picked) choose(null);
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });

  // 숨겨졌다 다시 보이면 파일이 바뀌었는지 바로 본다.
  const watcher = new ResizeObserver(() => {
    const now = $scroll.clientHeight > 0;
    if (now && !shown) { shown = true; check(); } else if (!now) shown = false;
  });
  watcher.observe($scroll);
  const timer = setInterval(() => { if (shown && !document.hidden) check(); }, CHECK_MS);

  function focus() {
    $scroll.focus({ preventScroll: true });
  }

  refreshBar();
  return {
    ready: path ? load(path).then(() => {}, () => {}) : Promise.resolve(),
    focus,
    title,
    state: () => ({ path, header, filter, sort }),
    close: () => Promise.resolve(true),
    dispose() {
      disposed = true;
      clearInterval(timer);
      clearTimeout(filterTimer);
      watcher.disconnect();
    },
    path: () => path,
    reload,
    /** 지금 표에 든 것(자동 검증용): 열 이름, 전체 줄 수, 보이는 줄 수, 그려 둔 줄 수. */
    summary: () => ({ columns: [...table.columns], rows: table.rows.length, shown: order.length, drawn, note: $note.textContent }),
    setFilter,
    sortBy: (name, dir = 1) => { sort = name ? { name, dir: dir < 0 ? -1 : 1 } : null; draw(); onChange(); },
  };
}
