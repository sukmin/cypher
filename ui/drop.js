// 끌어다 놓기: 창에 놓인 파일·글자·링크·그림을 알맞은 곳에 넣거나, 알맞은 사이드바 항목으로 만든다.
//
// - 터미널 창에 놓으면: 파일은 경로를, 글자·링크는 그 글자를 입력한다.
// - 메모 창에 놓으면: 글자 파일은 그 메모로 연다(글자는 편집기가 직접 받는다).
// - 그 밖의 곳(사이드바, 다른 종류의 창, 빈 화면)에 놓으면 항목으로 만든다:
//   이미지 → 이미지 항목 · 글자 파일·글자 → 메모 · .http → HTTP · 링크·바로 가기 → 사이트.
//   사이드바의 묶음(머리나 그 안의 줄)에 놓으면 그 묶음에, 창에 놓으면 그 창 항목의 묶음에 만든다.
//
// 파일의 경로는 화면에서는 보이지 않으므로 앱에 물어서 받는다(drops.rs).

let api = null;
const waiting = new Map(); // 경로를 물은 번호 → 답을 받을 함수
let asked = 0;
let internal = false;      // 이 화면 안에서 시작한 끌기(편집기에서 고른 글자 등)

const MAX_ITEMS = 12;      // 한 번에 만드는 항목 수
const MAX_TEXT = 8 << 20;  // 메모로 만들 글자 크기
const IMAGE_NAME = /\.(png|jpe?g|jfif|gif|webp|bmp|svg|avif|ico)$/i;
const TEXT_NAME = /\.(txt|md|markdown|json|jsonl|log|csv|tsv|xml|ya?ml|toml|ini|conf|env|sql|sh|ps1|bat|py|js|ts|tsx|jsx|rs|go|java|kt|c|h|cpp|css|html?)$/i;

/**
 * given: { invoke, listen, setStatus, quote, baseName, itemOfPane(pane), itemOfRow(li), contextOfHead(li),
 *          newItem(kind, opts), addImages(list, opts), pasteInto(item, text), screenEmpty(), reveal(items) }
 */
export function initDrop(given) {
  api = given;
  api.listen('drop-paths', ({ payload }) => {
    waiting.get(payload.id)?.(payload.paths || []);
    waiting.delete(payload.id);
  });
  document.addEventListener('dragstart', () => { internal = true; }, true);
  document.addEventListener('dragend', () => { internal = false; clearMarks(); }, true);
  document.addEventListener('dragover', onOver);
  document.addEventListener('dragleave', (ev) => { if (!ev.relatedTarget) clearMarks(); });
  document.addEventListener('drop', onDrop);
}

const has = (ev, type) => [...(ev.dataTransfer?.types || [])].includes(type);
const elementOf = (ev) => (ev.target instanceof Element ? ev.target : ev.target?.parentElement);

/** 놓는 자리가 무엇인지: 사이드바(어느 묶음), 창(어느 항목), 글자 칸, 그 밖. */
function placeOf(ev) {
  const el = elementOf(ev);
  if (!el) return { zone: 'none' };
  const files = has(ev, 'Files');
  const field = el.closest('textarea, input');
  // 글자·링크를 글자 칸에 놓는 것은 그 칸이 직접 받는다.
  if (field && !files) return { zone: 'field' };
  if (el.closest('dialog, #menu, #diff')) return { zone: files ? 'blocked' : 'none' };
  if (el.closest('#sidebar, #sidebar-grip')) {
    const row = el.closest('#sessions li');
    let head = null, near = null;
    if (row?.classList.contains('group')) head = row;
    else if (row) {
      near = api.itemOfRow(row);
      for (head = row.previousElementSibling; head && !head.classList.contains('group'); head = head.previousElementSibling);
    }
    return { zone: 'items', head, near };
  }
  if (field?.id === 'composer-text') return { zone: 'composer', field };
  const item = api.itemOfPane(el.closest('.pane'));
  if (item?.kind === 'term') return { zone: 'term', item };
  if (item?.kind === 'memo') return { zone: 'memo', item };
  if (item) return { zone: 'items', near: item };
  return { zone: 'items' };
}

function clearMarks() {
  document.getElementById('sidebar').classList.remove('dropping');
  for (const node of document.querySelectorAll('#sessions li.group.drop, .pane.dropping')) node.classList.remove('drop', 'dropping');
}

function onOver(ev) {
  const place = placeOf(ev);
  if (place.zone === 'field' || place.zone === 'none') return clearMarks();
  ev.preventDefault();
  // 이 화면 안에서 끈 글자는 사이드바에 놓을 때만 받는다(창 위에서는 아무 일도 하지 않는다).
  const accepted = place.zone !== 'blocked' && !(internal && place.zone !== 'items');
  ev.dataTransfer.dropEffect = accepted ? 'copy' : 'none';
  clearMarks();
  if (!accepted) return;
  const el = elementOf(ev);
  if (el.closest('#sidebar')) {
    document.getElementById('sidebar').classList.add('dropping');
    place.head?.classList.add('drop');
  } else {
    el.closest('.pane')?.classList.add('dropping');
  }
}

/** 놓인 것의 내용을 꺼낸다(놓는 순간에만 읽을 수 있다). */
function snapshot(dt) {
  return {
    files: [...dt.files],
    html: dt.getData('text/html') || '',
    uri: (dt.getData('text/uri-list') || '').split(/\r?\n/).find((line) => line && !line.startsWith('#')) || '',
    text: dt.getData('text/plain') || '',
  };
}

async function onDrop(ev) {
  const place = placeOf(ev);
  clearMarks();
  if (place.zone === 'field' || place.zone === 'none') return;
  ev.preventDefault();
  if (place.zone === 'blocked' || (internal && place.zone !== 'items')) return;
  const data = snapshot(ev.dataTransfer);
  try {
    if (place.zone === 'term' || place.zone === 'composer') await intoInput(place, data);
    else if (place.zone === 'memo') await intoMemo(place.item, data);
    else await intoItems(data, contextOf(place));
  } catch (err) {
    api.setStatus(`끌어다 놓기 실패: ${err}`);
  }
}

/** 놓인 파일들의 경로(파일 순서대로). 경로가 없는 것은 빈 글. */
function pathsOf(files) {
  const none = files.map(() => '');
  const post = window.chrome?.webview?.postMessageWithAdditionalObjects;
  if (!files.length || !post) return Promise.resolve(none);
  const id = ++asked;
  return new Promise((resolve) => {
    const timer = setTimeout(() => { waiting.delete(id); resolve(none); }, 3000);
    waiting.set(id, (paths) => { clearTimeout(timer); resolve(files.map((_, index) => paths[index] || '')); });
    try {
      // 글자 메시지로 보낸다(객체 메시지는 앱의 명령 처리기에서 걸려 우리 처리기까지 가지 않는다).
      window.chrome.webview.postMessageWithAdditionalObjects(`cypher-drop:${id}`, files);
    } catch {
      clearTimeout(timer);
      waiting.delete(id);
      resolve(none);
    }
  });
}

/** 터미널이나 프롬프트 편집기에 놓음: 파일은 경로를, 글자·링크는 그 글자를 넣는다. */
async function intoInput(place, data) {
  let text = data.text || data.uri;
  if (data.files.length) {
    const paths = (await pathsOf(data.files)).filter(Boolean);
    if (!paths.length) return api.setStatus('경로가 없는 파일이라 입력할 수 없습니다. 사이드바에 놓으면 항목으로 만들 수 있습니다.');
    text = paths.map(api.quote).join(' ');
  }
  if (!text) return;
  if (place.zone === 'term') return api.pasteInto(place.item, text);
  const field = place.field;
  field.setRangeText(text, field.selectionStart, field.selectionEnd, 'end');
  field.dispatchEvent(new Event('input', { bubbles: true }));
  field.focus();
}

/** 메모 창에 놓음: 글자 파일은 그 메모로 열고(빈 메모면 거기에, 아니면 새 메모로), 이미지는 이미지 항목으로 만든다. */
async function intoMemo(memo, data) {
  if (!data.files.length) return intoItems(data, { near: memo });
  const paths = await pathsOf(data.files);
  const known = paths.some(Boolean) ? await api.invoke('drop_classify', { paths: paths.filter(Boolean) }) : [];
  const images = new Set(known.filter((info) => info.kind === 'image').map((info) => info.path));
  const rest = { ...data, files: [] };
  let opened = 0;
  for (const [index, file] of data.files.entries()) {
    const path = paths[index];
    if (path && !images.has(path) && opened < 8) { await memo.view.openFile(path); opened += 1; }
    else rest.files.push(file);
  }
  if (rest.files.length) await intoItems(rest, { near: memo });
}

/** 놓은 자리에서 새 항목이 들어갈 묶음과 폴더. */
function contextOf(place) {
  if (place.head) return api.contextOfHead(place.head);
  if (place.near) return { near: place.near };
  return {};
}

/** 그림 한 장을 끌어 온 것이면 그 그림의 주소와 이름. (브라우저에서 그림을 끌면 `<img>` 하나가 든 HTML이 온다.) */
function draggedImage(html) {
  if (!html) return null;
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const images = [...doc.images];
  if (images.length !== 1 || doc.body.textContent.trim()) return null;
  const src = images[0].getAttribute('src') || '';
  if (!/^(https?:|data:image\/)/i.test(src)) return null;
  return { src, name: images[0].getAttribute('alt') || '' };
}

/** 링크를 끌어 온 것이면 그 링크의 글자(사이트 항목의 이름으로 쓴다). */
function linkText(html) {
  if (!html) return '';
  const doc = new DOMParser().parseFromString(html, 'text/html');
  return (doc.querySelector('a')?.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 80);
}

const isUrl = (text) => /^https?:\/\/\S+$/i.test(text.trim());
const dirName = (path) => path.replace(/[\\/][^\\/]*$/, '');
const stem = (name) => name.replace(/\.[^.]*$/, '');

/** 놓인 것을 사이드바 항목으로 만든다. */
async function intoItems(data, context) {
  const { invoke, newItem, addImages } = api;
  const made = [], skipped = [];
  // 화면이 비어 있으면 처음 만든 항목을 바로 보여 주고, 아니면 사이드바에만 놓는다(보던 화면을 바꾸지 않는다).
  const show = () => (api.screenEmpty() && !made.length ? 'replace' : 'hidden');
  // 창에 놓았으면 그 창 항목과 같은 묶음·폴더에 만든다. 빈 이미지 항목에 놓은 그림은 그 항목을 채운다(near).
  const place = (extra = {}) => ({
    ...(context.near ? { near: context.near, group: context.near.group, cwd: context.near.project || context.near.startCwd } : context),
    ...extra,
  });
  const picture = draggedImage(data.html);

  if (data.files.length) {
    const files = data.files.slice(0, MAX_ITEMS);
    if (data.files.length > MAX_ITEMS) skipped.push(`한 번에 ${MAX_ITEMS}개까지만 만듭니다`);
    const paths = await pathsOf(files);
    const known = paths.some(Boolean) ? await invoke('drop_classify', { paths: paths.filter(Boolean) }) : [];
    const byPath = new Map(known.map((info) => [info.path, info]));
    for (const [index, file] of files.entries()) {
      const info = byPath.get(paths[index]);
      try {
        if (info?.kind === 'image') {
          made.push(...await addImages([await invoke('image_import', { path: info.path })], { ...place(), show: show() }));
        } else if (info?.kind === 'text') {
          made.push(await newItem('memo', place({ saved: { open: info.path }, show: show() })));
        } else if (info?.kind === 'http') {
          // HTTP 항목은 폴더 단위로 요청 파일을 찾으므로, 그 파일이 있는 폴더의 항목으로 만든다.
          made.push(await newItem('http', place({ cwd: dirName(info.path), saved: { file: info.path }, show: show() })));
        } else if (info?.kind === 'link') {
          made.push(await newItem('site', place({ saved: { url: info.url, title: stem(info.name) }, show: show() })));
        } else if (info) {
          skipped.push(`${info.name}: ${info.kind === 'folder' ? '폴더는 항목으로 만들 수 없습니다' : '글자나 이미지가 아닌 파일입니다'}`);
        } else if (file.type.startsWith('image/') || IMAGE_NAME.test(file.name)) {
          // 경로가 없는 파일(다른 프로그램이 내용만 준 것): 내용을 그대로 받아 넣는다.
          const stored = await invoke('image_store', new Uint8Array(await file.arrayBuffer()), {
            headers: { 'x-name': encodeURIComponent(stem(file.name)), 'x-ext': (file.name.match(/\.([^.]+)$/)?.[1] || '').toLowerCase() },
          });
          made.push(...await addImages([stored], { ...place(), source: picture?.src.startsWith('http') ? picture.src : data.uri, show: show() }));
        } else if ((file.type.startsWith('text/') || /json|xml/.test(file.type) || TEXT_NAME.test(file.name)) && file.size <= MAX_TEXT) {
          made.push(await newItem('memo', place({ saved: { text: await file.text() }, show: show() })));
        } else {
          skipped.push(`${file.name}: 글자나 이미지가 아닌 파일입니다`);
        }
      } catch (err) {
        skipped.push(`${file.name}: ${err}`);
      }
    }
  } else if (picture) {
    // 브라우저에서 끌어 온 그림: 그 주소의 그림을 받아 이미지 항목으로 만든다.
    try {
      const stored = await invoke('image_fetch', { url: picture.src });
      if (!stored.name && picture.name) stored.name = picture.name;
      made.push(...await addImages([stored], { ...place(), source: picture.src.startsWith('http') ? picture.src : data.uri, show: show() }));
    } catch (err) {
      // 받지 못하면(로그인이 필요한 그림 등) 링크라도 사이트 항목으로 남긴다.
      const url = isUrl(data.uri) ? data.uri : picture.src.startsWith('http') ? picture.src : '';
      if (url) made.push(await newItem('site', place({ saved: { url }, show: show() })));
      skipped.push(`그림을 받지 못했습니다(${err})${url ? ' — 사이트 항목으로 넣었습니다' : ''}`);
    }
  } else if (isUrl(data.uri) || isUrl(data.text)) {
    const url = (isUrl(data.uri) ? data.uri : data.text).trim();
    made.push(await newItem('site', place({ saved: { url, title: linkText(data.html) }, show: show() })));
  } else if (data.text.trim()) {
    made.push(await newItem('memo', place({ saved: { text: data.text }, show: show() })));
  } else {
    skipped.push('항목으로 만들 수 있는 내용이 없습니다');
  }

  const items = made.filter(Boolean);
  api.reveal(items);
  const parts = [];
  if (items.length) parts.push(`항목 ${items.length}개를 만들었습니다`);
  if (skipped.length) parts.push(`넣지 못한 것: ${skipped.join(' · ')}`);
  api.setStatus(parts.join(' — '));
}
