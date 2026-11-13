// 모양 설정: 테마(어두운·밝은·Windows 고전·매트릭스), 글꼴, 글자 크기. 창 상단 메뉴 "설정 → 모양"에서 연다.
//
// 고른 값은 이 컴퓨터에 기억해 두고(localStorage) 바로 적용한다. 다시 시작할 필요가 없다.
// - 테마: <html data-theme="…">를 바꾸면 theme.css가 색을 바꾼다. 터미널 색은 여기서 따로 정한다.
// - 화면 글자 크기: 본창 전체의 배율로 바꾼다(글자와 함께 단추·여백도 같이 커져 화면이 깨지지 않는다).
// - 터미널 글자 크기: 화면 글자 크기와 따로 정한다(배율만큼 나눠서 넣는다).

import { setRain } from './matrix.js';

const KEY = 'cypher.look';
const BASE_SIZE = 13; // 화면 글자 크기의 기준(배율 100%)

export const THEMES = {
  dark: {
    label: '어두운 테마 (기본)',
    contrast: 1,
    term: { background: '#16181d', foreground: '#d7dae0', cursor: '#d97757', selectionBackground: '#3e4451' },
    search: { matchBackground: '#5c4a1f', activeMatchBackground: '#d97757', matchOverviewRuler: '#5c4a1f', activeMatchColorOverviewRuler: '#d97757' },
  },
  light: {
    label: '밝은 테마',
    contrast: 4.5, // 어두운 바탕에 맞춘 옅은 색 글자도 읽히게 한다.
    term: {
      background: '#ffffff', foreground: '#1f2328', cursor: '#c2410c', cursorAccent: '#ffffff', selectionBackground: '#b6d7ff',
      black: '#24292f', red: '#cf222e', green: '#116329', yellow: '#7d4e00', blue: '#0969da', magenta: '#8250df', cyan: '#1b7c83', white: '#6e7781',
      brightBlack: '#57606a', brightRed: '#a40e26', brightGreen: '#1a7f37', brightYellow: '#633c01', brightBlue: '#218bff',
      brightMagenta: '#a475f9', brightCyan: '#3192aa', brightWhite: '#8c959f',
    },
    search: { matchBackground: '#fff3b0', activeMatchBackground: '#ffb86b', matchOverviewRuler: '#d4a72c', activeMatchColorOverviewRuler: '#c2410c' },
  },
  win32: {
    label: 'Windows 고전 (Win32)',
    contrast: 3,
    term: {
      background: '#000000', foreground: '#c0c0c0', cursor: '#c0c0c0', cursorAccent: '#000000', selectionBackground: '#0000a8',
      black: '#000000', red: '#800000', green: '#008000', yellow: '#808000', blue: '#000080', magenta: '#800080', cyan: '#008080', white: '#c0c0c0',
      brightBlack: '#808080', brightRed: '#ff0000', brightGreen: '#00ff00', brightYellow: '#ffff00', brightBlue: '#0000ff',
      brightMagenta: '#ff00ff', brightCyan: '#00ffff', brightWhite: '#ffffff',
    },
    search: { matchBackground: '#808000', activeMatchBackground: '#ffff00', matchOverviewRuler: '#808000', activeMatchColorOverviewRuler: '#ffff00' },
  },
  matrix: {
    label: '매트릭스 (녹색 글자가 흐르는 화면)',
    contrast: 1,
    rain: true, // 본창 바탕에 흐르는 글자를 깐다(matrix.js). 모양 설정에서 끌 수 있다.
    term: {
      // 바탕은 비워 둔다: 터미널 칸의 반투명한 검은 바탕(theme.css의 --term-bg) 뒤로 흐르는 글자가 비친다.
      // 비침을 켜지 않고 만든 터미널에서는 그냥 검은 바탕이 된다.
      background: 'rgba(0, 0, 0, 0)', foreground: '#39f57a', cursor: '#c8ffd8', cursorAccent: '#001a08', selectionBackground: 'rgba(0, 255, 102, 0.28)',
      black: '#001a08', red: '#ff5f5f', green: '#00ff66', yellow: '#c8ff3d', blue: '#2bd4a4', magenta: '#8dffb0', cyan: '#38ffd0', white: '#a8f5c0',
      brightBlack: '#1f8a45', brightRed: '#ff8a8a', brightGreen: '#6dffa3', brightYellow: '#e2ff7a', brightBlue: '#5cf0c8',
      brightMagenta: '#b6ffcf', brightCyan: '#7dffe2', brightWhite: '#e6fff0',
    },
    search: { matchBackground: '#0d5c28', activeMatchBackground: '#00ff66', matchOverviewRuler: '#0d5c28', activeMatchColorOverviewRuler: '#00ff66' },
  },
};

const UI_FONTS = ['Malgun Gothic', 'Segoe UI', '굴림', '돋움', '바탕', 'Pretendard', 'Noto Sans KR', '나눔고딕', 'Tahoma'];
const TERM_FONTS = ['Cascadia Mono', 'Cascadia Code', 'D2Coding', 'Consolas', '굴림체', '나눔고딕코딩', 'Lucida Console', 'JetBrains Mono'];

let api = null;   // { invoke, make, openPopup, setStatus, config, onChange(look) }
let saved = {};   // { theme, uiFont, uiSize, termFont, termSize } — 비어 있는 값은 기본값을 쓴다.

const clamp = (value, low, high, fallback) => {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? Math.min(high, Math.max(low, n)) : fallback;
};
const quoted = (name) => (/^[\w-]+$/.test(name) ? name : `"${name.replace(/"/g, '')}"`);

/** 지금 적용된 모양. */
export function currentLook() {
  const theme = THEMES[saved.theme] ? saved.theme : 'dark';
  const uiSize = clamp(saved.uiSize, 10, 24, BASE_SIZE);
  const zoom = uiSize / BASE_SIZE;
  const termSize = clamp(saved.termSize, 6, 72, api.config.font.size);
  const termBase = api.config.font.family;
  // 글꼴 이름은 글이어야 한다. 기억해 둔 값이 다른 것(숫자 등)으로 망가져 있으면 고르지 않은 것으로 본다.
  const fontName = (value) => (typeof value === 'string' ? value.trim() : '');
  const uiFont = fontName(saved.uiFont), termFont = fontName(saved.termFont);
  return {
    theme, zoom, uiSize, termSize, uiFont, termFont,
    termFamily: termFont ? `${quoted(termFont)}, ${termBase}` : termBase,
    // 화면 배율이 터미널 글자까지 키우지 않게 그만큼 나눠 둔다.
    termPixels: Math.round((termSize / zoom) * 100) / 100,
    termTheme: THEMES[theme].term,
    searchColors: THEMES[theme].search,
    contrast: THEMES[theme].contrast,
    // 흐르는 글자를 까는 테마인지, 그리고 지금 켜 두었는지.
    rainy: !!THEMES[theme].rain,
    rain: !!THEMES[theme].rain && saved.rain !== false,
  };
}

function apply() {
  const now = currentLook();
  const root = document.documentElement;
  root.dataset.theme = now.theme;
  // 흐르는 글자: 켜져 있으면 본창 바탕을 비워(theme.css) 그 뒤의 캔버스가 보이게 한다.
  root.dataset.rain = now.rain ? 'on' : 'off';
  setRain(now.rain);
  // 글꼴을 고르지 않았으면 테마의 기본 글꼴을 쓴다. 골랐으면 그 글꼴을 맨 앞에 두고 한글 글꼴을 뒤에 받친다.
  if (now.uiFont) root.style.setProperty('--ui-font', `${quoted(now.uiFont)}, "Malgun Gothic", "Segoe UI", sans-serif`);
  else root.style.removeProperty('--ui-font');
  if (now.termFont) root.style.setProperty('--mono-font', now.termFamily);
  else root.style.removeProperty('--mono-font');
  api.invoke('ui_zoom', { factor: now.zoom }).catch(() => {});
  api.onChange?.(now);
}

function store() {
  try { localStorage.setItem(KEY, JSON.stringify(saved)); } catch { /* 저장 공간 문제는 무시 */ }
}

export function initLook(given) {
  api = given;
  try { saved = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch { saved = {}; }
  if (!saved || typeof saved !== 'object' || Array.isArray(saved)) saved = {}; // 기억해 둔 값이 설정 묶음이 아니면 버린다.
  apply();
}

/** 설정을 바꾸고 바로 적용한다(자동 검증에서도 쓴다). */
export function setLook(change) {
  saved = { ...saved, ...change };
  for (const key of Object.keys(saved)) if (saved[key] === '' || saved[key] == null) delete saved[key];
  store();
  apply();
}

/** 모양 설정 창. 고치는 즉시 적용된다. */
export function showLook() {
  const { make, openPopup } = api;
  const row = (label, control) => {
    const line = make('div', 'look-row');
    line.append(make('label', '', label), control);
    return line;
  };
  const now = currentLook();

  const theme = make('select');
  for (const [id, def] of Object.entries(THEMES)) {
    const option = make('option', '', def.label);
    option.value = id;
    theme.append(option);
  }
  theme.value = now.theme;
  theme.onchange = () => { setLook({ theme: theme.value }); refresh(true); };

  // 흐르는 글자를 까는 테마에서만 보이는 줄: 끄면 색만 남는다.
  const rain = make('input');
  rain.type = 'checkbox';
  rain.onchange = () => { setLook({ rain: rain.checked ? null : false }); refresh(); };
  const rainLabel = make('label', 'look-check');
  rainLabel.append(rain, ' 바탕에 녹색 글자가 흐르게 한다');
  const rainRow = row('흐르는 글자', rainLabel);

  const fontInput = (value, list, listId, placeholder, key) => {
    const input = make('input');
    Object.assign(input, { value, placeholder, spellcheck: false, autocomplete: 'off' });
    const options = make('datalist');
    options.id = listId;
    for (const name of list) { const option = make('option'); option.value = name; options.append(option); }
    input.setAttribute('list', listId);
    input.onchange = () => { setLook({ [key]: input.value.trim() }); refresh(); };
    const box = make('div');
    box.append(input, options);
    return { box, input };
  };
  const uiFont = fontInput(now.uiFont, UI_FONTS, 'look-ui-fonts', '테마 기본 글꼴', 'uiFont');
  const termFont = fontInput(now.termFont, TERM_FONTS, 'look-term-fonts', '기본 글꼴 (설정 파일의 [font])', 'termFont');

  const sizeInput = (value, low, high, key, note) => {
    const input = make('input');
    Object.assign(input, { type: 'number', min: low, max: high, step: 1, value });
    input.onchange = () => { setLook({ [key]: clamp(input.value, low, high, '') }); refresh(); };
    const box = make('div', 'look-size');
    box.append(input, make('span', '', note));
    return { box, input };
  };
  const uiSize = sizeInput(now.uiSize, 10, 24, 'uiSize', `px · 기본 ${BASE_SIZE} (메뉴·사이드바·메모·항목 화면 전체)`);
  const termSize = sizeInput(now.termSize, 6, 72, 'termSize', `px · 기본 ${api.config.font.size} (터미널)`);

  const sample = make('div', 'look-sample', '');
  const note = make('p', 'note', '');
  const reset = make('button', 'row add', '기본값으로 되돌리기');
  reset.onclick = () => {
    saved = {};
    store();
    apply();
    refresh(true);
  };

  /** 보기 글과 안내를 지금 설정에 맞춘다. */
  function refresh(fill = false) {
    const look = currentLook();
    rainRow.hidden = !look.rainy;
    rain.checked = look.rain;
    if (fill) {
      theme.value = look.theme;
      uiFont.input.value = look.uiFont;
      termFont.input.value = look.termFont;
      uiSize.input.value = look.uiSize;
      termSize.input.value = look.termSize;
    }
    sample.textContent = 'Claude Code 터미널 글자 보기\nconst total = items.length; // 0O 1lI 가나다';
    Object.assign(sample.style, {
      color: look.termTheme.foreground, fontFamily: look.termFamily, fontSize: `${look.termPixels}px`,
    });
    const missing = [look.uiFont, look.termFont].filter((name) => name && !document.fonts.check(`12px ${quoted(name)}`));
    note.textContent = missing.length
      ? `이 컴퓨터에 없는 글꼴입니다: ${missing.join(', ')} (대신 기본 글꼴로 보입니다)`
      : look.theme === 'dark' ? ''
      : look.rainy ? 'Claude Code 화면의 색은 Claude Code의 /theme 설정을 따릅니다. 터미널 뒤로 글자가 비치는 것은 이 테마를 고른 뒤에 연 세션(또는 Cypher를 다시 시작한 뒤)부터입니다.'
      : 'Claude Code 화면의 색은 Claude Code의 /theme 설정을 따릅니다. 밝은 바탕에서는 밝은 테마로 맞추면 더 잘 보입니다.';
  }
  refresh();

  openPopup('모양: 테마 · 글꼴 · 글자 크기', [
    row('테마', theme),
    rainRow,
    row('화면 글꼴', uiFont.box),
    row('화면 글자 크기', uiSize.box),
    row('터미널·코드 글꼴', termFont.box),
    row('터미널 글자 크기', termSize.box),
    sample, note, reset,
    make('p', 'note', '고치는 즉시 적용되고 이 컴퓨터에 기억됩니다. 글꼴은 목록에서 고르거나 설치된 글꼴 이름을 직접 적습니다.'),
  ]);
}
