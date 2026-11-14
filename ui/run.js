// 명령 항목: 자주 돌리는 명령(시험, 빌드, 개발 서버 …)을 단추로 두고 실행해, 돌고 있는지·성공했는지·실패했는지를 본다.
//
// 명령 한 줄을 그 항목의 폴더에서 PowerShell로 실행한다(commands.rs의 run_start). 출력은 이 화면의 터미널에 보이고,
// 끝나면 종료 코드로 성공·실패를 가린다. 세션이 아니므로 셸 프롬프트가 남지 않고, 다시 실행하면 처음부터 돈다.
// 다시 시작한 뒤에는 명령만 되살리고 스스로 실행하지는 않는다(사용자가 ▶를 눌러야 돈다).

let api = null; // { invoke, listen, make, setStatus, showMenu, Channel, look(), scrollback, toClaude(item, text, ev), finished(item, info) }

const live = new Map();   // 돌고 있는 프로세스의 번호 → 끝났을 때 부를 함수
const early = new Map();  // 번호를 돌려받기 전에 먼저 온 종료 알림(바로 끝나는 명령): 번호 → 종료 코드
const EARLY_KEEP_MS = 10000;
const FLOW_HIGH = 2 << 20;      // 화면에 못 그린 출력이 이만큼 쌓이면 읽기를 멈춘다.
const FLOW_LOW = 256 << 10;     // 이만큼으로 줄면 다시 읽는다.
const CLAUDE_LINES = 60;        // "Claude에 넣기"가 넣는 출력의 줄 수

export function initRun(given) {
  api = given;
  // 종료 알림은 세션의 것과 같은 사건으로 온다. 명령 항목이 띄운 프로세스의 것만 여기서 받는다.
  api.listen('session-exit', ({ payload }) => {
    const done = live.get(payload.id);
    if (done) {
      live.delete(payload.id);
      done(payload.code);
    } else {
      early.set(payload.id, payload.code);
      setTimeout(() => early.delete(payload.id), EARLY_KEEP_MS);
    }
  });
}

const clip = (text, max = 40) => (text.length > max ? `${text.slice(0, max)}…` : text);
const two = (n) => String(n).padStart(2, '0');

/** 걸린 시간을 말로: 0.8초, 42초, 5분 12초, 1시간 3분. */
function duration(ms) {
  if (ms < 1000) return `${(ms / 1000).toFixed(1)}초`;
  const total = Math.round(ms / 1000);
  const hours = Math.floor(total / 3600), minutes = Math.floor((total % 3600) / 60), seconds = total % 60;
  if (hours) return `${hours}시간 ${minutes}분`;
  return minutes ? `${minutes}분 ${seconds}초` : `${seconds}초`;
}

/** 경과 시간을 시계 모양으로: 3:24, 1:02:07. */
function clock(ms) {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600), minutes = Math.floor((total % 3600) / 60), seconds = total % 60;
  return hours ? `${hours}:${two(minutes)}:${two(seconds)}` : `${minutes}:${two(seconds)}`;
}

/**
 * 명령 항목의 화면을 host 안에 만든다.
 * saved: { command(명령 한 줄), start(만들자마자 실행할지 — 사용자가 방금 명령을 적어 만든 항목만) }
 */
export function createRun(item, host, saved = {}, onChange = () => {}) {
  const { invoke, make, setStatus } = api;
  let command = typeof saved.command === 'string' ? saved.command.trim() : '';
  let phase = 'idle';          // idle(아직 실행하지 않음) | starting | running | ok | fail | stopped
  let code = null;             // 종료 코드(모르면 null)
  let pid = null;              // 돌고 있는 프로세스의 번호
  let startedAt = 0, took = 0;
  let stopping = false;        // 사용자가 멈춤을 눌렀다
  let runSeq = 0;              // 실행마다 하나씩 는다(지난 실행의 출력·종료 알림을 가려낸다)
  let queued = 0, paused = false;
  let pending = '', sending = false;
  let disposed = false;
  let clockTimer = 0;

  const button = (label, tip, action) => {
    const b = make('button', '', label);
    b.title = tip;
    b.onclick = action;
    return b;
  };
  const $go = button('▶ 실행', '', () => (busy() ? stop() : start()));
  $go.classList.add('run-go');
  const $command = make('input', 'run-command');
  Object.assign($command, { value: command, placeholder: '실행할 명령 (예: cargo test) — Enter: 실행', spellcheck: false, autocomplete: 'off' });
  const $state = make('span', 'run-state');
  const bar = make('div', 'doc-bar');
  bar.append(
    $go, $command, $state,
    button('Claude에 넣기', '명령과 출력의 끝부분(고른 글이 있으면 그 글)을 Claude 세션 입력창에 넣기', (ev) => api.toClaude?.(item, report(), ev)),
    button('출력 복사', '출력 전체를 복사 (고른 글이 있으면 그 글)', () => {
      const text = term.getSelection() || output(Infinity);
      if (!text) return setStatus('복사할 출력이 없습니다.');
      return invoke('clipboard_write', { text }).then(() => setStatus('출력을 복사했습니다.')).catch((err) => setStatus(String(err)));
    }),
  );
  const $term = make('div', 'term run-term');
  const box = make('div', 'doc runner'); // `run`은 HTTP 항목의 실행 단추가 쓰는 이름이라 쓰지 않는다.
  box.append(bar, $term);
  host.append(box);

  const look = api.look();
  const term = new window.Terminal({
    fontFamily: look.termFamily, fontSize: look.termPixels, theme: look.termTheme, minimumContrastRatio: look.contrast,
    cursorBlink: false, scrollback: api.scrollback, allowProposedApi: true, allowTransparency: look.rain,
  });
  const fitAddon = new window.FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  term.loadAddon(new window.Unicode11Addon.Unicode11Addon());
  term.unicode.activeVersion = '11';
  term.open($term);

  const busy = () => phase === 'starting' || phase === 'running';
  const title = () => clip(command) || '명령';

  /** 지금 상태를 말로. `brief`이면 사이드바 줄에 맞게 짧게. */
  function stateText(brief = false) {
    if (phase === 'idle') return brief ? (command ? '아직 실행하지 않음' : '') : (command ? '아직 실행하지 않음 · ▶를 누르면 실행' : '명령을 적고 Enter');
    if (busy()) {
      const at = new Date(startedAt);
      return brief ? `실행 중 · ${two(at.getHours())}:${two(at.getMinutes())} 시작` : `실행 중 · ${clock(Date.now() - startedAt)}`;
    }
    if (phase === 'ok') return `성공 · ${duration(took)}`;
    if (phase === 'stopped') return `멈춤 · ${duration(took)}`;
    return code == null ? '시작하지 못함' : `실패 · 종료 코드 ${code} · ${duration(took)}`;
  }

  function refresh() {
    $go.textContent = busy() ? '■ 멈춤' : phase === 'idle' ? '▶ 실행' : '▶ 다시 실행';
    $go.title = busy() ? '명령 멈추기 (프로세스를 끝냅니다)' : `이 폴더에서 실행: ${item.cwd} (Ctrl+Enter)`;
    $go.classList.toggle('stop', busy());
    $command.readOnly = busy();
    $command.title = `폴더: ${item.cwd}${busy() ? '\n실행 중에는 명령을 바꿀 수 없습니다.' : ''}`;
    $state.textContent = stateText();
    $state.className = `run-state ${busy() ? 'busy' : phase}`;
    clearInterval(clockTimer);
    clockTimer = busy() ? setInterval(() => { $state.textContent = stateText(); }, 1000) : 0;
  }

  function fit() {
    if (!$term.clientWidth || !$term.clientHeight) return;
    try { fitAddon.fit(); } catch { /* 아직 크기를 잴 수 없는 상태 */ }
  }

  function setPaused(value) {
    paused = value;
    if (pid != null) invoke('session_pause', { id: pid, paused: value }).catch(() => {});
  }

  /** 명령을 실행한다. 돌고 있으면 아무것도 하지 않는다. */
  async function start() {
    const line = $command.value.trim();
    if (busy()) return;
    if (!line) { $command.focus(); return setStatus('실행할 명령을 적어 주세요.'); }
    command = line;
    $command.value = line;
    phase = 'starting';
    code = null;
    stopping = false;
    startedAt = Date.now();
    took = 0;
    queued = 0;
    paused = false;
    pending = '';
    const run = ++runSeq;
    term.reset();
    // 명령에 든 제어 문자가 화면을 건드리지 않게 뺀 뒤 머리말로 적는다.
    term.write(`\x1b[90m${item.cwd}> ${line.replace(/[\x00-\x1f\x7f]/g, ' ')}\x1b[0m\r\n`);
    fit();
    refresh();
    onChange();
    const channel = new api.Channel();
    channel.onmessage = (data) => {
      if (run !== runSeq || disposed) return;
      const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data);
      queued += bytes.length;
      term.write(bytes, () => {
        queued -= bytes.length;
        if (paused && queued < FLOW_LOW) setPaused(false);
      });
      if (!paused && queued > FLOW_HIGH) setPaused(true);
    };
    let id;
    try {
      id = await invoke('run_start', { cwd: item.cwd, command: line, cols: term.cols, rows: term.rows, onOutput: channel });
    } catch (err) {
      if (run !== runSeq || disposed) return;
      phase = 'fail';
      term.write(`\x1b[31m명령을 시작하지 못했습니다: ${err}\x1b[0m\r\n`);
      refresh();
      onChange();
      return;
    }
    // 띄우는 사이에 항목을 닫았으면 방금 뜬 프로세스를 끝낸다.
    if (disposed || run !== runSeq) { invoke('session_close', { id }).catch(() => {}); return; }
    pid = id;
    phase = 'running';
    drain(); // 띄우는 사이에 모아 둔 입력(터미널이 스스로 보내는 답 포함)을 보낸다.
    const finish = (exit) => onExit(run, exit);
    if (early.has(id)) { const exit = early.get(id); early.delete(id); finish(exit); } else live.set(id, finish);
    if (stopping) invoke('session_close', { id }).catch(() => {}); // 띄우는 사이에 멈춤을 눌렀다.
    refresh();
    onChange();
  }

  /** 돌고 있는 명령을 멈춘다(프로세스를 끝낸다). */
  function stop() {
    if (!busy()) return;
    stopping = true;
    if (pid != null) invoke('session_close', { id: pid }).catch(() => {});
    setStatus(`멈추는 중: ${command}`);
  }

  function onExit(run, exit) {
    if (run !== runSeq || disposed) return;
    pid = null;
    pending = '';
    if (stopping) setStatus(`멈췄습니다: ${command}`);
    took = Date.now() - startedAt;
    code = exit ?? null;
    phase = stopping ? 'stopped' : exit === 0 ? 'ok' : 'fail';
    const color = phase === 'ok' ? '32' : phase === 'fail' ? '31' : '90';
    // 남은 출력이 다 그려진 뒤에 끝 줄을 적는다.
    setTimeout(() => { if (run === runSeq && !disposed) term.write(`\r\n\x1b[${color}m[${stateText()}]\x1b[0m\r\n`); }, 80);
    refresh();
    onChange();
    if (!stopping) api.finished?.(item, { ok: phase === 'ok', code, took, command, text: stateText() });
  }

  /** 화면에 남아 있는 출력의 끝 `max`줄(빈 끝 줄은 뺀다). */
  function output(max = 80) {
    const buffer = term.buffer.active;
    const lines = [];
    for (let i = 0; i < buffer.length; i += 1) lines.push(buffer.getLine(i).translateToString(true));
    while (lines.length && !lines[lines.length - 1].trim()) lines.pop();
    return (Number.isFinite(max) ? lines.slice(-max) : lines).join('\n');
  }

  /** Claude 세션에 넘길 글: 명령, 폴더, 상태, 출력의 끝부분(고른 글이 있으면 그 글). */
  function report() {
    const picked = term.getSelection().trim();
    const body = picked || output(CLAUDE_LINES);
    return `명령 \`${command || $command.value.trim()}\` (폴더: ${item.cwd}) — ${stateText()}\n${picked ? '고른 출력' : `출력의 끝부분(${CLAUDE_LINES}줄까지)`}:\n${body || '(출력 없음)'}\n`;
  }

  /**
   * 돌고 있는 명령에 입력을 순서대로 보낸다. 띄우는 중(프로세스 번호를 아직 모를 때)에 들어온 것은 모아 두었다가 보낸다.
   * 가상 터미널은 시작하자마자 커서 위치를 묻고 답이 올 때까지 프로그램을 진행시키지 않는데, 터미널이 스스로 내는 그 답이
   * 번호를 돌려받기 전에 나올 수 있다. 그 답을 버리면 명령이 "실행 중"에서 멈춘다.
   */
  function send(data) {
    if (!busy()) return;
    pending += data;
    drain();
  }

  async function drain() {
    if (sending || pid == null) return;
    sending = true;
    while (pending && pid != null) {
      const chunk = pending;
      pending = '';
      try { await invoke('session_write', { id: pid, data: chunk }); } catch { break; }
    }
    sending = false;
  }

  async function paste() {
    try {
      const found = await invoke('clipboard_read');
      if (found.kind === 'text' || (found.kind !== 'empty' && found.kind !== 'image' && found.text)) term.paste(found.text);
    } catch (err) {
      setStatus(`붙여넣기 실패: ${err}`);
    }
  }

  function copy() {
    const text = term.getSelection();
    if (text) invoke('clipboard_write', { text }).catch(() => {});
    term.clearSelection();
  }

  term.onData((data) => send(data));
  term.onResize(({ cols, rows }) => { if (pid != null) invoke('session_resize', { id: pid, cols, rows }).catch(() => {}); });
  term.attachCustomKeyEventHandler((ev) => {
    if (ev.type !== 'keydown' || !ev.ctrlKey || ev.altKey) return true;
    const key = ev.key.toLowerCase();
    if (key === 'v') { ev.preventDefault(); paste(); return false; }
    if (key === 'c' && (ev.shiftKey || term.hasSelection())) { ev.preventDefault(); copy(); return false; }
    if (key === 'enter') return false; // Ctrl+Enter는 이 화면의 "실행"이다(아래에서 받는다).
    return true;
  });
  $term.addEventListener('contextmenu', (ev) => {
    ev.preventDefault();
    if (term.hasSelection()) copy(); else if (phase === 'running') paste();
  });
  $command.addEventListener('keydown', (ev) => {
    if (ev.key !== 'Enter') return;
    ev.preventDefault();
    ev.stopPropagation();
    if (busy()) setStatus('실행 중입니다. 다시 실행하려면 먼저 [■ 멈춤]을 누르세요.'); else start();
  });
  $command.addEventListener('input', () => { if (!busy()) { command = $command.value.trim(); onChange(); } });
  box.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' && ev.ctrlKey && !ev.altKey && !ev.shiftKey) {
      ev.preventDefault();
      ev.stopPropagation();
      if (!busy()) start();
    }
  });

  const resize = new ResizeObserver(fit);
  resize.observe($term);
  const relook = () => {
    if (disposed) return;
    const now = api.look();
    Object.assign(term.options, { theme: now.termTheme, fontFamily: now.termFamily, fontSize: now.termPixels, minimumContrastRatio: now.contrast });
    requestAnimationFrame(fit);
  };
  window.addEventListener('cypher-look', relook);

  refresh();
  // 사용자가 방금 명령을 적어 만든 항목은 바로 실행한다(화면에 놓여 크기가 잡힌 뒤에).
  const ready = Promise.resolve().then(() => {
    if (saved.start === true && command) requestAnimationFrame(() => { if (!disposed) start(); });
  });

  return {
    ready,
    focus: () => { fit(); if (busy()) term.focus(); else if (command) $go.focus(); else $command.focus(); },
    title,
    state: () => ({ command }),
    close: () => Promise.resolve(true),
    dispose: () => {
      disposed = true;
      clearInterval(clockTimer);
      resize.disconnect();
      window.removeEventListener('cypher-look', relook);
      if (pid != null) { live.delete(pid); invoke('session_close', { id: pid }).catch(() => {}); }
      term.dispose();
    },
    /** 명령 한 줄. */
    command: () => command,
    /** idle | starting | running | ok | fail | stopped */
    phase: () => phase,
    exitCode: () => code,
    stateText: () => stateText(),
    /** 사이드바 줄 아래에 보일 글. */
    sub: () => stateText(true),
    /** 사이드바 줄의 상태(색): 'busy' | 'ok' | 'fail' | ''. */
    tone: () => (busy() ? 'busy' : phase === 'ok' ? 'ok' : phase === 'fail' ? 'fail' : ''),
    output,
    start,
    stop,
  };
}
