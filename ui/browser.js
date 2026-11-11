// 브라우저 창의 주소 줄. 아래쪽 웹 페이지와는 별개의 화면이며, 이 화면만 앱 명령을 부를 수 있다.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const $address = $('address');

const NAV_HEIGHT = 44;        // 주소 줄만 있을 때의 높이
const BOOKMARKS_HEIGHT = 30;  // 즐겨찾기 줄이 더해질 때 늘어나는 높이
const BOOKMARKS_KEY = 'cypher.bookmarks';

let current = '';             // 지금 페이지 주소
let bookmarks = [];           // [{ title, url }]
try { bookmarks = JSON.parse(localStorage.getItem(BOOKMARKS_KEY) || '[]'); } catch { /* 깨진 기록은 버린다 */ }

$('back').onclick = () => invoke('browser_back');
$('forward').onclick = () => invoke('browser_forward');
$('reload').onclick = () => invoke('browser_reload');
$('handoff').onclick = () => invoke('browser_handoff', { fresh: false });
$('handoff-new').onclick = () => invoke('browser_handoff', { fresh: true });

$address.addEventListener('keydown', async (ev) => {
  if (ev.key !== 'Enter') return;
  try {
    $address.value = await invoke('browser_navigate', { url: $address.value });
    $address.blur();
  } catch (err) {
    $address.value = String(err);
    $address.select();
  }
});
$address.addEventListener('focus', () => $address.select());

// ───────────────────────── 즐겨찾기 ─────────────────────────

function saveBookmarks() {
  localStorage.setItem(BOOKMARKS_KEY, JSON.stringify(bookmarks));
}

/** 즐겨찾기 줄을 그리고, 있고 없음에 따라 주소 줄 높이를 맞춘다. */
function renderBookmarks() {
  const $bar = $('bookmarks');
  $bar.hidden = !bookmarks.length;
  $bar.replaceChildren(...bookmarks.map((bookmark) => {
    const chip = document.createElement('div');
    chip.className = 'bookmark';
    chip.title = `${bookmark.title}\n${bookmark.url}`;
    chip.onclick = () => invoke('browser_navigate', { url: bookmark.url });
    const name = document.createElement('span');
    name.className = 'name';
    name.textContent = bookmark.title;
    const remove = document.createElement('span');
    remove.className = 'remove';
    remove.textContent = '×';
    remove.title = '즐겨찾기에서 삭제';
    remove.onclick = (ev) => {
      ev.stopPropagation();
      bookmarks = bookmarks.filter((b) => b !== bookmark);
      saveBookmarks();
      renderBookmarks();
    };
    chip.append(name, remove);
    return chip;
  }));
  invoke('browser_bar_height', { height: NAV_HEIGHT + (bookmarks.length ? BOOKMARKS_HEIGHT : 0) });
  renderStar();
}

/** 지금 페이지가 즐겨찾기에 있으면 별을 채운다. */
function renderStar() {
  const on = bookmarks.some((b) => b.url === current);
  $('star').textContent = on ? '★' : '☆';
  $('star').classList.toggle('on', on);
}

$('star').onclick = async () => {
  const page = await invoke('browser_page').catch(() => null);
  const url = page?.url || current;
  if (!url || url === 'about:blank') return;
  if (bookmarks.some((b) => b.url === url)) {
    bookmarks = bookmarks.filter((b) => b.url !== url);
  } else {
    let title = (page?.title || '').trim();
    if (!title) {
      try { title = new URL(url).hostname; } catch { title = url; }
    }
    bookmarks.push({ title, url });
  }
  current = url;
  saveBookmarks();
  renderBookmarks();
};

// 페이지가 스스로 주소를 바꾸는 경우까지 따라가도록 주기적으로 확인한다.
async function refresh() {
  const url = await invoke('browser_current').catch(() => null);
  if (!url) return;
  if (url !== current) { current = url; renderStar(); }
  if (document.activeElement !== $address && url !== $address.value) $address.value = url === 'about:blank' ? '' : url;
}
setInterval(refresh, 700);
refresh();
renderBookmarks();

document.addEventListener('contextmenu', (ev) => ev.preventDefault());
