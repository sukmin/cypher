// 알림 기록: 떴다가 사라지는 알림(완료, 승인 대기, 컨텍스트 경고, Claude가 보낸 알림, 오류 등)을 모아 둔다.
//
// 자리를 비운 사이 어느 세션이 언제 끝났고 얼마나 걸렸는지 나중에 볼 수 있다.
// 이 컴퓨터에 기억하고(localStorage) 7일이 지난 것과 200개를 넘는 것은 버린다.

const KEY = 'cypher.notices';
const MAX = 200;
const KEEP_MS = 7 * 24 * 60 * 60 * 1000;
const ICONS = { done: '✔', approval: '❗', context: '◔', claude: '✉', item: '＋', info: '·' };
const KIND_TEXT = { done: '완료', approval: '승인·입력 대기', context: '컨텍스트 경고', claude: 'Claude가 보낸 알림', item: '새 항목', info: '알림' };

// 이번 실행의 표식. 항목 번호(key)는 실행마다 새로 매겨지므로, 같은 실행에서 남긴 기록만 번호로 찾아간다.
const BOOT = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

let api = null; // { make, openPopup, setStatus, jump({ key, sameRun, title, group }) → 갔으면 true, onChange() }
let list = null; // 최근 것이 앞

export function initNotices(given) {
  api = given;
  load();
  api.onChange?.();
}

function load() {
  if (list) return list;
  let saved = [];
  try { saved = JSON.parse(localStorage.getItem(KEY) || '[]'); } catch { /* 깨진 기록은 버린다 */ }
  const limit = Date.now() - KEEP_MS;
  list = Array.isArray(saved) ? saved.filter((entry) => entry && entry.at > limit && typeof entry.title === 'string') : [];
  return list;
}

function save() {
  try { localStorage.setItem(KEY, JSON.stringify(list.slice(0, MAX))); } catch { /* 저장 공간 문제는 무시 */ }
}

/** 아직 보지 않은 알림 수. */
export const unreadCount = () => load().filter((entry) => entry.unread).length;

/**
 * 알림 하나를 기록한다.
 * kind: done | approval | context | claude | item | info · key: 그 알림이 가리키는 항목의 번호(없으면 null)
 * group: 그 항목의 묶음 이름 · unread: 사용자가 그 자리에서 보지 못했을 알림이면 참
 */
export function recordNotice({ kind = 'info', title = '', body = '', key = null, group = '', unread = true }) {
  load();
  list.unshift({
    at: Date.now(), kind, title: String(title).slice(0, 200), body: String(body).slice(0, 600),
    key, boot: BOOT, group, unread: !!unread,
  });
  if (list.length > MAX) list.length = MAX;
  save();
  api?.onChange?.();
}

/** 그 항목을 가리키는 알림을 읽은 것으로 한다(사용자가 그 항목을 열었을 때). */
export function markRead(key) {
  let changed = false;
  for (const entry of load()) {
    if (entry.unread && entry.boot === BOOT && entry.key === key) { entry.unread = false; changed = true; }
  }
  if (changed) { save(); api?.onChange?.(); }
}

/** 시각을 늘 같은 자릿수로 적는다(11/13 18:00:05). */
function when(at) {
  const d = new Date(at);
  const two = (n) => String(n).padStart(2, '0');
  return `${two(d.getMonth() + 1)}/${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
}

/** 알림 기록 창. 누르면 그 항목으로 간다. 열면 모두 읽은 것으로 한다. */
export function showNotices() {
  const { make, openPopup, setStatus } = api;
  const entries = load();
  const rows = entries.map((entry) => {
    const row = make('div', `row notice-row${entry.unread ? ' unread' : ''}`);
    const use = make('button', 'row-title');
    use.append(
      make('span', `notice-icon ${entry.kind}`, ICONS[entry.kind] || ICONS.info),
      make('b', '', entry.title),
      make('span', 'notice-body', entry.body),
    );
    use.title = `${KIND_TEXT[entry.kind] || KIND_TEXT.info}${entry.group ? ` · ${entry.group}` : ''}\n${entry.title}\n${entry.body}${entry.key != null ? '\n누르면 그 항목으로 갑니다' : ''}`;
    use.onclick = () => {
      close();
      if (entry.key == null) return;
      const went = api.jump({ key: entry.key, sameRun: entry.boot === BOOT, title: entry.title, group: entry.group });
      if (!went) setStatus(`알림을 남긴 항목이 지금은 없습니다: ${entry.title}`);
    };
    row.append(use, make('span', 'row-note', when(entry.at)));
    return row;
  });
  const clear = make('button', 'row add', '기록 지우기');
  clear.hidden = !entries.length;
  clear.onclick = () => { list = []; save(); api.onChange?.(); close(); showNotices(); };
  const unread = unreadCount();
  const close = openPopup(`알림 기록${unread ? ` — 새 알림 ${unread}개` : ''}`, [
    ...rows,
    make('p', 'note', entries.length
      ? '완료·승인 대기·컨텍스트 경고와 Claude가 보낸 알림이 7일 동안 남습니다. 왼쪽에 색 줄이 있는 것은 이 창을 열기 전까지 보지 않은 알림입니다.'
      : '아직 알림이 없습니다. Claude 세션의 응답이 끝나거나 승인을 기다리면 여기에 쌓입니다.'),
    clear,
  ]);
  // 창을 열었으니 모두 읽은 것으로 한다(표시는 이 창이 닫힐 때까지 남는다).
  if (unread) {
    for (const entry of entries) entry.unread = false;
    save();
    api.onChange?.();
  }
}
