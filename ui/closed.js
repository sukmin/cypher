// 닫은 항목 다시 열기(Ctrl+Shift+R): 닫은 항목을 기억해 두었다가 같은 자리(그룹·폴더·별칭·색)에 되살린다.
//
// - Claude 세션: 닫을 때의 대화 번호로 대화를 이어 연다(`claude --resume`). 대화가 없던 세션과 다른 터미널은 같은 폴더에서 새로 시작한다.
// - 메모·마인드맵·ERD·할 일·이미지: 닫을 때 저장소의 `closed` 폴더로 옮겨 둔 내용을 제자리로 되돌린 뒤 연다(30일 보관).
// - 사이트·HTTP·문서·표: 보던 주소·파일로 다시 연다. 명령: 적어 둔 명령으로 다시 연다(실행하지는 않는다).
//
// 목록은 이 컴퓨터에 기억한다(localStorage). 보관 기간(30일)이 지난 것은 목록에서도 뺀다.

const KEY = 'cypher.closed';
const MAX = 30;
const KEEP_MS = 30 * 24 * 60 * 60 * 1000;
/** 내용을 저장소에 두는 종류: 되살리기 전에 보관해 둔 파일을 제자리로 되돌려야 한다. */
const STORED = {
  memo: (saved) => (saved.memo ? { kind: 'memo', id: saved.memo, ext: null } : null),
  mindmap: (saved) => (saved.map ? { kind: 'mindmap', id: saved.map, ext: null } : null),
  erd: (saved) => (saved.erd ? { kind: 'erd', id: saved.erd, ext: null } : null),
  todo: (saved) => (saved.todo ? { kind: 'todo', id: saved.todo, ext: null } : null),
  image: (saved) => (saved.image ? { kind: 'image', id: saved.image, ext: saved.ext } : null),
};

let api = null; // { invoke, make, openPopup, setStatus, notice, revive(record) → 항목, icon(record), kindLabel(record), baseName }

export function initClosed(given) {
  api = given;
}

function load() {
  let list = [];
  try { list = JSON.parse(localStorage.getItem(KEY) || '[]'); } catch { /* 깨진 기록은 버린다 */ }
  const limit = Date.now() - KEEP_MS;
  return Array.isArray(list) ? list.filter((record) => record && record.at > limit) : [];
}

function save(list) {
  try { localStorage.setItem(KEY, JSON.stringify(list.slice(0, MAX))); } catch { /* 저장 공간 문제는 무시 */ }
}

/** 닫은 항목 수. */
export const closedCount = () => load().length;

/** 방금 닫은 항목을 목록 맨 앞에 적어 둔다. record: { kind, cwd, name, group, color, label, profile?, resume?, saved? } */
export function rememberClosed(record) {
  save([{ ...record, at: Date.now(), ref: `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}` }, ...load()]);
}

/** 닫은 시각을 늘 같은 자릿수로 적는다(11/13 18:00). */
function when(at) {
  const d = new Date(at);
  const two = (n) => String(n).padStart(2, '0');
  return `${two(d.getMonth() + 1)}/${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}

/** 목록의 그 항목을 되살린다. 되살린 항목을 돌려준다(못 했으면 null). */
async function reopen(record) {
  const { invoke, setStatus, notice } = api;
  const stored = STORED[record.kind]?.(record.saved || {});
  if (stored) {
    let back = false;
    try {
      back = await invoke('closed_restore', stored);
    } catch (err) {
      notice('다시 열지 못했습니다', `${record.label}: ${err}`);
      return null;
    }
    // 파일에 연결된 메모는 보관본이 없어도 그 파일로 다시 연다. 그 밖에는 되살릴 내용이 없다.
    if (!back && !(record.kind === 'memo' && record.saved.path)) {
      save(load().filter((other) => other.ref !== record.ref));
      notice('다시 열지 못했습니다', `${record.label}: 보관해 둔 내용이 없습니다(보관 기간 30일이 지났거나 지워졌습니다).`);
      return null;
    }
  }
  save(load().filter((other) => other.ref !== record.ref));
  const item = await api.revive(record);
  if (item?.exited) {
    // 세션을 띄우지 못했다(폴더가 없어졌거나 프로그램을 찾지 못함). 까닭은 그 세션 화면에 적혀 있다.
    setStatus(`다시 열었지만 세션을 시작하지 못했습니다: ${record.label}`);
  } else if (item) {
    const note = record.kind !== 'term' ? '' : record.resume ? ' (대화를 이어 갑니다)' : ' (같은 폴더에서 새로 시작했습니다)';
    setStatus(`다시 열었습니다: ${record.label}${note}`);
  }
  return item;
}

/** 가장 나중에 닫은 항목을 바로 되살린다. */
export async function reopenLast() {
  const [record] = load();
  if (!record) { api.setStatus('다시 열 닫은 항목이 없습니다.'); return null; }
  return reopen(record);
}

/** 닫은 항목 목록 창. 누르면 그 항목을 되살린다. */
export function showClosed() {
  const { make, openPopup } = api;
  const list = load();
  const rows = list.map((record) => {
    const row = make('div', 'row');
    const use = make('button', 'row-title', `${api.icon(record)} ${record.label}`);
    const where = record.group ? `◆ ${record.group}` : api.baseName(record.cwd || '');
    use.title = `${api.kindLabel(record)} · ${where}\n${record.cwd || ''}\n클릭: 같은 자리에 다시 열기`;
    use.onclick = () => { close(); reopen(record); };
    const note = make('span', 'row-note', `${where} · ${when(record.at)}`);
    const remove = make('button', 'close', '×');
    remove.title = '목록에서 지우기';
    remove.onclick = () => { save(load().filter((other) => other.ref !== record.ref)); close(); showClosed(); };
    row.append(use, note, remove);
    return row;
  });
  const clear = make('button', 'row add', '목록 비우기');
  clear.hidden = !list.length;
  clear.onclick = () => { save([]); close(); showClosed(); };
  const close = openPopup('닫은 항목 다시 열기', [
    ...rows,
    make('p', 'note', list.length
      ? 'Claude 세션은 대화를 이어서 열고, 메모·마인드맵·이미지는 보관해 둔 내용(30일)으로 엽니다. Ctrl+Shift+R은 가장 나중에 닫은 것을 바로 엽니다.'
      : '닫은 항목이 없습니다. 항목을 닫으면 여기에 30일 동안 남고, Ctrl+Shift+R로 가장 나중에 닫은 것을 바로 다시 엽니다.'),
    clear,
  ]);
}
