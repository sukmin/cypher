// 그룹 틀: 그룹 하나(세션 폴더, 메모, 사이트 주소, HTTP 폴더 …)를 파일로 저장해 두고,
// 같은 종류의 업무를 시작할 때 한 번에 펼친다.
//
// 틀에 담기는 것: 터미널 세션(프로필과 폴더 — 대화는 담지 않고 새로 시작한다), 메모(글 또는 연결된 파일),
// 마인드맵(개요 글), 사이트(주소), HTTP(요청 폴더와 열어 둔 파일), 문서(파일 경로), 별칭·색.
// 이미지는 담지 않는다. 항목을 어떻게 담고 펼칠지는 main.js가 알고, 여기서는 목록 창과 저장소를 맡는다.

let api = null;
// { invoke, make, openPopup, ask, setStatus, notice, folder() → 새 세션 폴더, baseName,
//   current() → 지금 보고 있는 묶음 { name, list } | null,
//   pack(group) → { items, skipped, summary },
//   open(template, { group, cwd }) → Promise<{ made, skipped }> }

export function initTemplate(given) {
  api = given;
}

/** 저장한 때를 늘 같은 자릿수로 적는다(2026-11-13 18:00). */
function when(ms) {
  if (!ms) return '';
  const d = new Date(ms);
  const two = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}

/** 그 묶음을 틀로 저장한다. 이름을 묻고, 같은 이름의 틀이 있으면 바꿔 써도 되는지 묻는다. 저장했으면 참. */
export async function saveTemplate(group) {
  const { invoke, ask, setStatus, notice } = api;
  const packed = api.pack(group);
  if (!packed.items.length) {
    setStatus(`틀로 저장할 항목이 없습니다${packed.skipped.length ? ` (${packed.skipped.join(' · ')})` : ''}.`);
    return false;
  }
  const name = await ask('틀 이름 (이 묶음의 항목 구성을 이 이름으로 저장합니다)', '예: 결제 개편', group.name);
  if (!name) return false;
  try {
    const existing = await invoke('template_list');
    const same = existing.find((t) => t.name.toLowerCase() === name.toLowerCase());
    if (same && !confirm(`"${same.name}" 틀이 이미 있습니다(항목 ${same.items}개, ${when(same.saved)} 저장). 바꿔 쓸까요?`)) return false;
    const path = await invoke('template_save', {
      name,
      body: { version: 1, name, savedAt: Date.now(), summary: packed.summary, items: packed.items },
    });
    // 이름만 다르게 적혀 다른 파일로 저장된 옛 틀은 지운다. 저장에 성공한 뒤에 지운다(먼저 지우면 저장이 실패했을 때 옛 틀만 사라진다).
    if (same && !path.toLowerCase().endsWith(`\\${same.file.toLowerCase()}`)) await invoke('template_remove', { file: same.file }).catch(() => {});
    setStatus(`틀로 저장했습니다: ${name} — 항목 ${packed.items.length}개${packed.skipped.length ? ` (담지 않은 것: ${packed.skipped.join(' · ')})` : ''} · ${path}`);
    return true;
  } catch (err) {
    notice('틀을 저장하지 못했습니다', String(err));
    return false;
  }
}

/** 틀을 펼친다. `here`이면 모든 항목을 지금의 새 세션 폴더에서 연다(같은 구성을 다른 폴더의 업무에 쓸 때). */
async function openTemplate(info, here) {
  const { invoke, ask, setStatus, notice } = api;
  let template;
  try {
    template = await invoke('template_read', { file: info.file });
  } catch (err) {
    return notice('틀을 열지 못했습니다', `${info.name}: ${err}`);
  }
  const group = await ask(
    `"${info.name}" 틀을 펼칠 그룹 이름${here ? ` (모든 항목을 ${api.folder()}에서 엽니다)` : ''}`,
    '있는 그룹 이름을 적으면 그 그룹에 더합니다', info.name,
  );
  if (!group) return;
  setStatus(`틀을 펼치는 중: ${info.name}…`);
  try {
    const { made, skipped } = await api.open(template, { group, cwd: here ? api.folder() : null });
    setStatus(`틀을 펼쳤습니다: ${info.name} → 그룹 "${group}"에 항목 ${made}개${skipped.length ? ` — ${skipped.join(' · ')}` : ''}`);
  } catch (err) {
    notice('틀을 펼치지 못했습니다', `${info.name}: ${err}`);
  }
}

/** 그룹 틀 창: 저장해 둔 틀을 펼치거나 지우고, 지금 보고 있는 묶음을 틀로 저장한다. */
export async function showTemplates() {
  const { invoke, make, openPopup, setStatus } = api;
  let list = [];
  try {
    list = await invoke('template_list');
  } catch (err) {
    return setStatus(`틀 목록을 읽지 못했습니다: ${err}`);
  }
  const rows = list.map((info) => {
    const row = make('div', 'row');
    const use = make('button', 'row-title', `◆ ${info.name}`);
    use.append(make('span', 'template-summary', `항목 ${info.items}개${info.summary ? ` · ${info.summary}` : ''}`));
    use.title = `${info.name}\n${info.summary || `항목 ${info.items}개`}\n저장: ${when(info.saved)}\n클릭: 저장해 둔 폴더대로 펼치기`;
    use.onclick = () => { close(); openTemplate(info, false); };
    const here = make('button', 'template-here', '이 폴더에서');
    here.title = `모든 항목을 새 세션 폴더에서 열기: ${api.folder()}`;
    here.onclick = () => { close(); openTemplate(info, true); };
    const remove = make('button', 'close', '×');
    remove.title = '이 틀 지우기';
    remove.onclick = async () => {
      if (!confirm(`"${info.name}" 틀을 지울까요? (열려 있는 항목에는 영향이 없습니다)`)) return;
      try { await invoke('template_remove', { file: info.file }); } catch (err) { setStatus(String(err)); }
      close();
      showTemplates();
    };
    row.append(use, make('span', 'row-note', when(info.saved)), here, remove);
    return row;
  });
  const current = api.current();
  const save = make('button', 'row add', current ? `지금 보고 있는 묶음을 틀로 저장… (${current.name} · 항목 ${current.list.length}개)` : '틀로 저장할 묶음이 없습니다');
  save.disabled = !current;
  save.onclick = async () => { close(); if (await saveTemplate(current)) showTemplates(); };
  const folder = make('button', 'row add', '틀 폴더 열기 (파일을 복사해 다른 PC로 옮길 수 있습니다)');
  folder.onclick = () => invoke('template_folder').catch((err) => setStatus(String(err)));
  const close = openPopup('그룹 틀', [
    ...rows,
    make('p', 'note', list.length
      ? '틀을 누르면 그 구성대로 그룹이 만들어집니다. 세션은 대화를 잇지 않고 새로 시작하고, 메모·마인드맵은 저장할 때의 글로, 사이트·HTTP·문서는 그 주소·파일로 엽니다.'
      : '저장해 둔 틀이 없습니다. 업무 하나를 그룹으로 꾸민 뒤(세션, 메모, 사이트, HTTP …) 틀로 저장해 두면, 같은 종류의 업무를 시작할 때 한 번에 펼칩니다. 묶음 머리를 오른쪽 클릭해도 저장할 수 있습니다.'),
    save, folder,
  ]);
}
