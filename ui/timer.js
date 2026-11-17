// 타이머·알람: 얼마 뒤(타이머)나 몇 시 몇 분(알람)에 알려 준다. 24시간 이내로만 맞출 수 있다.
//
// 맞춰 둔 것은 이 컴퓨터에 기억한다(localStorage). Cypher가 꺼져 있는 동안 시간이 지나갔으면 다시 켠 직후에
// "지난 알람"으로 알려 준다. 시각은 벽시계 시각(epoch)으로 적어 두므로 컴퓨터가 잠들었다 깨어나도 지난 것을 바로 알린다.

const KEY = 'cypher.timers';
const SOUND_KEY = 'cypher.timers.sound';
const DAY_MS = 24 * 60 * 60 * 1000;
const MAX_TIMERS = 30;          // 한꺼번에 맞춰 둘 수 있는 수
const LATE_MS = 60 * 1000;      // 이보다 늦게 알리게 됐으면(꺼져 있었다, 잠들어 있었다) 얼마나 지났는지도 적는다
const QUICK = [5, 10, 15, 30, 60, 120]; // 한 번 눌러 맞추는 타이머(분)

let api = null; // { make, openPopup, setStatus, ring({ title, body, late }), onChange(), wake(ms) → Promise }
let list = [];  // { id, at(울릴 때, epoch ms), made(맞춘 때), kind: 'timer' | 'alarm', label }
let ticker = 0;

const two = (n) => String(n).padStart(2, '0');
/** 시각을 "14:05"로. 오늘이 아니면 "내일 14:05"처럼 날짜도 붙인다. */
function clockText(at, now = Date.now()) {
  const d = new Date(at), today = new Date(now);
  const sameDay = d.getFullYear() === today.getFullYear() && d.getMonth() === today.getMonth() && d.getDate() === today.getDate();
  const tomorrow = new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1);
  const isTomorrow = d.getFullYear() === tomorrow.getFullYear() && d.getMonth() === tomorrow.getMonth() && d.getDate() === tomorrow.getDate();
  const day = sameDay ? '' : isTomorrow ? '내일 ' : `${d.getMonth() + 1}/${d.getDate()} `;
  return `${day}${two(d.getHours())}:${two(d.getMinutes())}`;
}

/** 남은 시간을 "1:02:07", "12:34"로. */
function leftText(ms) {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const hours = Math.floor(total / 3600), minutes = Math.floor((total % 3600) / 60), seconds = total % 60;
  return hours ? `${hours}:${two(minutes)}:${two(seconds)}` : `${minutes}:${two(seconds)}`;
}

/** 걸린·지난 시간을 말로: "42초", "5분", "1시간 3분". */
function spanText(ms) {
  const total = Math.max(0, Math.round(ms / 1000));
  const hours = Math.floor(total / 3600), minutes = Math.floor((total % 3600) / 60), seconds = total % 60;
  if (hours) return `${hours}시간${minutes ? ` ${minutes}분` : ''}`;
  if (minutes) return `${minutes}분${seconds && minutes < 10 ? ` ${seconds}초` : ''}`;
  return `${seconds}초`;
}

/**
 * 타이머로 적은 글을 밀리초로 읽는다: "25"(분), "25분", "90초", "1시간 30분", "1h 30m", "1:30"(시간:분), "0:05:30"(시:분:초).
 * 읽지 못하면 null.
 */
export function parseSpan(text) {
  // "1시간 반"은 1시간 30분이다.
  // "1시간 30"처럼 시간 뒤에 숫자만 적은 것은 분이다.
  const given = String(text ?? '').trim().toLowerCase().replace(/시간\s*반/, '시간 30분').replace(/(시간|h)\s*(\d+)\s*$/, (_all, unit, minutes) => `${unit} ${minutes}분`);
  if (!given) return null;
  if (/^\d+(\.\d+)?$/.test(given)) return Math.round(Number(given) * 60000);
  const clock = /^(\d{1,2}):(\d{1,2})(?::(\d{1,2}))?$/.exec(given);
  if (clock) {
    const [, a, b, c] = clock;
    if (Number(b) > 59 || Number(c || 0) > 59) return null;
    return (Number(a) * 3600 + Number(b) * 60 + Number(c || 0)) * 1000;
  }
  let total = 0, rest = given;
  const unit = /^\s*(\d+(?:\.\d+)?)\s*(시간|시|hours?|hrs?|h|분|minutes?|mins?|m|초|seconds?|secs?|s)/;
  while (rest.trim()) {
    const m = unit.exec(rest);
    if (!m) return null;
    const size = /^(시간|시|h)/.test(m[2]) ? 3600000 : /^(분|m)/.test(m[2]) ? 60000 : 1000;
    total += Number(m[1]) * size;
    rest = rest.slice(m[0].length);
  }
  return Math.round(total);
}

/**
 * 알람으로 적은 시각을 다음에 그 시각이 되는 때(epoch ms)로 읽는다: "14:30", "1430", "오후 2:30", "2:30pm", "7시", "7시 30분", "오전 7시 반".
 * 이미 지난 시각이면 내일의 그 시각이다(그래서 늘 24시간 이내다). 읽지 못하면 null.
 */
export function parseClock(text, now = Date.now()) {
  let given = String(text ?? '').trim().toLowerCase();
  if (!given) return null;
  let half = null; // 오전(false)·오후(true)
  const mark = /(오전|오후|am|pm|a\.m\.|p\.m\.)/.exec(given);
  if (mark) {
    half = mark[1] === '오후' || mark[1].startsWith('p');
    given = given.replace(mark[1], ' ').trim();
  }
  let hour, minute;
  let m = /^(\d{1,2})\s*[:시]\s*(\d{1,2})?\s*분?$/.exec(given);
  if (m) [hour, minute] = [Number(m[1]), Number(m[2] || 0)];
  else if ((m = /^(\d{1,2})\s*시\s*반$/.exec(given))) [hour, minute] = [Number(m[1]), 30];
  else if ((m = /^(\d{2})(\d{2})$/.exec(given))) [hour, minute] = [Number(m[1]), Number(m[2])];
  else if (half != null && (m = /^(\d{1,2})$/.exec(given))) [hour, minute] = [Number(m[1]), 0];
  else return null;
  if (half != null) {
    if (hour < 1 || hour > 12) return null;
    hour = (hour % 12) + (half ? 12 : 0);
  }
  if (hour > 23 || minute > 59) return null;
  const base = new Date(now);
  const at = new Date(base.getFullYear(), base.getMonth(), base.getDate(), hour, minute, 0, 0).getTime();
  return at > now ? at : new Date(base.getFullYear(), base.getMonth(), base.getDate() + 1, hour, minute, 0, 0).getTime();
}

function load() {
  let saved = [];
  try { saved = JSON.parse(localStorage.getItem(KEY) || '[]'); } catch { /* 깨진 기록은 버린다 */ }
  return (Array.isArray(saved) ? saved : [])
    .filter((entry) => entry && Number.isFinite(entry.at) && typeof entry.id === 'string')
    .map((entry) => ({
      id: entry.id, at: entry.at, made: Number.isFinite(entry.made) ? entry.made : entry.at,
      kind: entry.kind === 'alarm' ? 'alarm' : 'timer', label: typeof entry.label === 'string' ? entry.label.slice(0, 200) : '',
    }))
    .sort((a, b) => a.at - b.at);
}

function save() {
  try { localStorage.setItem(KEY, JSON.stringify(list)); } catch { /* 저장 공간 문제는 무시 */ }
}

/** 소리로도 알릴지(기본은 켬). */
const soundOn = () => { try { return localStorage.getItem(SOUND_KEY) !== '0'; } catch { return true; } };

/** 짧은 알림음(두 번 울리는 맑은 소리). 소리를 낼 수 없는 상태면 조용히 넘어간다. */
function chime() {
  if (!soundOn()) return;
  try {
    const Audio = window.AudioContext || window.webkitAudioContext;
    const audio = new Audio();
    // 화면을 한 번도 누르지 않은 채로 만든 소리 장치는 멈춘 채로 시작한다. 깨워 본다(안 되면 화면 알림만 뜬다).
    audio.resume?.().catch(() => {});
    const tone = (start, pitch) => {
      const osc = audio.createOscillator(), gain = audio.createGain();
      osc.type = 'sine';
      osc.frequency.value = pitch;
      gain.gain.setValueAtTime(0.0001, audio.currentTime + start);
      gain.gain.exponentialRampToValueAtTime(0.18, audio.currentTime + start + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, audio.currentTime + start + 0.45);
      osc.connect(gain).connect(audio.destination);
      osc.start(audio.currentTime + start);
      osc.stop(audio.currentTime + start + 0.5);
    };
    tone(0, 880);
    tone(0.25, 1175);
    tone(0.9, 880);
    tone(1.15, 1175);
    setTimeout(() => audio.close().catch(() => {}), 2500);
  } catch { /* 소리를 낼 수 없으면 화면 알림만 */ }
}

/** 울릴 때가 된 것을 알린다. `startup`이면 켜자마자 보는 것(꺼져 있는 동안 지나간 것)이다. */
function fire(entry, now, startup) {
  const late = now - entry.at;
  const what = entry.kind === 'alarm' ? '알람' : '타이머';
  const set = entry.kind === 'alarm' ? `${clockText(entry.at, now)} 알람` : `${spanText(entry.at - entry.made)} 타이머`;
  const missed = startup && late > 0;
  const title = missed ? `지난 ${what}${entry.label ? `: ${entry.label}` : ''}` : `${what}${entry.label ? `: ${entry.label}` : ' 시간이 됐습니다'}`;
  const body = missed
    ? `Cypher가 꺼져 있는 동안 시간이 지났습니다 — ${set} (${clockText(entry.at, now)} 예정, ${spanText(late)} 지남)`
    : late > LATE_MS ? `${set} — 예정보다 ${spanText(late)} 늦게 알립니다(컴퓨터가 잠들어 있었습니다).` : `${set}${entry.kind === 'timer' ? `가 끝났습니다 (${clockText(entry.at, now)})` : ''}`;
  api.ring({ title, body, late: missed });
}

/**
 * 울릴 때가 된 것을 찾아 알리고 목록에서 뺀다. 1초마다, 그리고 켠 직후에 부른다.
 * 알리기 전에 저장된 목록을 다시 읽어, 그사이 다른 Cypher 창이 이미 알리고 지운 것은 알리지 않는다.
 */
function check(startup = false) {
  const now = Date.now();
  // 저장된 목록이 기준이다: 다른 Cypher 창이 맞추거나 지운 것, 이미 알리고 지운 것을 그대로 따른다.
  list = load();
  const due = list.filter((entry) => entry.at <= now);
  if (due.length) {
    list = list.filter((entry) => entry.at > now);
    save();
    for (const entry of due) fire(entry, now, startup);
    chime();
    refreshOpen?.();
  }
  api.onChange?.();
  wakeForNext();
}

// 창이 가려지거나 내려가 있으면 브라우저가 화면의 타이머를 늦춘다(오래 가려져 있으면 1분에 한 번). 알람이 그만큼 늦지 않게,
// 다음에 울릴 때까지(길면 30초씩 끊어) 앱이 대신 기다렸다가 알려 주게 한다. 한 번에 하나만 걸어 둔다.
const WAKE_STEP_MS = 30000;
let waking = false;
function wakeForNext() {
  const next = list[0];
  if (waking || !next || !api.wake) return;
  waking = true;
  const wait = Math.max(200, Math.min(WAKE_STEP_MS, next.at - Date.now() + 50));
  Promise.resolve().then(() => api.wake(wait)).then(() => {
    waking = false;
    check(false);
  }, () => {
    // 앱이 기다려 주지 못하면 더 청하지 않는다(1초마다 살피는 것으로 족하다).
    waking = false;
    api.wake = null;
  });
}

export function initTimer(given) {
  api = given;
  list = load();
  check(true); // 꺼져 있는 동안 지나간 것을 켠 직후에 알린다.
  clearInterval(ticker);
  ticker = setInterval(() => check(false), 1000);
}

/** 맞춰 둔 타이머·알람(가장 먼저 울릴 것이 앞). */
export const timers = () => list.map((entry) => ({ ...entry }));

/** 사이드바 단추에 보일 글: 가장 먼저 울릴 것까지 남은 시간(없으면 빈 글)과 맞춰 둔 수. */
export function nextTimer(now = Date.now()) {
  const next = list[0];
  return next ? { left: leftText(next.at - now), count: list.length, at: next.at, label: next.label, kind: next.kind } : null;
}

/**
 * 타이머·알람을 맞춘다. `at`은 울릴 때(epoch ms). 24시간을 넘거나 이미 지난 때면 맞추지 않고 까닭을 돌려준다.
 * 맞췄으면 { ok: true, entry }, 아니면 { ok: false, reason }.
 */
export function addTimer({ at, kind = 'timer', label = '' }, now = Date.now()) {
  if (!Number.isFinite(at)) return { ok: false, reason: '시간을 읽지 못했습니다.' };
  if (at - now < 1000) return { ok: false, reason: '1초 뒤부터 맞출 수 있습니다.' };
  if (at - now > DAY_MS) return { ok: false, reason: '24시간 이내로만 맞출 수 있습니다.' };
  list = load(); // 다른 창이 맞춘 것을 지우지 않게, 저장된 목록에 더한다.
  if (list.length >= MAX_TIMERS) return { ok: false, reason: `한꺼번에 ${MAX_TIMERS}개까지 맞춰 둘 수 있습니다. 필요 없는 것을 지우고 다시 맞추세요.` };
  const entry = {
    id: `${now.toString(36)}${Math.random().toString(36).slice(2, 8)}`, at: Math.round(at), made: now,
    kind: kind === 'alarm' ? 'alarm' : 'timer', label: String(label ?? '').trim().slice(0, 200),
  };
  list = [...list, entry].sort((a, b) => a.at - b.at);
  save();
  api?.onChange?.();
  if (api) wakeForNext();
  return { ok: true, entry };
}

/** 맞춰 둔 것을 지운다. */
export function removeTimer(id) {
  list = load();
  const before = list.length;
  list = list.filter((entry) => entry.id !== id);
  if (list.length === before) return false;
  save();
  api?.onChange?.();
  return true;
}

/** 열려 있는 타이머 창을 다시 그리는 함수(창이 닫혀 있으면 null). */
let refreshOpen = null;

/** 타이머·알람 창: 맞춰 둔 것을 보고 지우고, 새로 맞춘다. */
export function showTimers() {
  const { make, openPopup, setStatus } = api;
  const $list = make('div', 'timer-list');
  const problem = make('p', 'form-problem', '');

  const draw = () => {
    const now = Date.now();
    $list.replaceChildren(...(list.length ? list.map((entry) => {
      const row = make('div', 'row timer-row');
      const text = make('span', 'row-title');
      text.append(
        make('b', 'timer-left', leftText(entry.at - now)),
        make('span', '', `${entry.kind === 'alarm' ? '알람' : `타이머 ${spanText(entry.at - entry.made)}`} · ${clockText(entry.at, now)}에 알림`),
        ...(entry.label ? [make('span', 'timer-label', entry.label)] : []),
      );
      const remove = make('button', 'close', '×');
      remove.title = '이 타이머·알람 지우기';
      remove.onclick = () => { removeTimer(entry.id); draw(); };
      row.append(text, remove);
      return row;
    }) : [make('p', 'note', '맞춰 둔 타이머·알람이 없습니다.')]));
  };
  /** 남은 시간만 고친다(1초마다). 줄 수가 달라졌으면(울려서 빠졌다) 다시 그린다. */
  const tick = () => {
    const cells = $list.querySelectorAll('.timer-left');
    if (cells.length !== list.length) return draw();
    const now = Date.now();
    cells.forEach((cell, index) => { cell.textContent = leftText(list[index].at - now); });
  };

  const label = make('input', 'timer-input');
  Object.assign(label, { placeholder: '(선택) 무엇을 알릴지 · 예: 배포 확인, 회의 들어가기', spellcheck: false, autocomplete: 'off' });
  const set = (at, kind) => {
    const made = addTimer({ at, kind, label: label.value });
    problem.textContent = made.ok ? '' : made.reason;
    if (!made.ok) return false;
    setStatus(`${kind === 'alarm' ? '알람' : '타이머'}를 맞췄습니다: ${clockText(made.entry.at)}에 알립니다${made.entry.label ? ` (${made.entry.label})` : ''}.`);
    label.value = '';
    draw();
    return true;
  };

  const quick = make('div', 'timer-quick');
  for (const minutes of QUICK) {
    const button = make('button', '', minutes >= 60 ? `${minutes / 60}시간` : `${minutes}분`);
    button.title = `${minutes >= 60 ? `${minutes / 60}시간` : `${minutes}분`} 뒤에 알림`;
    button.onclick = () => set(Date.now() + minutes * 60000, 'timer');
    quick.append(button);
  }

  const line = (caption, placeholder, title, button, onGo) => {
    const row = make('div', 'timer-line');
    const input = make('input', 'timer-input');
    Object.assign(input, { placeholder, title, spellcheck: false, autocomplete: 'off' });
    const go = make('button', '', button);
    go.onclick = () => { if (onGo(input.value)) input.value = ''; else input.focus(); };
    input.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); go.click(); } });
    row.append(make('span', '', caption), input, go);
    return { row, input };
  };
  const span = line('타이머', '예: 25 (분) · 1시간 30분 · 90초 · 1:30', '지금부터 얼마 뒤에 알릴지. 숫자만 적으면 분입니다. 24시간까지.', '시작', (text) => {
    const ms = parseSpan(text);
    if (ms == null) { problem.textContent = '시간을 읽지 못했습니다. 예: 25 (분), 1시간 30분, 90초, 1:30'; return false; }
    return set(Date.now() + ms, 'timer');
  });
  const clock = line('알람', '예: 14:30 · 오후 2:30 · 7시 반', '알릴 시각. 이미 지난 시각이면 내일의 그 시각입니다(늘 24시간 이내).', '맞추기', (text) => {
    const at = parseClock(text);
    if (at == null) { problem.textContent = '시각을 읽지 못했습니다. 예: 14:30, 오후 2:30, 7시 반'; return false; }
    return set(at, 'alarm');
  });

  const sound = make('label', 'timer-sound');
  const box = make('input');
  box.type = 'checkbox';
  box.checked = soundOn();
  box.onchange = () => { try { localStorage.setItem(SOUND_KEY, box.checked ? '1' : '0'); } catch { /* 무시 */ } if (box.checked) chime(); };
  sound.append(box, ' 소리로도 알리기');

  const labelRow = make('div', 'timer-line');
  labelRow.append(make('span', '', '할 말'), label);
  const close = openPopup('타이머·알람', [
    $list,
    make('div', 'menu-caption', '새로 맞추기 (24시간 이내)'),
    labelRow, quick, span.row, clock.row, problem, sound,
    make('p', 'note', '시간이 되면 화면 오른쪽 아래의 알림(누를 때까지 남음)과 Windows 알림으로 알려 줍니다. Cypher가 꺼져 있는 동안 시간이 지나갔으면 다시 켠 직후에 알려 줍니다.'),
  ]);
  draw();
  refreshOpen = tick;
  const dialog = document.getElementById('popup');
  const done = () => { if (refreshOpen === tick) refreshOpen = null; clearInterval(timer); dialog.removeEventListener('close', done); };
  const timer = setInterval(() => { if (!dialog.open || !$list.isConnected) done(); else tick(); }, 1000);
  dialog.addEventListener('close', done);
  span.input.focus();
  return close;
}
