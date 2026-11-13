//! 브라우저 엔진 "chrome": 이 컴퓨터에 설치된 Chrome(또는 Edge·Chromium·Brave)을 Cypher 전용 프로필로 띄우고,
//! 개발자 도구 프로토콜(CDP)을 **파이프**로 주고받아 조작한다.
//!
//! 내장 웹뷰(WebView2)는 Windows에만 있다. macOS의 웹뷰는 Chromium이 아니어서 같은 방법으로 조작할 수 없으므로,
//! 다른 OS로 옮길 때는 이 엔진이 브라우저 기능을 맡는다. 설치된 브라우저를 쓰므로 늘 최신 Chromium이고
//! (브라우저가 스스로 갱신한다) 배포 파일이 커지지 않는다.
//!
//! - 디버깅 포트를 열지 않는다. 이 프로세스와 브라우저만 아는 파이프로만 통신한다.
//! - 프로필은 Cypher 전용으로 따로 둔다(사용자의 평소 프로필은 건드리지 않는다. Chrome 136부터는 기본 프로필에서
//!   원격 조작이 막혀 있기도 하다). 브라우저는 그룹마다 따로 띄우고 프로필도 그룹마다 다르다
//!   (`%LOCALAPPDATA%\cypher\chrome-profiles\<그룹의 프로필 번호>`). 그래서 한 그룹의 Claude 세션은 다른 그룹의
//!   브라우저를 조작할 수 없고, 로그인 상태도 그룹끼리 섞이지 않는다. 그룹이 없는 곳에서 사용자가 띄운 브라우저는
//!   `chrome-profile`을 쓰며, Claude 세션은 그것을 조작할 수 없다.
//! - Cypher가 끝나 파이프가 닫히면 이 브라우저들도 함께 끝난다.
//!
//! 파이프를 넘기는 부분(`spawn`)만 OS에 따라 다르다. Windows는 상속할 핸들 번호를
//! `--remote-debugging-io-pipes=<읽기>,<쓰기>`로 알려 주고, macOS·Linux는 파일 번호 3·4로 넘긴다.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

type Pending = Arc<Mutex<HashMap<u64, Sender<Result<Value, String>>>>>;

/// 띄워 둔 브라우저 하나.
pub struct Chrome {
    /// 이 브라우저가 쓰는 프로필의 번호(그룹마다 다르다. 그룹이 없으면 빈 글).
    key: String,
    child: Mutex<Child>,
    writer: Mutex<File>,
    pending: Pending,
    next: AtomicU64,
    /// 브라우저가 살아 있는지. 끝난 것을 알아채면 거짓이 되고, 그 뒤의 호출은 기다리지 않고 실패한다.
    alive: AtomicBool,
    /// 페이지(탭)마다 붙여 둔 조작 통로: 탭 번호 → 세션 번호.
    sessions: Mutex<HashMap<String, String>>,
    /// 지금 조작 대상인 탭과, 그렇게 정한 시각(잠깐 동안은 다시 묻지 않는다).
    current: Mutex<Option<(String, Instant)>>,
}

/// 띄워 둔 브라우저들: 프로필 번호 → 브라우저.
static CHROMES: LazyLock<Mutex<HashMap<String, Arc<Chrome>>>> = LazyLock::new(Default::default);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 설치된 Chromium 계열 브라우저를 찾는다. 설정에 경로가 있으면 그것만 쓴다.
pub fn find_browser(configured: &str) -> Result<PathBuf, String> {
    if !configured.trim().is_empty() {
        let path = PathBuf::from(configured.trim());
        return if path.is_file() { Ok(path) } else { Err(format!("설정의 chrome_path에 파일이 없습니다: {}", path.display())) };
    }
    let roots: Vec<PathBuf> =
        ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    let programs = [
        r"Google\Chrome\Application\chrome.exe",
        r"Microsoft\Edge\Application\msedge.exe",
        r"Chromium\Application\chrome.exe",
        r"BraveSoftware\Brave-Browser\Application\brave.exe",
    ];
    programs
        .iter()
        .flat_map(|program| roots.iter().map(move |root| root.join(program)))
        .find(|path| path.is_file())
        .ok_or_else(|| "Chrome·Edge·Chromium·Brave를 찾지 못했습니다. 설치하거나 설정 파일의 [browser] chrome_path에 실행 파일 경로를 적어 주세요.".to_string())
}

/// 그 프로필 번호의 브라우저가 쓸 폴더. 번호가 비어 있으면(그룹 없음) 예전부터 쓰던 `chrome-profile`.
fn profile_dir(key: &str) -> Result<PathBuf, String> {
    let base = crate::data_dir().ok_or("데이터 폴더를 찾지 못했습니다")?;
    let dir = if key.is_empty() { base.join("chrome-profile") } else { base.join("chrome-profiles").join(crate::site::profile_key(key)?) };
    std::fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

// ───────────────────────── 파이프로 띄우기 (Windows) ─────────────────────────

#[cfg(windows)]
mod pipe {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::FromRawHandle;

    type Handle = *mut c_void;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }

    const HANDLE_FLAG_INHERIT: u32 = 1;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreatePipe(read: *mut Handle, write: *mut Handle, attributes: *const SecurityAttributes, size: u32) -> i32;
        fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    /// 브라우저에 넘길 쪽(상속되는 핸들)과 우리가 쓸 쪽.
    pub struct Pair {
        pub ours: File,
        theirs: Handle,
    }

    impl Pair {
        /// 브라우저에 알려 줄 핸들 번호.
        pub fn number(&self) -> usize {
            self.theirs as usize
        }
    }

    impl Drop for Pair {
        /// 브라우저를 띄운 뒤에는 넘긴 쪽을 닫는다(닫아야 브라우저가 끝났을 때 읽기가 끝난다).
        fn drop(&mut self) {
            // SAFETY: CreatePipe가 준 핸들이고 여기서 한 번만 닫는다.
            unsafe { CloseHandle(self.theirs) };
        }
    }

    /// 파이프 하나를 만든다. `we_write`가 참이면 우리가 쓰고 브라우저가 읽는다.
    pub fn create(we_write: bool) -> std::io::Result<Pair> {
        let attributes = SecurityAttributes { length: std::mem::size_of::<SecurityAttributes>() as u32, descriptor: std::ptr::null_mut(), inherit: 1 };
        let (mut read, mut write): (Handle, Handle) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: 유효한 포인터 둘과 구조체 하나를 넘긴다.
        if unsafe { CreatePipe(&mut read, &mut write, &attributes, 1 << 16) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let (ours, theirs) = if we_write { (write, read) } else { (read, write) };
        // 우리 쪽은 자식에게 넘어가지 않게 한다.
        // SAFETY: 방금 만든 유효한 핸들이다.
        unsafe { SetHandleInformation(ours, HANDLE_FLAG_INHERIT, 0) };
        // SAFETY: 이 핸들의 소유권을 File로 넘긴다(다른 곳에서 닫지 않는다).
        Ok(Pair { ours: unsafe { File::from_raw_handle(ours as _) }, theirs })
    }
}

/// 브라우저를 띄우고 (프로세스, 우리가 쓰는 쪽, 우리가 읽는 쪽)을 돌려준다.
#[cfg(windows)]
fn spawn(program: &Path, profile: &Path, url: &str) -> Result<(Child, File, File), String> {
    let to_browser = pipe::create(true).map_err(err)?;
    let from_browser = pipe::create(false).map_err(err)?;
    let child = Command::new(program)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-pipe")
        .arg(format!("--remote-debugging-io-pipes={},{}", to_browser.number(), from_browser.number()))
        .args(["--no-first-run", "--no-default-browser-check", "--new-window"])
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("브라우저를 실행하지 못했습니다({}): {e}", program.display()))?;
    let writer = to_browser.ours.try_clone().map_err(err)?;
    let reader = from_browser.ours.try_clone().map_err(err)?;
    Ok((child, writer, reader)) // 여기서 넘긴 쪽 핸들이 닫힌다.
}

// ───────────────────────── 주고받기 ─────────────────────────

/// 보통 명령의 답을 기다리는 시간.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// 탭이 보이는지 묻는 것처럼 가벼운 확인의 답을 기다리는 시간. 대화상자가 떠 있거나 멈춘 탭 때문에
/// 도구 호출 전체가 오래 묶이지 않게 짧게 둔다.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

impl Chrome {
    /// 띄운 브라우저와의 연결을 만들고, 읽기 스레드와 종료 감시 스레드를 붙인다.
    fn connect(key: &str, child: Child, writer: File, reader: File) -> Arc<Chrome> {
        let chrome = Arc::new(Chrome {
            key: key.to_string(),
            child: Mutex::new(child),
            writer: Mutex::new(writer),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            sessions: Mutex::new(HashMap::new()),
            current: Mutex::new(None),
        });
        let for_reader = chrome.clone();
        thread::spawn(move || read_loop(reader, for_reader));
        // 브라우저 프로세스가 끝났는지 따로 지켜본다. 파이프의 끝(EOF)만 믿으면, 띄우는 순간에 뜬 다른
        // 자식 프로세스가 파이프 핸들을 물려받은 경우 브라우저가 끝난 것을 영영 모를 수 있다.
        let for_watch = chrome.clone();
        thread::spawn(move || {
            while for_watch.alive.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(500));
                let exited = !matches!(for_watch.child.lock().unwrap().try_wait(), Ok(None));
                if exited {
                    for_watch.retire();
                }
            }
        });
        chrome
    }

    /// 브라우저가 끝났다: 기다리던 호출을 모두 끝내고, 다음에 다시 띄울 수 있게 자리를 비운다.
    fn retire(self: &Arc<Self>) {
        self.alive.store(false, Ordering::SeqCst);
        for (_, waiter) in self.pending.lock().unwrap().drain() {
            let _ = waiter.send(Err("브라우저가 닫혔습니다".into()));
        }
        // 떠 있던 확인 창도 브라우저와 함께 사라졌다.
        crate::browser::forget_dialogs(&crate::browser::chrome_dialogs(&self.key));
        let mut all = CHROMES.lock().unwrap();
        if all.get(&self.key).is_some_and(|current| Arc::ptr_eq(current, self)) {
            all.remove(&self.key);
        }
    }

    /// 명령 하나를 보내고 답을 기다린다. `session`이 있으면 그 탭에 보낸다.
    fn send(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        self.send_within(method, params, session, REPLY_TIMEOUT)
    }

    fn send_within(&self, method: &str, params: Value, session: Option<&str>, timeout: Duration) -> Result<Value, String> {
        const CLOSED: &str = "브라우저가 닫혔습니다";
        if !self.alive.load(Ordering::SeqCst) {
            return Err(CLOSED.into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0); // 메시지는 널 바이트로 끝난다.
        let written = self.writer.lock().unwrap().write_all(&bytes);
        // 등록하는 사이에 브라우저가 끝났으면(끝낸 쪽은 이 요청을 못 봤다) 기다리지 않는다.
        if written.is_err() || !self.alive.load(Ordering::SeqCst) {
            self.pending.lock().unwrap().remove(&id);
            return Err(match written {
                Err(e) => format!("브라우저와 연결이 끊겼습니다: {e}"),
                Ok(()) => CLOSED.into(),
            });
        }
        match rx.recv_timeout(timeout) {
            Ok(reply) => reply,
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("브라우저가 응답하지 않습니다".into())
            }
        }
    }

    /// 열려 있는 탭 목록(개발자 도구 창 같은 것은 뺀다).
    fn pages(&self) -> Result<Vec<Value>, String> {
        let reply = self.send("Target.getTargets", json!({}), None)?;
        Ok(reply["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["type"] == "page" && !t["url"].as_str().unwrap_or("").starts_with("devtools://"))
            .cloned()
            .collect())
    }

    /// 그 탭에 붙은 조작 통로. 없으면 붙인다.
    fn session_for(&self, target: &str) -> Result<String, String> {
        if let Some(session) = self.sessions.lock().unwrap().get(target) {
            return Ok(session.clone());
        }
        let reply = self.send("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None)?;
        let session = reply["sessionId"].as_str().ok_or("탭에 연결하지 못했습니다")?.to_string();
        self.sessions.lock().unwrap().insert(target.to_string(), session.clone());
        // 이 탭이 띄우는 확인 창을 알 수 있게 한다(`on_event`). 켜 둔 뒤에 뜬 창만 알리고 닫을 수 있다.
        let _ = self.send_within("Page.enable", json!({}), Some(&session), PROBE_TIMEOUT);
        Ok(session)
    }

    /// 명령을 보내기만 한다(답은 기다리지 않는다). 읽기 스레드에서 쓴다: 거기서 답을 기다리면 그 답을 읽을 수 없다.
    fn post(&self, method: &str, params: Value, session: &str) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut bytes = json!({ "id": id, "method": method, "params": params, "sessionId": session }).to_string().into_bytes();
        bytes.push(0);
        let _ = self.writer.lock().unwrap().write_all(&bytes);
    }

    /// 브라우저가 알려 온 일(답이 아닌 것). 페이지가 확인 창을 띄웠을 때, Claude의 도구가 이 브라우저를 쓰는 중이면
    /// 바로 닫는다(닫지 않으면 그 탭에 보낸 명령의 답이 오지 않는다). 쓰는 중이 아니면 사용자가 답하게 둔다.
    fn on_event(&self, message: &Value) {
        let dialogs = crate::browser::chrome_dialogs(&self.key);
        let tab = message["sessionId"].as_str().unwrap_or("");
        match message["method"].as_str() {
            Some("Page.javascriptDialogOpening") => {
                let dialog = crate::browser::Dialog::from_event(&message["params"], tab);
                if let Some(answer) = crate::browser::dialog_opened(&dialogs, dialog.clone()) {
                    self.post("Page.handleJavaScriptDialog", crate::browser::dialog_params(&dialog, &answer), tab);
                    if let Some(script) = crate::browser::dialog_follow_up(&dialog, &answer) {
                        self.post("Runtime.evaluate", script, tab);
                    }
                }
            }
            Some("Page.javascriptDialogClosed") => crate::browser::dialog_closed(&dialogs, Some(tab)),
            _ => {}
        }
    }

    fn is_visible(&self, target: &str) -> bool {
        let Ok(session) = self.session_for(target) else { return false };
        let probe = json!({ "expression": "document.visibilityState", "returnByValue": true });
        self.send_within("Runtime.evaluate", probe, Some(&session), PROBE_TIMEOUT).is_ok_and(|reply| reply["result"]["value"] == "visible")
    }

    /// 지금 조작할 탭: 사용자가 보고 있는(화면에 보이는) 탭. 방금 정했으면 다시 묻지 않는다.
    ///
    /// 보이는 탭이 하나도 없으면(창을 최소화했거나 다른 창에 가려짐) 쓰던 탭을 그대로 쓴다.
    /// 읽기와 누르기 사이에 조작 대상이 다른 탭으로 바뀌면 안 되기 때문이다.
    fn current_target(&self) -> Result<String, String> {
        if let Some((target, at)) = self.current.lock().unwrap().clone() {
            if at.elapsed() < Duration::from_millis(1500) {
                return Ok(target);
            }
        }
        let pages = self.pages()?;
        let ids: Vec<&str> = pages.iter().filter_map(|p| p["targetId"].as_str()).collect();
        // 닫힌 탭의 통로는 잊는다.
        self.sessions.lock().unwrap().retain(|target, _| ids.contains(&target.as_str()));
        let previous = self.current.lock().unwrap().as_ref().map(|(target, _)| target.clone());
        let previous = previous.filter(|target| ids.contains(&target.as_str()));
        let chosen = match &previous {
            Some(target) if self.is_visible(target) => target.clone(),
            _ => {
                let others = ids.iter().filter(|id| Some(**id) != previous.as_deref());
                let visible = others.clone().find(|id| self.is_visible(id)).map(|id| id.to_string());
                match visible.or(previous).or_else(|| ids.first().map(|id| id.to_string())) {
                    Some(target) => target,
                    // 창을 모두 닫았는데 브라우저는 아직 살아 있다: 새 창을 하나 연다.
                    None => self
                        .send("Target.createTarget", json!({ "url": "about:blank", "newWindow": true }), None)?["targetId"]
                        .as_str()
                        .ok_or("브라우저에 새 창을 열지 못했습니다")?
                        .to_string(),
                }
            }
        };
        *self.current.lock().unwrap() = Some((chosen.clone(), Instant::now()));
        Ok(chosen)
    }
}

/// 브라우저가 보낸 것을 읽어, 기다리는 쪽에 답을 넘긴다. 브라우저가 끝나면 함께 끝난다.
fn read_loop(reader: File, chrome: Arc<Chrome>) {
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(0, &mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if buffer.last() == Some(&0) {
            buffer.pop();
        }
        let Ok(message) = serde_json::from_slice::<Value>(&buffer) else { continue };
        // 번호가 없는 것은 답이 아니라 브라우저가 알려 온 일이다.
        let Some(id) = message["id"].as_u64() else {
            chrome.on_event(&message);
            continue;
        };
        if let Some(waiter) = chrome.pending.lock().unwrap().remove(&id) {
            let reply = match message.get("error") {
                Some(problem) => Err(problem["message"].as_str().unwrap_or("브라우저 오류").to_string()),
                None => Ok(message["result"].clone()),
            };
            let _ = waiter.send(reply);
        }
    }
    chrome.retire();
}

fn running(key: &str) -> Option<Arc<Chrome>> {
    CHROMES.lock().unwrap().get(key).cloned()
}

/// 그 프로필의 브라우저가 떠 있는지.
pub fn is_open(key: &str) -> bool {
    running(key).is_some()
}

/// 같은 프로필의 브라우저가 이미 떠 있는지. Chromium은 실행 중에 프로필 폴더의 `lockfile`을 잡고 있고,
/// 끝나면 그 파일이 사라진다. 떠 있는 채로 또 띄우면 새 프로세스가 기존 브라우저에 창만 하나 열고 끝나서,
/// 이쪽은 연결하지 못한 채 남의 브라우저에 창만 계속 늘리게 된다.
fn profile_in_use(profile: &Path) -> bool {
    let lock = profile.join("lockfile");
    lock.exists() && std::fs::OpenOptions::new().write(true).open(&lock).is_err()
}

/// 브라우저를 처음 띄운 뒤 첫 페이지가 실제 주소로 넘어갈 때까지(최대 6초) 기다린다.
/// 뜨자마자는 빈 문서(about:blank)라서, 바로 읽으면 "다 읽힘" 상태의 빈 페이지가 읽힌다.
fn wait_first_page(chrome: &Chrome, start: &str) {
    if start == "about:blank" {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        let arrived = chrome.pages().is_ok_and(|pages| pages.iter().any(|p| p["url"].as_str().is_some_and(|url| url.starts_with("http"))));
        if arrived {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// 그 프로필의 브라우저를 띄운다. 이미 떠 있으면 앞으로 가져오고, 주소가 있으면 그리로 간다.
pub fn open(key: &str, configured_path: &str, home: &str, url: Option<&str>) -> Result<(), String> {
    // 띄우기는 한 번에 하나만 한다(두 세션이 동시에 도구를 불러 브라우저를 둘 띄우지 않게).
    static OPENING: Mutex<()> = Mutex::new(());
    let _opening = OPENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(chrome) = running(key) {
        let target = chrome.current_target()?;
        if let Some(url) = url {
            let session = chrome.session_for(&target)?;
            chrome.send("Page.navigate", json!({ "url": crate::browser::normalize_url(url)?.to_string() }), Some(&session))?;
        }
        let _ = chrome.send("Target.activateTarget", json!({ "targetId": target }), None);
        return Ok(());
    }
    let program = find_browser(configured_path)?;
    let profile = profile_dir(key)?;
    if profile_in_use(&profile) {
        return Err("이 그룹의 Cypher 전용 프로필로 띄운 브라우저가 이미 떠 있습니다(다른 Cypher 창이 띄운 것일 수 있습니다). \
                    그 브라우저 창을 모두 닫은 뒤 다시 해 보세요."
            .into());
    }
    let start = crate::browser::normalize_url(url.unwrap_or(home))?.to_string();
    let (child, writer, reader) = spawn(&program, &profile, &start)?;
    let chrome = Chrome::connect(key, child, writer, reader);

    // 연결을 확인한 뒤에야 "떠 있음"으로 올린다.
    match chrome.send_within("Browser.getVersion", json!({}), None, Duration::from_secs(15)) {
        Ok(version) => {
            CHROMES.lock().unwrap().insert(key.to_string(), chrome.clone());
            crate::log(&format!("브라우저 엔진 chrome: {} ({})", version["product"].as_str().unwrap_or("?"), program.display()));
            wait_first_page(&chrome, &start);
            // 첫 탭에 미리 붙어 둔다: 붙은 뒤에 뜬 확인 창만 알 수 있고 닫을 수 있다.
            let _ = chrome.current_target();
            Ok(())
        }
        Err(problem) => {
            let _ = chrome.child.lock().unwrap().kill();
            chrome.retire();
            Err(format!("브라우저에 연결하지 못했습니다: {problem}"))
        }
    }
}

/// Cypher가 끝날 때: 띄워 둔 브라우저를 모두 닫는다(파이프가 닫히면 스스로 끝나지만, 확실히 해 둔다).
pub fn shutdown() {
    let all: Vec<Arc<Chrome>> = CHROMES.lock().unwrap().values().cloned().collect();
    // 닫으라는 말을 모두에게 한꺼번에 보내고 함께 기다린다(응답 없는 브라우저가 여럿이어도 기다리는 시간이 늘지 않게).
    let closing: Vec<_> = all
        .into_iter()
        .map(|chrome| thread::spawn(move || drop(chrome.send_within("Browser.close", json!({}), None, Duration::from_millis(1500)))))
        .collect();
    for handle in closing {
        let _ = handle.join();
    }
}

/// 그 프로필의 브라우저에서 지금 탭에 CDP 명령을 보낸다.
pub fn cdp(key: &str, method: &str, params: Value) -> Result<Value, String> {
    let chrome = running(key).ok_or("브라우저가 열려 있지 않습니다")?;
    let target = chrome.current_target()?;
    let session = chrome.session_for(&target)?;
    chrome.send(method, params, Some(&session))
}

/// 그 프로필의 브라우저에서 그 탭(조작 통로 번호)에 CDP 명령을 보낸다. 확인 창은 그 창이 뜬 탭에 답해야 한다.
pub fn cdp_on(key: &str, session: &str, method: &str, params: Value) -> Result<Value, String> {
    running(key).ok_or("브라우저가 열려 있지 않습니다")?.send(method, params, Some(session))
}

/// 떠 있는 브라우저(어느 그룹의 것이든)의 Chromium 버전(예: 157.0.8103.52). 떠 있는 것이 없으면 None.
pub fn running_version() -> Option<String> {
    let any = CHROMES.lock().unwrap().values().next().cloned()?;
    let reply = any.send_within("Browser.getVersion", json!({}), None, PROBE_TIMEOUT).ok()?;
    reply["product"].as_str()?.split('/').nth(1).map(str::to_string)
}

/// 그 프로필의 브라우저에서 지금 탭의 주소.
pub fn current_url(key: &str) -> Option<String> {
    let chrome = running(key)?;
    let target = chrome.current_target().ok()?;
    chrome.pages().ok()?.iter().find(|p| p["targetId"] == target.as_str())?["url"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 설정한_경로가_없으면_알린다() {
        assert!(find_browser(r"C:\없는 폴더\chrome.exe").unwrap_err().contains("chrome_path"));
    }

    /// 설치된 브라우저를 실제로 띄워 파이프로 조작해 본다(브라우저가 없으면 건너뛴다).
    /// 창이 잠깐 뜨므로 평소 시험에서는 빼고, `cargo test -- --ignored`로 돌린다.
    #[test]
    #[ignore]
    fn 설치된_브라우저를_파이프로_조작한다() {
        let Ok(program) = find_browser("") else { return };
        let profile = std::env::temp_dir().join("cypher-chrome-test-profile");
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        assert!(!profile_in_use(&profile));
        let (child, writer, reader) = spawn(&program, &profile, "about:blank").unwrap();
        let chrome = Chrome::connect("test", child, writer, reader);

        let version = chrome.send("Browser.getVersion", json!({}), None).unwrap();
        assert!(version["product"].as_str().unwrap().contains('/'), "{version}");
        assert!(profile_in_use(&profile), "떠 있는 동안에는 프로필이 쓰이는 중으로 보여야 한다");
        let target = chrome.current_target().unwrap();
        let session = chrome.session_for(&target).unwrap();
        let value = chrome
            .send("Runtime.evaluate", json!({ "expression": "6 * 7", "returnByValue": true }), Some(&session))
            .unwrap();
        assert_eq!(value["result"]["value"], 42);
        assert!(chrome.send("없는.명령", json!({}), None).is_err());

        // 브라우저가 끝나면 곧 알아채고, 그 뒤의 호출은 기다리지 않고 바로 실패한다.
        let _ = chrome.send("Browser.close", json!({}), None);
        let deadline = Instant::now() + Duration::from_secs(10);
        while chrome.alive.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
        }
        assert!(!chrome.alive.load(Ordering::SeqCst));
        let asked = Instant::now();
        assert!(chrome.send("Browser.getVersion", json!({}), None).is_err());
        assert!(asked.elapsed() < Duration::from_secs(1));
        thread::sleep(Duration::from_millis(500));
        let _ = std::fs::remove_dir_all(&profile);
    }

    /// 설치된 브라우저를 실제로 띄워, 페이지가 띄우는 확인 창을 도구가 닫고 지나가는지 본다(브라우저가 없으면 건너뛴다).
    /// 창이 잠깐 뜨므로 평소 시험에서는 빼고, `cargo test -- --ignored`로 돌린다.
    #[test]
    #[ignore]
    fn 페이지가_띄운_확인_창을_닫고_지나간다() {
        let Ok(program) = find_browser("") else { return };
        let profile = std::env::temp_dir().join("cypher-chrome-dialog-test-profile");
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        let (child, writer, reader) = spawn(&program, &profile, "about:blank").unwrap();
        let key = "dialogtest";
        let chrome = Chrome::connect(key, child, writer, reader);
        chrome.send("Browser.getVersion", json!({}), None).unwrap();
        CHROMES.lock().unwrap().insert(key.to_string(), chrome.clone());
        let target = chrome.current_target().unwrap();
        let session = chrome.session_for(&target).unwrap();
        let page = "data:text/html,<title>시험</title><button style='width:300px;height:200px'>단추</button>";
        chrome.send("Page.navigate", json!({ "url": page }), Some(&session)).unwrap();

        let tool = |name: &str, args: Value| {
            let asked = Instant::now();
            let outcome = crate::browser::on_chrome(key, name, &args);
            let took = asked.elapsed();
            let text = match outcome {
                Ok(content) => content.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n"),
                Err(problem) => format!("오류: {problem}"),
            };
            println!("── {name} {args} ({took:?})\n{text}");
            (text, took)
        };
        let quick = Duration::from_secs(8);

        // 조작 중에 뜬 확인 창: 기본은 취소. 도구가 멈추지 않고, 닫은 사실이 답에 실린다.
        let (text, took) = tool("browser_eval", json!({ "script": "confirm('지울까요?') ? '지움' : '취소함'" }));
        assert!(text.contains("취소함") && text.contains("(confirm): \"지울까요?\" → 취소했습니다."), "{text}");
        assert!(took < quick, "{took:?}");

        // 답을 정해 두면 다음 창 하나에 쓰인다.
        tool("browser_dialog", json!({ "accept": true }));
        let (text, _) = tool("browser_eval", json!({ "script": "confirm('지울까요?') ? '지움' : '취소함'" }));
        assert!(text.contains("지움") && text.contains("→ 수락했습니다. (browser_dialog로 정해 둔 답입니다)"), "{text}");
        let (text, _) = tool("browser_eval", json!({ "script": "confirm('또?') ? '지움' : '취소함'" }));
        assert!(text.contains("취소함"), "{text}");

        // 입력 창에 값을 넣는다. 알림은 확인으로 닫는다.
        tool("browser_dialog", json!({ "accept": true, "text": "김철수" }));
        let (text, _) = tool("browser_eval", json!({ "script": "prompt('이름?', '홍길동')" }));
        assert!(text.contains("김철수") && text.contains("(prompt): \"이름?\" → 수락했습니다(입력값: \"김철수\")"), "{text}");
        let (text, took) = tool("browser_eval", json!({ "script": "alert('저장했습니다'); '끝'" }));
        assert!(text.contains("끝") && text.contains("(alert): \"저장했습니다\" → 확인을 눌렀습니다."), "{text}");
        assert!(took < quick, "{took:?}");

        // 정해 둔 답은 쓰이지 않아도 조작 한 번이 끝나면 지워진다.
        tool("browser_dialog", json!({ "accept": true }));
        tool("browser_eval", json!({ "script": "1 + 1" }));
        let (text, _) = tool("browser_eval", json!({ "script": "confirm('지울까요?') ? '지움' : '취소함'" }));
        assert!(text.contains("취소함"), "{text}");

        // 누르기로 뜬 확인 창(마우스 입력의 답이 창이 닫힐 때까지 오지 않는 경우).
        tool("browser_eval", json!({ "script": "document.querySelector('button').onclick = () => { document.title = confirm('누름?') ? '예' : '아니오'; }; 1" }));
        let (text, took) = tool("browser_click_at", json!({ "x": 100, "y": 100 }));
        assert!(text.contains("제목: 아니오") && text.contains("(confirm): \"누름?\" → 취소했습니다."), "{text}");
        assert!(took < quick, "{took:?}");

        // 도구가 쓰고 있지 않을 때 뜬 창은 그대로 둔다. 다음 도구가 시작할 때 닫고 그 사실을 적는다.
        tool("browser_eval", json!({ "script": "setTimeout(() => { document.title = confirm('나중에 뜬 창') ? '예2' : '아니오2'; }, 300); 1" }));
        thread::sleep(Duration::from_millis(1500));
        let (text, took) = tool("browser_eval", json!({ "script": "document.title" }));
        assert!(text.contains("아니오2") && text.contains("페이지에 확인 창이 떠 있었습니다(confirm): \"나중에 뜬 창\" → 취소했습니다."), "{text}");
        assert!(took < quick, "{took:?}");

        // 나갈지 묻는 창: 기본은 나가기(이동이 이어진다). 머무르기로 정해 두면 이동하지 않고, 기다리다 끝나지도 않는다.
        let guard = "window.onbeforeunload = (e) => { e.preventDefault(); e.returnValue = '나갈까요'; }; 1";
        tool("browser_eval", json!({ "script": guard }));
        tool("browser_dialog", json!({ "accept": false }));
        let (text, took) = tool("browser_navigate", json!({ "url": "about:blank" }));
        assert!(text.contains("제목: 아니오2") && text.contains("(beforeunload)") && text.contains("→ 머무르기로 답했습니다."), "{text}");
        assert!(took < quick, "{took:?}");
        let (text, took) = tool("browser_navigate", json!({ "url": "about:blank" }));
        assert!(text.contains("주소: about:blank") && text.contains("(beforeunload)") && text.contains("→ 나가기로 답했습니다."), "{text}");
        assert!(took < quick, "{took:?}");

        let _ = chrome.send("Browser.close", json!({}), None);
        thread::sleep(Duration::from_millis(1500));
        let _ = std::fs::remove_dir_all(&profile);
    }
}
