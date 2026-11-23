// SSH 세션: 접속할 곳을 고르거나 적는 창, 저장해 둔 서버 목록.
//
// 서버는 주소와 포트만 저장한다(사이드바에 보일 이름은 붙일 수 있다). 사용자 이름·비밀번호·키 파일의 암호·일회용 번호·
// 처음 보는 서버의 키는 접속할 때마다 세션 화면에서 차례로 묻고(키보드 대화식), 저장하지 않는다.
// ~/.ssh/config에 User를 적어 둔 호스트는 그 사용자로 접속한다(사용자 이름을 묻지 않는다).
// 접속은 Cypher에 든 SSH(앱의 sshc 모듈)로 한다. ~/.ssh/config·키 파일·known_hosts·SSH 에이전트(Windows OpenSSH, Pageant)를
// 명령줄의 ssh처럼 쓴다. 같은 워크스페이스에서 같은 서버에 같은 사용자로 인증을 마친 연결이 있으면 사용자 이름을 받은 뒤 그 연결을 같이 써서
// 비밀번호를 다시 묻지 않는다. 연결 복제(SSH 탭이나 세션의 오른쪽 클릭 → 세션 복제, 바로 옆 탭으로 열림)는 사용자 이름도 묻지 않는다.
// ~/.ssh/config에서 내장 SSH가 따르지 못하는 설정(경유, 포트 포워딩, 원격 명령, 따질 수 없는 Match 등)이 걸리면 그 세션은
// Windows의 OpenSSH(ssh.exe)로 띄운다(사용자 이름을 모르면 띄우기 전에 묻는다).
// 저장해 둔 서버는 이 컴퓨터에 기억한다(localStorage): 주소, 포트, 이름.

const KEY = 'cypher.ssh.hosts';
const MAX_HOSTS = 100;

let api = null; // { invoke, make, openPopup, setStatus, connect(spec, opts) → Promise<세션 | null> }

export function initSsh(given) {
  api = given;
}

/** 주소에서 사용자 이름(`user@`)을 뗀다: 예전에 `user@host`로 저장한 서버·세션도 주소만 남긴다. */
const hostOnly = (target) => target.slice(target.lastIndexOf('@') + 1).trim();

/**
 * 접속할 곳을 저장·전달하기 좋게 다듬는다: 주소·포트·이름만 남기고(예전에 저장한 사용자 이름·키 파일·추가 옵션은 버린다),
 * 빈 값은 뺀다. 쓸 수 없으면 null.
 */
export function cleanSpec(spec) {
  if (!spec || typeof spec !== 'object') return null;
  const text = (value) => (typeof value === 'string' ? value.trim() : '');
  const target = hostOnly(text(spec.target));
  if (!target) return null;
  const port = Number(spec.port);
  return {
    target,
    ...(text(spec.name) ? { name: text(spec.name) } : {}),
    ...(Number.isInteger(port) && port > 0 && port < 65536 ? { port } : {}),
  };
}

/** 사이드바와 창 제목에 보일 이름: 붙여 둔 이름, 없으면 주소(포트가 있으면 함께). */
export const specLabel = (spec) => spec?.name || (spec?.target ? `${spec.target}${spec.port ? `:${spec.port}` : ''}` : '');

/** 두 접속이 같은 서버인지(주소와 포트. 이름은 보지 않는다). */
const sameServer = (a, b) => a.target === b.target && (a.port || 0) === (b.port || 0);

/** 두 저장 항목이 같은 것인지(같은 서버이고 이름도 같다). */
const sameEntry = (a, b) => sameServer(a, b) && (a.name || '') === (b.name || '');

function load() {
  let stored = [];
  try { stored = JSON.parse(localStorage.getItem(KEY) || '[]'); } catch { /* 깨진 기록은 버린다 */ }
  if (!Array.isArray(stored)) return [];
  const list = [];
  for (const spec of stored.map(cleanSpec)) {
    if (spec && !list.some((other) => sameEntry(other, spec))) list.push(spec);
  }
  // 예전 판이 저장해 둔 사용자 이름·키 파일·추가 옵션은 기록에서도 지운다(주소와 포트만 저장한다).
  if (JSON.stringify(list) !== JSON.stringify(stored)) save(list);
  return list;
}

function save(list) {
  try { localStorage.setItem(KEY, JSON.stringify(list.slice(0, MAX_HOSTS))); } catch { /* 저장 공간 문제는 무시 */ }
}

/** 저장해 둔 서버 목록(시험·빠른 이동용). */
export const savedHosts = () => load();

/**
 * 서버를 저장한다. `original`(고치려고 불러온 항목)을 주면 그 자리의 것을 바꾸고, 아니면 같은 서버가 이미 있을 때 그것을 바꾼다
 * (이름만 고쳐 저장할 때). 이름만 같은 다른 서버는 건드리지 않는다.
 */
export function saveHost(spec, original = null) {
  const clean = cleanSpec(spec);
  if (!clean) return false;
  const list = load();
  const before = original && cleanSpec(original);
  let at = before ? list.findIndex((other) => sameEntry(other, before)) : -1;
  if (at < 0) at = list.findIndex((other) => sameServer(other, clean));
  if (at >= 0) list[at] = clean; else { list.unshift(clean); at = 0; }
  // 고친 결과가 다른 항목과 똑같아졌으면 하나만 남긴다.
  save(list.filter((other, index) => index === at || !sameEntry(other, clean)));
  return true;
}

/**
 * 주소 칸과 포트 칸에 적은 것을 읽는다. 주소 칸에 포트를 붙여 적었으면(`host:2222`, `[::1]:2222`) 포트를 떼어 낸다.
 * { target, port } 또는 { problem: 까닭, field: 'target' | 'port' }.
 */
function readAddress(given, portText) {
  let target = (given || '').trim();
  let port = (portText || '').trim();
  if (!target) return { problem: '접속할 주소를 적어 주세요 (예: 192.168.0.10, example.com).', field: 'target' };
  if (target.includes('@')) return { problem: '주소만 적어 주세요. 사용자 이름은 접속할 때 세션 화면에서 묻습니다.', field: 'target' };
  // `host:2222`(콜론이 하나뿐인 것. 대괄호 없는 IPv6 주소는 콜론이 여럿이라 그대로 둔다)나 `[IPv6 주소]:2222`
  const joined = /^([^:[\]]+):(\d{1,5})$/.exec(target) || /^\[([^\]]+)\](?::(\d{1,5}))?$/.exec(target);
  if (joined && !joined[2]) target = joined[1];
  else if (joined) {
    if (port && port !== joined[2]) return { problem: '포트를 두 곳에 다르게 적었습니다. 포트는 포트 칸에만 적어 주세요.', field: 'port' };
    [target, port] = [joined[1], joined[2]];
  }
  if (/\s/.test(target) || target.startsWith('-')) return { problem: '주소는 빈칸 없이 적습니다 (예: example.com).', field: 'target' };
  if (port && !(/^\d{1,5}$/.test(port) && Number(port) > 0 && Number(port) < 65536)) return { problem: '포트는 1~65535의 숫자입니다.', field: 'port' };
  return { target, port: port ? Number(port) : 0 };
}

/**
 * SSH 접속 창. 저장해 둔 서버나 ~/.ssh/config의 호스트를 누르면 바로 접속하고, 아래 칸에 새로 적어 접속하거나 저장한다.
 * `opts`는 만들 세션에 그대로 넘긴다(워크스페이스, 폴더, 화면에 놓는 방식). `fill`은 칸에 미리 채워 둘 접속(고칠 때).
 */
export async function showSsh(opts = {}, fill = null) {
  const { invoke, make, openPopup, setStatus } = api;
  const saved = load();
  const fromConfig = await invoke('ssh_hosts').catch(() => []);
  const connect = (spec) => {
    const clean = cleanSpec(spec);
    if (!clean) return;
    close();
    api.connect(clean, opts);
  };

  const field = (labelText, placeholder, value = '', title = '') => {
    const row = make('label', 'ssh-field');
    const input = make('input');
    Object.assign(input, { placeholder, value: value ?? '', spellcheck: false, autocomplete: 'off', title });
    row.append(make('span', '', labelText), input);
    return { row, input };
  };
  const target = field('주소', '예: 192.168.0.10, example.com 또는 ~/.ssh/config의 이름', fill?.target, '서버의 주소(호스트 이름이나 IP)나 ~/.ssh/config에 적어 둔 이름.\n사용자 이름은 적지 않습니다: 접속할 때 세션 화면에서 묻습니다.');
  const port = field('포트', '(선택) 기본 22', fill?.port ? String(fill.port) : '');
  const name = field('이름', '(선택) 사이드바에 보일 이름 · 예: 운영 웹 1', fill?.name);
  port.input.inputMode = 'numeric';

  const problem = make('p', 'form-problem', '');
  /** 칸에 적은 것을 접속으로 읽는다. 잘못됐으면 까닭을 보여 주고 null. */
  const read = () => {
    const address = readAddress(target.input.value, port.input.value);
    problem.textContent = address.problem || '';
    if (address.problem) { (address.field === 'port' ? port : target).input.focus(); return null; }
    return cleanSpec({ target: address.target, port: address.port, name: name.input.value });
  };

  const go = make('button', 'ssh-go', '접속');
  go.title = '적은 곳에 접속 (Enter) · 사용자 이름과 비밀번호는 세션 화면에서 묻습니다';
  go.onclick = () => { const spec = read(); if (spec) connect(spec); };
  const keep = make('button', '', '저장하고 접속');
  keep.title = '주소와 포트를 저장해 두고(다음에 한 번 눌러 접속) 접속';
  keep.onclick = () => { const spec = read(); if (spec && saveHost(spec, fill)) connect(spec); };
  const only = make('button', '', '저장만');
  only.onclick = () => {
    const spec = read();
    if (!spec || !saveHost(spec, fill)) return;
    setStatus(`SSH 서버를 저장했습니다: ${specLabel(spec)}`);
    close();
    showSsh(opts);
  };
  const buttons = make('div', 'ssh-buttons');
  buttons.append(go, keep, only);
  const form = make('div', 'ssh-form');
  form.append(target.row, port.row, name.row, problem, buttons);
  form.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' && ev.target.tagName === 'INPUT') { ev.preventDefault(); go.click(); }
  });

  const serverRow = (spec, note, stored) => {
    const row = make('div', 'row');
    const use = make('button', 'row-title', `⇌ ${specLabel(spec)}`);
    if (spec.name) use.append(make('span', 'template-summary', `${spec.target}${spec.port ? `:${spec.port}` : ''}`));
    use.title = `${specLabel(spec)}${note ? `\n${note}` : ''}\n클릭: 접속`;
    use.onclick = () => connect(spec);
    row.append(use, make('span', 'row-note', note));
    if (stored) {
      const edit = make('button', 'close', '✎');
      edit.title = '이 서버를 아래 칸에 불러와 고치기';
      edit.onclick = () => { close(); showSsh(opts, spec); };
      const remove = make('button', 'close', '×');
      remove.title = '저장해 둔 서버에서 지우기 (열려 있는 세션에는 영향이 없습니다)';
      remove.onclick = () => {
        save(load().filter((other) => !sameEntry(other, spec)));
        close();
        showSsh(opts);
      };
      row.append(edit, remove);
    }
    return row;
  };
  const configRows = fromConfig
    .filter((host) => host?.name)
    .map((host) => serverRow(
      { target: host.name },
      [host.user && host.hostname ? `${host.user}@${host.hostname}` : host.hostname || host.user, host.port ? `포트 ${host.port}` : '', '~/.ssh/config'].filter(Boolean).join(' · '),
      false,
    ));

  const close = openPopup(`SSH 접속${opts.group ? ` — 워크스페이스 "${opts.group}"` : ''}`, [
    ...(saved.length ? [make('div', 'menu-caption', '저장해 둔 서버 (누르면 접속)')] : []),
    ...saved.map((spec) => serverRow(spec, '', true)),
    ...(configRows.length ? [make('div', 'menu-caption', '~/.ssh/config에 적어 둔 호스트 (누르면 접속)'), ...configRows] : []),
    make('div', 'menu-caption', fill ? '고쳐서 접속하거나 저장' : '새로 접속'),
    form,
    make('p', 'note', '주소와 포트만 저장합니다. 사용자 이름·비밀번호·키 파일의 암호·일회용 번호·처음 보는 서버의 키는 접속할 때마다 세션 화면에서 차례로 묻고, 저장하지 않습니다 '
      + '(로그인하지 못하면 사용자 이름부터 다시 묻습니다 · ~/.ssh/config에 User를 적어 둔 호스트는 그 사용자로 접속). '
      + 'Cypher에 든 SSH로 접속하며, ~/.ssh/config·키 파일·known_hosts·SSH 에이전트(Windows OpenSSH, Pageant)는 명령줄의 ssh처럼 씁니다. '
      + '같은 워크스페이스에서 같은 서버에 같은 사용자로 접속해 있으면 비밀번호를 다시 묻지 않고 그 연결을 같이 씁니다(워크스페이스가 다르면 같이 쓰지 않고 다시 인증합니다). '
      + '같은 워크스페이스의 SSH 창에서 새로 접속하면 그 창의 옆 탭으로 열립니다. 사용자 이름도 묻지 않고 세션을 더 열려면 SSH 탭을 오른쪽 클릭 → "세션 복제"(바로 옆 탭으로, Ctrl+Shift+T·D). '
      + '글자 크기는 SSH 창 위쪽의 [가−]·[가+](Ctrl+휠)로 SSH 세션만 따로 바꿉니다. 출력을 파일로 남기려면 SSH 창 위쪽의 [● 출력 기록]을 누르세요. '
      + '여러 서버에 같은 명령을 넣으려면 SSH 탭이나 사이드바의 세션을 화면 가장자리로 끌어다 놓아 나란히 띄운 뒤 SSH 창 위쪽의 [≫ 동시 입력]을 누르세요.'),
  ]);
  target.input.focus();
}
