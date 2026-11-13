// 사이트 항목(사이드바의 브라우저): 웹 페이지를 그 항목의 창 안에서 본다. 같은 그룹의 Claude 세션이 조작할 수 있다.
//
// 페이지는 화면 요소가 아니라 본창에 덧붙인 웹뷰다(많은 사이트가 iframe으로 끼워 넣는 것을 막기 때문).
// 여기서는 그 웹뷰가 놓일 자리(빈 칸)만 만들고, 자리의 위치·크기를 앱에 알려 웹뷰를 그 위에 겹쳐 놓게 한다.
// 웹뷰는 화면 요소보다 항상 위에 그려지므로, 메뉴·대화상자가 열리거나 무언가를 끄는 동안에는 잠시 숨긴다.

// { invoke, listen, make, setStatus, showMenu, captured(찍은 그림, 이 항목), zoom(), claudes(항목), controller(항목), handoff(항목, 세션),
//   profile(항목) → 그 항목의 그룹이 쓰는 브라우저 프로필의 번호(그룹이 없으면 빈 글) }
//
// 로그인 상태는 같은 그룹의 사이트 항목끼리만 함께 쓴다(그룹마다 브라우저 프로필이 따로다).
// 항목이 다른 그룹으로 옮겨지면 웹뷰를 닫고 새 그룹의 프로필로 다시 연다(보던 주소는 그대로, 로그인 상태는 새 그룹의 것).
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
  let openedProfile = '';                         // 그 웹뷰를 어느 프로필로 만들었는지
  let toldProfile = null;                         // 앱에 알려 둔 "이 항목이 써야 하는 프로필"
  let told = Promise.resolve();                   // 앱에 프로필을 알리는 일의 줄(보낸 순서대로 처리되게 한다)

  /**
   * 이 항목이 써야 하는 프로필을 앱에 알린다. 앱은 다른 프로필로 뜬 웹뷰를 닫고, 그 뒤로는 이 프로필로만 만든다
   * (Claude의 도구 호출이 옛 그룹의 프로필을 들고 뒤늦게 와도 받지 않는다).
   * 알리는 일은 앞의 것이 끝난 뒤에 보낸다: 그룹을 연달아 옮겼을 때 늦게 보낸 것이 먼저 처리되면 앱이 옛 프로필로 알게 된다.
   * 돌려주는 약속은 앱이 여기까지 알게 된 때 끝난다.
   */
  function tell(profile) {
    if (profile !== toldProfile) {
      toldProfile = profile;
      retold = true;
      told = told.then(() => invoke('site_profile', { id, profile })).catch((err) => setStatus(String(err)));
    }
    return told;
  }
  let opening = false;
  let retold = false;                             // 여는 동안 앱에 다른 프로필을 알렸는지(그랬다면 앱이 그 웹뷰를 닫았을 수 있다)
  let wanted = null;                              // 웹뷰가 뜨기 전에 친 주소(뜨면 그리로 간다)
  let failed = null;                              // 열지 못한 시도 { attempt(프로필 + 주소), note(오류 글) }. 같은 조건으로는 다시 열지 않는다.
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
  $address.title = '주소나 검색어 입력 후 Enter';
  const $home = button('⌂', '처음 간 주소로', () => (home ? go(home) : $address.focus()));
  const $star = button('☆', '즐겨찾기 넣기·빼기·가기', (ev) => bookmarkMenu(ev));
  const $claude = button('Claude에게', '', (ev) => handoff(ev, false));
  $claude.oncontextmenu = (ev) => { ev.preventDefault(); ev.stopPropagation(); handoff(ev, true); };
  const $external = button('외부 브라우저', 'Windows 기본 브라우저로 열기', () => {
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
    button('캡처', '화면을 찍어 이미지 항목으로 넣기 · Shift+클릭: 페이지 전체', (ev) => capture(ev.shiftKey)),
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
    $claude.title = `${by ? `"${by}" 세션이 조작 중 · ` : ''}같은 그룹의 Claude 세션에 넘기기`
      + ' · 오른쪽 클릭: 세션 고르기·새 세션';
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
    if (!opened) {
      // 웹뷰가 아직 없다(만드는 중이거나 열지 못했다). 친 주소를 적어 두고 연다: 만드는 중이었으면 다 뜬 뒤에 그리로 간다.
      // 열지 못한 뒤라면 주소가 그대로여도 다시 해 본다.
      failed = null;
      if (target.trim()) { wanted = target; setUrl(target); $address.value = target; }
      return sync();
    }
    if (!target.trim()) return;
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
    if (opened) $note.textContent = visible ? '' : '메뉴나 창이 열려 있는 동안에는 페이지를 잠시 가립니다.';
    else $note.textContent = failed ? failed.note : '';
    // 웹뷰의 자리는 창 기준 크기로 알려야 한다. 화면 글자 크기 설정(본창 배율)만큼 곱한다.
    const zoom = api.zoom();
    const bounds = { x: rect.left * zoom, y: rect.top * zoom, width: rect.width * zoom, height: rect.height * zoom };
    const now = visible ? `${bounds.x},${bounds.y},${bounds.width},${bounds.height}` : 'hidden';
    const profile = api.profile(item);
    tell(profile);
    // 그룹이 바뀌어 프로필이 달라졌으면 다시 연다(보이는 중이면 아래에서 바로, 아니면 다시 보일 때).
    if (opened && profile !== openedProfile) {
      opened = false;
      last = '';
    }
    if (!opened) {
      if (!visible || opening) return;
      // 같은 프로필·주소로 열다가 실패했으면 다시 열지 않고 오류 글을 그대로 둔다(해석할 수 없는 주소처럼 되지 않을 일을
      // 화면을 그릴 때마다 되풀이하게 된다). 주소를 다시 치거나 그룹(프로필)이 바뀌면 다시 해 본다.
      const sent = (wanted ?? url) || 'about:blank';
      const attempt = `${profile}\n${sent}`;
      if (failed) {
        if (failed.attempt === attempt) return;
        failed = null;
        $note.textContent = '';
      }
      opening = true;
      retold = false;
      let typed = null;
      try {
        await told; // 앞서 알린 프로필이 앱에 닿은 뒤에 연다.
        // 기다리는 사이에 그룹이 또 바뀌었으면 이번에는 열지 않는다(아래의 syncSites가 새 프로필로 다시 온다).
        if (!disposed && !retold) {
          const at = await invoke('site_open', { id, url: sent, ...bounds, profile });
          // 여는 사이에 다른 프로필을 알렸다면(그룹을 연달아 옮김) 앱이 이 웹뷰를 닫았을 수 있다.
          // 열린 것으로 치지 않고 지금의 프로필로 다시 연다(같은 프로필의 웹뷰가 남아 있으면 앱은 그것을 그대로 쓴다).
          if (!retold) {
            setUrl(at);
            // 여는 사이에 주소를 쳤으면 그 주소는 아직 보내지 않았다(돌아온 것은 앞서 보낸 주소다). 아래에서 친 주소로 간다.
            if (wanted != null && wanted !== sent) typed = wanted;
            wanted = null;
            opened = true;
            openedProfile = profile;
            last = now;
          }
        }
      } catch (err) {
        // 여는 사이에 다른 프로필을 알렸다면 그 때문에 거절된 것일 수 있다(그룹이 옮겨졌다). 기억하지 않고 지금의 프로필로 다시 연다.
        if (!retold) {
          failed = { attempt, note: `페이지를 열지 못했습니다: ${err} (주소 칸에서 Enter를 누르면 다시 시도합니다)` };
          $note.textContent = failed.note;
        }
      } finally {
        opening = false;
      }
      if (disposed) invoke('site_close', { id });
      else {
        if (typed != null) go(typed);
        syncSites(); // 여는 사이에 자리나 그룹이 바뀌었을 수 있다.
      }
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
      if (payload.url) {
        setUrl(payload.url);
        // 열지 못했다고 적어 둔 뒤에 웹뷰가 페이지를 읽었다(Claude의 도구가 만들었다). 이 창에도 다시 연다.
        if (failed) { failed = null; wanted = null; syncSites(); }
      }
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
    /** 이 항목이 써야 하는 프로필을 앱에 알리고, 앱이 알게 될 때까지 기다린다(Claude의 도구가 이 항목을 쓰기 직전에 부른다). */
    assign: (profile) => tell(profile),
    /** 다시 시작할 때 되살릴 내용: 보던 주소와 처음 간 주소. */
    state: () => ({ url, home, title }),
    close: async () => true,
    dispose: () => {
      disposed = true;
      observer.disconnect();
      views.delete(id);
      // 알리던 것이 남아 있으면 그 뒤에 닫는다(닫은 뒤에 닿으면 없는 항목의 프로필이 앱에 남는다).
      told.then(() => invoke('site_close', { id }));
    },
  };
}
