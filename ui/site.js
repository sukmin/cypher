// 사이트 항목: 저장해 둔 웹 페이지를 그 항목의 창 안에서 본다(내장 브라우저처럼 따로 뜨는 창이 아니다).
//
// 페이지는 화면 요소가 아니라 본창에 덧붙인 웹뷰다(많은 사이트가 iframe으로 끼워 넣는 것을 막기 때문).
// 여기서는 그 웹뷰가 놓일 자리(빈 칸)만 만들고, 자리의 위치·크기를 앱에 알려 웹뷰를 그 위에 겹쳐 놓게 한다.
// 웹뷰는 화면 요소보다 항상 위에 그려지므로, 메뉴·대화상자가 열리거나 무언가를 끄는 동안에는 잠시 숨긴다.

let api = null; // { invoke, listen, make, ask, setStatus }
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

/**
 * 사이트 항목의 화면을 host 안에 만든다.
 * saved: { url(저장해 둔 주소) } · onChange: 제목·주소가 바뀌었을 때 사이드바와 상태 저장을 갱신하게 알린다.
 */
export function createSite(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus } = api;
  const id = item.key;
  let url = saved.url || 'about:blank';   // 지금(또는 마지막으로) 보던 주소
  const home = saved.home || url;          // 처음 저장한 주소
  let title = saved.title || '';
  let opened = false;                      // 웹뷰를 만들었는지(처음 화면에 보일 때 만든다)
  let opening = false;
  let last = '';                           // 마지막으로 알린 자리
  let disposed = false;

  const button = (text, tip, action) => {
    const b = make('button', '', text);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const act = (action) => invoke('site_action', { id, action }).catch((err) => setStatus(String(err)));
  const $address = make('input', 'site-address');
  Object.assign($address, { spellcheck: false, autocomplete: 'off', value: url });
  $address.title = '주소를 고치고 Enter를 누르면 그 주소로 갑니다';
  const bar = make('div', 'doc-bar');
  bar.append(
    button('←', '뒤로', () => act('back')),
    button('→', '앞으로', () => act('forward')),
    button('⟳', '새로 고침', () => act('reload')),
    button('⌂', `처음 저장한 주소로: ${home}`, () => go(home)),
    $address,
    button('브라우저로', '이 주소를 내장 브라우저 창에서 열기 (Claude가 조작할 수 있는 창)', () => invoke('browser_open', { url }).catch((err) => setStatus(String(err)))),
  );
  const $view = make('div', 'site-view');
  const $note = make('p', 'note', '');
  $view.append($note);
  const root = make('div', 'doc site');
  root.append(bar, $view);
  host.append(root);

  async function go(target) {
    if (!opened) { url = target; $address.value = target; return sync(); }
    try {
      url = await invoke('site_navigate', { id, url: target });
      $address.value = url;
      onChange();
    } catch (err) {
      setStatus(String(err));
    }
  }
  $address.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') { ev.preventDefault(); go($address.value); }
    else if (ev.key === 'Escape') { $address.value = url; $address.blur(); }
  });

  /** 웹뷰를 이 창의 자리에 맞춘다. 창이 화면에 없거나 가려야 하는 상태면 숨긴다. */
  async function sync() {
    if (disposed) return;
    const rect = $view.getBoundingClientRect();
    const visible = rect.width > 8 && rect.height > 8 && !covered();
    $note.textContent = visible || !opened ? '' : '메뉴나 창이 열려 있는 동안에는 페이지를 잠시 가립니다.';
    const bounds = { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
    const now = visible ? `${bounds.x},${bounds.y},${bounds.width},${bounds.height}` : 'hidden';
    if (!opened) {
      if (!visible || opening) return;
      opening = true;
      try {
        url = await invoke('site_open', { id, url, ...bounds });
        opened = true;
        $address.value = url;
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
      if (payload.url && payload.url !== 'about:blank') { url = payload.url; if (document.activeElement !== $address) $address.value = url; }
      if (payload.title != null) title = payload.title;
      onChange();
    },
  });
  syncSites();

  return {
    ready: Promise.resolve(),
    focus: () => {}, // 초점은 본창에 둔다(페이지 안으로 옮기면 앱 단축키가 듣지 않는다).
    /** 사이드바에 보일 기본 이름: 페이지 제목, 없으면 사이트 이름. */
    title: () => title || hostOf(url) || '사이트',
    /** 다시 시작할 때 되살릴 내용: 보던 주소와 처음 저장한 주소. */
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
