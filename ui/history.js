// 삭제한 워크스페이스의 기록: 워크스페이스를 삭제하면 그때의 모습을 3년 동안 보관한다.
//
// - 기록 하나가 파일 하나다(`%APPDATA%\cypher\history\<삭제한 시각>-<번호>.json`, 앱의 history 모듈). 닫은 항목 목록(30일, 30개)과 달리
//   이 컴퓨터의 디스크에 두므로 3년 동안 쌓여도 저장 공간(localStorage)을 쓰지 않는다.
// - 글로 보관할 수 있는 것은 내용째 담는다: 메모·할 일·마인드맵·ERD의 글, 명령 항목의 명령.
// - 세션은 프로필·폴더·이어 갈 대화 번호(SSH는 서버 주소)만, 사이트·HTTP·문서·표·코드·헥사·이미지 같은 산출물은 이름과
//   가리키던 주소·경로만 담는다(파일 내용은 보관하지 않는다).
// - 기록 창에서 내용을 보고 복사하거나, 워크스페이스로 되살린다(세션은 쉬는 세션으로 놓이고, 누르면 대화를 이어 재개한다).
//
// 무엇을 담고 어떻게 되살릴지는 main.js가 알고(틀 펼치기와 같은 길), 여기서는 저장소와 창을 맡는다.

let api = null;
// { invoke, make, openPopup, ask, setStatus, notice, icon(item) → 기호, kindLabel(item) → 종류 이름,
//   taken(name) → 그 이름의 워크스페이스가 지금 있는지, restore(record, name) → Promise<{ made, skipped }> }

export function initHistory(given) {
  api = given;
}

/** 때를 늘 같은 자릿수로 적는다(2026-11-25 18:00). `dateOnly`이면 날짜만. */
function when(ms, dateOnly = false) {
  if (!ms) return '';
  const d = new Date(ms);
  const two = (n) => String(n).padStart(2, '0');
  const date = `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}`;
  return dateOnly ? date : `${date} ${two(d.getHours())}:${two(d.getMinutes())}`;
}

/**
 * 삭제하려는 워크스페이스의 기록을 적어 둔다. record: { name, folder, summary, items }.
 * 기록 번호를 돌려준다(적지 못했으면 오류를 던진다). 삭제를 그만두게 되면 `dropHistory`로 지운다.
 */
export function keepHistory(record) {
  return api.invoke('history_save', { record });
}

/** 기록 하나를 지운다(워크스페이스를 끝내 삭제하지 않았을 때). */
export function dropHistory(id) {
  return api.invoke('history_remove', { id }).catch(() => {});
}

/** 보관해 둔 기록 수(시험·빠른 이동용). */
export const historyCount = () => api.invoke('history_list').then((list) => list.length).catch(() => 0);

/** 항목 하나에서 보여 줄 것: 보관해 둔 글(text)과 가리키던 곳(link). */
function contentOf(item) {
  const saved = item.saved && typeof item.saved === 'object' ? item.saved : {};
  const pick = (...values) => values.find((value) => typeof value === 'string' && value) || '';
  return {
    text: pick(saved.text, saved.command),
    link: pick(saved.url, saved.file, saved.path, saved.root, saved.open, item.file),
  };
}

/** 세션 항목의 한 줄 설명: 서버 주소(SSH)나 이어 갈 대화가 있는지. */
function sessionNote(item) {
  if (item.ssh?.target) return `서버 ${item.ssh.target}${item.ssh.port ? `:${item.ssh.port}` : ''} (사용자 이름·비밀번호는 보관하지 않습니다)`;
  return item.resume ? '이어 갈 대화가 있습니다 (되살리면 쉬는 세션으로 놓이고, 누르면 대화를 이어 갑니다)' : '대화 없이 닫힌 세션 (되살리면 같은 폴더에서 새로 시작합니다)';
}

/** 기록을 워크스페이스로 되살린다: 이름을 묻고(있는 워크스페이스 이름을 적으면 그 워크스페이스에 더한다) 항목을 다시 만든다. */
async function restore(info, record) {
  const { ask, setStatus, notice } = api;
  const suggested = api.taken(info.name) ? `${info.name} (되살림)` : info.name;
  const name = await ask(
    `"${info.name}"을(를) 되살릴 워크스페이스 이름`,
    '있는 워크스페이스 이름을 적으면 그 워크스페이스에 더합니다', suggested,
  );
  if (!name) return;
  setStatus(`기록을 되살리는 중: ${info.name}…`);
  try {
    const { made, skipped } = await api.restore(record, name);
    setStatus(`되살렸습니다: ${info.name} → 워크스페이스 "${name}"에 항목 ${made}개${skipped.length ? ` — ${skipped.join(' · ')}` : ''} (기록은 그대로 남습니다)`);
  } catch (err) {
    notice('기록을 되살리지 못했습니다', `${info.name}: ${err}`);
  }
}

/** 기록 하나를 지울지 묻고 지운다. 지웠으면 참. */
async function remove(info) {
  if (!confirm(`삭제한 워크스페이스 "${info.name}"의 기록을 지울까요?\n(${when(info.deleted)}에 삭제 · 지우면 보관해 둔 메모·할 일의 글도 함께 사라집니다)`)) return false;
  try {
    await api.invoke('history_remove', { id: info.id });
    return true;
  } catch (err) {
    api.setStatus(`기록을 지우지 못했습니다: ${err}`);
    return false;
  }
}

/** 기록 하나의 내용 창: 삭제할 때 있던 항목과 보관해 둔 글. */
async function showRecord(info) {
  const { invoke, make, openPopup, setStatus, notice } = api;
  let record;
  try {
    record = await invoke('history_read', { id: info.id });
  } catch (err) {
    return notice('기록을 열지 못했습니다', `${info.name}: ${err}`);
  }
  const items = (Array.isArray(record.items) ? record.items : []).filter((item) => item && typeof item === 'object');
  const rows = items.map((item) => {
    const box = make('div', 'history-item');
    const head = make('div', 'history-head');
    const title = make('b', '', `${api.icon(item)} ${typeof item.label === 'string' && item.label ? item.label : api.kindLabel(item)}`);
    head.append(title, make('span', 'history-kind', api.kindLabel(item)));
    box.append(head);
    const term = !item.kind || item.kind === 'term';
    const { text, link } = contentOf(item);
    if (term) box.append(make('div', 'history-link', sessionNote(item)));
    if (link) box.append(make('div', 'history-link', link));
    else if (!term && typeof item.cwd === 'string' && item.cwd && !text) box.append(make('div', 'history-link', item.cwd));
    if (text) {
      const copy = make('button', 'history-copy', '복사');
      copy.title = '보관해 둔 글을 클립보드에 복사';
      copy.onclick = () => invoke('clipboard_write', { text })
        .then(() => setStatus(`복사했습니다: ${item.label || api.kindLabel(item)} (${text.length}자)`))
        .catch((err) => setStatus(String(err)));
      head.append(copy);
      box.append(make('pre', 'history-text', text));
    } else if (!term && !link) {
      box.append(make('div', 'history-link none', '내용은 보관하지 않았습니다 (이름만 남깁니다)'));
    }
    return box;
  });
  const back = make('button', 'row add', '← 기록 목록으로');
  back.onclick = () => { close(); showHistory(); };
  const revive = make('button', 'row add', '워크스페이스로 되살리기… (메모·할 일은 보관해 둔 글로, 세션은 쉬는 세션으로)');
  revive.disabled = !items.length;
  revive.onclick = () => { close(); restore(info, record); };
  const erase = make('button', 'row add history-erase', '이 기록 지우기…');
  erase.onclick = async () => { if (await remove(info)) { close(); showHistory(); } };
  const folder = typeof record.folder === 'string' && record.folder ? record.folder : '';
  const close = openPopup(`삭제한 워크스페이스: ${info.name}`, [
    make('p', 'note', `${when(info.deleted)}에 삭제 · ${when(info.until, true)}까지 보관${folder ? ` · 폴더: ${folder}` : ''}${info.summary ? ` · ${info.summary}` : ''}`),
    ...(rows.length ? rows : [make('p', 'note', '삭제할 때 항목이 없던 워크스페이스입니다.')]),
    revive, back, erase,
  ]);
}

/** 삭제한 워크스페이스 기록 창: 누르면 내용을 보고, 되살리거나 지운다. */
export async function showHistory() {
  const { invoke, make, openPopup, setStatus } = api;
  let list = [];
  try {
    list = await invoke('history_list');
  } catch (err) {
    return setStatus(`삭제한 워크스페이스의 기록을 읽지 못했습니다: ${err}`);
  }
  const rows = list.map((info) => {
    const row = make('div', 'row');
    const open = make('button', 'row-title', `◆ ${info.name}`);
    open.append(make('span', 'template-summary', info.items ? `항목 ${info.items}개${info.summary ? ` · ${info.summary}` : ''}` : '항목 없음'));
    open.title = `${info.name}\n${info.folder ? `폴더: ${info.folder}\n` : ''}삭제: ${when(info.deleted)}\n보관: ${when(info.until, true)}까지\n클릭: 내용 보기 · 되살리기`;
    open.onclick = () => { close(); showRecord(info); };
    const erase = make('button', 'close', '×');
    erase.title = '이 기록 지우기';
    erase.onclick = async () => { if (await remove(info)) { close(); showHistory(); } };
    row.append(open, make('span', 'row-note', `${when(info.deleted)} 삭제`), erase);
    return row;
  });
  const close = openPopup('삭제한 워크스페이스 기록', [
    ...rows,
    make('p', 'note', list.length
      ? '삭제한 워크스페이스는 3년 동안 여기에 남습니다. 누르면 그때 있던 항목과 보관해 둔 글(메모·할 일·마인드맵·ERD·명령)을 보고, 워크스페이스로 되살릴 수 있습니다. 세션은 프로필·폴더·이어 갈 대화 번호만, 사이트·문서·표·이미지 같은 산출물은 이름과 주소·경로만 남깁니다.'
      : '삭제한 워크스페이스가 없습니다. 워크스페이스를 삭제하면 그때 있던 항목과 메모·할 일의 글이 3년 동안 여기에 남습니다.'),
  ]);
}
