//! 내장 SSH: Cypher가 직접 SSH로 서버에 접속해 터미널 세션 하나를 연다.
//!
//! - 화면은 주소와 포트만 넘긴다. 접속·인증에 필요한 물음(사용자 이름, 처음 보는 서버의 키, 키 파일의 암호, 비밀번호,
//!   서버가 묻는 2단계 인증)은 그 세션의 화면에서 묻고 그 화면에서 친 글로 답한다. 어느 것도 저장하지 않는다.
//!   `~/.ssh/config`에 User를 적어 둔 호스트는 그 사용자로 접속한다(묻지 않는다).
//! - 인증을 마친 연결은 세션끼리 나눠 쓸 수 있다: "연결 복제"는 같은 연결에 채널을 하나 더 열어, 다시 인증하지 않는다.
//!   같은 서버에 같은 사용자로 인증을 마친 연결이 있으면 새 세션도 사용자 이름을 받은 뒤 그 연결을 같이 쓴다.
//!   연결은 그 연결을 쓰는 마지막 세션이 끝날 때 닫는다.
//! - 키는 사용자의 `~/.ssh`(설정 파일의 IdentityFile, 없으면 id_ed25519·id_ecdsa·id_rsa)와 Windows의 OpenSSH 에이전트,
//!   Pageant에서 찾는다. 서버의 키는 `~/.ssh/known_hosts`로 확인하고, 처음 보는 서버는 물어본 뒤 거기에 적는다.

use cypher_core::ssh::{self as rules, HostKey, HostSettings, SshTarget};
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect, MethodKind, Pty};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

/// 접속에 쓰는 이 컴퓨터의 값. 시험에서는 사용자의 `~/.ssh` 대신 시험 폴더를 준다.
#[derive(Clone, Debug)]
pub struct SshEnv {
    /// 홈 폴더(`~`). `~/.ssh`가 그 아래에 있다.
    pub home: PathBuf,
    /// 이 컴퓨터의 사용자 이름(접속할 곳에 사용자를 적지 않았을 때 쓴다).
    pub local_user: String,
    /// 이만큼(초) 서버에서 아무것도 오지 않으면 살아 있는지 묻는다. 0이면 묻지 않는다.
    pub keepalive: u64,
    /// SSH 에이전트(Windows OpenSSH, Pageant)의 키를 쓸지.
    pub agent: bool,
    /// 모든 사용자 공통의 known_hosts(`C:\ProgramData\ssh\ssh_known_hosts`).
    pub global_known_hosts: Option<PathBuf>,
}

impl SshEnv {
    /// 이 컴퓨터의 값.
    pub fn system(keepalive: u64) -> Self {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"));
        let local_user = std::env::var("USERNAME").unwrap_or_default();
        let global = std::env::var_os("ProgramData").map(|dir| PathBuf::from(dir).join("ssh").join("ssh_known_hosts"));
        Self { home, local_user, keepalive, agent: true, global_known_hosts: global }
    }

    fn ssh_dir(&self) -> PathBuf {
        self.home.join(".ssh")
    }

    /// `~/.ssh/config`의 글(없으면 빈 글).
    fn config_text(&self) -> String {
        std::fs::read(self.ssh_dir().join("config")).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default()
    }

    /// 그 호스트에 걸리는 설정 파일의 값. `Include`는 `~/.ssh` 기준으로 읽고, 이름 끝의 `*`는 그 폴더의 파일들로 푼다.
    pub fn settings(&self, host: &str) -> HostSettings {
        let base = self.ssh_dir();
        let home = self.home.to_string_lossy().into_owned();
        let include = |given: &str| -> Vec<String> {
            let expanded = rules::expand_path(given, &home, &self.local_user, host, "");
            let path = if Path::new(&expanded).is_absolute() { PathBuf::from(&expanded) } else { base.join(&expanded) };
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let files: Vec<PathBuf> = if name.contains(['*', '?']) {
                let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
                let pattern = name.to_lowercase();
                let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|entry| wildcard(&pattern, &entry.file_name().to_string_lossy().to_lowercase()))
                    .map(|entry| entry.path())
                    .collect();
                found.sort();
                found
            } else {
                vec![path]
            };
            files.iter().filter_map(|file| std::fs::read(file).ok()).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).collect()
        };
        rules::host_settings(&self.config_text(), host, &include)
    }
}

/// 파일 이름의 `*`·`?` 맞추기(Include의 와일드카드).
fn wildcard(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    go(&pattern.chars().collect::<Vec<_>>(), &name.chars().collect::<Vec<_>>())
}

/// 내장 SSH가 이 접속을 다룰 수 있는지. 다루지 못하면 그 까닭(그때는 OpenSSH로 접속한다).
pub fn unsupported(spec: &SshTarget, env: &SshEnv) -> Option<String> {
    let (_, host) = rules::split_target(&spec.target);
    let settings = env.settings(host);
    // 따르지 못하는 설정(경유, 포트 포워딩, 원격 명령, 따질 수 없는 Match…)은 무시하지 않고 OpenSSH에 맡긴다.
    settings.proxy.or(settings.unsupported).map(|what| format!("~/.ssh/config의 {what}"))
}

/// 묻지 않고도 아는 사용자 이름: 접속할 곳에 적힌 것(`사용자@호스트`), 없으면 `~/.ssh/config`의 User.
/// 모르면 None(세션 화면에서 묻는다).
pub fn login_user(spec: &SshTarget, env: &SshEnv) -> Option<String> {
    let (given, alias) = rules::split_target(&spec.target);
    known_user(given, &env.settings(alias))
}

fn known_user(given: Option<&str>, settings: &HostSettings) -> Option<String> {
    given.map(str::to_string).or_else(|| settings.user.clone()).filter(|user| !user.trim().is_empty())
}

// ───────────────────────── 연결(인증을 마친 것, 세션끼리 나눠 씀) ─────────────────────────

/// 인증을 마친 연결. 그 위에 세션마다 채널을 하나씩 연다.
pub struct Conn {
    handle: client::Handle<Client>,
    /// 이 연결을 쓰고 있는 세션 수. 0이 되면 연결을 닫는다(그 뒤에는 늘리지 않는다).
    users: AtomicUsize,
    /// 화면에 보일 이름(`user@host:port`).
    label: String,
    /// 접속한 서버(HostName을 푼 이름)·포트와 인증한 사용자. 새 세션이 같이 쓸 연결을 찾을 때 맞춰 본다.
    host: String,
    port: u16,
    user: String,
}

impl Conn {
    /// 세션 하나가 이 연결을 쓰기 시작한다. 이미 닫았거나 닫는 중이면 거짓.
    fn acquire(&self) -> bool {
        if self.handle.is_closed() {
            return false;
        }
        let mut now = self.users.load(Ordering::SeqCst);
        loop {
            if now == 0 {
                return false;
            }
            match self.users.compare_exchange(now, now + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return true,
                Err(actual) => now = actual,
            }
        }
    }

    /// 세션 하나가 이 연결을 그만 쓴다. 마지막이면 연결을 닫는다.
    fn release(self: &Arc<Self>) {
        if self.users.fetch_sub(1, Ordering::SeqCst) == 1 {
            let conn = self.clone();
            tauri::async_runtime::spawn(async move {
                let _ = conn.handle.disconnect(Disconnect::ByApplication, "", "").await;
            });
        }
    }

    pub fn alive(&self) -> bool {
        !self.handle.is_closed() && self.users.load(Ordering::SeqCst) > 0
    }
}

/// 인증을 마친 연결들. 같은 서버에 같은 사용자로 새로 여는 세션(다시 시작한 뒤 쉬던 세션을 차례로 누를 때, 같은 서버에
/// 세션을 새로 열 때)은 사용자 이름을 받은 뒤 이것을 같이 써서 비밀번호를 거듭 묻지 않는다. 사용자 이름은 묻기 전에는
/// 모르므로, 주소만으로 맞추지 않는다(다른 사용자로 인증한 연결에 붙지 않게).
static LIVE: Mutex<Vec<Weak<Conn>>> = Mutex::new(Vec::new());

/// 끝난 연결을 목록에서 치운다.
fn prune(live: &mut Vec<Weak<Conn>>) {
    live.retain(|weak| weak.upgrade().is_some_and(|conn| conn.alive()));
}

fn remember(conn: &Arc<Conn>) {
    let mut live = LIVE.lock().unwrap();
    prune(&mut live);
    live.push(Arc::downgrade(conn));
}

/// 그 서버·포트에 그 사용자로 인증을 마친 살아 있는 연결.
fn live_conn(host: &str, port: u16, user: &str) -> Option<Arc<Conn>> {
    let mut live = LIVE.lock().unwrap();
    prune(&mut live);
    live.iter().filter_map(Weak::upgrade).find(|conn| conn.host.eq_ignore_ascii_case(host) && conn.port == port && conn.user == user)
}

// ───────────────────────── 세션 하나 ─────────────────────────

/// 화면에서 오는 것.
enum Input {
    Data(Vec<u8>),
    Resize(u16, u16),
}

/// 내장 SSH 세션 하나(터미널 하나 = 채널 하나).
pub struct SshTerm {
    input: mpsc::UnboundedSender<Input>,
    paused: watch::Sender<bool>,
    killed: watch::Sender<bool>,
    conn: Arc<Mutex<Option<Arc<Conn>>>>,
}

type Output = Arc<dyn Fn(&[u8]) + Send + Sync>;

impl SshTerm {
    /// 세션을 시작한다. 접속은 뒤에서 이어지고, 그동안의 안내와 물음은 `on_output`으로 화면에 나간다.
    /// `share`가 살아 있는 연결이면 그 연결에 채널만 연다(다시 인증하지 않는다). 로그인을 마치고 셸이 열리면 `on_ready`가,
    /// 끝나면 `on_exit`가 한 번 불린다.
    pub fn start(
        spec: SshTarget,
        env: SshEnv,
        share: Option<Arc<Conn>>,
        size: (u16, u16),
        on_output: impl Fn(&[u8]) + Send + Sync + 'static,
        on_ready: impl FnOnce() + Send + 'static,
        on_exit: impl FnOnce(Option<u32>) + Send + 'static,
    ) -> SshTerm {
        let (input, input_rx) = mpsc::unbounded_channel();
        let (paused, paused_rx) = watch::channel(false);
        let (killed, killed_rx) = watch::channel(false);
        let conn = Arc::new(Mutex::new(None));
        let slot = conn.clone();
        let output: Output = Arc::new(on_output);
        tauri::async_runtime::spawn(async move {
            let mut io = Io { input: input_rx, output, killed: killed_rx, size, ready: Some(Box::new(on_ready)) };
            let code = run(&mut io, spec, env, share, paused_rx, slot).await;
            on_exit(code);
        });
        SshTerm { input, paused, killed, conn }
    }

    pub fn write(&self, data: &[u8]) {
        let _ = self.input.send(Input::Data(data.to_vec()));
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.input.send(Input::Resize(cols, rows));
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.paused.send(paused);
    }

    pub fn kill(&self) {
        let _ = self.killed.send(true);
    }

    /// 인증을 마친 연결(연결 복제에 쓴다). 아직 접속 중이거나 끝났으면 None.
    pub fn connection(&self) -> Option<Arc<Conn>> {
        self.conn.lock().unwrap().clone().filter(|conn| conn.alive())
    }
}

/// 세션의 화면 입출력: 출력 보내기, 입력 받기, 닫기 알림.
struct Io {
    input: mpsc::UnboundedReceiver<Input>,
    output: Output,
    killed: watch::Receiver<bool>,
    /// 화면의 크기(가로, 세로). 접속·인증하는 동안 바뀐 크기도 적어 두었다가 터미널을 요청할 때 쓴다.
    size: (u16, u16),
    /// 셸이 열렸을 때 한 번 부른다(화면은 그때까지 로그인 중으로 보고 동시 입력에서 뺀다).
    ready: Option<Box<dyn FnOnce() + Send>>,
}

const GRAY: &str = "\x1b[90m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

impl Io {
    fn say(&self, text: &str) {
        (self.output)(text.replace('\n', "\r\n").as_bytes());
    }

    fn note(&self, text: &str) {
        self.say(&format!("{GRAY}{text}{RESET}\n"));
    }

    fn fail(&self, text: &str) {
        self.say(&format!("{RED}{text}{RESET}\n"));
    }

    fn is_killed(&self) -> bool {
        *self.killed.borrow()
    }

    /// 한 줄을 묻는다. `echo`가 거짓이면 친 글을 보이지 않는다(비밀번호). Ctrl+C·Ctrl+D(빈 줄)·세션 닫기면 None.
    async fn ask(&mut self, prompt: &str, echo: bool) -> Option<String> {
        self.say(prompt);
        let mut answer = String::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut escape = 0u8; // 0: 보통, 1: ESC 다음, 2: CSI 안
        loop {
            let data = tokio::select! {
                got = self.input.recv() => match got {
                    Some(Input::Data(data)) => data,
                    Some(Input::Resize(cols, rows)) => {
                        self.size = (cols, rows);
                        continue;
                    }
                    None => return None,
                },
                _ = self.killed.changed() => return None,
            };
            pending.extend_from_slice(&data);
            // 온전한 글자까지만 읽고, 조각 끝에서 잘린 글자는 다음 조각과 잇는다.
            let valid = match std::str::from_utf8(&pending) {
                Err(err) if err.error_len().is_none() => err.valid_up_to(),
                _ => pending.len(),
            };
            let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
            pending.drain(..valid);
            match self.feed_answer(&text, &mut answer, echo, &mut escape) {
                Step::Done => {
                    self.say("\n");
                    return Some(answer);
                }
                Step::Cancel => return None,
                Step::More => {}
            }
        }
    }

    /// 친 글을 답에 더한다.
    fn feed_answer(&self, text: &str, answer: &mut String, echo: bool, escape: &mut u8) -> Step {
        for c in text.chars() {
            match *escape {
                1 if c == '[' || c == 'O' => {
                    *escape = 2;
                    continue;
                }
                // Esc만 누른 것: 다음 글자는 그대로 받는다.
                1 => *escape = 0,
                2 => {
                    if ('@'..='~').contains(&c) {
                        *escape = 0;
                    }
                    continue;
                }
                _ => {}
            }
            match c {
                '\r' | '\n' => return Step::Done,
                '\u{3}' => {
                    self.say("^C\n");
                    return Step::Cancel;
                }
                '\u{4}' if answer.is_empty() => {
                    self.say("\n");
                    return Step::Cancel;
                }
                '\u{1b}' => *escape = 1,
                '\u{7f}' | '\u{8}' => {
                    if let Some(gone) = answer.pop() {
                        if echo {
                            let width = if cypher_core::termlog::is_wide(gone) { 2 } else { 1 };
                            self.say(&"\u{8} \u{8}".repeat(width));
                        }
                    }
                }
                '\u{15}' => {
                    // Ctrl+U: 친 것을 모두 지운다.
                    if echo {
                        let width: usize = answer.chars().map(|c| if cypher_core::termlog::is_wide(c) { 2 } else { 1 }).sum();
                        self.say(&"\u{8} \u{8}".repeat(width));
                    }
                    answer.clear();
                }
                c if (c as u32) < 0x20 => {}
                c => {
                    if answer.chars().count() < 4096 {
                        answer.push(c);
                        if echo {
                            let mut buf = [0u8; 4];
                            (self.output)(c.encode_utf8(&mut buf).as_bytes());
                        }
                    }
                }
            }
        }
        Step::More
    }
}

/// 묻는 한 줄에 친 글을 더한 결과.
enum Step {
    More,
    Done,
    Cancel,
}

/// 접속하고(또는 나눠 쓸 연결에 붙고) 셸을 열어 끝날 때까지 잇는다. 종료 코드를 돌려준다.
async fn run(io: &mut Io, spec: SshTarget, env: SshEnv, share: Option<Arc<Conn>>, paused: watch::Receiver<bool>, slot: Arc<Mutex<Option<Arc<Conn>>>>) -> Option<u32> {
    // 연결 복제: 그 연결에 붙는다(사용자 이름도 묻지 않는다). 연결이 답하지 않으면 새로 접속한다.
    let mut shared = None;
    if let Some(conn) = share {
        match join(io, conn).await {
            Ok(found) => shared = found,
            Err(Closed) => return None,
        }
    }
    let conn = match shared {
        Some(conn) => conn,
        None => match login(io, &spec, &env).await {
            Some(conn) => conn,
            None => return Some(255),
        },
    };
    *slot.lock().unwrap() = Some(conn.clone());
    let code = shell(io, &conn, paused).await;
    *slot.lock().unwrap() = None;
    conn.release();
    code
}

/// 붙으려던 사이에 사용자가 세션을 닫았다.
struct Closed;

/// 나눠 쓸 연결에 붙는다. 정말 살아 있는지 먼저 묻는다: 컴퓨터가 잠들었다 깨어 서버 쪽에서는 끊긴 연결(반쯤 열린 것)을
/// 고르면 채널 열기가 keepalive 시간 초과까지 멈춘다. 답이 없거나 이미 닫혔으면 Ok(None)(새로 접속한다).
async fn join(io: &mut Io, conn: Arc<Conn>) -> Result<Option<Arc<Conn>>, Closed> {
    if !conn.acquire() {
        return Ok(None);
    }
    let ping = tokio::time::timeout(SHARE_CHECK, conn.handle.send_ping());
    let alive = tokio::select! {
        result = ping => matches!(result, Ok(Ok(()))) && !conn.handle.is_closed(),
        _ = io.killed.changed() => {
            conn.release();
            return Err(Closed);
        }
    };
    if alive {
        io.note(&format!("[{} 연결을 같이 씁니다 · 다시 인증하지 않습니다]", conn.label));
        Ok(Some(conn))
    } else {
        io.note(&format!("[{} 연결이 답하지 않아 새로 접속합니다]", conn.label));
        conn.release();
        Ok(None)
    }
}

/// 사용자 이름을 정하고 로그인한다. 사용자 이름을 모르면(화면은 주소와 포트만 넘긴다) 세션 화면에서 묻는다.
/// 같은 서버에 같은 사용자로 인증을 마친 연결이 있으면 그것을 같이 쓴다.
/// 물어서 정한 사용자로 로그인하지 못했으면(비밀번호가 틀림, 비밀번호를 묻는 중에 Ctrl+C) 사용자 이름부터 다시 묻는다.
/// 사용자 이름을 묻는 데서 Ctrl+C·Ctrl+D를 치거나 접속 자체가 안 되면(주소, 서버 키) 그만둔다.
async fn login(io: &mut Io, spec: &SshTarget, env: &SshEnv) -> Option<Arc<Conn>> {
    let (given, alias) = rules::split_target(&spec.target);
    let settings = env.settings(alias);
    let known = known_user(given, &settings);
    if known.is_none() {
        let (host, port) = endpoint(spec, &settings, alias);
        io.note(&format!("[내장 SSH로 {}에 접속합니다 · 사용자 이름과 비밀번호는 여기서 묻고 저장하지 않습니다]", host_port(&host, port)));
    }
    loop {
        let user = match &known {
            Some(user) => user.clone(),
            None => ask_user(io).await?,
        };
        let plan = plan(spec, env, &user);
        if let Some(conn) = live_conn(&plan.host, plan.port, &plan.user) {
            match join(io, conn).await {
                Ok(Some(conn)) => return Some(conn),
                Ok(None) => {}
                Err(Closed) => return None,
            }
        }
        match connect(io, &plan, env).await {
            Login::Done(conn) => {
                remember(&conn);
                return Some(conn);
            }
            Login::Retry if known.is_none() && !io.is_killed() => io.note("[사용자 이름부터 다시 묻습니다 · 그만두려면 Ctrl+C]"),
            Login::Retry | Login::Stop => return None,
        }
    }
}

/// 사용자 이름을 묻는다(친 글이 보인다). 비워 두면 다시 묻는다. Ctrl+C·Ctrl+D·세션 닫기면 None.
async fn ask_user(io: &mut Io) -> Option<String> {
    loop {
        let answer = io.ask("사용자 이름: ", true).await?;
        let user = answer.trim();
        if !user.is_empty() {
            return Some(user.to_string());
        }
    }
}

/// 화면에 보일 `호스트` 또는 `호스트:포트`(기본 포트는 적지 않는다).
fn host_port(host: &str, port: u16) -> String {
    match port {
        22 => host.to_string(),
        _ if host.contains(':') => format!("[{host}]:{port}"), // IPv6 주소
        _ => format!("{host}:{port}"),
    }
}

/// 나눠 쓸 연결이 살아 있는지 묻고 기다리는 시간.
const SHARE_CHECK: Duration = Duration::from_secs(5);
/// 채널(세션)을 열고 터미널·셸을 요청하는 데 기다리는 시간.
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// 화면이 출력을 따라가지 못해 멈춘 동안 모아 두는 출력의 한도. 그동안에도 채널을 비워야 같은 연결의 다른 세션이 멈추지 않는다.
const HOLD_LIMIT: usize = 8 << 20;

// ───────────────────────── 접속과 서버 키 확인 ─────────────────────────

/// 접속하는 동안 연결 쪽(서버 키 확인, 배너)이 화면에 묻거나 알리는 것.
enum Ask {
    Notice(String),
    YesNo(String, oneshot::Sender<bool>),
}

/// 서버 키를 확인하는 쪽. russh가 처음 키 교환 때 부른다(다시 하는 키 교환은 이미 맺은 연결 안에서 하므로 부르지 않는다).
pub struct Client {
    host: String,
    port: u16,
    /// 읽을 known_hosts 파일들(앞의 것이 사용자의 것: 새 키를 거기에 적는다).
    known: Vec<PathBuf>,
    /// 새 키를 적을 파일. None이면 적지 않는다(UserKnownHostsFile none).
    learn: Option<PathBuf>,
    strict: bool,
    /// HashKnownHosts yes: 호스트 이름 대신 해시로 적는다.
    hash: bool,
    asks: mpsc::UnboundedSender<Ask>,
}

/// 해시로 적을 때의 소금(20바이트). 비밀일 필요는 없고 줄마다 달라야 한다.
fn salt() -> [u8; 20] {
    use std::hash::{BuildHasher, Hasher};
    let mut out = [0u8; 20];
    for chunk in out.chunks_mut(8) {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let bytes = hasher.finish().to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    out
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn auth_banner(&mut self, banner: &str, _session: &mut client::Session) -> Result<(), Self::Error> {
        let _ = self.asks.send(Ask::Notice(banner.trim_end().to_string()));
        Ok(())
    }

    async fn check_server_key(&mut self, offered: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key = match offered {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => PublicKey::new(cert.public_key().clone(), ""),
        };
        let openssh = key.to_openssh().unwrap_or_default();
        let mut parts = openssh.split_whitespace();
        let (algorithm, blob) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let shown = rules::known_host_name(&self.host, self.port);
        let mut others: Vec<String> = Vec::new();
        // 모든 파일을 본 뒤에 정한다: 막아 둔 키(@revoked)가 가장 먼저이고(공용 파일에서 막은 키가 사용자 파일에 남아 있어도
        // 접속하지 않는다), 그다음이 아는 키, 바뀐 키 차례다.
        let results: Vec<(&PathBuf, HostKey)> = self
            .known
            .iter()
            .filter_map(|file| std::fs::read(file).ok().map(|bytes| (file, rules::check_known_host(&String::from_utf8_lossy(&bytes), &self.host, self.port, &algorithm, &blob))))
            .collect();
        let rank = |result: &HostKey| match result {
            HostKey::Revoked { .. } => 0,
            HostKey::Known => 1,
            HostKey::Changed { .. } => 2,
            HostKey::Unknown { .. } => 3,
        };
        let mut ordered: Vec<&(&PathBuf, HostKey)> = results.iter().collect();
        ordered.sort_by_key(|(_, result)| rank(result));
        for (file, result) in ordered {
            match result.clone() {
                HostKey::Known => return Ok(true),
                HostKey::Changed { line, algorithm } => {
                    let _ = self.asks.send(Ask::Notice(format!(
                        "{RED}@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                         경고: {shown}의 서버 키가 바뀌었습니다!\n\
                         @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@{RESET}\n\
                         누군가 연결을 가로채고 있을 수 있습니다(중간자 공격). 서버를 새로 설치했거나 키를 바꾼 것일 수도 있습니다.\n\
                         서버가 보인 {algorithm} 키의 지문: {fingerprint}\n\
                         기록된 키: {} {line}번째 줄\n\
                         서버 관리자에게 바뀐 것이 맞는지 확인한 뒤 그 줄을 지우고 다시 접속하세요. 접속하지 않습니다.",
                        file.display()
                    )));
                    return Ok(false);
                }
                HostKey::Revoked { line } => {
                    let _ = self.asks.send(Ask::Notice(format!(
                        "{RED}{shown}의 서버 키는 막아 둔 키입니다({} {line}번째 줄의 @revoked). 접속하지 않습니다.{RESET}",
                        file.display()
                    )));
                    return Ok(false);
                }
                HostKey::Unknown { others: more } => others.extend(more),
            }
        }
        if self.strict {
            let _ = self.asks.send(Ask::Notice(format!(
                "{RED}{shown}의 서버 키가 known_hosts에 없고, ~/.ssh/config의 StrictHostKeyChecking이 yes라 접속하지 않습니다.{RESET}\n{algorithm} 키의 지문: {fingerprint}"
            )));
            return Ok(false);
        }
        others.sort();
        others.dedup();
        let (reply, answer) = oneshot::channel();
        // 다른 종류의 키만 기록돼 있는 서버(접속할 때 그 종류를 먼저 달라고 했는데도 다른 종류를 보였다)는 처음 보는 서버가 아니다.
        let head = if others.is_empty() {
            format!("{YELLOW}{shown}에 처음 접속합니다. 서버가 보인 키가 진짜인지 확인할 길이 없습니다.{RESET}")
        } else {
            format!(
                "{RED}주의: {shown}의 키는 다른 종류({})로 기록돼 있는데, 서버가 기록에 없는 {algorithm} 키를 보였습니다.{RESET}\n\
                 서버가 키를 바꿨거나, 누군가 연결을 가로채고 있을 수 있습니다.",
                others.join(", ")
            )
        };
        let question = format!(
            "{head}\n\
             {algorithm} 키의 지문: {fingerprint}\n\
             서버 관리자에게 받은 지문과 같으면 yes를 치세요. 이 키를 known_hosts에 적고 접속합니다. (yes/no) "
        );
        if self.asks.send(Ask::YesNo(question, reply)).is_err() || !answer.await.unwrap_or(false) {
            return Ok(false);
        }
        if let Some(learn) = &self.learn {
            let line = if self.hash {
                rules::known_host_line_hashed(&self.host, self.port, &algorithm, &blob, &salt())
            } else {
                rules::known_host_line(&self.host, self.port, &algorithm, &blob)
            };
            if let Err(err) = append_known_host(learn, &line) {
                let _ = self.asks.send(Ask::Notice(format!("{YELLOW}known_hosts에 적지 못했습니다({}): {err} · 이번 접속에만 믿습니다{RESET}", learn.display())));
            } else {
                let _ = self.asks.send(Ask::Notice(format!("{GRAY}[{shown}의 키를 {}에 적었습니다]{RESET}", learn.display())));
            }
        }
        Ok(true)
    }
}

/// known_hosts 끝에 한 줄 덧붙인다(마지막 줄이 줄 바꿈으로 끝나지 않았으면 먼저 줄을 바꾼다).
fn append_known_host(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let needs_newline = std::fs::read(path).map(|bytes| bytes.last().is_some_and(|&b| b != b'\n')).unwrap_or(false);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    if needs_newline {
        file.write_all(b"\n")?;
    }
    writeln!(file, "{line}")
}

/// 접속할 곳의 실제 값: 호스트, 포트, 사용자, 키 파일들, known_hosts.
struct Plan {
    host: String,
    port: u16,
    user: String,
    identities: Vec<PathBuf>,
    identities_only: bool,
    known: Vec<PathBuf>,
    learn: Option<PathBuf>,
    strict: bool,
    hash: bool,
    timeout: Duration,
}

impl Plan {
    /// known_hosts에 이 서버에 대해 적혀 있는 키의 종류들.
    fn known_algorithms(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for file in &self.known {
            let Ok(bytes) = std::fs::read(file) else { continue };
            for kind in rules::known_host_algorithms(&String::from_utf8_lossy(&bytes), &self.host, self.port) {
                if !out.contains(&kind) {
                    out.push(kind);
                }
            }
        }
        out
    }
}

/// 서버에 달라고 할 호스트 키 종류의 차례: known_hosts에 적혀 있는 종류를 앞으로 당긴다(OpenSSH와 같다).
/// 그러지 않으면 ECDSA 키만 적어 둔 서버도 Ed25519 키를 보여, 이미 믿은 서버를 처음 보는 서버로 묻게 된다.
fn key_order(known: &[String]) -> Vec<russh::keys::Algorithm> {
    let mut all: Vec<russh::keys::Algorithm> = russh::Preferred::default().key.to_vec();
    let recorded = |algorithm: &russh::keys::Algorithm| {
        let name = algorithm.as_str();
        // RSA 키는 known_hosts에 ssh-rsa로 적히고, 서명은 rsa-sha2-512·rsa-sha2-256으로 한다.
        known.iter().any(|kind| kind == name || (kind == "ssh-rsa" && matches!(algorithm, russh::keys::Algorithm::Rsa { .. })))
    };
    all.sort_by_key(|algorithm| !recorded(algorithm)); // 안정 정렬: 같은 무리 안의 차례는 그대로
    all
}

/// 실제로 접속할 호스트(설정 파일의 HostName을 푼 것)와 포트(적어 준 것, 설정 파일의 Port, 22 차례).
fn endpoint(spec: &SshTarget, settings: &HostSettings, alias: &str) -> (String, u16) {
    let host = settings.hostname.as_deref().map(|name| name.replace("%h", alias).replace("%%", "%")).unwrap_or_else(|| alias.to_string());
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = spec.port.filter(|port| *port != 0).or(settings.port).unwrap_or(22);
    (host, port)
}

/// 그 사용자로 접속할 때의 값(키 파일·known_hosts 경로의 `%r`은 그 사용자로 푼다).
fn plan(spec: &SshTarget, env: &SshEnv, user: &str) -> Plan {
    let (_, alias) = rules::split_target(&spec.target);
    let settings = env.settings(alias);
    let home = env.home.to_string_lossy().into_owned();
    let (host, port) = endpoint(spec, &settings, alias);
    let user = user.to_string();
    let expand = |value: &str| PathBuf::from(rules::expand_path(value, &home, &env.local_user, &host, &user));
    let mut identities: Vec<PathBuf> = settings.identity_files.iter().map(|file| expand(file)).collect();
    if identities.is_empty() {
        // ssh가 기본으로 찾는 키 파일(있는 것만 쓴다).
        identities.extend(["id_rsa", "id_ecdsa", "id_ed25519"].iter().map(|name| env.ssh_dir().join(name)));
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    identities.retain(|path| {
        let fresh = !seen.contains(path);
        seen.push(path.clone());
        fresh
    });
    let (known, learn) = if settings.user_known_hosts.iter().any(|file| file.eq_ignore_ascii_case("none")) {
        (Vec::new(), None)
    } else if settings.user_known_hosts.is_empty() {
        let file = env.ssh_dir().join("known_hosts");
        (vec![file.clone(), env.ssh_dir().join("known_hosts2")], Some(file))
    } else {
        let files: Vec<PathBuf> = settings.user_known_hosts.iter().map(|file| expand(file)).collect();
        let first = files.first().cloned();
        (files, first)
    };
    let mut known = known;
    known.extend(env.global_known_hosts.clone());
    Plan {
        host,
        port,
        user,
        identities,
        identities_only: settings.identities_only.unwrap_or(false),
        known,
        learn,
        strict: settings.strict_host_key_checking.as_deref() == Some("yes"),
        hash: settings.hash_known_hosts.unwrap_or(false),
        timeout: Duration::from_secs(settings.connect_timeout.unwrap_or(15).clamp(1, 300)),
    }
}

/// 접속해 본 결과.
enum Login {
    Done(Arc<Conn>),
    /// 인증하지 못했다(답이 틀림, 묻는 중에 Ctrl+C, 인증하는 동안 끊김). 사용자 이름을 물었으면 처음부터 다시 묻는다.
    Retry,
    /// 접속 자체가 안 됐거나(주소·포트, 서버 키) 세션을 닫았다. 까닭은 화면에 적었다.
    Stop,
}

/// 접속하고 인증한다. 실패하면 까닭을 화면에 적는다.
async fn connect(io: &mut Io, plan: &Plan, env: &SshEnv) -> Login {
    let label = format!("{}@{}", plan.user, host_port(&plan.host, plan.port));
    io.note(&format!("[내장 SSH로 {label}에 접속하는 중…]"));

    // TCP 연결에만 시간 제한을 둔다(그 뒤의 물음에는 사용자가 답하는 시간이 든다).
    let address = (plan.host.clone(), plan.port);
    let stream = tokio::select! {
        result = tokio::time::timeout(plan.timeout, tokio::net::TcpStream::connect(address)) => match result {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
                io.fail(&format!("{}에 연결하지 못했습니다: {err}", host_port(&plan.host, plan.port)));
                return Login::Stop;
            }
            Err(_) => {
                io.fail(&format!("{}에 {}초 동안 연결되지 않았습니다(주소·포트·방화벽을 확인하세요)", host_port(&plan.host, plan.port), plan.timeout.as_secs()));
                return Login::Stop;
            }
        },
        _ = io.killed.changed() => return Login::Stop,
    };
    let _ = stream.set_nodelay(true);

    let (asks, mut questions) = mpsc::unbounded_channel();
    let handler = Client {
        host: plan.host.clone(),
        port: plan.port,
        known: plan.known.clone(),
        learn: plan.learn.clone(),
        strict: plan.strict,
        hash: plan.hash,
        asks,
    };
    let mut config = client::Config {
        client_id: russh::SshId::Standard(format!("SSH-2.0-Cypher_{}", env!("CARGO_PKG_VERSION")).into()),
        nodelay: true,
        ..Default::default()
    };
    config.preferred.key = key_order(&plan.known_algorithms()).into();
    if env.keepalive > 0 {
        config.keepalive_interval = Some(Duration::from_secs(env.keepalive));
        config.keepalive_max = 3;
    }
    let handshake = client::connect_stream(Arc::new(config), stream, handler);
    tokio::pin!(handshake);
    // 키 교환이 끝날 때까지, 연결 쪽이 묻는 것(처음 보는 서버의 키)에 답한다.
    let mut handle = loop {
        tokio::select! {
            result = &mut handshake => match result {
                Ok(handle) => break handle,
                Err(err) => {
                    // 서버 키를 받아들이지 않은 것은 이미 까닭을 적었다.
                    while let Ok(Ask::Notice(text)) = questions.try_recv() {
                        io.say(&format!("{text}\n"));
                    }
                    if !matches!(err, russh::Error::UnknownKey) {
                        io.fail(&format!("SSH 연결을 맺지 못했습니다: {err}"));
                    }
                    return Login::Stop;
                }
            },
            Some(question) = questions.recv() => match question {
                Ask::Notice(text) => io.say(&format!("{text}\n")),
                Ask::YesNo(text, reply) => {
                    let mut answer = io.ask(&text, true).await;
                    loop {
                        match answer.as_deref().map(|a| a.trim().to_lowercase()) {
                            None => {
                                let _ = reply.send(false);
                                return Login::Stop;
                            }
                            Some(a) if matches!(a.as_str(), "yes" | "y" | "예" | "네") => {
                                let _ = reply.send(true);
                                break;
                            }
                            Some(a) if matches!(a.as_str(), "no" | "n" | "아니오" | "아니요") => {
                                let _ = reply.send(false);
                                io.fail("접속하지 않습니다.");
                                return Login::Stop;
                            }
                            Some(_) => answer = io.ask("yes 또는 no를 치세요: ", true).await,
                        }
                    }
                }
            },
            _ = io.killed.changed() => return Login::Stop,
        }
    };
    // 배너는 인증하는 동안에 올 수 있다: 인증하면서도 받아 보여 준다.
    let authed = authenticate(io, &mut handle, plan, env, &mut questions).await;
    while let Ok(Ask::Notice(text)) = questions.try_recv() {
        io.say(&format!("{text}\n"));
    }
    if !authed {
        let _ = handle.disconnect(Disconnect::ByApplication, "", "").await;
        return Login::Retry;
    }
    Login::Done(Arc::new(Conn { handle, users: AtomicUsize::new(1), label, host: plan.host.clone(), port: plan.port, user: plan.user.clone() }))
}

// ───────────────────────── 인증 ─────────────────────────

enum Outcome {
    Done,
    /// 실패했다. 서버가 이어서 받는 방법들.
    More(Vec<MethodKind>),
    /// 받아들였지만 서버가 인증을 더 요구한다(2단계). 이어서 해야 하는 방법들.
    Partial(Vec<MethodKind>),
    /// 해 볼 것이 없었다(키가 없다, 서버가 묻지도 않고 거절했다).
    Skipped,
    /// 사용자가 그만뒀거나 연결이 끊겼다.
    Abort,
}

fn methods_of(set: &russh::MethodSet) -> Vec<MethodKind> {
    set.iter().cloned().collect()
}

fn method_names(methods: &[MethodKind]) -> String {
    methods.iter().map(<&str>::from).collect::<Vec<_>>().join(", ")
}

/// 서버가 받는 방법을 차례로 해 본다: 공개 키(에이전트, 키 파일) → 키보드 대화식(2단계 인증) → 비밀번호.
async fn authenticate(io: &mut Io, handle: &mut client::Handle<Client>, plan: &Plan, env: &SshEnv, questions: &mut mpsc::UnboundedReceiver<Ask>) -> bool {
    let first = handle.authenticate_none(plan.user.clone()).await;
    let mut methods = match first {
        Ok(result) if result.success() => return true,
        Ok(russh::client::AuthResult::Failure { remaining_methods, .. }) => methods_of(&remaining_methods),
        Ok(_) => Vec::new(),
        Err(err) => {
            io.fail(&format!("인증을 시작하지 못했습니다: {err}"));
            return false;
        }
    };
    // 한 번 받는다고 한 방법은 실패한 뒤에도 해 본다(서버에 따라 실패한 방법을 다음 목록에서 빼기도 하는데, 비밀번호를 한 번 틀렸다고
    // 다시 물을 기회가 없으면 안 된다). 부분 성공(2단계 인증)이면 서버가 새로 준 목록으로 처음부터 센다.
    let (mut keys_done, mut kbd_left, mut password_left) = (false, 3u32, 3u32);
    loop {
        while let Ok(Ask::Notice(text)) = questions.try_recv() {
            io.say(&format!("{text}\n"));
        }
        if io.is_killed() {
            return false;
        }
        let tried_kbd = methods.contains(&MethodKind::KeyboardInteractive) && kbd_left > 0 && (keys_done || !methods.contains(&MethodKind::PublicKey));
        let outcome = if methods.contains(&MethodKind::PublicKey) && !keys_done {
            keys_done = true;
            public_keys(io, handle, plan, env).await
        } else if tried_kbd {
            kbd_left -= 1;
            keyboard(io, handle, plan).await
        } else if methods.contains(&MethodKind::Password) && password_left > 0 {
            password_left -= 1;
            password(io, handle, plan).await
        } else {
            let known = if methods.is_empty() { "없음".to_string() } else { method_names(&methods) };
            io.fail(&format!("{}@{}에 인증하지 못했습니다(서버가 받는 방법: {known}).", plan.user, plan.host));
            return false;
        };
        match outcome {
            Outcome::Done => return true,
            Outcome::More(next) => {
                for method in next {
                    if !methods.contains(&method) {
                        methods.push(method);
                    }
                }
            }
            Outcome::Partial(next) => {
                methods = next;
                (keys_done, kbd_left, password_left) = (false, 3, 3);
            }
            // 서버가 묻지도 않고 거절한 키보드 대화식은 다시 하지 않는다.
            Outcome::Skipped if tried_kbd => kbd_left = 0,
            Outcome::Skipped => {}
            Outcome::Abort => return false,
        }
    }
}

/// 공개 키: 에이전트의 키와 키 파일을 차례로 보인다.
async fn public_keys(io: &mut Io, handle: &mut client::Handle<Client>, plan: &Plan, env: &SshEnv) -> Outcome {
    let mut tried_any = false;
    let rsa_hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
    // 키 파일의 공개 키(`.pub`)들: IdentitiesOnly면 에이전트의 키 가운데 이것만 쓴다.
    let wanted: Vec<PublicKey> = plan
        .identities
        .iter()
        .filter_map(|path| russh::keys::load_public_key(PathBuf::from(format!("{}.pub", path.display()))).ok())
        .collect();
    // 이미 보인 키(OpenSSH 에이전트와 Pageant에 같은 키가 있으면 한 번만 보인다: 서버의 시도 횟수 한도에 걸리지 않게).
    let mut shown: Vec<PublicKey> = Vec::new();
    if env.agent {
        for (agent_name, identities) in agent_identities().await {
            for identity in identities {
                let key = identity.public_key().into_owned();
                if plan.identities_only && !wanted.iter().any(|w| w.key_data() == key.key_data()) {
                    continue;
                }
                if shown.iter().any(|seen| seen.key_data() == key.key_data()) {
                    continue;
                }
                let hash = if key.algorithm().is_rsa() { rsa_hash } else { None };
                // 서명하지 못했으면(에이전트가 거절함, 하드웨어 키가 빠짐) 서버에 보인 것이 아니다: 같은 키의 키 파일은 그대로 해 본다.
                let result = match agent_sign(agent_name, handle, &plan.user, key.clone(), hash).await {
                    Some(result) => result,
                    None => continue,
                };
                shown.push(key.clone());
                tried_any = true;
                match result {
                    russh::client::AuthResult::Success => {
                        io.note(&format!("[에이전트의 키로 인증했습니다: {}]", identity.comment()));
                        return Outcome::Done;
                    }
                    russh::client::AuthResult::Failure { remaining_methods, partial_success: true } => {
                        io.note("[공개 키는 받아들였고, 서버가 인증을 더 요구합니다]");
                        return Outcome::Partial(methods_of(&remaining_methods));
                    }
                    russh::client::AuthResult::Failure { .. } => {}
                }
            }
        }
    }
    let mut last_methods: Option<Vec<MethodKind>> = None;
    for path in &plan.identities {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        // 에이전트가 이미 보인 키면 암호를 묻지 않고 건너뛴다.
        let public = russh::keys::load_public_key(PathBuf::from(format!("{}.pub", path.display()))).ok();
        if public.is_some_and(|public| shown.iter().any(|seen| seen.key_data() == public.key_data())) {
            continue;
        }
        let mut key = russh::keys::decode_secret_key(&text, None);
        if matches!(key, Err(russh::keys::Error::KeyIsEncrypted)) {
            key = Err(russh::keys::Error::KeyIsEncrypted);
            for attempt in 0..3 {
                let prompt = if attempt == 0 { format!("키 파일 {}의 암호(비우면 건너뜀): ", path.display()) } else { "암호가 맞지 않습니다. 다시: ".into() };
                let Some(phrase) = io.ask(&prompt, false).await else { return Outcome::Abort };
                if phrase.is_empty() {
                    break;
                }
                key = russh::keys::decode_secret_key(&text, Some(&phrase));
                if key.is_ok() {
                    break;
                }
            }
        }
        let key = match key {
            Ok(key) => key,
            Err(russh::keys::Error::KeyIsEncrypted) => continue,
            Err(err) => {
                io.note(&format!("[키 파일을 읽지 못해 건너뜁니다({}): {err}]", path.display()));
                continue;
            }
        };
        let public = key.public_key().clone();
        if shown.iter().any(|seen| seen.key_data() == public.key_data()) {
            continue;
        }
        shown.push(public);
        tried_any = true;
        let hash = if key.algorithm().is_rsa() { rsa_hash } else { None };
        match handle.authenticate_publickey(plan.user.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await {
            Ok(russh::client::AuthResult::Success) => {
                io.note(&format!("[키 파일로 인증했습니다: {}]", path.display()));
                return Outcome::Done;
            }
            Ok(russh::client::AuthResult::Failure { remaining_methods, partial_success }) => {
                if partial_success {
                    io.note("[공개 키는 받아들였고, 서버가 인증을 더 요구합니다]");
                    return Outcome::Partial(methods_of(&remaining_methods));
                }
                last_methods = Some(methods_of(&remaining_methods));
            }
            Err(err) => {
                io.fail(&format!("공개 키 인증 중에 연결이 끊겼습니다: {err}"));
                return Outcome::Abort;
            }
        }
    }
    match last_methods {
        Some(methods) if tried_any => Outcome::More(methods),
        _ => Outcome::Skipped,
    }
}

/// 이 컴퓨터의 SSH 에이전트들(Windows OpenSSH 에이전트, Pageant)과 그 키들. 쓸 수 없는 에이전트는 뺀다.
async fn agent_identities() -> Vec<(&'static str, Vec<russh::keys::agent::AgentIdentity>)> {
    let mut found = Vec::new();
    if let Ok(Ok(mut agent)) = tokio::time::timeout(Duration::from_secs(2), russh::keys::agent::client::AgentClient::connect_named_pipe(OPENSSH_AGENT)).await {
        if let Ok(Ok(keys)) = tokio::time::timeout(Duration::from_secs(3), agent.request_identities()).await {
            if !keys.is_empty() {
                found.push(("openssh", keys));
            }
        }
    }
    if let Ok(Ok(mut agent)) = tokio::time::timeout(Duration::from_secs(2), russh::keys::agent::client::AgentClient::connect_pageant()).await {
        if let Ok(Ok(keys)) = tokio::time::timeout(Duration::from_secs(3), agent.request_identities()).await {
            if !keys.is_empty() {
                found.push(("pageant", keys));
            }
        }
    }
    found
}

const OPENSSH_AGENT: &str = r"\\.\pipe\openssh-ssh-agent";

/// 에이전트에 서명을 맡겨 공개 키로 인증한다. 에이전트에 다시 붙지 못하면 None.
async fn agent_sign(agent: &str, handle: &mut client::Handle<Client>, user: &str, key: PublicKey, hash: Option<HashAlg>) -> Option<russh::client::AuthResult> {
    use russh::keys::agent::client::AgentClient;
    let result = if agent == "openssh" {
        let mut client = AgentClient::connect_named_pipe(OPENSSH_AGENT).await.ok()?;
        handle.authenticate_publickey_with(user.to_string(), key, hash, &mut client).await
    } else {
        let mut client = AgentClient::connect_pageant().await.ok()?;
        handle.authenticate_publickey_with(user.to_string(), key, hash, &mut client).await
    };
    result.ok()
}

/// 키보드 대화식: 서버가 묻는 것(비밀번호, 일회용 번호)을 화면에 묻고 답을 보낸다.
async fn keyboard(io: &mut Io, handle: &mut client::Handle<Client>, plan: &Plan) -> Outcome {
    let mut reply = match handle.authenticate_keyboard_interactive_start(plan.user.clone(), None).await {
        Ok(reply) => reply,
        Err(err) => {
            io.fail(&format!("인증 중에 연결이 끊겼습니다: {err}"));
            return Outcome::Abort;
        }
    };
    let mut asked = false;
    for _ in 0..16 {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Outcome::Done,
            KeyboardInteractiveAuthResponse::Failure { remaining_methods, partial_success } => {
                let next = methods_of(&remaining_methods);
                return if partial_success {
                    Outcome::Partial(next)
                } else if asked {
                    io.fail("인증하지 못했습니다(답이 맞지 않습니다).");
                    Outcome::More(next)
                } else {
                    // 묻지도 않고 거절했다(이 방법을 쓰지 않는 서버).
                    Outcome::Skipped
                };
            }
            KeyboardInteractiveAuthResponse::InfoRequest { name, instructions, prompts } => {
                asked = true;
                for line in [name.trim(), instructions.trim()].into_iter().filter(|text| !text.is_empty()) {
                    io.say(&format!("{line}\n"));
                }
                let mut answers = Vec::new();
                for prompt in &prompts {
                    let shown = if prompt.prompt.trim().is_empty() { "응답: ".to_string() } else { prompt.prompt.clone() };
                    // 서버가 그냥 "Password:"라고 물으면 어느 계정의 비밀번호인지 알 수 있게 바꿔 보인다.
                    let shown = if !prompt.echo && shown.trim().eq_ignore_ascii_case("password:") {
                        format!("{}@{}의 비밀번호: ", plan.user, plan.host)
                    } else {
                        shown
                    };
                    let Some(answer) = io.ask(&shown, prompt.echo).await else { return Outcome::Abort };
                    answers.push(answer);
                }
                reply = match handle.authenticate_keyboard_interactive_respond(answers).await {
                    Ok(reply) => reply,
                    Err(err) => {
                        io.fail(&format!("인증 중에 연결이 끊겼습니다: {err}"));
                        return Outcome::Abort;
                    }
                };
            }
        }
    }
    Outcome::Abort
}

async fn password(io: &mut Io, handle: &mut client::Handle<Client>, plan: &Plan) -> Outcome {
    let Some(secret) = io.ask(&format!("{}@{}의 비밀번호: ", plan.user, plan.host), false).await else { return Outcome::Abort };
    match handle.authenticate_password(plan.user.clone(), secret).await {
        Ok(russh::client::AuthResult::Success) => Outcome::Done,
        Ok(russh::client::AuthResult::Failure { remaining_methods, partial_success }) => {
            if partial_success {
                return Outcome::Partial(methods_of(&remaining_methods));
            }
            io.fail("비밀번호가 맞지 않습니다.");
            Outcome::More(methods_of(&remaining_methods))
        }
        Err(err) => {
            io.fail(&format!("인증 중에 연결이 끊겼습니다: {err}"));
            Outcome::Abort
        }
    }
}

// ───────────────────────── 셸 ─────────────────────────

/// 연결에 채널을 열어 터미널과 셸을 요청하고, 끝날 때까지 화면과 잇는다.
async fn shell(io: &mut Io, conn: &Arc<Conn>, mut paused: watch::Receiver<bool>) -> Option<u32> {
    // 채널 열기와 요청은 시간 제한을 두고, 그동안 세션을 닫으면 바로 그만둔다(답하지 않는 연결에서 멈춰 있지 않게).
    let late = || format!("서버가 {}초 동안 세션을 열어 주지 않았습니다", OPEN_TIMEOUT.as_secs());
    let channel = tokio::select! {
        result = tokio::time::timeout(OPEN_TIMEOUT, conn.handle.channel_open_session()) => match result {
            Ok(Ok(channel)) => channel,
            Ok(Err(err)) => {
                io.fail(&format!("세션을 열지 못했습니다: {err}"));
                return Some(255);
            }
            Err(_) => {
                io.fail(&late());
                return Some(255);
            }
        },
        _ = io.killed.changed() => return None,
    };
    let (mut read, write) = channel.split();
    // 접속·인증하는 동안 화면 크기가 바뀌었으면 바뀐 크기로 요청한다.
    let (cols, rows) = io.size;
    let modes = [(Pty::VERASE, 127), (Pty::IUTF8, 1)];
    let requested = tokio::select! {
        result = tokio::time::timeout(OPEN_TIMEOUT, async {
            write.request_pty(true, "xterm-256color", u32::from(cols), u32::from(rows), 0, 0, &modes).await.map_err(|err| format!("터미널을 요청하지 못했습니다: {err}"))?;
            write.request_shell(true).await.map_err(|err| format!("셸을 요청하지 못했습니다: {err}"))
        }) => Some(result.unwrap_or_else(|_| Err(late()))),
        _ = io.killed.changed() => None,
    };
    if !matches!(requested, Some(Ok(()))) {
        // 연 채널은 닫는다: 버리기만 하면 같은 연결을 쓰는 다른 세션이 있는 동안 서버에 채널이 남는다.
        let _ = tokio::time::timeout(Duration::from_secs(2), write.close()).await;
        if let Some(Err(err)) = requested {
            io.fail(&err);
            return Some(255);
        }
        return None;
    }
    if let Some(ready) = io.ready.take() {
        ready();
    }

    // 입력은 따로 보낸다: 서버가 입력을 받지 않아 보내기가 기다리는 동안에도 출력은 계속 읽어야 서로 기다리지 않는다.
    let write = Arc::new(write);
    let writer = write.clone();
    let mut inputs = std::mem::replace(&mut io.input, mpsc::unbounded_channel().1);
    let sender = tauri::async_runtime::spawn(async move {
        while let Some(input) = inputs.recv().await {
            let sent = match input {
                Input::Data(data) => writer.data_bytes(data).await,
                Input::Resize(cols, rows) => writer.window_change(u32::from(cols), u32::from(rows), 0, 0).await,
            };
            if sent.is_err() {
                break;
            }
        }
    });

    let mut code: Option<u32> = None;
    let mut replies = 0u8; // pty·셸 요청의 답(성공/실패)을 센다
    // 화면이 멈춘 동안(출력을 따라가지 못함) 받은 출력. 이 동안에도 채널을 비워야 같은 연결을 쓰는 다른 세션이 멈추지 않는다.
    let mut held: Vec<u8> = Vec::new();
    loop {
        let stopped = *paused.borrow();
        if !stopped && !held.is_empty() {
            // 다시 받기 시작했다: 모아 둔 것을 조각으로 보낸다(그사이 다시 멈추면 나머지는 그대로 둔다).
            let chunk: Vec<u8> = held.drain(..held.len().min(64 * 1024)).collect();
            (io.output)(&chunk);
            tokio::task::yield_now().await;
            continue;
        }
        let reading = !stopped || held.len() < HOLD_LIMIT;
        tokio::select! {
            message = read.wait(), if reading => match message {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if stopped || !held.is_empty() { held.extend_from_slice(&data) } else { (io.output)(&data) }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status),
                Some(ChannelMsg::ExitSignal { signal_name, error_message, .. }) => {
                    code = Some(255);
                    io.note(&format!("[원격 프로그램이 신호로 끝났습니다: {signal_name:?} {error_message}]"));
                }
                Some(ChannelMsg::Success) => replies += 1,
                Some(ChannelMsg::Failure) => {
                    replies += 1;
                    io.fail(if replies == 1 { "서버가 터미널(PTY) 요청을 거절했습니다." } else { "서버가 셸 요청을 거절했습니다." });
                }
                Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
            Ok(()) = paused.changed() => {}
            Ok(()) = io.killed.changed() => {
                let _ = write.close().await;
                break;
            }
        }
    }
    sender.abort();
    if !held.is_empty() {
        (io.output)(&held);
    }
    if code.is_none() && conn.handle.is_closed() {
        io.note("\n[서버와의 연결이 끊겼습니다]");
    }
    code
}

// ───────────────────────── 시험 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use russh::server::{self, Auth, Msg, Session};
    use russh::{Channel, ChannelId};

    /// 시험용 서버: 비밀번호 `secret`(사용자 `tester`)을 받고, 셸은 받은 글을 `echo:`를 붙여 돌려준다.
    /// `exit`를 받으면 종료 코드 7로 끝낸다. 크기가 바뀌면 `size:가로x세로`를 보낸다.
    #[derive(Clone)]
    struct Server {
        auths: Arc<AtomicUsize>,
        /// 참이면 비밀번호와 일회용 번호를 키보드 대화식으로 묻는다(PAM을 쓰는 리눅스 서버처럼).
        keyboard: bool,
    }

    /// OpenSSH처럼 틀려도 같은 방법을 다시 받는다.
    fn again() -> Auth {
        Auth::Reject { proceed_with_methods: Some(russh::MethodSet::from(&[MethodKind::Password, MethodKind::KeyboardInteractive][..])), partial_success: false }
    }

    impl server::Server for Server {
        type Handler = Self;
        fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
            self.clone()
        }
    }

    impl server::Handler for Server {
        type Error = russh::Error;

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
            if !self.keyboard && user == "tester" && password == "secret" {
                self.auths.fetch_add(1, Ordering::SeqCst);
                Ok(Auth::Accept)
            } else {
                Ok(again())
            }
        }

        async fn auth_keyboard_interactive<'a>(&'a mut self, user: &str, _: &str, response: Option<server::Response<'a>>) -> Result<Auth, Self::Error> {
            if !self.keyboard {
                return Ok(again());
            }
            match response {
                None => Ok(Auth::Partial {
                    name: "".into(),
                    instructions: "".into(),
                    prompts: vec![("Password: ".into(), false), ("일회용 번호: ".into(), true)].into(),
                }),
                Some(answers) => {
                    let answers: Vec<String> = answers.map(|a| String::from_utf8_lossy(&a).into_owned()).collect();
                    if user == "tester" && answers == ["secret", "123456"] {
                        self.auths.fetch_add(1, Ordering::SeqCst);
                        Ok(Auth::Accept)
                    } else {
                        Ok(again())
                    }
                }
            }
        }

        async fn channel_open_session(&mut self, _channel: Channel<Msg>, reply: server::ChannelOpenHandle, _session: &mut Session) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        #[allow(clippy::too_many_arguments)]
        async fn pty_request(&mut self, channel: ChannelId, term: &str, cols: u32, rows: u32, _: u32, _: u32, _: &[(Pty, u32)], session: &mut Session) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            session.data(channel, format!("pty:{term}:{cols}x{rows}\r\n").into_bytes())?;
            Ok(())
        }

        async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            session.data(channel, b"welcome\r\n".to_vec())?;
            Ok(())
        }

        async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
            if data == b"flood\r" {
                // 큰 출력(약 3MB)을 쏟아 낸 뒤 끝 표시를 보낸다.
                for _ in 0..200 {
                    session.data(channel, vec![b'x'; 16 * 1024])?;
                }
                session.data(channel, b"\r\nflood-end\r\n".to_vec())?;
                return Ok(());
            }
            if data == b"exit\r" {
                session.exit_status_request(channel, 7)?;
                session.eof(channel)?;
                session.close(channel)?;
                return Ok(());
            }
            session.data(channel, [b"echo:".as_slice(), data, b"\r\n"].concat())?;
            Ok(())
        }

        async fn window_change_request(&mut self, channel: ChannelId, cols: u32, rows: u32, _: u32, _: u32, session: &mut Session) -> Result<(), Self::Error> {
            session.data(channel, format!("size:{cols}x{rows}\r\n").into_bytes())?;
            Ok(())
        }
    }

    /// 시험 서버를 띄운다. (포트, 서버의 공개 키, 인증한 횟수)
    async fn serve() -> (u16, PublicKey, Arc<AtomicUsize>) {
        serve_with(false).await
    }

    async fn serve_with(keyboard: bool) -> (u16, PublicKey, Arc<AtomicUsize>) {
        use russh::keys::ssh_key::private::Ed25519Keypair;
        let key = russh::keys::PrivateKey::from(Ed25519Keypair::from_seed(&[9u8; 32]));
        let public = key.public_key().clone();
        let config = Arc::new(server::Config {
            keys: vec![key],
            auth_rejection_time: Duration::from_millis(10),
            auth_rejection_time_initial: Some(Duration::from_millis(0)),
            ..Default::default()
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let auths = Arc::new(AtomicUsize::new(0));
        let mut server = Server { auths: auths.clone(), keyboard };
        tauri::async_runtime::spawn(async move {
            use russh::server::Server as _;
            let _ = server.run_on_socket(config, &listener).await;
        });
        (port, public, auths)
    }

    /// 세션 하나를 띄우고 화면 출력을 모은다.
    struct Probe {
        term: SshTerm,
        seen: Arc<Mutex<String>>,
        exit: Arc<Mutex<Option<Option<u32>>>>,
        ready: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Probe {
        /// 사용자 이름을 적어 준 접속(`tester@127.0.0.1`): 사용자 이름을 묻지 않는다.
        fn start(port: u16, env: &SshEnv, share: Option<Arc<Conn>>) -> Probe {
            Probe::open("tester@127.0.0.1", port, env, share)
        }

        fn open(target: &str, port: u16, env: &SshEnv, share: Option<Arc<Conn>>) -> Probe {
            let seen = Arc::new(Mutex::new(String::new()));
            let exit = Arc::new(Mutex::new(None));
            let (out, done) = (seen.clone(), exit.clone());
            let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let opened = ready.clone();
            let spec = SshTarget { target: target.into(), port: Some(port) };
            let term = SshTerm::start(
                spec,
                env.clone(),
                share,
                (80, 24),
                move |bytes| out.lock().unwrap().push_str(&String::from_utf8_lossy(bytes)),
                move || opened.store(true, Ordering::SeqCst),
                move |code| *done.lock().unwrap() = Some(code),
            );
            Probe { term, seen, exit, ready }
        }

        /// 화면에 그 글이 나올 때까지 기다린다.
        async fn wait_for(&self, text: &str) {
            for _ in 0..500 {
                if self.seen.lock().unwrap().contains(text) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("화면에 {text:?}가 나오지 않았습니다. 지금 화면:\n{}", self.seen.lock().unwrap());
        }

        /// 화면에 그 글이 `times`번 나올 때까지 기다린다(같은 물음을 다시 묻는 것을 볼 때).
        async fn wait_count(&self, text: &str, times: usize) {
            for _ in 0..500 {
                if self.seen.lock().unwrap().matches(text).count() >= times {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("화면에 {text:?}가 {times}번 나오지 않았습니다. 지금 화면:\n{}", self.seen.lock().unwrap());
        }

        fn saw(&self, text: &str) -> bool {
            self.seen.lock().unwrap().contains(text)
        }

        async fn wait_exit(&self) -> Option<u32> {
            for _ in 0..500 {
                if let Some(code) = *self.exit.lock().unwrap() {
                    return code;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("세션이 끝나지 않았습니다. 지금 화면:\n{}", self.seen.lock().unwrap());
        }
    }

    fn test_env(name: &str) -> SshEnv {
        // 사용자의 ~/.ssh를 건드리지 않게 빌드 폴더 아래에 시험용 홈을 둔다(키 파일은 두지 않는다).
        let home = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join(format!("sshc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        SshEnv { home, local_user: "nobody".into(), keepalive: 0, agent: false, global_known_hosts: None }
    }

    #[test]
    fn 처음_보는_서버는_물어본_뒤_적고_비밀번호로_접속한다() {
        tauri::async_runtime::block_on(async {
            let (port, public, auths) = serve().await;
            let env = test_env("first");
            let probe = Probe::start(port, &env, None);
            probe.wait_for("(yes/no)").await;
            // 지문을 보여 준다.
            let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
            probe.wait_for(&fingerprint).await;
            // 엉뚱한 답은 다시 묻는다. 한글은 지워 가며 고칠 수 있다.
            probe.term.write("몰라\r".as_bytes());
            probe.wait_for("yes 또는 no").await;
            probe.term.write(b"yes\r");
            probe.wait_for("비밀번호: ").await;
            let known = std::fs::read_to_string(env.home.join(".ssh").join("known_hosts")).unwrap();
            assert!(known.starts_with(&format!("[127.0.0.1]:{port} ssh-ed25519 ")), "{known}");
            // 틀린 비밀번호는 다시 묻고, 친 글은 화면에 보이지 않는다.
            probe.term.write(b"wrong\r");
            probe.wait_for("비밀번호가 맞지 않습니다").await;
            assert!(!probe.ready.load(Ordering::SeqCst), "로그인 전에는 셸이 열렸다고 알리지 않는다");
            probe.term.write(b"sec");
            probe.term.write(b"ret\r");
            probe.wait_for("pty:xterm-256color:80x24").await;
            probe.wait_for("welcome").await;
            assert!(probe.ready.load(Ordering::SeqCst));
            assert!(!probe.seen.lock().unwrap().contains("secret") && !probe.seen.lock().unwrap().contains("wrong"));
            // 입력과 크기 바꾸기
            probe.term.write(b"ls\r");
            probe.wait_for("echo:ls").await;
            probe.term.resize(120, 40);
            probe.wait_for("size:120x40").await;
            assert_eq!(auths.load(Ordering::SeqCst), 1);

            // 복제: 같은 연결에 채널을 하나 더 연다(다시 묻지 않고, 서버에 다시 인증하지 않는다).
            let conn = probe.term.connection().expect("인증을 마친 연결");
            let clone = Probe::start(port, &env, Some(conn.clone()));
            clone.wait_for("welcome").await;
            assert!(!clone.seen.lock().unwrap().contains("비밀번호"));
            assert_eq!(auths.load(Ordering::SeqCst), 1);
            clone.term.write(b"pwd\r");
            clone.wait_for("echo:pwd").await;
            assert!(!probe.seen.lock().unwrap().contains("echo:pwd"));

            // 원래 세션이 끝나도 복제한 세션은 그대로 돈다. 마지막 세션이 끝나면 연결을 닫는다.
            probe.term.write(b"exit\r");
            assert_eq!(probe.wait_exit().await, Some(7));
            clone.term.write(b"still\r");
            clone.wait_for("echo:still").await;
            assert!(conn.alive());
            clone.term.kill();
            clone.wait_exit().await;
            assert!(!conn.alive());

            // 두 번째 접속은 known_hosts에 있으니 묻지 않는다. 닫힌 연결은 나눠 쓰지 않고 새로 인증한다.
            let again = Probe::start(port, &env, Some(conn));
            again.wait_for("비밀번호: ").await;
            assert!(!again.seen.lock().unwrap().contains("(yes/no)"));
            again.term.kill();
            again.wait_exit().await;
            let _ = std::fs::remove_dir_all(&env.home);
        });
    }

    #[test]
    fn 주소만_주면_사용자_이름부터_묻고_같은_사용자의_연결은_같이_쓴다() {
        tauri::async_runtime::block_on(async {
            let (port, public, auths) = serve().await;
            let env = test_env("ask-user");
            let blob = public.to_openssh().unwrap().split_whitespace().nth(1).unwrap().to_string();
            std::fs::write(env.home.join(".ssh").join("known_hosts"), format!("{}\n", rules::known_host_line("127.0.0.1", port, "ssh-ed25519", &blob))).unwrap();

            // 사용자 이름을 묻는다(친 글이 보인다). 비워 두면 다시 묻는다. 이 컴퓨터의 사용자 이름(nobody)으로 넘어가지 않는다.
            let probe = Probe::open("127.0.0.1", port, &env, None);
            probe.wait_for(&format!("127.0.0.1:{port}에 접속합니다")).await;
            probe.wait_for("사용자 이름: ").await;
            probe.term.write(b"  \r");
            probe.wait_count("사용자 이름: ", 2).await;
            probe.term.write(b"tester\r");
            probe.wait_for("tester@127.0.0.1의 비밀번호: ").await;
            assert!(probe.saw("사용자 이름: tester") && !probe.saw("nobody"));
            probe.term.write(b"secret\r");
            probe.wait_for("welcome").await;
            assert_eq!(auths.load(Ordering::SeqCst), 1);

            // 같은 서버에 같은 사용자로 여는 새 세션은 사용자 이름만 받고 그 연결을 같이 쓴다(비밀번호를 묻지 않는다).
            let second = Probe::open("127.0.0.1", port, &env, None);
            second.wait_for("사용자 이름: ").await;
            second.term.write(b"tester\r");
            second.wait_for("연결을 같이 씁니다").await;
            second.wait_for("welcome").await;
            assert!(!second.saw("의 비밀번호: "));
            assert_eq!(auths.load(Ordering::SeqCst), 1);

            // 다른 사용자는 그 연결을 쓰지 않고 새로 접속한다. 비밀번호를 묻는 중에 Ctrl+C면 사용자 이름부터 다시 묻는다.
            let other = Probe::open("127.0.0.1", port, &env, None);
            other.wait_for("사용자 이름: ").await;
            other.term.write(b"other\r");
            other.wait_for("other@127.0.0.1의 비밀번호: ").await;
            assert!(!other.saw("연결을 같이 씁니다"));
            other.term.write(b"\x03");
            other.wait_for("사용자 이름부터 다시 묻습니다").await;
            other.wait_count("사용자 이름: ", 2).await;
            // 틀린 비밀번호를 세 번 넣어 인증하지 못해도 사용자 이름부터 다시 묻는다.
            other.term.write(b"other\r");
            for attempt in 2..=4 {
                other.wait_count("other@127.0.0.1의 비밀번호: ", attempt).await;
                other.term.write(b"wrong\r");
            }
            other.wait_for("인증하지 못했습니다").await;
            other.wait_count("사용자 이름: ", 3).await;
            assert!(other.exit.lock().unwrap().is_none(), "세션은 끝나지 않고 다시 묻는다");
            other.term.write(b"tester\r");
            other.wait_for("welcome").await;
            assert_eq!(auths.load(Ordering::SeqCst), 1);
            assert!(!other.saw("secret") && !other.saw("wrong"));

            // 사용자 이름을 묻는 데서 Ctrl+C면 그만둔다.
            let quit = Probe::open("127.0.0.1", port, &env, None);
            quit.wait_for("사용자 이름: ").await;
            quit.term.write(b"\x03");
            assert_eq!(quit.wait_exit().await, Some(255));

            // 연결 복제는 사용자 이름도 묻지 않는다.
            let clone = Probe::open("127.0.0.1", port, &env, probe.term.connection());
            clone.wait_for("welcome").await;
            assert!(!clone.saw("사용자 이름"));
            for p in [&probe, &second, &other, &clone] {
                p.term.kill();
                p.wait_exit().await;
            }
            let _ = std::fs::remove_dir_all(&env.home);
        });
    }

    #[test]
    fn 키보드_대화식으로_묻는_것에_답한다() {
        tauri::async_runtime::block_on(async {
            let (port, public, auths) = serve_with(true).await;
            let env = test_env("keyboard");
            // 이미 아는 서버(묻지 않는다)
            let blob = public.to_openssh().unwrap().split_whitespace().nth(1).unwrap().to_string();
            std::fs::write(env.home.join(".ssh").join("known_hosts"), format!("{}\n", rules::known_host_line("127.0.0.1", port, "ssh-ed25519", &blob))).unwrap();
            let count = |probe: &Probe, text: &str| probe.seen.lock().unwrap().matches(text).count();
            let probe = Probe::start(port, &env, None);
            probe.wait_for("tester@127.0.0.1의 비밀번호: ").await;
            assert!(count(&probe, "(yes/no)") == 0);
            probe.term.write(b"secret\r");
            probe.wait_for("일회용 번호: ").await;
            // 보이는 칸(echo)은 친 글이 보인다. 틀리면 다시 묻는다.
            probe.term.write(b"000000\r");
            probe.wait_for("답이 맞지 않습니다").await;
            assert!(count(&probe, "000000") == 1 && count(&probe, "secret") == 0);
            for _ in 0..300 {
                if count(&probe, "의 비밀번호: ") >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            probe.term.write(b"secret\r");
            for _ in 0..300 {
                if count(&probe, "일회용 번호: ") >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            probe.term.write(b"123456\r");
            probe.wait_for("welcome").await;
            assert_eq!(auths.load(Ordering::SeqCst), 1);
            probe.term.kill();
            probe.wait_exit().await;
            let _ = std::fs::remove_dir_all(&env.home);
        });
    }

    #[test]
    fn 서버_키가_바뀌었으면_접속하지_않는다() {
        tauri::async_runtime::block_on(async {
            let (port, _, auths) = serve().await;
            let env = test_env("changed");
            std::fs::write(env.home.join(".ssh").join("known_hosts"), format!("[127.0.0.1]:{port} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n")).unwrap();
            let probe = Probe::start(port, &env, None);
            probe.wait_for("서버 키가 바뀌었습니다").await;
            assert_eq!(probe.wait_exit().await, Some(255));
            assert!(!probe.seen.lock().unwrap().contains("비밀번호"));
            assert_eq!(auths.load(Ordering::SeqCst), 0);
            // no로 답하면 접속하지 않고, 적지도 않는다.
            let env = test_env("refuse");
            let probe = Probe::start(port, &env, None);
            probe.wait_for("(yes/no)").await;
            probe.term.write(b"no\r");
            assert_eq!(probe.wait_exit().await, Some(255));
            assert!(!env.home.join(".ssh").join("known_hosts").exists());
            // 묻는 동안 Ctrl+C로 그만둘 수 있다.
            let probe = Probe::start(port, &env, None);
            probe.wait_for("(yes/no)").await;
            probe.term.write(b"\x03");
            assert_eq!(probe.wait_exit().await, Some(255));
            let _ = std::fs::remove_dir_all(&env.home);
        });
    }

    #[test]
    fn 검토에서_나온_경우들() {
        tauri::async_runtime::block_on(async {
            let (port, public, auths) = serve().await;
            let env = test_env("review");
            let blob = public.to_openssh().unwrap().split_whitespace().nth(1).unwrap().to_string();
            let known = env.home.join(".ssh").join("known_hosts");
            std::fs::write(&known, format!("{}\n", rules::known_host_line("127.0.0.1", port, "ssh-ed25519", &blob))).unwrap();

            // 비밀번호를 묻는 동안 바꾼 창 크기로 터미널을 요청한다.
            let probe = Probe::start(port, &env, None);
            probe.wait_for("비밀번호: ").await;
            probe.term.resize(132, 50);
            probe.term.write(b"secret\r");
            probe.wait_for("pty:xterm-256color:132x50").await;
            probe.wait_for("welcome").await;

            // 한 세션이 멈춰 있어도(화면이 출력을 따라가지 못함) 같은 연결의 복제 세션은 멈추지 않는다.
            let clone = Probe::start(port, &env, probe.term.connection());
            clone.wait_for("welcome").await;
            let before = probe.seen.lock().unwrap().matches('x').count(); // "xterm"·"132x50"의 x
            probe.term.set_paused(true);
            probe.term.write(b"flood\r");
            tokio::time::sleep(Duration::from_millis(300)).await;
            clone.term.write(b"two\r");
            clone.wait_for("echo:two").await;
            assert!(!probe.seen.lock().unwrap().contains("flood-end"), "멈춘 동안에는 화면에 보내지 않는다");
            // 다시 받으면 모아 둔 것을 빠짐없이 보낸다.
            probe.term.set_paused(false);
            probe.wait_for("flood-end").await;
            assert_eq!(probe.seen.lock().unwrap().matches('x').count() - before, 200 * 16 * 1024);
            assert_eq!(auths.load(Ordering::SeqCst), 1);
            probe.term.kill();
            clone.term.kill();
            probe.wait_exit().await;
            clone.wait_exit().await;

            // 공용 known_hosts에서 막은 키는, 사용자 파일에 그 키가 있어도 접속하지 않는다.
            let global = env.home.join("ssh_known_hosts");
            std::fs::write(&global, format!("@revoked [127.0.0.1]:{port} ssh-ed25519 {blob}\n")).unwrap();
            let revoked_env = SshEnv { global_known_hosts: Some(global.clone()), ..env.clone() };
            let probe = Probe::start(port, &revoked_env, None);
            probe.wait_for("막아 둔 키").await;
            assert_eq!(probe.wait_exit().await, Some(255));
            assert!(!probe.seen.lock().unwrap().contains("비밀번호"));

            // HashKnownHosts yes면 호스트 이름 대신 해시로 적는다.
            std::fs::remove_file(&known).unwrap();
            std::fs::write(env.home.join(".ssh").join("config"), "HashKnownHosts yes\n").unwrap();
            let probe = Probe::start(port, &env, None);
            probe.wait_for("(yes/no)").await;
            probe.term.write(b"yes\r");
            probe.wait_for("비밀번호: ").await;
            let written = std::fs::read_to_string(&known).unwrap();
            assert!(written.starts_with("|1|") && !written.contains("127.0.0.1"), "{written}");
            assert_eq!(rules::check_known_host(&written, "127.0.0.1", port, "ssh-ed25519", &blob), HostKey::Known);
            probe.term.kill();
            probe.wait_exit().await;
            let _ = std::fs::remove_dir_all(&env.home);
        });
        // known_hosts에 적힌 종류의 호스트 키를 먼저 달라고 한다.
        let first = |known: &[&str]| key_order(&known.iter().map(|k| k.to_string()).collect::<Vec<_>>())[0].as_str().to_string();
        assert_eq!(first(&[]), "ssh-ed25519");
        assert_eq!(first(&["ecdsa-sha2-nistp256"]), "ecdsa-sha2-nistp256");
        assert!(first(&["ssh-rsa"]).starts_with("rsa-sha2-"));
        assert_eq!(key_order(&["ecdsa-sha2-nistp256".into()]).len(), russh::Preferred::default().key.len());
    }

    #[test]
    fn 설정_파일의_값으로_접속할_곳을_정한다() {
        let env = test_env("plan");
        std::fs::write(
            env.home.join(".ssh").join("config"),
            "Include conf.d/*.conf\nHost web\n  HostName 10.1.2.3\n  User deploy\n  Port 2200\n  IdentityFile ~/.ssh/web_key\nHost jump\n  ProxyJump gate\n",
        )
        .unwrap();
        std::fs::create_dir_all(env.home.join(".ssh").join("conf.d")).unwrap();
        std::fs::write(env.home.join(".ssh").join("conf.d").join("a.conf"), "Host db\n  HostName db.internal\n").unwrap();
        let spec = |target: &str| SshTarget { target: target.into(), ..Default::default() };
        // 사용자 이름: 적어 준 것, 설정 파일의 User 차례. 둘 다 없으면 모른다(세션 화면에서 묻는다 — 이 컴퓨터의 사용자 이름을 쓰지 않는다).
        assert_eq!(login_user(&spec("web"), &env).as_deref(), Some("deploy"));
        assert_eq!(login_user(&spec("root@web"), &env).as_deref(), Some("root"));
        assert_eq!(login_user(&spec("db"), &env), None);
        assert_eq!(login_user(&spec("10.0.0.9"), &env), None);
        let web = plan(&spec("web"), &env, "deploy");
        assert_eq!((web.host.as_str(), web.port, web.user.as_str()), ("10.1.2.3", 2200, "deploy"));
        assert_eq!(web.identities, [env.home.join(".ssh").join("web_key")]);
        assert_eq!(web.learn, Some(env.home.join(".ssh").join("known_hosts")));
        // 적어 준 포트가 설정 파일보다 먼저다.
        let given = plan(&SshTarget { target: "web".into(), port: Some(22) }, &env, "root");
        assert_eq!((given.user.as_str(), given.port), ("root", 22));
        // Include한 파일의 값, 기본 키 파일
        let db = plan(&spec("db"), &env, "물은사람");
        assert_eq!((db.host.as_str(), db.user.as_str()), ("db.internal", "물은사람"));
        assert_eq!(db.identities.len(), 3);
        // 경유 서버를 쓰는 호스트는 내장 SSH가 다루지 않는다.
        assert!(unsupported(&spec("jump"), &env).is_some_and(|why| why.contains("proxyjump")));
        assert!(unsupported(&spec("web"), &env).is_none());
        assert_eq!((host_port("h", 22), host_port("h", 2200), host_port("::1", 2200)), ("h".into(), "h:2200".into(), "[::1]:2200".into()));
        let _ = std::fs::remove_dir_all(&env.home);
    }
}
