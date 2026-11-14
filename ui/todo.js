// 할 일 항목: 그룹(업무)의 체크 목록.
//
// 내용은 Markdown 체크 목록 글이다(다른 곳에서도 그대로 읽힌다). 화면은 그 글을 목록으로 보여 주고,
// 체크·고치기·옮기기는 모두 글의 그 줄을 고치는 일이다. 그래서 같은 그룹의 Claude 세션이 글을 통째로 쓰거나
// (todo_write) 파일을 직접 고쳐도 같은 것을 본다.
//
//   # 결제 개편              ← 제목(사이드바의 이름)
//   ## 설계                  ← 구역
//   - [x] 요구 사항 정리
//   - [ ] 화면 설계
//     - [ ] 결제 수단 고르기  ← 하위 일(두 칸 들여쓰기)
//   그 밖의 줄은 설명으로 보인다.
//
// 내용은 글 그대로 자동 저장된다(todo.rs).

let api = null; // { invoke, make, setStatus, showMenu, toClaude(item, text, ev) }

export function initTodo(given) {
  api = given;
}

// ───────────────────────── 글 읽기·고치기 ─────────────────────────

/** 할 일 줄: 들여쓰기, 목록 표시(- * + 1.), 체크 칸([ ] [x], 없어도 된다), 글. */
const TASK = /^(\s*)([-*+]|\d+[.)])\s+(?:\[([ xX])\](?:\s+|$))?(.*)$/;
/** 제목 줄: # 제목, ## 구역 */
const HEAD = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const MAX_UNDO = 100;

/** 들여쓴 너비(빈칸 하나 1, 탭 하나 2). */
function widthOf(line) {
  let width = 0;
  for (const ch of line) {
    if (ch === ' ') width += 1; else if (ch === '\t') width += 2; else break;
  }
  return width;
}

/**
 * 체크 목록 글을 읽는다.
 * → { title, rows: [{ line(0부터), kind: 'task' | 'head' | 'note', depth, done, text, level, n(할 일 번호, 1부터) }], done, total }
 * 빈 줄은 rows에 넣지 않는다.
 */
export function parseTodo(source) {
  const rows = [];
  let title = '', done = 0, total = 0;
  String(source ?? '').split('\n').forEach((raw, line) => {
    if (!raw.trim()) return;
    const head = HEAD.exec(raw);
    if (head) {
      if (!title && head[1].length === 1) title = head[2];
      rows.push({ line, kind: 'head', level: head[1].length, text: head[2], depth: 0 });
      return;
    }
    const task = TASK.exec(raw);
    if (task) {
      const checked = (task[3] || '').toLowerCase() === 'x';
      total += 1;
      if (checked) done += 1;
      rows.push({ line, kind: 'task', depth: Math.floor(widthOf(raw) / 2), done: checked, text: task[4].trim(), n: total });
      return;
    }
    rows.push({ line, kind: 'note', depth: Math.floor(widthOf(raw) / 2), text: raw.trim() });
  });
  return { title, rows, done, total };
}

/** 그 줄에서 시작하는 덩어리(그 줄과, 더 깊이 들여쓴 바로 아래 줄들)의 끝(그다음 줄 번호). */
function blockEnd(lines, at) {
  const own = widthOf(lines[at]);
  let end = at + 1;
  while (end < lines.length && lines[end].trim() && !HEAD.test(lines[end]) && widthOf(lines[end]) > own) end += 1;
  return end;
}

/** 할 일 줄의 체크와 글을 바꾼 줄. 체크 칸이 없던 줄에는 체크 칸을 붙인다. */
function taskLine(raw, { done, text }) {
  const m = TASK.exec(raw);
  if (!m) return raw;
  const checked = done ?? ((m[3] || '').toLowerCase() === 'x');
  return `${m[1]}${m[2]} [${checked ? 'x' : ' '}] ${text ?? m[4].trim()}`.trimEnd();
}

/** 그 줄의 덩어리를 한 칸 위(-1)나 아래(1)의 같은 깊이 할 일과 바꾼다. → { lines, at(옮긴 뒤의 줄) } 또는 null(옮길 곳이 없다). */
export function moveBlock(lines, at, step) {
  const own = widthOf(lines[at]);
  const end = blockEnd(lines, at);
  if (step < 0) {
    let prev = -1;
    for (let k = at - 1; k >= 0; k -= 1) {
      if (!lines[k].trim() || HEAD.test(lines[k]) || widthOf(lines[k]) < own) break;
      if (widthOf(lines[k]) === own) { if (TASK.test(lines[k])) prev = k; break; }
    }
    if (prev < 0) return null;
    return { lines: [...lines.slice(0, prev), ...lines.slice(at, end), ...lines.slice(prev, at), ...lines.slice(end)], at: prev };
  }
  if (end >= lines.length || !lines[end].trim() || widthOf(lines[end]) !== own || !TASK.test(lines[end])) return null;
  const next = blockEnd(lines, end);
  return { lines: [...lines.slice(0, at), ...lines.slice(end, next), ...lines.slice(at, end), ...lines.slice(next)], at: at + (next - end) };
}

/** 그 줄의 덩어리를 한 단계 들여쓰거나(1) 내어쓴다(-1). → 바뀐 줄들 또는 null. */
export function indentBlock(lines, at, step) {
  const own = widthOf(lines[at]);
  const end = blockEnd(lines, at);
  if (step > 0) {
    // 바로 위에 이 줄을 품을 할 일(같거나 더 깊은 줄)이 있어야 하위 일이 된다.
    const above = lines[at - 1];
    if (at === 0 || !above.trim() || !TASK.test(above) || widthOf(above) < own) return null;
    return lines.map((line, index) => (index >= at && index < end ? `  ${line}` : line));
  }
  if (!own) return null;
  return lines.map((line, index) => (index >= at && index < end ? line.replace(/^(?: {1,2}|\t)/, '') : line));
}

/**
 * 할 일을 가리키는 말(번호 또는 글)로 그 줄을 찾는다. → { row } 또는 { problem }.
 * 글은 전체가 같은 것을 먼저 찾고, 없으면 그 글이 들어 있는 할 일이 하나뿐일 때 그것을 고른다.
 */
function findTask(tasks, ref) {
  const number = typeof ref === 'number' ? ref : (/^\d+$/.test(String(ref).trim()) ? Number(ref) : null);
  const text = String(ref ?? '').trim();
  if (typeof ref !== 'number' && text) {
    const same = tasks.filter((row) => row.text === text);
    if (same.length === 1) return { row: same[0] };
    if (same.length > 1) return { problem: `"${text}": 같은 글의 할 일이 ${same.length}개입니다(번호로 가리키세요: ${same.map((row) => row.n).join(', ')})` };
    if (number == null) {
      const lower = text.toLowerCase();
      const part = tasks.filter((row) => row.text.toLowerCase().includes(lower));
      if (part.length === 1) return { row: part[0] };
      return { problem: part.length ? `"${text}": 들어맞는 할 일이 ${part.length}개입니다(번호로 가리키세요: ${part.map((row) => row.n).join(', ')})` : `"${text}": 그런 할 일이 없습니다` };
    }
  }
  if (number != null && Number.isInteger(number) && number >= 1 && number <= tasks.length) return { row: tasks[number - 1] };
  return { problem: number != null ? `${number}번: 할 일은 1번부터 ${tasks.length}번까지입니다` : '빈 값은 할 일을 가리키지 못합니다' };
}

// ───────────────────────── 화면 ─────────────────────────

const newId = () => `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
const SAVE_MS = 500;          // 자동 저장까지 기다리는 시간
const SAVE_RETRY_MS = 5000;   // 자동 저장에 실패했을 때 다시 해 보기까지의 시간
const clip = (text, max = 40) => (text.length > max ? `${text.slice(0, max)}…` : text);

/**
 * 할 일 항목의 화면을 host 안에 만든다.
 * saved: { todo(저장 번호), code(글 편집 화면을 열어 두었는지), hide(끝낸 일 숨기기), text(처음 넣을 내용) }
 */
export function createTodo(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  const id = typeof saved.todo === 'string' && /^[a-z0-9]{1,40}$/i.test(saved.todo) ? saved.todo : newId();
  let text = '';               // 지금의 글
  let stored = '';             // 저장소에 실제로 들어 있는 글
  let unread = false;          // 시작할 때 저장소를 읽지 못했다(그동안은 자동 저장하지 않는다)
  let model = parseTodo('');
  let selected = -1;           // 고른 줄(글의 줄 번호)
  let editing = null;          // 고치는 중인 줄 { line, input, fresh(새로 넣은 빈 줄) }
  let codeShown = saved.code === true;
  let hideDone = saved.hide === true;
  let saveTimer = 0;
  let disposed = false;
  const undo = [], redo = [];

  const button = (label, tip, action) => {
    const b = make('button', '', label);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const $count = make('span', 'todo-count');
  const $meter = make('span', 'todo-meter');
  const $fill = make('i');
  $meter.append($fill);
  const $title = make('span', 'doc-title');
  const $hide = button('끝낸 일 숨기기', '끝낸 일을 목록에서 숨기기·보이기', () => { hideDone = !hideDone; draw(); onChange(); });
  const $toggle = button('글로 편집', 'Markdown 글로 고치기 ↔ 목록으로 보기 (Ctrl+E)', () => showCode(!codeShown));
  const bar = make('div', 'doc-bar');
  bar.append(
    $meter, $count, $title, $hide, $toggle,
    button('Claude에 넣기', '남은 할 일을 Claude 세션 입력창에 넣기', (ev) => {
      const left = model.rows.filter((row) => row.kind === 'task' && !row.done);
      if (!left.length) return setStatus('남은 할 일이 없습니다.');
      return api.toClaude?.(item, `남은 할 일:\n${left.map((row) => `${'  '.repeat(row.depth)}- [ ] ${row.text}`).join('\n')}\n`, ev);
    }),
    button('복사', 'Markdown 글로 복사', () => invoke('clipboard_write', { text }).then(() => setStatus('할 일 목록을 복사했습니다.')).catch((err) => setStatus(String(err)))),
    button('정리', '끝낸 일 지우기 · 모두 체크 풀기', (ev) => showMenu(ev, [
      { caption: `끝낸 일 ${model.done}개 · 남은 일 ${model.total - model.done}개` },
      { label: '끝낸 일 지우기 (Ctrl+Z로 되살립니다)', action: () => clearDone() },
      { label: '모두 체크 풀기', action: () => checkAll(false) },
      { label: '모두 끝냄으로', action: () => checkAll(true) },
    ])),
  );
  const $list = make('div', 'todo-list');
  $list.tabIndex = 0;
  const $empty = make('div', 'reader-empty', '아래 칸에 할 일을 적고 Enter를 누르면 목록에 들어갑니다. 같은 그룹의 Claude 세션에게 "할 일 목록을 만들어 줘"라고 해도 됩니다.');
  const $add = make('input', 'todo-add');
  Object.assign($add, { placeholder: '할 일을 적고 Enter (목록 끝에 더하기)', spellcheck: false, autocomplete: 'off' });
  const $listView = make('div', 'todo-view');
  $listView.append($empty, $list, $add);
  const $code = make('textarea', 'todo-code');
  Object.assign($code, { spellcheck: false, autocomplete: 'off' });
  $code.placeholder = '# 제목\n- [ ] 할 일\n  - [ ] 하위 일\n- [x] 끝낸 일';
  const box = make('div', 'doc todo');
  box.append(bar, $listView, $code);
  host.append(box);

  const lines = () => text.split('\n');
  const rowAt = (line) => model.rows.find((row) => row.line === line);
  const tasks = () => model.rows.filter((row) => row.kind === 'task');
  const title = () => clip(model.title.trim()) || '할 일';

  /** 글을 바꾼다. 목록에서 한 일은 되돌리기(Ctrl+Z)에 남긴다. */
  function commit(next, { remember = true } = {}) {
    if (next === text) return;
    if (remember) {
      undo.push({ text, selected });
      if (undo.length > MAX_UNDO) undo.shift();
      redo.length = 0;
    }
    text = next;
    model = parseTodo(text);
    if (codeShown && $code.value !== text) $code.value = text;
    draw();
    scheduleSave();
    onChange();
  }

  function stepUndo(from, to) {
    const last = from.pop();
    if (!last) return;
    cancelEdit();
    to.push({ text, selected });
    selected = last.selected;
    commit(last.text, { remember: false });
  }

  // ── 그리기 ──

  function draw() {
    const { done, total } = model;
    $count.textContent = total ? `${done}/${total} 완료` : '할 일 없음';
    $fill.style.width = total ? `${Math.round((done / total) * 100)}%` : '0';
    $meter.classList.toggle('full', total > 0 && done === total);
    $title.textContent = title();
    $hide.classList.toggle('on', hideDone);
    $hide.hidden = codeShown;
    $toggle.classList.toggle('on', codeShown);
    $toggle.textContent = codeShown ? '목록으로' : '글로 편집';
    $listView.hidden = codeShown;
    $code.hidden = !codeShown;
    if (codeShown) return;
    if (editing) return; // 고치는 중에는 입력칸이 사라지지 않게 목록을 그대로 둔다(끝나면 다시 그린다).
    const shown = model.rows.filter((row) => !(hideDone && row.kind === 'task' && row.done));
    $empty.hidden = model.rows.length > 0;
    if (selected >= 0 && !shown.some((row) => row.line === selected && row.kind === 'task')) selected = -1;
    $list.replaceChildren(...shown.map(drawRow));
  }

  function drawRow(row) {
    if (row.kind === 'head') {
      const head = make('div', `todo-head level-${Math.min(row.level, 3)}`, row.text);
      head.dataset.line = row.line;
      return head;
    }
    if (row.kind === 'note') {
      const note = make('div', 'todo-note', row.text);
      note.style.paddingLeft = `${12 + row.depth * 22}px`;
      note.dataset.line = row.line;
      return note;
    }
    const node = make('div', `todo-row${row.done ? ' done' : ''}${row.line === selected ? ' sel' : ''}`);
    node.dataset.line = row.line;
    node.style.paddingLeft = `${8 + row.depth * 22}px`;
    const check = make('input');
    check.type = 'checkbox';
    check.checked = row.done;
    check.tabIndex = -1;
    check.title = row.done ? '체크 풀기 (Space)' : '끝냄으로 체크 (Space)';
    const label = make('span', 'todo-text', row.text || '(빈 할 일)');
    if (!row.text) label.classList.add('blank');
    const give = make('button', 'todo-act', '→ Claude');
    give.title = '이 할 일을 Claude 세션 입력창에 넣기';
    give.dataset.act = 'give';
    const remove = make('button', 'todo-act close', '×');
    remove.title = '지우기 (Delete)';
    remove.dataset.act = 'remove';
    node.append(check, label, give, remove);
    return node;
  }

  /** 고른 줄이 보이게 하고 표시한다. */
  function select(line, { scroll = true } = {}) {
    selected = line;
    for (const node of $list.querySelectorAll('.todo-row')) node.classList.toggle('sel', Number(node.dataset.line) === line);
    if (scroll) $list.querySelector('.todo-row.sel')?.scrollIntoView({ block: 'nearest' });
  }

  // ── 목록에서 하는 일 ──

  function toggle(line) {
    const row = rowAt(line);
    if (row?.kind !== 'task') return;
    const all = lines();
    all[line] = taskLine(all[line], { done: !row.done });
    selected = line;
    commit(all.join('\n'));
  }

  function remove(line) {
    if (rowAt(line)?.kind !== 'task') return;
    const all = lines();
    const end = blockEnd(all, line);
    const order = tasks();
    const index = order.findIndex((row) => row.line === line);
    const after = order.find((row) => row.line >= end), before = order[index - 1];
    all.splice(line, end - line);
    // 지운 자리의 다음 할 일(없으면 앞의 것)을 고른다. 다음 것은 지운 줄 수만큼 위로 올라온다.
    selected = after ? after.line - (end - line) : before ? before.line : -1;
    commit(all.join('\n'));
  }

  function move(line, step) {
    if (rowAt(line)?.kind !== 'task') return;
    const moved = moveBlock(lines(), line, step);
    if (!moved) return;
    selected = moved.at;
    commit(moved.lines.join('\n'));
  }

  function indent(line, step) {
    if (rowAt(line)?.kind !== 'task') return false;
    const moved = indentBlock(lines(), line, step);
    if (!moved) return false;
    selected = line;
    commit(moved.join('\n'));
    return true;
  }

  function checkAll(done) {
    cancelEdit();
    commit(lines().map((raw) => (TASK.test(raw) && !HEAD.test(raw) ? taskLine(raw, { done }) : raw)).join('\n'));
  }

  function clearDone() {
    cancelEdit();
    const all = lines();
    const kept = [];
    for (let at = 0; at < all.length;) {
      const m = !HEAD.test(all[at]) && TASK.exec(all[at]);
      if (m && (m[3] || '').toLowerCase() === 'x') at = blockEnd(all, at); // 끝낸 일은 하위 일과 함께 지운다.
      else { kept.push(all[at]); at += 1; }
    }
    if (kept.length === all.length) return setStatus('끝낸 일이 없습니다.');
    selected = -1;
    commit(kept.join('\n'));
    return setStatus('끝낸 일을 지웠습니다. (Ctrl+Z: 되살리기)');
  }

  /** 목록 끝에 할 일을 더한다. */
  function addLast(words) {
    const body = text.replace(/\n+$/, '');
    commit(`${body}${body ? '\n' : ''}- [ ] ${words}\n`);
    const last = tasks().pop();
    if (last) select(last.line);
  }

  /** 그 줄 아래(하위 일들 다음)에 같은 깊이의 빈 할 일을 넣고 바로 고친다. */
  function insertAfter(line) {
    const all = lines();
    const lead = /^\s*/.exec(all[line])[0];
    const at = blockEnd(all, line);
    all.splice(at, 0, `${lead}- [ ] `);
    // 되돌리면(Ctrl+Z) 넣기 전에 있던 줄이 골라져 있게, 고른 줄은 되돌리기에 적어 둔 뒤에 새 줄로 옮긴다.
    selected = line;
    commit(all.join('\n'));
    edit(at, true);
  }

  // ── 그 자리에서 고치기 ──

  function edit(line, fresh = false) {
    const row = rowAt(line);
    const node = $list.querySelector(`.todo-row[data-line="${line}"]`);
    if (row?.kind !== 'task' || !node) return;
    finishEdit();
    const input = make('input', 'todo-edit');
    Object.assign(input, { value: row.text, spellcheck: false, autocomplete: 'off' });
    node.querySelector('.todo-text').replaceWith(input);
    editing = { line, input, fresh };
    select(line);
    input.focus();
    input.select();
    input.addEventListener('keydown', (ev) => {
      ev.stopPropagation(); // 목록의 단축키(Space, Delete …)가 글자 입력을 가로채지 않게 한다.
      if (ev.key === 'Enter') {
        ev.preventDefault();
        const had = input.value.trim();
        const at = finishEdit();
        // 글을 적고 Enter: 그 아래에 다음 할 일을 이어서 적는다. 빈 채로 Enter: 그만 적는다.
        if (had && at >= 0) insertAfter(at); else $list.focus();
      } else if (ev.key === 'Escape') {
        ev.preventDefault();
        cancelEdit();
        $list.focus();
      } else if (ev.key === 'Tab') {
        ev.preventDefault();
        const at = finishEdit();
        if (at >= 0) { indent(at, ev.shiftKey ? -1 : 1); edit(at); }
      }
    });
    input.addEventListener('blur', () => { if (editing?.input === input) finishEdit(); });
  }

  /** 고치던 것을 반영한다. 글을 비웠으면 그 할 일을 지운다. → 그 줄 번호(지웠으면 -1). */
  function finishEdit() {
    if (!editing) return -1;
    const { line, input, fresh } = editing;
    editing = null;
    const words = input.value.trim().replace(/\s*\n\s*/g, ' ');
    const all = lines();
    if (rowAt(line)?.kind !== 'task') { draw(); return -1; }
    if (!words) {
      // 새로 넣은 빈 줄은 되돌리기에 남기지 않고 없앤다.
      all.splice(line, blockEnd(all, line) - line);
      const before = tasks().filter((row) => row.line < line).pop();
      selected = before ? before.line : -1;
      if (fresh) undo.pop();
      commit(all.join('\n'), { remember: !fresh });
      draw();
      return -1;
    }
    all[line] = taskLine(all[line], { text: words });
    selected = line;
    const next = all.join('\n');
    // 새로 넣은 줄에 글을 적은 것은 "줄을 넣은 일"과 한 번의 되돌리기로 묶는다(빈 줄이 중간 상태로 되살아나지 않게).
    if (next === text) draw(); else commit(next, { remember: !fresh });
    return line;
  }

  /** 고치던 것을 버린다(새로 넣은 빈 줄이면 그 줄도 없앤다). */
  function cancelEdit() {
    if (!editing) return;
    const { input, fresh } = editing;
    if (fresh) { input.value = ''; finishEdit(); return; }
    editing = null;
    draw();
  }

  // ── 마우스·키보드 ──

  // 고치는 중에 목록의 다른 곳을 누르면 입력칸이 초점을 잃으면서 목록을 다시 그리는데, 누름과 뗌 사이에 줄이 새것으로
  // 바뀌면 그 클릭이 사라진다. 그래서 누를 때는 초점을 그대로 두고, 클릭을 처리할 때 고치던 것을 먼저 반영한다.
  $list.addEventListener('mousedown', (ev) => {
    if (editing && !ev.target.closest('.todo-edit')) ev.preventDefault();
  });
  $list.addEventListener('click', (ev) => {
    const node = ev.target.closest('.todo-row');
    if (ev.target.closest('.todo-edit')) return;
    let line = node ? Number(node.dataset.line) : -1;
    if (editing) {
      // 고치던 줄을 비워 지웠으면 그 아래 줄들의 번호가 당겨진다.
      const edited = editing.line, before = lines().length;
      finishEdit();
      const removed = before - lines().length;
      if (removed && line === edited) line = -1;
      else if (removed && line > edited) line -= removed;
      $list.focus({ preventScroll: true });
    }
    if (line < 0 || rowAt(line)?.kind !== 'task') return undefined;
    if (ev.target.matches('input[type="checkbox"]')) { ev.preventDefault(); return toggle(line); }
    const act = ev.target.closest('[data-act]')?.dataset.act;
    if (act === 'remove') return remove(line);
    if (act === 'give') return api.toClaude?.(item, `${rowAt(line)?.text || ''} `, ev);
    return select(line, { scroll: false });
  });
  $list.addEventListener('dblclick', (ev) => {
    const node = ev.target.closest('.todo-row');
    if (node && !ev.target.closest('input, button')) edit(Number(node.dataset.line));
  });
  $list.addEventListener('contextmenu', (ev) => {
    const node = ev.target.closest('.todo-row');
    if (!node) return;
    ev.preventDefault();
    const line = Number(node.dataset.line);
    const row = rowAt(line);
    select(line, { scroll: false });
    showMenu(ev, [
      { label: row.done ? '체크 풀기  (Space)' : '끝냄으로 체크  (Space)', action: () => toggle(line) },
      { label: '고치기  (Enter, F2)', action: () => edit(line) },
      { label: '아래에 할 일 넣기  (Ctrl+Enter)', action: () => insertAfter(line) },
      { label: '하위 일로  (Tab)', action: () => indent(line, 1) },
      { label: '한 단계 위로  (Shift+Tab)', action: () => indent(line, -1) },
      { label: '위로 옮기기  (Alt+↑)', action: () => move(line, -1) },
      { label: '아래로 옮기기  (Alt+↓)', action: () => move(line, 1) },
      { label: 'Claude 세션 입력창에 넣기', action: () => api.toClaude?.(item, `${row.text} `, ev) },
      { label: '지우기  (Delete)', action: () => remove(line) },
    ]);
  });
  $list.addEventListener('keydown', (ev) => {
    const key = ev.key;
    const order = tasks().filter((row) => !(hideDone && row.done));
    const at = order.findIndex((row) => row.line === selected);
    let handled = true;
    if (ev.ctrlKey && !ev.altKey && key.toLowerCase() === 'z') stepUndo(ev.shiftKey ? redo : undo, ev.shiftKey ? undo : redo);
    else if (ev.ctrlKey && !ev.altKey && key.toLowerCase() === 'y') stepUndo(redo, undo);
    else if (key === 'ArrowDown' && !ev.altKey) { if (order.length) select(order[Math.min(order.length - 1, at + 1)].line); }
    else if (key === 'ArrowUp' && !ev.altKey) { if (order.length) select(order[Math.max(0, at < 0 ? 0 : at - 1)].line); }
    else if (key === 'Home') { if (order.length) select(order[0].line); }
    else if (key === 'End') { if (order.length) select(order[order.length - 1].line); }
    else if (at < 0) handled = false;
    else if (key === ' ') toggle(selected);
    else if (key === 'Enter' && ev.ctrlKey) insertAfter(selected);
    else if (key === 'Enter' || key === 'F2') edit(selected);
    else if (key === 'Delete') remove(selected);
    else if (key === 'ArrowUp' && ev.altKey) move(selected, -1);
    else if (key === 'ArrowDown' && ev.altKey) move(selected, 1);
    else if (key === 'Tab' && !ev.ctrlKey) indent(selected, ev.shiftKey ? -1 : 1);
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });
  $add.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' && $add.value.trim()) {
      ev.preventDefault();
      addLast($add.value.trim());
      $add.value = '';
    } else if (ev.key === 'ArrowUp' && !$add.value) {
      // 빈 칸에서 ↑: 목록의 마지막 할 일로 간다.
      const last = tasks().filter((row) => !(hideDone && row.done)).pop();
      if (last) { ev.preventDefault(); $list.focus(); select(last.line); }
    }
  });
  $code.addEventListener('input', () => {
    text = $code.value.replace(/\r\n?/g, '\n');
    model = parseTodo(text);
    draw();
    scheduleSave();
    onChange();
  });
  $code.addEventListener('keydown', (ev) => {
    if (ev.key === 'Tab' && !ev.ctrlKey && !ev.altKey) {
      ev.preventDefault();
      document.execCommand('insertText', false, '  ');
    }
  });
  box.addEventListener('keydown', (ev) => {
    if (ev.ctrlKey && !ev.altKey && !ev.shiftKey && ev.key.toLowerCase() === 'e') {
      ev.preventDefault();
      ev.stopPropagation();
      showCode(!codeShown);
    }
  });

  function showCode(shown) {
    finishEdit();
    codeShown = shown;
    if (shown) $code.value = text;
    draw();
    focus();
    onChange();
  }

  function focus() {
    if (codeShown) $code.focus();
    else if (!model.rows.length) $add.focus();
    else $list.focus({ preventScroll: true });
  }

  // ── 저장 ──

  function scheduleSave(wait = SAVE_MS) {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(flush, wait);
  }

  /** 자동 저장. 저장소를 읽지 못한 채로는 적지 않는다(읽지 못한 내용을 덮어쓰지 않게). */
  async function flush() {
    clearTimeout(saveTimer);
    saveTimer = 0;
    if (unread) return;
    const now = text;
    if (now === stored) return;
    try {
      await invoke('todo_write', { id, text: now });
      stored = now;
    } catch (err) {
      if (!disposed) {
        setStatus(`할 일을 저장하지 못했습니다: ${err} · 잠시 뒤 다시 저장합니다.`);
        scheduleSave(SAVE_RETRY_MS);
      }
    }
  }

  /** 밖에서(같은 그룹의 Claude 세션이 파일을 직접 고친 경우 등) 저장소의 글이 바뀌었으면 읽어 들인다. */
  async function reload() {
    if (disposed || editing) return;
    const base = stored;
    if (!unread && text !== base) return; // 여기서 고치는 중인 글은 덮어쓰지 않는다.
    let found = null;
    try { found = await invoke('todo_read', { id }); } catch { return; }
    if (disposed || found == null) return;
    found = found.replace(/\r\n?/g, '\n');
    if (unread) {
      unread = false;
      stored = found;
      if (found !== text) commit(found, { remember: false });
      return setStatus('할 일의 내용을 다시 읽었습니다.');
    }
    if (found === base || stored !== base || text !== base || editing) return;
    stored = found;
    selected = -1;
    commit(found);
  }

  /** 밖에서 내용을 바꾸기 전에: 고치던 것을 먼저 반영한다(사용자가 치던 글이 사라지지 않게). */
  function settle() {
    finishEdit();
    unread = false;
  }

  /** Claude에게 돌려줄 지금의 목록: 할 일마다 번호를 붙인다. */
  function listing() {
    if (!model.total) return '(할 일이 없습니다)';
    return model.rows.map((row) => (row.kind === 'head' ? `${'#'.repeat(row.level)} ${row.text}`
      : row.kind === 'note' ? `${'  '.repeat(row.depth)}${row.text}`
      : `${'  '.repeat(row.depth)}${row.n}. [${row.done ? 'x' : ' '}] ${row.text}`)).join('\n');
  }

  const ready = (async () => {
    // 처음 넣을 내용이 주어졌으면(Claude 세션이 만든 것, 그룹 틀) 그것으로, 아니면 자동 저장소에서 읽는다.
    const given = typeof saved.text === 'string' ? saved.text : null;
    let kept = null;
    if (given == null) {
      try {
        kept = await invoke('todo_read', { id }); // 아직 적어 둔 것이 없으면 null, 읽지 못하면 오류다.
      } catch (err) {
        unread = true;
        setStatus(`할 일의 내용을 읽지 못했습니다: ${err} · 이 항목을 다시 보면 다시 읽습니다(그때까지 자동 저장하지 않습니다).`);
      }
    }
    stored = (kept ?? '').replace(/\r\n?/g, '\n');
    text = (given ?? kept ?? '').replace(/\r\n?/g, '\n');
    model = parseTodo(text);
    if (codeShown) $code.value = text;
    draw();
    if (text !== stored) await flush();
    onChange();
  })();

  return {
    ready,
    focus: () => { focus(); reload(); },
    title,
    state: () => ({ todo: id, code: codeShown, hide: hideDone }),
    /** 저장할 것을 바로 저장한다(앱이 꺼지기 직전). */
    flush: () => { finishEdit(); return flush(); },
    /** Markdown 체크 목록 글. */
    text: () => text,
    /** 내용을 통째로 바꾼다. 같은 그룹의 Claude 세션이 고칠 때 쓴다(초점을 옮기지 않는다). */
    setText: (source) => {
      settle();
      selected = -1; // 줄이 통째로 바뀌므로 고른 줄 표시는 푼다(옛 줄 번호가 엉뚱한 줄을 가리키지 않게).
      commit(String(source ?? '').replace(/\r\n?/g, '\n'));
      return true;
    },
    /** 목록 끝에 덧붙인다. */
    append: (source) => {
      settle();
      const more = String(source ?? '').replace(/\r\n?/g, '\n').replace(/^\n+|\n+$/g, '');
      const body = text.replace(/\n+$/, '');
      commit(`${body}${body && more ? '\n' : ''}${more}\n`);
      return true;
    },
    /**
     * 할 일을 체크하거나(done) 푼다. refs: 번호(1부터)나 글의 목록.
     * → { changed(바꾼 수), already(이미 그 상태이던 수), problems: [찾지 못한 것의 설명] }
     */
    check: (refs, done) => {
      settle();
      const all = lines();
      const problems = [];
      let changed = 0, already = 0;
      for (const ref of Array.isArray(refs) ? refs : [refs]) {
        const found = findTask(tasks(), ref);
        if (!found.row) { problems.push(found.problem); continue; }
        if (found.row.done === !!done && TASK.exec(all[found.row.line])[3] != null) { already += 1; continue; }
        all[found.row.line] = taskLine(all[found.row.line], { done: !!done });
        changed += 1;
      }
      commit(all.join('\n'));
      return { changed, already, problems };
    },
    /** 끝낸 수와 전체 수(사이드바의 진행 표시). */
    progress: () => ({ done: model.done, total: model.total }),
    /** 사이드바 줄 아래에 보일 글. */
    sub: () => (model.total ? `${model.done}/${model.total} 완료` : ''),
    /** 사이드바 줄의 상태(색): 모두 끝냈으면 'ok'. */
    tone: () => (model.total && model.done === model.total ? 'ok' : ''),
    listing,
    /** 글자를 넣는다(끌어다 놓기 등): 줄마다 할 일 하나로 목록 끝에 더한다. */
    insert: (given) => {
      const words = String(given ?? '').split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
      if (!words.length) return;
      settle();
      const body = text.replace(/\n+$/, '');
      commit(`${body}${body ? '\n' : ''}${words.map((line) => (TASK.test(line) ? line : `- [ ] ${line}`)).join('\n')}\n`);
    },
    setCode: (shown) => showCode(!!shown),
    /** 자동 저장소의 번호(파일 이름). */
    store: () => id,
    close: async () => { finishEdit(); await flush(); return true; },
    dispose: () => {
      disposed = true;
      clearTimeout(saveTimer);
      // 닫은 항목의 내용은 바로 지우지 않고 보관해 둔다(30일).
      invoke('todo_remove', { id, keep: true }).catch(() => {});
    },
  };
}
