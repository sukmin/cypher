// 사이트 항목(사이드바의 브라우저): 웹 페이지를 그 항목의 창 안에서 본다. 같은 그룹의 Claude 세션이 조작할 수 있다.
//
// 페이지는 화면 요소가 아니라 본창에 덧붙인 웹뷰다(많은 사이트가 iframe으로 끼워 넣는 것을 막기 때문).
// 여기서는 그 웹뷰가 놓일 자리(빈 칸)만 만들고, 자리의 위치·크기를 앱에 알려 웹뷰를 그 위에 겹쳐 놓게 한다.
// 웹뷰는 화면 요소보다 항상 위에 그려지므로, 메뉴·대화상자가 열리거나 무언가를 끄는 동안에는 잠시 숨긴다.

// { invoke, listen, make, setStatus, showMenu, captured(찍은 그림, 이 항목), zoom(), claudes(항목), controller(항목), handoff(항목, 세션) }
let api = null;
const BOOKMARKS_KEY = 'cypher.bookmarks';
const views = new Map(); // 항목 번호 → { sync, changed }
let queued = false;

export function initSite(given) {
  api = given;
  api.listen('site-changed', ({ payload }) => views.get(payload.id)?.changed(payload));
  // 웹뷰를 가려야 하는 상태(메뉴, 대화상자, 끌기, 크기 조절)가 바뀌면 자리를 다시 맞춘다.
  const observer = new MutationObserver(syncSites);
  for (const id of ['menu', 'ask', 'popup', 'sessions', 'drop-hint', 'terms']) {
    const node = document.getElementById(id);
    if (node) observer.observe(node, { attributes: true, attributeFilter: ['hidden', 'open', 'class'] });
  }
  observer.observe(document.body, { attributes: true, attributeFilter: ['class'] });
  window.addEventListener('resize', syncSites);
}

/** 모든 사이트 항목의 웹뷰 자리를 다음 화면 갱신 때 다시 맞춘다. */
export function syncSites() {
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    let shown = false;
    for (const view of views.values()) {
      view.sync();
      shown = shown || view.shown();
    }
    // 사이트가 보이는 동안에는 알림을 사이드바 쪽으로 옮긴다(웹뷰에 가려지지 않게).
    document.body.classList.toggle('site-shown', shown);
  });
}

/** 화면 요소가 웹뷰에 가려지면 안 되는 상태인지(메뉴·대화상자가 열려 있거나, 끌기·크기 조절 중). */
function covered() {
  const $ = (id) => document.getElementById(id);
  return !$('menu').hidden || $('ask').open || $('popup').open || !$('drop-hint').hidden
    || $('sessions').classList.contains('dragging')
    || document.body.classList.contains('resizing-row') || document.body.classList.contains('resizing-col');
}

const hostOf = (url) => { try { return new URL(url).host; } catch { return url; } };
const blank = (address) => !address || address === 'about:blank';

/**
 * 사이트 항목의 화면을 host 안에 만든다.
 * saved: { url(보던 주소. 없으면 빈 페이지로 시작한다), home, title }
 * onChange: 제목·주소가 바뀌었을 때 사이드바와 상태 저장을 갱신하게 알린다.
 */
export function createSite(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  const id = item.key;
  let url = blank(saved.url) ? '' : saved.url;   // 지금(또는 마지막으로) 보던 주소. 빈 글이면 빈 페이지.
  let home = saved.home || url;                   // 처음 간 주소(⌂ 단추)
  let title = saved.title || '';
  let opened = false;                             // 웹뷰를 만들었는지(처음 화면에 보일 때 만든다)
  let opening = false;
  let last = '';                                  // 마지막으로 알린 자리
  let disposed = false;

  const button = (text, tip, action) => {
    const b = make('button', '', text);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const act = (action) => invoke('site_action', { id, action }).catch((err) => setStatus(String(err)));
  const $address = make('input', 'site-address');
  Object.assign($address, { spellcheck: false, autocomplete: 'off', value: url, placeholder: '주소나 검색어를 입력하고 Enter' });
  $address.title = '주소를 입력하고 Enter를 누르면 그 주소로 갑니다 (주소가 아니면 검색합니다)';
  const $home = button('⌂', '처음 간 주소로', () => (home ? go(home) : $address.focus()));
  const $star = button('☆', '즐겨찾기: 지금 페이지를 넣거나 빼고, 넣어 둔 페이지로 갑니다', (ev) => bookmarkMenu(ev));
  const $claude = button('Claude에게', '', (ev) => handoff(ev, false));
  $claude.oncontextmenu = (ev) => { ev.preventDefault(); ev.stopPropagation(); handoff(ev, true); };
  const $external = button('외부 브라우저', '지금 주소를 Windows 기본 브라우저에서 엽니다 (Claude가 조작하지 않는 바깥 브라우저)', () => {
    if (!url) return setStatus('아직 연 페이지가 없습니다.');
    invoke('link_open', { target: url, cwd: item.startCwd, openFile: false, line: null, col: null }).catch((err) => setStatus(String(err)));
  });
  const bar = make('div', 'doc-bar');
  bar.append(
    button('←', '뒤로', () => act('back')),
    button('→', '앞으로', () => act('forward')),
    button('⟳', '새로 고침', () => act('reload')),
    $home,
    $address,
    button('캡처', '보이는 화면을 찍어 이 항목과 같은 묶음에 이미지 항목으로 넣기 (Shift+클릭: 페이지 전체)', (ev) => capture(ev.shiftKey)),
    $star, $claude, $external,
  );
  const $view = make('div', 'site-view');
  const $note = make('p', 'note', '');
  $view.append($note);
  const root = make('div', 'doc site');
  root.append(bar, $view);
  host.append(root);

  /** 단추의 글자와 안내를 지금 상태에 맞춘다(즐겨찾기 여부, 조작 중인 Claude 세션). */
  function refresh() {
    const starred = !!url && bookmarks().some((b) => b.url === url);
    $star.textContent = starred ? '★' : '☆';
    const by = api.controller(item);
    $claude.textContent = by ? `Claude: ${by}` : 'Claude에게';
    $claude.classList.toggle('on', !!by);
    $claude.title = `${by ? `지금 "${by}" 세션이 이 브라우저를 조작합니다. ` : ''}이 브라우저를 같은 그룹의 Claude 세션에 넘깁니다(여러 개면 고릅니다). `
      + '넘겨받은 세션이 이 페이지를 읽고 조작합니다 · 오른쪽 클릭: 세션 고르기, 새 Claude 세션에 넘기기';
    $home.title = home ? `처음 간 주소로: ${home}` : '처음 간 주소로';
  }

  /** 이 브라우저를 Claude 세션에 넘긴다. 같은 그룹에 세션이 하나면 바로, 여러 개면(또는 `choose`) 골라서 넘긴다. */
  function handoff(ev, choose) {
    const pool = api.claudes(item);
    if (!pool.length && !choose) return api.handoff(item, null);
    if (pool.length === 1 && !choose) return api.handoff(item, pool[0].s);
    showMenu(ev, [
      ...pool.map((entry) => ({ label: `${entry.label}에게 넘기기`, action: () => api.handoff(item, entry.s) })),
      { label: '새 Claude 세션을 열어 넘기기', action: () => api.handoff(item, null) },
    ]);
  }

  // 즐겨찾기는 모든 사이트 항목이 함께 쓴다.
  function bookmarks() {
    try { return JSON.parse(localStorage.getItem(BOOKMARKS_KEY) || '[]'); } catch { return []; }
  }
  function bookmarkMenu(ev) {
    const list = bookmarks();
    const save = (next) => { localStorage.setItem(BOOKMARKS_KEY, JSON.stringify(next)); refresh(); };
    const here = list.find((b) => b.url === url);
    const items = [];
    if (url) {
      items.push(here
        ? { label: '☆ 지금 페이지를 즐겨찾기에서 빼기', action: () => save(list.filter((b) => b !== here)) }
        : { label: '★ 지금 페이지를 즐겨찾기에 넣기', action: () => save([...list, { title: title || hostOf(url), url }]) });
    }
    items.push(...list.map((b) => ({ label: b.title || b.url, action: () => go(b.url) })));
    if (!items.length) return setStatus('즐겨찾기가 없습니다. 페이지를 연 뒤 ☆를 눌러 넣으세요.');
    showMenu(ev, items);
  }

  /** 페이지를 찍어 이 항목과 같은 묶음에 이미지 항목으로 넣는다. */
  let capturing = false;
  async function capture(full) {
    if (!opened) return setStatus('페이지가 아직 열리지 않았습니다.');
    if (capturing) return;
    capturing = true;
    setStatus(full ? '페이지 전체를 찍는 중…' : '화면을 찍는 중…');
    try {
      const captured = await invoke('site_capture', { id, full });
      setStatus('');
      await api.captured(captured, item);
    } catch (err) {
      setStatus(`캡처하지 못했습니다: ${err}`);
    } finally {
      capturing = false;
    }
  }

  function setUrl(next) {
    url = blank(next) ? '' : next;
    if (url && !home) home = url;
    if (document.activeElement !== $address) $address.value = url;
    refresh();
  }

  async function go(target) {
    if (!target.trim()) return;
    if (!opened) { setUrl(target); $address.value = target; return sync(); }
    try {
      setUrl(await invoke('site_navigate', { id, url: target }));
      $address.value = url;
      $address.blur();
      onChange();
    } catch (err) {
      setStatus(String(err));
    }
  }
  $address.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') { ev.preventDefault(); go($address.value); }
    else if (ev.key === 'Escape') { $address.value = url; $address.blur(); }
  });
  $address.addEventListener('focus', () => $address.select());

  /** 웹뷰를 이 창의 자리에 맞춘다. 창이 화면에 없거나 가려야 하는 상태면 숨긴다. */
  async function sync() {
    if (disposed) return;
    refresh();
    const rect = $view.getBoundingClientRect();
    const visible = rect.width > 8 && rect.height > 8 && !covered();
    $note.textContent = visible || !opened ? '' : '메뉴나 창이 열려 있는 동안에는 페이지를 잠시 가립니다.';
    // 웹뷰의 자리는 창 기준 크기로 알려야 한다. 화면 글자 크기 설정(본창 배율)만큼 곱한다.
    const zoom = api.zoom();
    const bounds = { x: rect.left * zoom, y: rect.top * zoom, width: rect.width * zoom, height: rect.height * zoom };
    const now = visible ? `${bounds.x},${bounds.y},${bounds.width},${bounds.height}` : 'hidden';
    if (!opened) {
      if (!visible || opening) return;
      opening = true;
      try {
        setUrl(await invoke('site_open', { id, url: url || 'about:blank', ...bounds }));
        opened = true;
        last = now;
      } catch (err) {
        $note.textContent = `페이지를 열지 못했습니다: ${err}`;
      } finally {
        opening = false;
      }
      if (disposed) invoke('site_close', { id });
      else syncSites(); // 여는 사이에 자리가 바뀌었을 수 있다.
      return;
    }
    if (now === last) return;
    last = now;
    invoke('site_bounds', { id, ...bounds, visible });
  }

  const observer = new ResizeObserver(syncSites);
  observer.observe($view);
  views.set(id, {
    sync,
    /** 이 항목의 창이 지금 화면에 놓여 있는지. */
    shown: () => { const rect = $view.getBoundingClientRect(); return rect.width > 8 && rect.height > 8; },
    changed: (payload) => {
      if (payload.url) setUrl(payload.url);
      if (payload.title != null) title = blank(payload.title) ? '' : payload.title;
      onChange();
    },
  });
  syncSites();

  return {
    ready: Promise.resolve(),
    // 초점은 본창에 둔다(페이지 안으로 옮기면 앱 단축키가 듣지 않는다). 빈 페이지면 주소를 바로 칠 수 있게 주소 칸에 둔다.
    focus: () => { if (!url) $address.focus(); },
    /** 사이드바에 보일 기본 이름: 페이지 제목, 없으면 사이트 이름. */
    title: () => title || hostOf(url) || '새 사이트',
    /** 지금 주소(빈 페이지면 빈 글). */
    url: () => url,
    /** 다시 시작할 때 되살릴 내용: 보던 주소와 처음 간 주소. */
    state: () => ({ url, home, title }),
    close: async () => true,
    dispose: () => {
      disposed = true;
      observer.disconnect();
      views.delete(id);
      invoke('site_close', { id });
    },
  };
}
