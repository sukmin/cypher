// 헥사 에디터 항목: 파일을 바이트로 보고 고친다. 화면을 좌우 절반으로 나눠,
// 왼쪽에는 바이트 그대로(오프셋 · 16진수 16개 · 글자), 오른쪽에는 해석한 내용(매직 넘버로 알아본 형식, 머리·덩어리 같은 구조,
// 커서 자리의 값을 여러 자료형으로 읽은 것)을 보인다. 오른쪽에서 칸을 누르면 왼쪽의 그 바이트로 가고,
// 왼쪽에서 커서를 옮기면 오른쪽에서 그 바이트가 속한 칸이 표시된다.
//
// 파일 전체를 읽지 않는다: 보이는 줄의 바이트만 64KB씩 읽어 둔다(몇 GB짜리 파일도 연다).
// 고치기는 덮어쓰기만 한다(파일 크기는 바뀌지 않는다). 고친 바이트는 저장하기 전까지 화면이 들고 있고, 저장하면 앱이 파일에 쓴다.

import { emptyZone } from './empty.js';

let api = null; // { invoke, make, setStatus, showMenu, baseName, toClaude(item, path, ev) }

export function initHex(given) {
  api = given;
}

const COLS = 16;              // 한 줄의 바이트 수
const PAGE = 64 * 1024;       // 한 번에 읽는 바이트 수
const MAX_PAGES = 256;        // 기억해 둘 쪽 수(16MB)
const CHECK_MS = 2000;        // 파일이 바뀌었는지 살피는 간격
const ANALYZE_MS = 700;       // 고친 뒤 해석을 다시 하기까지 기다리는 시간
const MAX_COPY = 1 << 20;     // 한 번에 복사하는 바이트 수
const MAX_SCROLL = 8_000_000; // 스크롤 영역의 높이 한도(px). 넘으면 비율로 옮긴다(브라우저가 아주 긴 영역을 그리지 못한다).
const MAX_UNDO = 2000;
const MAX_KEEP = 65536;       // 저장하지 않은 고침을 앱을 다시 켤 때까지 들고 있는 바이트 수의 한도(넘으면 되살리지 못한다고 알린다)

const hex2 = (n) => n.toString(16).toUpperCase().padStart(2, '0');
const count = (n) => Number(n).toLocaleString('ko-KR');
const shown = (b) => (b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : '.');

/** "4D 5A 90", "4d5a90", "0x4D,0x5A" 같은 글을 바이트로. 읽지 못하면 null. */
export function parseHex(text) {
  const clean = String(text).replace(/0x/gi, '').replace(/[\s,;:-]+/g, '');
  if (!clean || clean.length % 2 || /[^0-9a-f]/i.test(clean)) return null;
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** "0x1F", "1f h", "31", "#1F" 같은 자리 글을 수로. 16진수 표시가 없으면 10진수. 읽지 못하면 null. */
export function parseOffset(text) {
  const t = String(text).trim().toLowerCase().replace(/[_,\s]/g, '');
  let m;
  if ((m = /^(?:0x|#|\$)([0-9a-f]+)$/.exec(t)) || (m = /^([0-9a-f]+)h$/.exec(t))) return parseInt(m[1], 16);
  if (/^\d+$/.test(t)) return Number(t);
  if (/^[0-9a-f]+$/.test(t)) return parseInt(t, 16); // 숫자가 아닌 16진수 글자가 섞였으면 16진수
  return null;
}

/** 고친 바이트(자리 → 값)를 이어진 조각들로 묶는다(앱에 넘길 꼴). */
export function patchesOf(edits) {
  const offsets = [...edits.keys()].sort((a, b) => a - b);
  const out = [];
  for (const at of offsets) {
    const last = out[out.length - 1];
    if (last && last.offset + last.bytes.length === at) last.bytes.push(edits.get(at));
    else out.push({ offset: at, bytes: [edits.get(at)] });
  }
  return out;
}

/**
 * 헥사 에디터 항목의 화면을 host 안에 만든다.
 * saved: { path(열어 둔 파일), caret(커서 자리), top(맨 위에 보이던 줄), little(값을 리틀 엔디언으로 읽을지),
 *          edits(저장하지 않은 고침: { size, modified, patches: [[자리, "16진수"]…] } — 파일이 그대로일 때만 되살린다) }
 */
export function createHex(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus } = api;
  let path = typeof saved.path === 'string' ? saved.path : '';
  let file = null;                       // { path, size, modified, readonly }
  let pages = new Map();                 // 쪽 번호 → Uint8Array(읽는 중이면 Promise)
  let edits = new Map();                 // 자리 → 고친 값
  let undo = [], redo = [];
  let caret = Number.isFinite(saved.caret) ? saved.caret : 0;
  let anchor = null;                     // 고르기를 시작한 자리(고른 것이 없으면 null)
  let nibble = 0;                        // 16진수로 칠 때 다음에 칠 반 바이트(0: 윗자리, 1: 아랫자리)
  let pane = 'hex';                      // 'hex' | 'text': 글쇠가 어느 쪽에 들어가는지
  let topRow = Number.isFinite(saved.top) ? Math.floor(saved.top / COLS) : 0;
  let little = saved.little !== false;
  let analysis = null, fieldsFlat = [], hoverField = null, pickedField = null;
  let findText = '', findHex = true, findAt = null;
  let disposed = false, isShown = false, checking = false, analyzing = 0, analyzeTimer = 0, loadSeq = 0;
  let saving = false;                    // 저장하는 동안에는 고치기·다른 파일 열기·다시 저장을 받지 않는다
  let statSeq = 0;                       // 파일 열기(hex_stat)의 차례: 늦게 온 옛 응답이 새로 연 파일을 덮지 않게
  let kept = saved.edits && typeof saved.edits === 'object' ? { ...saved.edits, path } : null; // 되살릴 고침
  let rowHeight = 18;

  const button = (label, tip, action) => {
    const b = make('button', '', label);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const $title = make('span', 'doc-title');
  const $note = make('span', 'reader-note');
  const $save = button('저장', '고친 바이트를 파일에 쓰기 (Ctrl+S)', () => saveFile(false));
  const bar = make('div', 'doc-bar');
  bar.append(
    $title, $note,
    button('파일 열기', '다른 파일 열기 (Ctrl+O)', () => pick()),
    $save,
    button('다른 이름으로', '다른 이름으로 저장 (Ctrl+Shift+S) · 원래 파일은 그대로', () => saveFile(true)),
    button('찾기', '바이트나 글자 찾기 (Ctrl+F · 다음 F3 · 앞 Shift+F3)', () => openFind()),
    button('이동', '자리로 가기 (Ctrl+G)', () => goPrompt()),
    button('되돌리기', '고친 것 되돌리기 (Ctrl+Z · 다시 Ctrl+Y)', () => step(undo, redo)),
    button('Claude에 넣기', '파일 경로를 Claude 세션 입력창에 넣기', (ev) => api.toClaude?.(item, path, ev)),
    button('탐색기에서 보기', '탐색기에서 보기', () => path && invoke('link_open', { target: path, cwd: item.cwd || '', openFile: false, line: null, col: null }).catch((err) => setStatus(String(err)))),
  );

  // 찾기 줄
  const $find = make('div', 'hex-find');
  $find.hidden = true;
  const $findInput = make('input', 'hex-find-input');
  Object.assign($findInput, { spellcheck: false, autocomplete: 'off', placeholder: '찾을 바이트(4D 5A) 또는 글자' });
  const $findMode = make('button', '', '16진수');
  $findMode.title = '찾을 것을 16진수 바이트로 읽을지, 글자(UTF-8)로 읽을지';
  const $findInfo = make('span', 'hex-find-info');
  $find.append($findInput, $findMode, button('다음', '다음 (F3, Enter)', () => runFind(false)), button('앞', '앞 (Shift+F3, Shift+Enter)', () => runFind(true)), $findInfo, button('×', '닫기 (Esc)', () => closeFind()));

  // 왼쪽: 바이트
  const $head = make('div', 'hex-head');
  const $scroll = make('div', 'hex-scroll');
  $scroll.tabIndex = 0;
  const $spacer = make('div', 'hex-spacer');
  const $rows = make('div', 'hex-rows');
  $scroll.append($spacer, $rows);
  const zone = emptyZone(make, {
    title: '헥사 에디터로 볼 파일을 여기에 끌어다 놓으세요',
    hint: '왼쪽 절반에는 바이트를 16진수와 글자로, 오른쪽 절반에는 파일 형식(매직 넘버)과 구조, 커서 자리의 값을 보여 줍니다. 바이트를 고쳐 저장할 수 있습니다(덮어쓰기).',
    pick: { label: '파일 고르기…', tip: '파일 고르기 (Ctrl+O)', action: () => pick() },
    path: { placeholder: '또는 파일 경로를 적고 Enter', action: (given) => takeFile(given) },
  });
  const $status = make('div', 'hex-status');
  const $left = make('div', 'hex-left');
  $left.append($head, $scroll, $status);

  // 오른쪽: 해석
  const $format = make('div', 'hex-format');
  const $summary = make('div', 'hex-summary');
  const $tree = make('div', 'hex-tree');
  const $notes = make('div', 'hex-notes');
  const $inspect = make('div', 'hex-inspect');
  const $right = make('div', 'hex-right');
  const $info = make('div', 'hex-info');
  $info.append($format, $summary, $notes, $tree);
  $right.append($info, $inspect);

  const $body = make('div', 'hex-body');
  $body.append($left, $right);
  const root = make('div', 'doc hexed');
  root.append(bar, $find, zone.node, $body);
  host.append(root);

  const size = () => file?.size ?? 0;
  const rows = () => Math.max(1, Math.ceil(size() / COLS));
  const dirty = () => edits.size > 0;
  const title = () => (path ? `${api.baseName(path)}${dirty() ? ' *' : ''}` : '헥사');
  const note = (text, fail = false) => { $note.textContent = text; $note.title = text; $note.classList.toggle('fail', fail); };
  const offsetWidth = () => Math.max(8, size().toString(16).length);
  const addr = (n) => n.toString(16).toUpperCase().padStart(offsetWidth(), '0');

  // ── 바이트 읽기 ──

  /** 그 자리의 바이트(고친 값 먼저). 아직 읽지 못했으면 undefined(읽기를 건다). */
  function byteAt(at) {
    if (at < 0 || at >= size()) return undefined;
    if (edits.has(at)) return edits.get(at);
    const page = pages.get(Math.floor(at / PAGE));
    if (page instanceof Uint8Array) return page[at % PAGE];
    if (!page) loadPage(Math.floor(at / PAGE));
    return undefined;
  }
  /** 파일에 든 원래 값(고친 값을 덮지 않은 것). */
  function originalAt(at) {
    const page = pages.get(Math.floor(at / PAGE));
    return page instanceof Uint8Array ? page[at % PAGE] : undefined;
  }

  function loadPage(index) {
    const seq = loadSeq;
    const wanted = path;
    const promise = invoke('hex_read', { path: wanted, offset: index * PAGE, len: PAGE }).then((buffer) => {
      if (disposed || seq !== loadSeq || wanted !== path) return;
      pages.set(index, new Uint8Array(buffer));
      // 오래 안 쓴 쪽은 버린다(맨 앞에 넣은 것부터).
      while (pages.size > MAX_PAGES) pages.delete(pages.keys().next().value);
      draw();
    }, (err) => {
      if (seq !== loadSeq) return;
      pages.delete(index);
      note(`읽지 못했습니다: ${err}`, true);
    });
    pages.set(index, promise);
  }

  // ── 그리기 ──

  function measure() {
    const probe = $rows.firstElementChild;
    if (probe && probe.offsetHeight) rowHeight = probe.offsetHeight;
  }
  const viewRows = () => Math.max(1, Math.floor($scroll.clientHeight / rowHeight));
  const maxTop = () => Math.max(0, rows() - viewRows());
  const scaled = () => rows() * rowHeight > MAX_SCROLL;
  const scrollFor = (row) => (scaled() ? (maxTop() ? (row / maxTop()) * ($spacer.offsetHeight - $scroll.clientHeight) : 0) : row * rowHeight);
  const rowFor = (top) => {
    if (!scaled()) return Math.floor(top / rowHeight);
    const room = $spacer.offsetHeight - $scroll.clientHeight;
    return room > 0 ? Math.round((top / room) * maxTop()) : 0;
  };
  // 스크롤 영역의 높이: 맨 아래까지 스크롤하면 마지막 줄이 꼭 보이게 한다(아주 큰 파일은 한도까지만 두고 비율로 옮긴다).
  const sizeSpacer = () => { $spacer.style.height = `${scaled() ? MAX_SCROLL : maxTop() * rowHeight + $scroll.clientHeight}px`; };
  let expectScroll = null; // 코드가 넣은 scrollTop(그 값으로 오는 scroll 사건은 사용자가 움직인 것이 아니다)
  let wheelRest = 0;       // 아주 큰 파일에서 한 줄이 안 되게 굴린 휠 양
  function setTop(row) {
    topRow = Math.max(0, Math.min(maxTop(), Math.floor(row)));
    sizeSpacer();
    $scroll.scrollTop = scrollFor(topRow);
    expectScroll = $scroll.scrollTop;
    draw();
  }

  const selection = () => (anchor == null ? null : { from: Math.min(anchor, caret), to: Math.max(anchor, caret) });

  function draw() {
    if (disposed || !file) return;
    sizeSpacer();
    const first = topRow;
    const visibleRows = viewRows() + 1;
    const sel = selection();
    const field = pickedField || hoverField || fieldAt(caret);
    const fieldEnd = field ? field.offset + field.len : -1;
    const hit = findAt != null ? { from: findAt.offset, to: findAt.offset + findAt.len - 1 } : null;
    const frag = document.createDocumentFragment();
    for (let r = first; r < Math.min(rows(), first + visibleRows); r++) {
      const base = r * COLS;
      const row = make('div', 'hx-row');
      row.append(make('span', 'hx-off', addr(base)));
      const bytes = make('span', 'hx-bytes');
      const text = make('span', 'hx-text');
      for (let i = 0; i < COLS; i++) {
        const at = base + i;
        const cell = make('span', 'hx-b');
        const ch = make('span', 'hx-c');
        if (at < size()) {
          const b = byteAt(at);
          cell.textContent = b === undefined ? '··' : hex2(b);
          ch.textContent = b === undefined ? ' ' : shown(b);
          cell.dataset.at = at;
          ch.dataset.at = at;
          const marks = [];
          if (b === 0) marks.push('zero');
          if (edits.has(at)) marks.push('edited');
          if (sel && at >= sel.from && at <= sel.to) marks.push('sel');
          if (field && at >= field.offset && at < fieldEnd) marks.push('fld');
          if (hit && at >= hit.from && at <= hit.to) marks.push('hit');
          if (at === caret) marks.push(pane === 'hex' ? 'caret' : 'caret2');
          if (marks.length) { cell.classList.add(...marks); ch.classList.add(...marks.map((m) => (m === 'caret' ? 'caret2' : m === 'caret2' ? 'caret' : m))); }
        } else {
          cell.textContent = '  ';
          ch.textContent = ' ';
        }
        if (i === 8) bytes.append(make('span', 'hx-gap'));
        bytes.append(cell);
        text.append(ch);
      }
      row.append(bytes, text);
      frag.append(row);
    }
    // 줄들은 보이는 자리에 붙여 둔다. 높이를 보이는 만큼으로 자르면(overflow-y: clip) 스크롤 범위를 늘리지 않는다.
    $rows.style.transform = `translateY(${$scroll.scrollTop}px)`;
    $rows.style.height = `${$scroll.clientHeight}px`;
    $rows.replaceChildren(frag);
    measure();
    drawStatus();
    drawInspect();
    markTree();
  }

  function drawHead() {
    const head = make('div', 'hx-row');
    head.append(make('span', 'hx-off', '오프셋'.padEnd(offsetWidth(), ' ')));
    const bytes = make('span', 'hx-bytes');
    for (let i = 0; i < COLS; i++) {
      if (i === 8) bytes.append(make('span', 'hx-gap'));
      bytes.append(make('span', 'hx-b', hex2(i)));
    }
    head.append(bytes, make('span', 'hx-text', '0123456789ABCDEF'));
    $head.replaceChildren(head);
  }

  function drawStatus() {
    const sel = selection();
    const parts = [`자리 ${addr(caret)}h (${count(caret)})`];
    if (sel && sel.to > sel.from) parts.push(`고름 ${addr(sel.from)}–${addr(sel.to)} (${count(sel.to - sel.from + 1)}바이트)`);
    parts.push(`크기 ${count(size())}바이트`);
    if (dirty()) parts.push(`고친 바이트 ${count(edits.size)}개`);
    parts.push(pane === 'hex' ? '16진수 칸에 입력' : '글자 칸에 입력 (Tab: 바꾸기)');
    if (file?.readonly) parts.push('읽기 전용 파일');
    $status.textContent = parts.join(' · ');
  }

  // ── 오른쪽: 해석 ──

  function flatten(fields, depth = 0, parent = null, out = []) {
    for (const field of fields) {
      field.depth = depth;
      field.parent = parent;
      out.push(field);
      if (field.children?.length) flatten(field.children, depth + 1, field, out);
    }
    return out;
  }

  /** 그 자리를 품은 가장 깊은 칸. */
  function fieldAt(at) {
    let best = null;
    for (const field of fieldsFlat) {
      if (field.len > 0 && at >= field.offset && at < field.offset + field.len && (!best || field.depth >= best.depth)) best = field;
    }
    return best;
  }

  function drawAnalysis() {
    if (!analysis) {
      $format.textContent = file ? '해석하는 중…' : '';
      $summary.replaceChildren();
      $tree.replaceChildren();
      $notes.replaceChildren();
      return;
    }
    $format.replaceChildren(make('b', '', analysis.format), ...(analysis.magic ? [make('span', 'hex-magic', `매직 넘버: ${analysis.magic}`)] : []));
    $summary.replaceChildren(...analysis.summary.map(([key, value]) => {
      const row = make('div', 'hex-kv');
      row.append(make('span', 'k', key), make('span', 'v', value));
      return row;
    }));
    $notes.replaceChildren(...analysis.notes.map((text) => make('div', 'hex-note', `! ${text}`)));
    fieldsFlat = flatten(analysis.fields);
    const rowsOut = [];
    const add = (field) => {
      const row = make('div', 'hex-field');
      row.style.paddingLeft = `${6 + field.depth * 14}px`;
      const toggle = make('span', 'tw', field.children?.length ? (field.open ? '▾' : '▸') : '');
      row.append(toggle, make('span', 'n', field.name), make('span', 'v', field.value || ''), make('span', 'o', `${addr(field.offset)}h · ${count(field.len)}`));
      row.title = `${field.name}${field.value ? ` = ${field.value}` : ''}\n자리 ${addr(field.offset)}h부터 ${count(field.len)}바이트\n누르면 그 바이트로 갑니다`;
      row.onclick = (ev) => {
        if (ev.target === toggle && field.children?.length) { field.open = !field.open; drawTree(); return; }
        pickField(field);
      };
      row.ondblclick = () => { if (field.children?.length) { field.open = !field.open; drawTree(); } };
      row.onmouseenter = () => { hoverField = field; draw(); };
      row.onmouseleave = () => { if (hoverField === field) { hoverField = null; draw(); } };
      field.row = row;
      rowsOut.push(row);
      if (field.open) for (const child of field.children || []) add(child);
    };
    const drawTree = () => {
      rowsOut.length = 0;
      for (const field of analysis.fields) add(field);
      $tree.replaceChildren(...rowsOut);
      markTree();
    };
    // 처음 보는 칸은 맨 위의 짧은 목록만 펼쳐 둔다.
    for (const field of analysis.fields) if (field.open === undefined && field.children?.length && field.children.length <= 12) field.open = true;
    drawTree();
  }

  let lastHere = null;
  /** 커서가 든 칸을 오른쪽 목록에 표시한다(접혀 있으면 보이는 조상 칸에). 바뀌었으면 그 줄이 보이게 스크롤한다. */
  function markTree() {
    let here = fieldAt(caret);
    while (here && !here.row?.isConnected) here = here.parent;
    for (const field of fieldsFlat) {
      if (!field.row) continue;
      field.row.classList.toggle('here', field === here);
      field.row.classList.toggle('picked', field === pickedField);
    }
    if (here && here !== lastHere && !$tree.matches(':hover')) here.row.scrollIntoView({ block: 'nearest' });
    lastHere = here;
  }

  /** 오른쪽에서 고른 칸: 그 바이트들을 고르고 첫 바이트가 보이게 한다(커서는 칸의 처음에 둔다). */
  function pickField(field) {
    pickedField = field;
    if (field.len > 0 && field.offset < size()) {
      caret = field.offset;
      anchor = field.len > 1 ? Math.min(size() - 1, field.offset + field.len - 1) : null;
      reveal(field.offset);
    }
    nibble = 0;
    findAt = null;
    draw();
    $scroll.focus({ preventScroll: true });
  }

  async function analyze() {
    if (!path) return;
    const seq = ++analyzing;
    try {
      const result = await invoke('hex_analyze', { path, patches: patchesOf(edits) });
      if (disposed || seq !== analyzing) return;
      // 펼치거나 접어 둔 칸은 자리와 이름이 같으면 그대로 둔다(고친 뒤 다시 해석해도 보던 모양이 남게).
      const before = new Map(fieldsFlat.filter((f) => f.open !== undefined).map((f) => [`${f.depth}:${f.offset}:${f.name}`, f.open]));
      analysis = result;
      for (const field of flatten(analysis.fields)) {
        const key = `${field.depth}:${field.offset}:${field.name}`;
        if (before.has(key)) field.open = before.get(key);
      }
      pickedField = null;
      hoverField = null;
      drawAnalysis();
      draw();
    } catch (err) {
      if (seq === analyzing) { analysis = { format: '해석하지 못했습니다', magic: '', summary: [], fields: [], notes: [String(err)] }; drawAnalysis(); }
    }
  }
  const analyzeSoon = () => { clearTimeout(analyzeTimer); analyzeTimer = setTimeout(analyze, ANALYZE_MS); };

  // ── 값 살펴보기(커서 자리의 바이트를 여러 자료형으로) ──

  function drawInspect() {
    const bytes = [];
    for (let i = 0; i < 8; i++) bytes.push(byteAt(caret + i));
    const have = (n) => bytes.slice(0, n).every((b) => b !== undefined);
    const view = (n) => { const buf = new DataView(new ArrayBuffer(8)); for (let i = 0; i < n; i++) buf.setUint8(i, bytes[i]); return buf; };
    const rowsOut = [];
    const add = (name, n, value) => {
      const row = make('div', 'hex-kv');
      row.append(make('span', 'k', name), make('span', 'v', have(n) ? value() : (caret + n > size() ? '—' : '…')));
      rowsOut.push(row);
    };
    const le = little;
    add('2진수', 1, () => bytes[0].toString(2).padStart(8, '0'));
    add('8비트', 1, () => `${bytes[0]} · 부호 ${view(1).getInt8(0)}`);
    add('16비트', 2, () => `${view(2).getUint16(0, le)} · 부호 ${view(2).getInt16(0, le)}`);
    add('32비트', 4, () => `${count(view(4).getUint32(0, le))} · 부호 ${count(view(4).getInt32(0, le))}`);
    add('64비트', 8, () => `${view(8).getBigUint64(0, le).toLocaleString('ko-KR')} · 부호 ${view(8).getBigInt64(0, le).toLocaleString('ko-KR')}`);
    add('32비트 실수', 4, () => String(view(4).getFloat32(0, le)));
    add('64비트 실수', 8, () => String(view(8).getFloat64(0, le)));
    add('UTF-8 글자', 1, () => utf8At(bytes));
    add('유닉스 시각', 4, () => { const t = view(4).getUint32(0, le); return t ? new Date(t * 1000).toISOString().replace('T', ' ').replace('.000Z', ' UTC') : '0'; });
    const head = make('div', 'hex-inspect-head');
    const order = make('button', '', little ? '리틀 엔디언' : '빅 엔디언');
    order.title = '여러 바이트 값을 읽는 바이트 순서 (누르면 바꾸기)';
    order.onclick = () => { little = !little; onChange(); drawInspect(); };
    head.append(make('b', '', `커서 자리(${addr(caret)}h)의 값`), order);
    $inspect.replaceChildren(head, ...rowsOut);
  }

  function utf8At(bytes) {
    const b = bytes.filter((x) => x !== undefined);
    const lead = b[0];
    const n = lead < 0x80 ? 1 : lead >= 0xf0 && lead < 0xf8 ? 4 : lead >= 0xe0 ? 3 : lead >= 0xc0 ? 2 : 0;
    if (!n || b.length < n) return '(글자의 시작이 아님)';
    try {
      const text = new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array(b.slice(0, n)));
      const code = text.codePointAt(0);
      return `${code < 0x20 ? '(제어 글자)' : `"${text}"`} U+${code.toString(16).toUpperCase().padStart(4, '0')} · ${n}바이트`;
    } catch { return '(올바른 UTF-8이 아님)'; }
  }

  // ── 커서·고르기 ──

  /** 그 자리가 보이게 스크롤한다. */
  function reveal(at) {
    const row = Math.floor(at / COLS);
    if (row < topRow) setTop(row);
    else if (row >= topRow + viewRows()) setTop(row - viewRows() + 1);
  }

  function moveTo(at, extend = false) {
    const next = Math.max(0, Math.min(Math.max(0, size() - 1), at));
    if (extend) { if (anchor == null) anchor = caret; } else anchor = null;
    caret = next;
    nibble = 0;
    pickedField = null;
    findAt = null;
    reveal(caret);
    draw();
    saveSoon();
  }

  // ── 고치기 ──

  /** 되돌리기 칸을 쌓는다(한도를 넘으면 가장 오래된 것부터 버린다). */
  function pushUndo(entry) {
    undo.push(entry);
    if (undo.length > MAX_UNDO) undo.shift();
    redo.length = 0;
  }

  /**
   * 한 바이트를 고친다. 원래 값을 아직 읽지 못한 자리도 고친 값으로 둔다(같은 값인지는 모르므로 고친 것으로 친다).
   * batch가 있으면 되돌리기 칸을 따로 쌓지 않고 거기에 모은다(붙여넣기 한 번을 한 번에 되돌리게).
   */
  function setByte(at, value, group, batch = null) {
    if (at < 0 || at >= size()) return false;
    const before = edits.has(at) ? edits.get(at) : null;
    if (originalAt(at) === value) edits.delete(at); else edits.set(at, value);
    if (batch) { batch.push({ at, before, after: value }); return true; }
    const last = undo[undo.length - 1];
    if (group && last?.group === group && last.at === at) { last.after = value; redo.length = 0; }
    else pushUndo({ at, before, after: value, group });
    return true;
  }
  const busy = () => (saving ? (setStatus('저장하는 중입니다. 저장이 끝난 뒤에 고치세요.'), true) : false);

  function changed() {
    $save.disabled = !dirty();
    $title.textContent = title();
    analyzeSoon();
    onChange();
  }

  /** 되돌리기(from=undo, to=redo)와 다시 하기(from=redo, to=undo). */
  function step(from, to) {
    if (busy()) return;
    const entry = from.pop();
    if (!entry) return setStatus(from === undo ? '되돌릴 것이 없습니다.' : '다시 할 것이 없습니다.');
    const back = from === undo;
    const list = entry.batch ? (back ? [...entry.batch].reverse() : entry.batch) : [entry];
    for (const one of list) {
      const target = back ? one.before : one.after;
      if (target == null || target === originalAt(one.at)) edits.delete(one.at); else edits.set(one.at, target);
    }
    to.push(entry);
    caret = Math.max(0, Math.min(size() - 1, entry.at));
    anchor = null;
    nibble = 0;
    reveal(caret);
    changed();
    draw();
  }

  let typingGroup = 0;
  function typeHex(digit) {
    if (file?.readonly) return setStatus('읽기 전용 파일입니다. "다른 이름으로"로 저장하면 고칠 수 있습니다.');
    if (busy() || !size()) return;
    const current = byteAt(caret);
    if (current === undefined) return setStatus('아직 읽지 못한 자리입니다. 잠시 뒤 다시 치세요.');
    if (nibble === 0) typingGroup += 1;
    const value = nibble === 0 ? (digit << 4) | (current & 0x0f) : (current & 0xf0) | digit;
    if (!setByte(caret, value, typingGroup)) return;
    if (nibble === 0) nibble = 1;
    else { nibble = 0; if (caret < size() - 1) caret += 1; }
    anchor = null;
    reveal(caret);
    changed();
    draw();
  }

  /** 바이트들을 덮어쓴다. 고른 것이 있으면 고른 처음부터 쓴다. 한 번에 쓴 것은 한 번에 되돌린다. */
  function typeBytes(bytes, label) {
    if (file?.readonly) return setStatus('읽기 전용 파일입니다. "다른 이름으로"로 저장하면 고칠 수 있습니다.');
    if (busy() || !bytes.length || !size()) return;
    const start = selection()?.from ?? caret;
    const room = size() - start;
    const list = bytes.slice(0, room);
    const batch = [];
    list.forEach((value, i) => setByte(start + i, value, null, batch));
    if (batch.length) pushUndo({ at: start, batch });
    if (bytes.length > room) setStatus(`파일 끝을 넘는 ${count(bytes.length - room)}바이트는 넣지 않았습니다(덮어쓰기만 합니다).`);
    else if (label) setStatus(label);
    caret = Math.max(0, Math.min(size() - 1, start + list.length));
    nibble = 0;
    anchor = null;
    reveal(caret);
    changed();
    draw();
  }

  async function copySelection() {
    const sel = selection() || { from: caret, to: caret };
    const length = sel.to - sel.from + 1;
    if (length > MAX_COPY) return setStatus(`${count(MAX_COPY)}바이트까지만 복사합니다.`);
    let bytes;
    try {
      const buffer = await invoke('hex_read', { path, offset: sel.from, len: length });
      bytes = new Uint8Array(buffer);
      for (let i = 0; i < bytes.length; i++) if (edits.has(sel.from + i)) bytes[i] = edits.get(sel.from + i);
    } catch (err) { return setStatus(`복사하지 못했습니다: ${err}`); }
    const text = pane === 'hex' ? [...bytes].map(hex2).join(' ') : new TextDecoder('utf-8').decode(bytes);
    invoke('clipboard_write', { text }).then(
      () => setStatus(`${count(bytes.length)}바이트를 ${pane === 'hex' ? '16진수로' : '글자로'} 복사했습니다.`),
      (err) => setStatus(String(err)),
    );
  }

  async function paste() {
    const clip = await invoke('clipboard_read').catch(() => null);
    // 클립보드의 그림·파일은 경로 글자로 오므로 받지 않는다(그 경로가 바이트로 덮어써진다).
    const text = clip?.kind === 'text' ? clip.text : '';
    if (!text) return setStatus(clip && clip.kind !== 'empty' && clip.kind !== 'text' ? '클립보드에 글자가 아닌 것(그림·파일)이 있어 붙여 넣지 않았습니다.' : '붙여 넣을 글자가 없습니다.');
    const asHex = pane === 'hex' ? parseHex(text) : null;
    if (pane === 'hex' && !asHex) return setStatus('16진수 칸에는 "4D 5A 90" 같은 16진수만 붙여 넣습니다. 글자를 넣으려면 Tab으로 글자 칸에 가세요.');
    const bytes = asHex ? [...asHex] : [...new TextEncoder().encode(text)];
    typeBytes(bytes, `${count(bytes.length)}바이트를 덮어썼습니다.`);
  }

  // ── 찾기·이동 ──

  function openFind() {
    $find.hidden = false;
    $findInput.focus();
    $findInput.select();
  }
  function closeFind() {
    $find.hidden = true;
    findAt = null;
    draw();
    $scroll.focus({ preventScroll: true });
  }
  $findMode.onclick = () => { findHex = !findHex; $findMode.textContent = findHex ? '16진수' : '글자'; $findMode.classList.toggle('on', !findHex); };
  $findInput.addEventListener('keydown', (ev) => {
    ev.stopPropagation();
    if (ev.isComposing || ev.keyCode === 229) return;
    if (ev.key === 'Enter') { ev.preventDefault(); runFind(ev.shiftKey); } else if (ev.key === 'Escape') { ev.preventDefault(); closeFind(); }
  });

  async function runFind(backward) {
    const given = $findInput.value;
    if (!given || !path) return;
    let pattern = findHex ? parseHex(given) : new TextEncoder().encode(given);
    if (!pattern) {
      // 16진수로 읽지 못하면 글자로 찾는다(바꿔 두면 다음에도 글자로).
      findHex = false;
      $findMode.textContent = '글자';
      $findMode.classList.add('on');
      pattern = new TextEncoder().encode(given);
    }
    findText = given;
    const from = backward ? (findAt ? findAt.offset : caret) : findAt ? findAt.offset + 1 : caret;
    $findInfo.textContent = '찾는 중…';
    try {
      let found = await invoke('hex_find', { path, patches: patchesOf(edits), pattern: [...pattern], from, backward, ignoreCase: !findHex });
      let wrapped = false;
      if (found == null) {
        found = await invoke('hex_find', { path, patches: patchesOf(edits), pattern: [...pattern], from: backward ? size() : 0, backward, ignoreCase: !findHex });
        wrapped = found != null;
      }
      if (findText !== given || disposed) return;
      if (found == null) { $findInfo.textContent = '없습니다'; findAt = null; draw(); return; }
      findAt = { offset: found, len: pattern.length };
      caret = found;
      anchor = null;
      nibble = 0;
      reveal(found);
      $findInfo.textContent = `${addr(found)}h${wrapped ? (backward ? ' (끝에서부터 다시 찾음)' : ' (처음부터 다시 찾음)') : ''}`;
      draw();
    } catch (err) {
      $findInfo.textContent = String(err);
    }
  }

  async function goPrompt() {
    if (!file) return;
    const given = await api.ask?.('갈 자리 (16진수는 0x1F처럼, 10진수는 그냥 숫자. +100·-10은 지금 자리에서)', '0x1F', `0x${addr(caret)}`);
    if (given == null) return;
    const text = String(given).trim();
    const relative = /^[+-]/.test(text);
    const value = parseOffset(text.replace(/^[+-]/, ''));
    if (value == null) return setStatus(`자리를 읽지 못했습니다: ${text}`);
    const target = relative ? caret + (text.startsWith('-') ? -value : value) : value;
    if (target < 0 || target >= size()) return setStatus(`파일 안의 자리가 아닙니다(0 ~ ${addr(Math.max(0, size() - 1))}h).`);
    moveTo(target);
    $scroll.focus({ preventScroll: true });
  }

  // ── 파일 ──

  async function load(target, { keep = false } = {}) {
    if (saving) { setStatus('저장하는 중입니다. 저장이 끝난 뒤에 여세요.'); return false; }
    const seq = ++statSeq;
    let info;
    try {
      info = await invoke('hex_stat', { path: target });
    } catch (err) {
      if (disposed || seq !== statSeq) return false;
      if (target !== path) setStatus(`열지 못했습니다: ${err}`); else note(String(err), true);
      return false;
    }
    if (disposed || seq !== statSeq || saving) return false;
    const moved = info.path !== path;
    if (moved && dirty() && !confirm(`${api.baseName(path)}에 저장하지 않은 고침 ${count(edits.size)}바이트가 있습니다. 버리고 다른 파일을 열까요?`)) return false;
    loadSeq += 1;
    pages = new Map();
    if (moved) { edits = new Map(); kept = null; }
    // 다른 파일이거나, 고친 것 없이 다시 읽었으면(밖에서 바뀜) 되돌리기 기록은 새 내용과 맞지 않는다.
    if (moved || !dirty()) { undo = []; redo = []; }
    file = info;
    path = info.path;
    if (moved && !keep) { caret = 0; anchor = null; topRow = 0; }
    caret = Math.min(caret, Math.max(0, size() - 1));
    nibble = 0;
    findAt = null;
    analysis = null;
    fieldsFlat = [];
    pickedField = null;
    hoverField = null;
    zone.node.hidden = true;
    $body.hidden = false;
    note(file.readonly ? '읽기 전용 파일' : '');
    if (kept) restoreKept();
    $save.disabled = !dirty();
    $title.textContent = title();
    drawHead();
    drawAnalysis();
    requestAnimationFrame(() => setTop(topRow));
    setTimeout(() => setTop(topRow), 0);
    analyze();
    if (moved) onChange();
    return true;
  }

  /** 앱을 닫기 전에 저장하지 않았던 고침을 되살린다. 파일이 그대로(크기·고친 시각이 같음)일 때만. */
  function restoreKept() {
    const k = kept;
    kept = null;
    if (!k || k.path !== path) return;
    if (k.lost) return note(`지난번에 저장하지 않은 고침 ${count(k.lost)}바이트는 너무 많아(${count(MAX_KEEP)}바이트까지) 되살리지 못했습니다.`, true);
    if (k.size !== file.size || (k.modified ?? null) !== (file.modified ?? null)) return note('지난번에 저장하지 않은 고침이 있었지만, 그사이 파일이 바뀌어 되살리지 않았습니다.', true);
    for (const piece of Array.isArray(k.patches) ? k.patches : []) {
      const [offset, text] = Array.isArray(piece) ? piece : [];
      const bytes = Number.isSafeInteger(offset) && offset >= 0 ? parseHex(text || '') : null;
      if (bytes) bytes.forEach((b, i) => { if (offset + i < size()) edits.set(offset + i, b); });
    }
    if (dirty()) note(`지난번에 저장하지 않은 고침 ${count(edits.size)}바이트를 되살렸습니다(아직 파일에는 쓰지 않았습니다).`);
  }

  async function pick() {
    if (saving) return setStatus('저장하는 중입니다. 저장이 끝난 뒤에 여세요.');
    const start = path ? path.replace(/[\\/][^\\/]*$/, '') : item.cwd || null;
    const chosen = await invoke('hex_pick', { start });
    if (chosen) await takeFile(chosen);
  }

  async function takeFile(target) {
    const ok = await load(target);
    if (ok) focus();
    return ok;
  }

  async function check() {
    if (disposed || !path || checking || saving) return;
    checking = true;
    try {
      // 되살릴 때 파일이 없었으면(빠진 USB, 다시 빌드하는 중인 파일) 다시 열어 본다.
      if (!file) { await load(path, { keep: true }); return; }
      const seq = statSeq;
      const info = await invoke('hex_stat', { path });
      // 그사이 다른 파일을 열기 시작했거나 저장을 시작했으면 이 결과는 쓰지 않는다.
      if (disposed || info.path !== path || seq !== statSeq || saving) return;
      if (info.size === file.size && info.modified === file.modified) return;
      if (dirty()) { note('파일이 밖에서 바뀌었습니다. 고친 것이 있어 다시 읽지 않았습니다(저장하면 덮어쓸지 묻습니다).', true); return; }
      await load(path, { keep: true });
      const d = new Date();
      const two = (n) => String(n).padStart(2, '0');
      note(`${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())} 바뀐 내용 반영`);
    } catch (err) {
      note(`파일을 읽지 못했습니다: ${err}`, true);
    } finally {
      checking = false;
    }
  }

  async function saveFile(asNew) {
    if (!file) return;
    if (saving) return setStatus('저장하는 중입니다.');
    if (!asNew && !dirty()) return setStatus('고친 것이 없습니다.');
    // 저장이 끝날 때까지 고치기·다른 파일 열기·다시 저장을 받지 않는다(그사이 친 것이 저장 뒤에 사라지거나 저장 둘이 겹치지 않게).
    saving = true;
    $save.disabled = true;
    try {
      let target = null;
      if (asNew) {
        target = await invoke('hex_save_pick', { name: api.baseName(path), start: path.replace(/[\\/][^\\/]*$/, '') });
        if (!target) return;
      }
      const patches = patchesOf(edits);
      const run = (force) => invoke('hex_save', { path, patches, expectedSize: file.size, expectedModified: file.modified ?? null, force, target });
      let saved;
      note('저장하는 중…');
      try {
        saved = await run(false);
      } catch (err) {
        if (String(err) !== 'changed-on-disk') { note(''); return setStatus(`저장하지 못했습니다: ${err}`); }
        if (!confirm('파일이 연 뒤에 밖에서 바뀌었습니다. 고친 바이트를 그 위에 덮어쓸까요?\n(취소하면 저장하지 않습니다)')) { note(''); return; }
        try { saved = await run(true); } catch (again) { note(''); return setStatus(`저장하지 못했습니다: ${again}`); }
      }
      if (disposed) return;
      // 넘긴 조각과 값이 같은 고침만 지운다.
      for (const p of patches) p.bytes.forEach((b, i) => { if (edits.get(p.offset + i) === b) edits.delete(p.offset + i); });
      if (!dirty()) { undo = []; redo = []; }
      pages = new Map();
      loadSeq += 1;
      file = saved;
      if (saved.path !== path) { path = saved.path; onChange(); }
      note('');
      setStatus(`저장했습니다: ${saved.path} (${count(patches.reduce((n, p) => n + p.bytes.length, 0))}바이트 고침)`);
    } finally {
      saving = false;
      if (!disposed) { changed(); draw(); }
    }
  }

  // ── 마우스·키보드 ──

  function cellAt(ev) {
    const cell = ev.target.closest?.('[data-at]');
    if (!cell) return null;
    return { at: Number(cell.dataset.at), pane: cell.classList.contains('hx-c') ? 'text' : 'hex' };
  }
  /** 누른 글쇠의 16진수 값. 한글 입력 상태여도(글쇠가 ㅁ·ㅠ로 와도) 글쇠의 자리로 읽는다. 16진수가 아니면 null. */
  function hexDigit(ev) {
    if (/^[0-9a-f]$/i.test(ev.key)) return parseInt(ev.key, 16);
    const m = /^(?:Digit|Numpad)(\d)$/.exec(ev.code) || /^Key([A-F])$/.exec(ev.code);
    return m && !ev.shiftKey ? parseInt(m[1], 16) : null;
  }

  let dragging = false;
  $rows.addEventListener('mousedown', (ev) => {
    const hit = cellAt(ev);
    if (!hit || ev.button !== 0) return;
    ev.preventDefault();
    $scroll.focus({ preventScroll: true });
    pane = hit.pane;
    if (ev.shiftKey) { if (anchor == null) anchor = caret; } else anchor = hit.at;
    caret = hit.at;
    nibble = 0;
    pickedField = null;
    findAt = null;
    dragging = true;
    draw();
  });
  $rows.addEventListener('mousemove', (ev) => {
    if (!dragging) return;
    const hit = cellAt(ev);
    if (hit && hit.at !== caret) { caret = hit.at; draw(); }
  });
  const stopDrag = () => {
    if (!dragging) return;
    dragging = false;
    if (anchor === caret) anchor = null;
    draw();
    saveSoon();
  };
  document.addEventListener('mouseup', stopDrag);

  $scroll.addEventListener('scroll', () => {
    $head.scrollLeft = $scroll.scrollLeft; // 왼쪽이 좁아 가로로 스크롤하면 머리 줄(00…0F)도 따라간다
    const fromCode = expectScroll != null && Math.abs($scroll.scrollTop - expectScroll) < 1;
    expectScroll = null;
    if (fromCode) return;
    topRow = Math.max(0, Math.min(maxTop(), rowFor($scroll.scrollTop)));
    draw();
    saveSoon();
  });
  $scroll.addEventListener('wheel', (ev) => {
    // 아주 큰 파일은 스크롤 영역을 비율로 줄여 두었으므로, 세로 휠은 줄 단위로 직접 옮긴다(가로 휠·Shift+휠은 그대로 둔다).
    if (!scaled() || ev.shiftKey || Math.abs(ev.deltaX) > Math.abs(ev.deltaY)) return;
    ev.preventDefault();
    wheelRest += ev.deltaMode === 1 ? ev.deltaY * rowHeight : ev.deltaMode === 2 ? ev.deltaY * $scroll.clientHeight : ev.deltaY;
    const lines = Math.trunc(wheelRest / rowHeight);
    if (!lines) return;
    wheelRest -= lines * rowHeight;
    setTop(topRow + lines);
    saveSoon();
  }, { passive: false });

  $scroll.addEventListener('keydown', (ev) => {
    if (!file) return;
    const ctrl = ev.ctrlKey || ev.metaKey;
    const key = ev.key;
    const shift = ev.shiftKey;
    let handled = true;
    if (key === 'ArrowRight') moveTo(caret + 1, shift);
    else if (key === 'ArrowLeft') moveTo(caret - 1, shift);
    else if (key === 'ArrowDown') moveTo(caret + COLS, shift);
    else if (key === 'ArrowUp') moveTo(caret - COLS, shift);
    else if (key === 'PageDown') moveTo(caret + COLS * viewRows(), shift);
    else if (key === 'PageUp') moveTo(caret - COLS * viewRows(), shift);
    else if (key === 'Home') moveTo(ctrl ? 0 : caret - (caret % COLS), shift);
    else if (key === 'End') moveTo(ctrl ? size() - 1 : caret - (caret % COLS) + COLS - 1, shift);
    else if (key === 'Tab') { pane = pane === 'hex' ? 'text' : 'hex'; nibble = 0; draw(); }
    else if (key === 'Backspace') moveTo(caret - 1);
    else if (key === 'Escape') { anchor = null; pickedField = null; findAt = null; draw(); }
    else if (ctrl && key.toLowerCase() === 'z') step(shift ? redo : undo, shift ? undo : redo);
    else if (ctrl && key.toLowerCase() === 'y') step(redo, undo);
    else if (ctrl && key.toLowerCase() === 's') saveFile(shift);
    else if (ctrl && key.toLowerCase() === 'o') pick();
    else if (ctrl && key.toLowerCase() === 'f') openFind();
    else if (ctrl && key.toLowerCase() === 'g') goPrompt();
    else if (ctrl && key.toLowerCase() === 'c') copySelection();
    else if (ctrl && key.toLowerCase() === 'v') paste();
    else if (ctrl && key.toLowerCase() === 'a') { if (size()) { anchor = 0; caret = size() - 1; draw(); } }
    else if (key === 'F3') { if (findText) runFind(shift); else openFind(); }
    else if (!ctrl && !ev.altKey && pane === 'hex' && hexDigit(ev) != null) typeHex(hexDigit(ev));
    else if (!ctrl && !ev.altKey && pane === 'text' && key.length === 1 && !ev.isComposing) typeBytes([...new TextEncoder().encode(key)]);
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });
  // 글자 칸에서 한글처럼 조합해 치는 글자는 조합이 끝난 뒤에 받는다.
  $scroll.addEventListener('compositionend', (ev) => { if (pane === 'text' && ev.data) typeBytes([...new TextEncoder().encode(ev.data)]); });

  let saveTimer = 0;
  const saveSoon = () => { clearTimeout(saveTimer); saveTimer = setTimeout(onChange, 600); };

  const watcher = new ResizeObserver(() => {
    const now = $scroll.clientHeight > 0;
    if (now && !isShown) { isShown = true; check(); }
    else if (!now) isShown = false;
    if (now && file) { setTop(topRow); }
  });
  watcher.observe($scroll);
  const timer = setInterval(() => { if (isShown && !document.hidden) check(); }, CHECK_MS);

  function focus() {
    if (path) $scroll.focus({ preventScroll: true }); else zone.focus();
  }

  $body.hidden = !path;
  zone.node.hidden = !!path;
  $save.disabled = true;
  $title.textContent = title();

  return {
    ready: path ? load(path, { keep: true }).then(() => {}, () => {}) : Promise.resolve(),
    focus,
    title,
    wantsFile: () => !path,
    takeFile,
    path: () => path,
    // 저장하지 않은 고침도 담는다: 앱을 닫거나 업데이트로 다시 시작해도 사라지지 않게(파일이 그대로일 때 되살린다).
    state: () => {
      const out = { path, caret, top: topRow * COLS, little };
      if (file && dirty()) {
        const base = { size: file.size, modified: file.modified ?? null };
        out.edits = edits.size <= MAX_KEEP
          ? { ...base, patches: patchesOf(edits).map((p) => [p.offset, p.bytes.map(hex2).join('')]) }
          : { ...base, lost: edits.size, patches: [] };
      } else if (kept) {
        out.edits = { size: kept.size, modified: kept.modified, lost: kept.lost, patches: kept.patches }; // 아직 열지 못한 파일의 고침
      }
      return out;
    },
    refresh: check,
    reload: () => (path ? load(path, { keep: true }) : Promise.resolve(false)),
    close: () => Promise.resolve(!dirty() || confirm(`${api.baseName(path)}에 저장하지 않은 고침 ${count(edits.size)}바이트가 있습니다. 저장하지 않고 닫을까요?`)),
    flush: () => Promise.resolve(),
    dispose() {
      disposed = true;
      clearInterval(timer);
      clearTimeout(analyzeTimer);
      clearTimeout(saveTimer);
      watcher.disconnect();
      document.removeEventListener('mouseup', stopDrag);
    },
    /** 시험·확인용: 지금 보이는 것. */
    summary: () => ({ path, size: size(), caret, top: topRow, edits: edits.size, format: analysis?.format || '', fields: fieldsFlat.length, pane, note: $note.textContent }),
    moveTo,
    typeHex,
    typeBytes,
    save: (asNew = false) => saveFile(asNew),
    find: (text, hex = true) => { $findInput.value = text; findHex = hex; return runFind(false); },
    pickField: (index) => fieldsFlat[index] && pickField(fieldsFlat[index]),
    undo: () => step(undo, redo),
  };
}
