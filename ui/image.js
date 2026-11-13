// 이미지 항목: 사이드바에 놓는 그림(화면 캡처, 파일, 붙여넣기, 끌어다 놓기).
//
// 그림 파일은 앱이 저장소에 복사해 두고(번호와 형식으로 가리킨다), 여기서는 그 내용을 받아 그린다.
// 보는 방식(창에 맞춤 / 실제 크기 / 커서 기준 확대·축소 / 끌어서 이동)은 이미지 뷰어 Aspartame의
// viewport.rs 계산을 그대로 옮긴 것이다.

let api = null; // { invoke, make, setStatus, showMenu, addImages, siblings, show, toClaude, baseName }

export function initImage(given) {
  api = given;
}

const INFO_KEY = 'cypher.image.info';
const MIME = {
  png: 'image/png', jpg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp',
  bmp: 'image/bmp', svg: 'image/svg+xml', avif: 'image/avif', ico: 'image/x-icon',
};

// ───────────────────────── 보는 틀(확대·이동 계산) ─────────────────────────

/** 가장 작은 배율(화면 픽셀 기준 1%)과 가장 큰 배율(3200%), 한 번에 바뀌는 배율. */
export const MIN_SCALE = 0.01;
export const MAX_SCALE = 32;
export const ZOOM_STEP = 1.25;

/**
 * 그림의 배율과 위치.
 * mode: 'fit'(창에 맞춤, 창 크기를 따라간다) | 'actual'(그림 1픽셀 = 화면 1픽셀) | 'custom'(사용자가 고른 배율)
 * scale: 그림 1픽셀이 차지하는 화면 길이(CSS 픽셀) · pan: 그림 가운데가 보는 칸 가운데에서 벗어난 거리
 */
export function createViewport() {
  const vp = { mode: 'fit', scale: 1, panX: 0, panY: 0 };

  /** 그림 전체가 들어가는 가장 큰 배율. */
  const fitScale = (view, image) => (image.w <= 0 || image.h <= 0 ? 1 : Math.min(view.w / image.w, view.h / image.h));

  /** 그림이 칸보다 작은 축은 가운데에 두고, 큰 축은 빈 틈이 생기지 않게 한다. */
  function clampPan(view, image) {
    const maxX = Math.max(0, (image.w * vp.scale - view.w) / 2);
    const maxY = Math.max(0, (image.h * vp.scale - view.h) / 2);
    vp.panX = Math.min(maxX, Math.max(-maxX, vp.panX));
    vp.panY = Math.min(maxY, Math.max(-maxY, vp.panY));
  }

  return Object.assign(vp, {
    fitScale,
    resetFit() { vp.mode = 'fit'; vp.panX = 0; vp.panY = 0; },
    setActual() { vp.mode = 'actual'; vp.panX = 0; vp.panY = 0; },
    /** 지금 방식에 맞게 배율을 정하고, 그림을 그릴 자리(칸 안의 좌표)를 돌려준다. */
    layout(view, image, dpr) {
      if (vp.mode === 'fit') { vp.scale = fitScale(view, image); vp.panX = 0; vp.panY = 0; }
      else if (vp.mode === 'actual') vp.scale = 1 / dpr;
      clampPan(view, image);
      const w = image.w * vp.scale, h = image.h * vp.scale;
      return { x: view.w / 2 + vp.panX - w / 2, y: view.h / 2 + vp.panY - h / 2, w, h };
    },
    /** 배율에 factor를 곱한다. anchor(없으면 칸 가운데) 아래의 그림 점이 제자리에 있게 한다. layout을 먼저 부른다. */
    zoomBy(factor, anchor, view, image, dpr) {
      const old = vp.scale;
      const next = Math.min(MAX_SCALE / dpr, Math.max(MIN_SCALE / dpr, old * factor));
      if (!Number.isFinite(next) || Math.abs(next - old) <= Number.EPSILON) return;
      const inside = anchor && anchor.x >= 0 && anchor.y >= 0 && anchor.x <= view.w && anchor.y <= view.h;
      const at = inside ? anchor : { x: view.w / 2, y: view.h / 2 };
      const centreX = view.w / 2 + vp.panX, centreY = view.h / 2 + vp.panY;
      vp.panX = at.x - (at.x - centreX) * (next / old) - view.w / 2;
      vp.panY = at.y - (at.y - centreY) * (next / old) - view.h / 2;
      vp.scale = next;
      vp.mode = 'custom';
      clampPan(view, image);
    },
    panBy(dx, dy, view, image) {
      vp.panX += dx;
      vp.panY += dy;
      clampPan(view, image);
    },
    /** 화면 픽셀 기준 배율(100% = 1:1). */
    percent: (dpr) => vp.scale * dpr * 100,
  });
}

/** 휠을 굴린 양을 온전한 칸 수로 바꾼다. 터치패드는 한 칸보다 작은 값을 연달아 보내므로 모아 둔다. */
export function createWheelStepper() {
  let accumulated = 0;
  return (notches) => {
    if (accumulated * notches < 0) accumulated = 0; // 방향을 바꾸면 모아 둔 값은 버린다.
    accumulated += notches;
    const steps = Math.trunc(accumulated);
    accumulated -= steps;
    return steps;
  };
}

export function sizeText(bytes) {
  if (bytes >= 2 ** 20) return `${(bytes / 2 ** 20).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

const hostOf = (url) => { try { return new URL(url).host; } catch { return ''; } };
const two = (n) => String(n).padStart(2, '0');
function timeText(ms) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}

// ───────────────────────── 이미지 항목 ─────────────────────────

/**
 * 이미지 항목의 화면을 host 안에 만든다.
 * saved: { image(저장소 번호), ext(형식), name, source(캡처한 페이지나 그림의 주소), at(넣은 때), shot(화면 캡처), full(페이지 전체 캡처) }
 * 비어 있으면(번호 없음) 파일을 고르거나 붙여넣을 수 있는 빈 화면으로 시작한다.
 */
export function createImage(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus, showMenu } = api;
  let image = saved.image ? { id: saved.image, ext: saved.ext } : null;
  let name = saved.name || '';
  let source = saved.source || '';
  let at = saved.at || 0;
  let shot = !!saved.shot || !!saved.full; // 화면을 찍은 것인지
  let full = !!saved.full;                  // 페이지 전체를 찍은 것인지
  let path = '';                 // 저장소 안의 파일 경로
  let bytes = 0;
  let blob = null, blobUrl = '';
  let natural = { w: 0, h: 0 };  // 그림 크기(픽셀)
  let showInfo = localStorage.getItem(INFO_KEY) !== 'off';
  let disposed = false;
  const vp = createViewport();
  const wheel = createWheelStepper();

  const button = (text, tip, action) => {
    const b = make('button', '', text);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const $title = make('span', 'doc-title');
  const $zoom = make('span', 'image-zoom', '');
  const $info = button('정보', '파일 정보 보이기/숨기기 (I)', () => toggleInfo());
  const tools = [
    button('맞춤', '창에 맞춤 (F, 0)', () => { vp.resetFit(); layout(); }),
    button('1:1', '실제 크기 (1)', () => { vp.setActual(); layout(); }),
    button('−', '축소 (-)', () => zoom(1 / ZOOM_STEP)),
    $zoom,
    button('+', '확대 (+) · Alt·Ctrl+휠: 커서 기준 확대·축소', () => zoom(ZOOM_STEP)),
    $info,
    button('복사', '복사 (Ctrl+C)', copy),
    button('저장', '다른 곳에 저장 (Ctrl+S)', save),
    button('Claude에 넣기', '그림 경로를 Claude 세션 입력창에 넣기', (ev) => api.toClaude(item, path, ev)),
  ];
  const $open = button('파일 열기', '이미지 파일 열기 (Ctrl+O)', pick);
  const $paste = button('붙여넣기', '클립보드의 그림 붙여넣기 (Ctrl+V)', paste);
  const bar = make('div', 'doc-bar');
  bar.append($title, $open, $paste, ...tools);

  const $img = make('img', 'image-pic');
  $img.draggable = false;
  $img.alt = '';
  const $panel = make('div', 'image-info');
  const $note = make('div', 'image-empty');
  const $stage = make('div', 'image-stage');
  $stage.tabIndex = 0;
  $stage.append($img, $panel, $note);
  const root = make('div', 'doc image');
  root.append(bar, $stage);
  host.append(root);

  const title = () => name || hostOf(source) || '이미지';
  const view = () => ({ w: $stage.clientWidth, h: $stage.clientHeight });
  const dpr = () => window.devicePixelRatio || 1;
  /** 같은 묶음의 이미지 항목들 중 이 항목의 자리. */
  const position = () => {
    const list = api.siblings(item);
    return { list, index: list.indexOf(item) };
  };

  // ── 그리기 ──

  function layout() {
    const has = !!blobUrl && natural.w > 0;
    $img.hidden = !has;
    for (const tool of tools) tool.hidden = !image;
    $open.hidden = $paste.hidden = !!image;
    $title.textContent = title();
    $title.title = [title(), source, path].filter(Boolean).join('\n');
    if (!has) { $panel.hidden = true; return; }
    const size = view();
    if (size.w < 2 || size.h < 2) return; // 화면에 없는 동안에는 계산하지 않는다.
    const rect = vp.layout(size, natural, dpr());
    Object.assign($img.style, {
      left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.w}px`, height: `${rect.h}px`,
    });
    // 그림이 칸보다 크면 끌어서 옮길 수 있다.
    $stage.classList.toggle('pannable', rect.w > size.w + 0.5 || rect.h > size.h + 0.5);
    $zoom.textContent = `${Math.round(vp.percent(dpr()))}%`;
    $info.classList.toggle('on', showInfo);
    renderInfo();
  }

  function renderInfo() {
    $panel.hidden = !showInfo;
    if (!showInfo) return;
    const mode = { fit: '창에 맞춤', actual: '실제 크기', custom: '사용자 지정' }[vp.mode];
    const { list, index } = position();
    const lines = [
      [title(), 'strong'],
      [`${natural.w} × ${natural.h} px`],
      [`${sizeText(bytes)} · ${image.ext.toUpperCase()}`],
      [`${Math.round(vp.percent(dpr()))}%  (${mode})`],
      list.length > 1 ? [`${index + 1} / ${list.length}`, 'dim'] : null,
      at ? [`${full ? '페이지 전체 캡처' : shot ? '화면 캡처' : '넣은 때'} ${timeText(at)}`, 'dim'] : null,
      source && !source.startsWith('data:') ? [source, 'dim'] : null,
    ].filter(Boolean);
    $panel.replaceChildren(...lines.map(([text, className]) => make('div', className || '', text)));
  }

  function showNote(text, buttons = []) {
    $note.replaceChildren(make('p', '', text), ...buttons);
    $note.hidden = false;
  }

  /** 저장소의 그림을 읽어 화면에 올린다. */
  async function load() {
    if (!image) {
      showNote('이미지 파일을 여기에 끌어다 놓거나, 파일을 고르거나, 클립보드의 그림을 붙여넣으세요.', [
        button('파일 열기', '이미지 파일 열기 (Ctrl+O)', pick),
        button('붙여넣기', '클립보드의 그림 붙여넣기 (Ctrl+V)', paste),
      ]);
      layout();
      return;
    }
    try {
      const info = await invoke('image_info', { id: image.id, ext: image.ext });
      if (!info) throw new Error('이미지 파일이 없습니다(저장소에서 지워졌습니다).');
      path = info.path;
      bytes = info.bytes;
      const data = await invoke('image_read', { id: image.id, ext: image.ext });
      if (disposed) return;
      blob = new Blob([data], { type: MIME[image.ext] || 'application/octet-stream' });
      if (blobUrl) URL.revokeObjectURL(blobUrl);
      blobUrl = URL.createObjectURL(blob);
      await new Promise((resolve, reject) => {
        $img.onload = resolve;
        $img.onerror = () => reject(new Error('그림을 그리지 못했습니다(깨졌거나 지원하지 않는 형식).'));
        $img.src = blobUrl;
      });
      // 크기 정보가 없는 SVG는 0으로 나온다. 그때는 기본 크기로 본다.
      natural = { w: $img.naturalWidth || 300, h: $img.naturalHeight || 150 };
      $note.hidden = true;
    } catch (err) {
      if (blobUrl) URL.revokeObjectURL(blobUrl);
      blobUrl = '';
      natural = { w: 0, h: 0 };
      showNote(String(err.message || err));
    }
    layout();
    onChange();
  }

  /** 빈 항목에 그림을 채운다. */
  function fill(stored, meta = {}) {
    image = { id: stored.id, ext: stored.ext };
    name = stored.name || '';
    source = meta.source || '';
    shot = !!meta.shot;
    full = !!meta.full;
    at = Date.now();
    vp.resetFit();
    return load();
  }

  /** 가져온 그림들을 넣는다. 이 항목이 비어 있으면 첫 장을 여기에, 나머지는 같은 묶음의 새 이미지 항목으로. */
  async function take(list) {
    if (!list.length) return;
    const rest = image ? list : list.slice(1);
    if (!image) await fill(list[0]);
    if (rest.length) await api.addImages(rest, { near: item });
  }

  async function pick() {
    try {
      await take(await invoke('image_pick', { start: item.startCwd }));
    } catch (err) {
      setStatus(`이미지를 열지 못했습니다: ${err}`);
    }
  }

  async function paste() {
    try {
      const list = await invoke('image_paste');
      if (!list.length) return setStatus('클립보드에 그림이 없습니다.');
      await take(list);
    } catch (err) {
      setStatus(`붙여넣기 실패: ${err}`);
    }
  }

  // ── 동작 ──

  function zoom(factor, anchor = null) {
    if (!blobUrl) return;
    const size = view();
    vp.layout(size, natural, dpr());
    vp.zoomBy(factor, anchor, size, natural, dpr());
    layout();
  }

  function toggleInfo() {
    showInfo = !showInfo;
    localStorage.setItem(INFO_KEY, showInfo ? 'on' : 'off');
    layout();
  }

  /** 같은 묶음의 다른 이미지 항목으로 간다(끝에서 돌아간다). */
  function go(step, absolute = false) {
    const { list, index } = position();
    if (list.length < 2) return;
    const next = absolute ? (step < 0 ? 0 : list.length - 1) : (index + step + list.length * 8) % list.length;
    if (list[next] !== item) api.show(list[next]);
  }

  /** 그림을 PNG로 바꾼 내용(클립보드에는 PNG로 넣는다). */
  async function pngBytes() {
    if (image.ext === 'png') return new Uint8Array(await blob.arrayBuffer());
    const canvas = document.createElement('canvas');
    // 너무 큰 그림은 줄여서 복사한다(캔버스 한도).
    const ratio = Math.min(1, 8192 / Math.max(natural.w, natural.h));
    canvas.width = Math.max(1, Math.round(natural.w * ratio));
    canvas.height = Math.max(1, Math.round(natural.h * ratio));
    canvas.getContext('2d').drawImage($img, 0, 0, canvas.width, canvas.height);
    const png = await new Promise((resolve) => canvas.toBlob(resolve, 'image/png'));
    if (!png) throw new Error('그림을 바꾸지 못했습니다');
    return new Uint8Array(await png.arrayBuffer());
  }

  async function copy() {
    if (!blobUrl) return;
    try {
      await invoke('image_copy', await pngBytes());
      setStatus(`그림을 복사했습니다: ${title()}`);
    } catch (err) {
      setStatus(`복사 실패: ${err}`);
    }
  }

  async function save() {
    if (!image) return;
    try {
      const saved = await invoke('image_export', { id: image.id, ext: image.ext, name: title(), start: item.startCwd });
      if (saved) setStatus(`저장했습니다: ${saved}`);
    } catch (err) {
      setStatus(`저장 실패: ${err}`);
    }
  }

  // 끌어서 이동
  let dragging = null;
  $stage.addEventListener('mousedown', (ev) => {
    $stage.focus();
    if (ev.button !== 0 || !blobUrl || ev.target.closest('button')) return;
    dragging = { x: ev.clientX, y: ev.clientY };
    ev.preventDefault();
  });
  const onMove = (ev) => {
    if (!dragging) return;
    const size = view();
    if (vp.mode === 'actual') vp.mode = 'custom'; // 옮긴 자리를 지킨다.
    vp.panBy(ev.clientX - dragging.x, ev.clientY - dragging.y, size, natural);
    dragging = { x: ev.clientX, y: ev.clientY };
    $stage.classList.add('panning');
    layout();
  };
  const onUp = () => { dragging = null; $stage.classList.remove('panning'); };
  document.addEventListener('mousemove', onMove);
  document.addEventListener('mouseup', onUp);

  // 휠: 이전/다음 이미지. Alt(또는 Ctrl)와 함께 굴리면 커서 기준 확대/축소.
  $stage.addEventListener('wheel', (ev) => {
    if (!blobUrl) return;
    ev.preventDefault();
    const notches = -ev.deltaY / (ev.deltaMode === 1 ? 3 : 100);
    if (ev.altKey || ev.ctrlKey) {
      const rect = $stage.getBoundingClientRect();
      zoom(ZOOM_STEP ** notches, { x: ev.clientX - rect.left, y: ev.clientY - rect.top });
    } else {
      const steps = wheel(notches);
      if (steps) go(-steps); // 아래로 굴리면 다음 이미지
    }
  }, { passive: false });

  $stage.addEventListener('dblclick', (ev) => {
    if (!blobUrl || ev.target.closest('button')) return;
    if (vp.mode === 'fit') vp.setActual(); else vp.resetFit();
    layout();
  });

  $stage.addEventListener('keydown', (ev) => {
    const key = ev.key.toLowerCase();
    if (ev.altKey || ev.shiftKey && ev.ctrlKey) return;
    let handled = true;
    if (ev.ctrlKey) {
      if (key === 'v') paste();
      else if (key === 'o') pick();
      else if (key === 'c') copy();
      else if (key === 's') save();
      else handled = false;
    } else if (key === 'f' || key === '0') { vp.resetFit(); layout(); }
    else if (key === '1') { vp.setActual(); layout(); }
    else if (key === '+' || key === '=') zoom(ZOOM_STEP);
    else if (key === '-') zoom(1 / ZOOM_STEP);
    else if (key === 'i') toggleInfo();
    else if (key === 'arrowright' || key === 'pagedown') go(1);
    else if (key === 'arrowleft' || key === 'pageup') go(-1);
    else if (key === 'home') go(-1, true);
    else if (key === 'end') go(1, true);
    else handled = false;
    if (handled) { ev.preventDefault(); ev.stopPropagation(); }
  });

  $stage.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    if (!image) {
      return showMenu(ev, [
        { label: '파일 열기…   Ctrl+O', action: pick },
        { label: '붙여넣기   Ctrl+V', action: paste },
      ]);
    }
    const many = position().list.length > 1;
    showMenu(ev, [
      { label: '창에 맞춤   F', action: () => { vp.resetFit(); layout(); } },
      { label: '실제 크기   1', action: () => { vp.setActual(); layout(); } },
      { label: '확대   +', action: () => zoom(ZOOM_STEP) },
      { label: '축소   -', action: () => zoom(1 / ZOOM_STEP) },
      ...(many ? [
        { label: '이전 이미지   ←', action: () => go(-1) },
        { label: '다음 이미지   →', action: () => go(1) },
      ] : []),
      { label: `정보 ${showInfo ? '숨기기' : '보이기'}   I`, action: toggleInfo },
      { label: '복사   Ctrl+C', action: copy },
      { label: '다른 곳에 저장…   Ctrl+S', action: save },
      { label: 'Claude 세션에 경로 넣기', action: () => api.toClaude(item, path, ev) },
      { label: '파일 위치 열기', action: () => invoke('link_open', { target: path, cwd: item.startCwd, openFile: false }).catch((err) => setStatus(String(err))) },
      ...(source.startsWith('http') ? [
        { label: '원본 페이지를 브라우저로 열기', action: () => api.openSource(item, source) },
      ] : []),
    ]);
  });

  const observer = new ResizeObserver(layout);
  observer.observe($stage);
  const ready = load();

  return {
    ready,
    focus: () => $stage.focus({ preventScroll: true }),
    title,
    /** 다시 시작할 때 되살릴 내용. */
    state: () => (image ? { image: image.id, ext: image.ext, name, source, at, shot, full } : {}),
    /** 빈 항목인지(끌어다 놓은 그림을 여기에 채울 수 있는지). */
    empty: () => !image,
    fill,
    /** 저장소 안의 그림 파일 경로. */
    path: () => path,
    /** 닫으면 그림은 보관 폴더로 옮겨 30일 동안 둔다. */
    close: async () => {
      if (!image) return true;
      try {
        const kept = await invoke('image_remove', { id: image.id, ext: image.ext });
        if (kept) setStatus(`닫은 이미지는 30일 동안 보관합니다: ${kept}`);
      } catch (err) {
        setStatus(String(err));
      }
      return true;
    },
    dispose: () => {
      disposed = true;
      observer.disconnect();
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      if (blobUrl) URL.revokeObjectURL(blobUrl);
    },
  };
}
