// SSH 세션: 접속할 곳을 고르거나 적는 창, 저장해 둔 서버 목록.
//
// 접속은 Cypher에 든 SSH(앱의 sshc 모듈)로 한다. ~/.ssh/config·키 파일·known_hosts·SSH 에이전트(Windows OpenSSH, Pageant)를
// 명령줄의 ssh처럼 쓰고, 처음 보는 서버의 키·키 파일의 암호·비밀번호·2단계 인증은 세션 화면에서 묻는다(저장하지 않는다).
// 같은 서버에 세션을 더 열 때(연결 복제)는 인증을 마친 연결에 채널만 하나 더 열어 다시 묻지 않는다.
// "추가 옵션"을 적었거나 ~/.ssh/config에서 내장 SSH가 따르지 못하는 설정(경유, 포트 포워딩, 원격 명령, 따질 수 없는 Match 등)이
// 걸리면 그 세션은 Windows의 OpenSSH(ssh.exe)로 띄운다.
// 저장해 둔 서버는 이 컴퓨터에 기억한다(localStorage): 이름, 접속할 곳(user@host), 포트, 키 파일, 추가 옵션.

const KEY = 'cypher.ssh.hosts';
const MAX_HOSTS = 100;

let api = null; // { invoke, make, openPopup, setStatus, connect(spec, opts) → Promise<세션 | null> }

export function initSsh(given) {
  api = given;
}

/** 접속할 곳을 저장·전달하기 좋게 다듬는다: 글자만 남기고, 빈 값은 뺀다. 쓸 수 없으면 null. */
export function cleanSpec(spec) {
  if (!spec || typeof spec !== 'object') return null;
  const text = (value) => (typeof value === 'string' ? value.trim() : '');
  const target = text(spec.target);
  if (!target) return null;
  const port = Number(spec.port);
  return {
    target,
    ...(text(spec.name) ? { name: text(spec.name) } : {}),
    ...(Number.isInteger(port) && port > 0 && port < 65536 ? { port } : {}),
    ...(text(spec.identity) ? { identity: text(spec.identity) } : {}),
    ...(text(spec.options) ? { options: text(spec.options) } : {}),
  };
}

/** 사이드바와 창 제목에 보일 이름: 붙여 둔 이름, 없으면 접속할 곳(포트가 있으면 함께). */
export const specLabel = (spec) => spec?.name || (spec?.target ? `${spec.target}${spec.port ? `:${spec.port}` : ''}` : '');

/** 두 접속이 같은 곳인지(이름은 보지 않는다). */
const sameServer = (a, b) => a.target === b.target && (a.port || 0) === (b.port || 0) && (a.identity || '') === (b.identity || '') && (a.options || '') === (b.options || '');

function load() {
  let list = [];
  try { list = JSON.parse(localStorage.getItem(KEY) || '[]'); } catch { /* 깨진 기록은 버린다 */ }
  return Array.isArray(list) ? list.map(cleanSpec).filter(Boolean) : [];
}

function save(list) {
  try { localStorage.setItem(KEY, JSON.stringify(list.slice(0, MAX_HOSTS))); } catch { /* 저장 공간 문제는 무시 */ }
}

/** 저장해 둔 서버 목록(시험·빠른 이동용). */
export const savedHosts = () => load();

/** 두 저장 항목이 같은 것인지(같은 곳이고 이름도 같다). */
const sameEntry = (a, b) => sameServer(a, b) && (a.name || '') === (b.name || '');

/**
 * 서버를 저장한다. `original`(고치려고 불러온 항목)을 주면 그 자리의 것을 바꾸고, 아니면 같은 곳이 이미 있을 때 그것을 바꾼다
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
  const target = field('접속할 곳', 'user@example.com 또는 ~/.ssh/config의 이름', fill?.target, '사용자@호스트. 사용자를 빼면 ~/.ssh/config의 User나 Windows 계정 이름을 씁니다.');
  const name = field('이름', '(선택) 사이드바에 보일 이름 · 예: 운영 웹 1', fill?.name);
  const port = field('포트', '(선택) 기본 22', fill?.port ? String(fill.port) : '');
  const identity = field('키 파일', '(선택) 개인 키 경로 · 비우면 ~/.ssh의 키와 에이전트를 씀', fill?.identity);
  const options = field('추가 옵션', '(선택) 예: -J jump@gate -L 8080:localhost:80 · 적으면 ssh.exe로 접속', fill?.options, 'ssh 명령에 그대로 붙일 옵션. 빈칸으로 나누고, 빈칸이 든 값은 큰따옴표로 감쌉니다.\n옵션을 적은 접속은 Windows의 OpenSSH(ssh.exe)로 띄웁니다(이 접속은 연결 복제 때 다시 인증합니다).');
  port.input.inputMode = 'numeric';
  const pick = make('button', '', '고르기…');
  pick.type = 'button';
  pick.onclick = async () => {
    const path = await invoke('ssh_pick_key').catch(() => null);
    if (path) identity.input.value = path;
  };
  identity.row.append(pick);

  const problem = make('p', 'form-problem', '');
  /** 칸에 적은 것을 접속으로 읽는다. 잘못됐으면 까닭을 보여 주고 null. */
  const read = () => {
    const given = target.input.value.trim();
    const portText = port.input.value.trim();
    let bad = null, message = '';
    if (!given) [bad, message] = [target, '접속할 곳을 적어 주세요 (예: user@example.com).'];
    else if (/\s/.test(given) || given.startsWith('-')) [bad, message] = [target, '접속할 곳은 빈칸 없이 user@host 꼴로 적습니다. 옵션은 "추가 옵션" 칸에 적으세요.'];
    else if (portText && !(/^\d{1,5}$/.test(portText) && Number(portText) > 0 && Number(portText) < 65536)) [bad, message] = [port, '포트는 1~65535의 숫자입니다.'];
    else if ((options.input.value.match(/"/g) || []).length % 2) [bad, message] = [options, '추가 옵션의 큰따옴표가 닫히지 않았습니다.'];
    problem.textContent = message;
    if (bad) { bad.input.focus(); return null; }
    return cleanSpec({ target: given, name: name.input.value, port: portText ? Number(portText) : 0, identity: identity.input.value, options: options.input.value });
  };

  const go = make('button', 'ssh-go', '접속');
  go.title = '적은 곳에 접속 (Enter)';
  go.onclick = () => { const spec = read(); if (spec) connect(spec); };
  const keep = make('button', '', '저장하고 접속');
  keep.title = '다음에 한 번 눌러 접속할 수 있게 저장해 두고 접속';
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
  form.append(target.row, name.row, port.row, identity.row, options.row, problem, buttons);
  form.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' && ev.target.tagName === 'INPUT') { ev.preventDefault(); go.click(); }
  });

  const serverRow = (spec, note, stored) => {
    const row = make('div', 'row');
    const use = make('button', 'row-title', `⇌ ${specLabel(spec)}`);
    if (spec.name) use.append(make('span', 'template-summary', `${spec.target}${spec.port ? `:${spec.port}` : ''}`));
    use.title = `${specLabel(spec)}\n${note}\n클릭: 접속`;
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
    ...saved.map((spec) => serverRow(spec, [spec.identity ? '키 지정' : '', spec.options ? '옵션 있음' : ''].filter(Boolean).join(' · '), true)),
    ...(configRows.length ? [make('div', 'menu-caption', '~/.ssh/config에 적어 둔 호스트 (누르면 접속)'), ...configRows] : []),
    make('div', 'menu-caption', fill ? '고쳐서 접속하거나 저장' : '새로 접속'),
    form,
    make('p', 'note', 'Cypher에 든 SSH로 접속합니다. 처음 보는 서버의 키·키 파일의 암호·비밀번호·일회용 번호는 세션 화면에서 묻고, 저장하지 않습니다. '
      + '~/.ssh/config·키 파일·known_hosts·SSH 에이전트(Windows OpenSSH, Pageant)는 명령줄의 ssh처럼 씁니다. '
      + '같은 서버에 세션을 더 열려면 세션의 오른쪽 클릭 → "이 연결 복제"(Ctrl+Shift+T·D): 열린 연결을 같이 써서 다시 인증하지 않습니다. '
      + '출력을 파일로 남기려면 창 제목 줄의 [기록]을 누르세요. '
      + '여러 서버에 같은 명령을 넣으려면 SSH 세션들을 나란히 띄우고 창 제목 줄의 [동시 입력]을 누르세요(워크스페이스 오른쪽 클릭 → "SSH 세션 N개 나란히 보고 동시 입력").'),
  ]);
  target.input.focus();
}
