//! Claude 세션이 쓰는 브라우저 조작 도구.
//!
//! 조작 대상은 그 세션과 같은 그룹에 있는 사이트 항목(사이드바의 브라우저)이다. 어느 사이트를 조작할지는
//! 화면 쪽이 정해 준다(사용자가 넘겨준 것 → 같은 그룹에서 마지막으로 쓴 것 → 없으면 새로 만든다).
//! 조작은 포트를 열지 않고 WebView2의 개발자 도구 프로토콜(CDP)을 앱 안에서 직접 부른다.
//! 웹 페이지 쪽 웹뷰에는 어떤 앱 명령 권한도 주지 않는다(capabilities 참고).
//!
//! 설정에서 엔진을 "chrome"으로 바꾸면 사이트 항목 대신 설치된 Chrome·Edge를 띄워 조작한다(`chrome.rs`).
//! 어느 엔진이든 브라우저(프로필)는 그룹마다 따로다: 세션은 자기 그룹의 브라우저만 조작하고, 로그인 상태도 그룹끼리 섞이지 않는다.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{mpsc, LazyLock, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Url};

const MAX_TEXT: usize = 15_000;
/// 이동·클릭 뒤 페이지가 다 읽히기를 기다리는 시간의 한도.
const MAX_WAIT: Duration = Duration::from_secs(12);

type ToolResult = Result<Vec<Value>, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 주소창에 친 글자를 실제 주소로 바꾼다. http/https만 연다.
/// 주소처럼 생기지 않은 글자는 검색어로 본다.
pub fn normalize_url(input: &str) -> Result<Url, String> {
    let text = input.trim();
    let lower = text.to_ascii_lowercase();
    if text.is_empty() || lower == "about:blank" {
        return Url::parse("about:blank").map_err(err);
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Url::parse(text).map_err(|_| format!("올바르지 않은 주소입니다: {text}"));
    }
    let local = lower.starts_with("localhost") || lower.starts_with("127.0.0.1");
    let looks_like_host = !text.contains(char::is_whitespace)
        && !lower.contains("://")
        && !lower.starts_with("javascript:")
        && !lower.starts_with("file:")
        && !lower.starts_with("data:")
        && (text.contains('.') || local);
    if looks_like_host {
        let scheme = if local { "http" } else { "https" };
        if let Ok(url) = Url::parse(&format!("{scheme}://{text}")) {
            return Ok(url);
        }
    }
    Url::parse_with_params("https://www.google.com/search", [("q", text)]).map_err(err)
}

/// 설정에서 고른 엔진이 "chrome"(설치된 브라우저를 띄워 조작)인지. 아니면 사이트 항목을 조작한다.
fn chrome_engine(app: &AppHandle) -> bool {
    app.state::<crate::AppState>().config.browser.engine.eq_ignore_ascii_case("chrome")
}

/// "chrome" 엔진에서 그 프로필(그룹)의 브라우저를 띄운다. 이미 떠 있으면 앞으로 가져오고, 주소가 있으면 그리로 간다.
fn open_chrome(app: &AppHandle, profile: &str, url: Option<&str>) -> Result<(), String> {
    let settings = &app.state::<crate::AppState>().config.browser;
    crate::chrome::open(profile, &settings.chrome_path, &settings.home, url)
}

/// 화면이 건넨 프로필 번호를 확인한다. 없거나 빈 글이면 그룹 없는 곳의 브라우저(공용 프로필)다.
fn chrome_profile(profile: Option<String>) -> Result<String, String> {
    let profile = profile.unwrap_or_default();
    if !profile.is_empty() {
        crate::site::profile_key(&profile)?;
    }
    Ok(profile)
}

// ───────────────────────── 화면에서 부르는 명령("chrome" 엔진 전용) ─────────────────────────

/// 설치된 Chrome(또는 Edge)을 Cypher 전용 프로필로 띄운다. `profile`은 지금 보고 있는 항목의 그룹이 쓰는 프로필의 번호.
#[tauri::command]
pub async fn browser_open(app: AppHandle, url: Option<String>, profile: Option<String>) -> Result<(), String> {
    if !chrome_engine(&app) {
        return Err("브라우저는 사이드바의 사이트 항목으로 엽니다".into());
    }
    open_chrome(&app, &chrome_profile(profile)?, url.as_deref()).inspect_err(|problem| crate::log(&format!("브라우저 열기 실패: {problem}")))
}

/// 그 프로필의 Chrome에 지금 열려 있는 페이지 주소. 닫혀 있으면 None.
#[tauri::command]
pub async fn browser_where(app: AppHandle, profile: Option<String>) -> Option<String> {
    if chrome_engine(&app) { crate::chrome::current_url(&chrome_profile(profile).ok()?) } else { None }
}

/// Chrome에 보이는 화면(또는 `full`이면 페이지 전체)을 찍어 이미지 저장소에 넣는다.
/// 화면 쪽이 돌려받은 그림으로, 찍으라고 한 그때 보고 있던 항목의 묶음에 이미지 항목을 만든다
/// (사건으로 따로 알리지 않는다: 찍는 사이 다른 그룹의 항목으로 옮겨 가면 그림이 그 그룹에 들어간다).
#[tauri::command]
pub async fn browser_capture(app: AppHandle, full: Option<bool>, profile: Option<String>) -> Result<crate::image::Captured, String> {
    let profile = chrome_profile(profile)?;
    if !chrome_engine(&app) || !crate::chrome::is_open(&profile) {
        return Err("브라우저가 열려 있지 않습니다".into());
    }
    crate::image::capture(&|method, params| crate::chrome::cdp(&profile, method, params), full.unwrap_or(false))
        .inspect_err(|problem| crate::log(&format!("브라우저 캡처 실패: {problem}")))
}

// ───────────────────────── 개발자 도구 프로토콜 ─────────────────────────

/// 도구가 조작하는 대상: 사이트 항목의 웹뷰, 또는 "chrome" 엔진이 그 그룹의 프로필로 띄운 브라우저.
enum Target {
    Site(Box<tauri::Webview>),
    /// 프로필 번호.
    Chrome(String),
}

impl Target {
    /// CDP 명령을 보내고 답을 받는다. 작업 스레드 전용.
    fn cdp(&self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Target::Site(webview) => webview_cdp(webview, method, params),
            Target::Chrome(profile) => crate::chrome::cdp(profile, method, params),
        }
    }

    /// 떠 있는 확인 창에 답한다(그 창이 뜬 탭에). 작업 스레드 전용.
    fn answer(&self, dialog: &Dialog, answer: &Answer) -> Result<(), String> {
        let params = dialog_params(dialog, answer);
        match self {
            Target::Site(webview) => webview_cdp(webview, HANDLE_DIALOG, params).map(drop),
            Target::Chrome(profile) => crate::chrome::cdp_on(profile, &dialog.tab, HANDLE_DIALOG, params).map(drop),
        }
    }
}

// ───────────────────────── 페이지가 띄운 확인 창 ─────────────────────────
//
// 페이지가 alert·confirm·prompt를 띄우면 그 창이 닫힐 때까지 페이지가 멈추고, 페이지에 보낸 명령(스크립트 실행,
// 마우스·키 입력)의 답도 오지 않는다. 그래서 Claude의 도구가 그 브라우저를 쓰는 동안 뜬 창은 Cypher가 바로 닫고,
// 닫은 사실을 그 도구의 답 끝에 적는다. 도구가 쓰고 있지 않을 때 뜬 창은 사용자가 답할 것이므로 건드리지 않는다.
// 다만 그런 창이 떠 있는 채로 도구가 시작되면 닫고 시작한다(두면 도구가 멈춘다).
//
// 창이 떴다는 것은 CDP의 `Page.javascriptDialogOpening` 사건으로 알고, `Page.handleJavaScriptDialog`로 닫는다.
// 이 명령은 브라우저 쪽에서 처리되므로 페이지가 멈춰 있어도 답이 온다. 둘 다 `Page.enable`을 해 둔 뒤에 뜬 창에만 된다.

const HANDLE_DIALOG: &str = "Page.handleJavaScriptDialog";
/// 답에 싣는 창의 글 길이(글자 수).
const MAX_DIALOG_TEXT: usize = 200;
/// `browser_dialog`로 정해 둔 답이 살아 있는 시간. 쓰이지 않은 채 오래 남아 엉뚱한 창에 쓰이지 않게 한다.
const PLAN_LIFE: Duration = Duration::from_secs(300);

/// 페이지가 띄운 확인 창.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    /// CDP가 알려 주는 종류: alert, confirm, prompt, beforeunload(페이지를 나갈 때).
    pub kind: String,
    pub message: String,
    /// 입력 창(prompt)에 미리 들어 있는 값.
    pub preset: String,
    /// 그 창이 뜬 탭("chrome" 엔진의 조작 통로 번호). 사이트 항목이면 빈 글.
    pub tab: String,
}

impl Dialog {
    /// `Page.javascriptDialogOpening` 사건의 내용에서 만든다.
    pub fn from_event(params: &Value, tab: &str) -> Dialog {
        let field = |name: &str| params[name].as_str().unwrap_or("").to_string();
        Dialog { kind: field("type"), message: field("message"), preset: field("defaultPrompt"), tab: tab.to_string() }
    }
}

/// 확인 창에 줄 답. `text`는 입력 창(prompt)을 수락할 때 넣을 값(없으면 미리 들어 있던 값 그대로).
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub accept: bool,
    pub text: Option<String>,
}

/// 정해 둔 답이 없을 때의 답: 알림은 확인, 확인·입력 창은 취소, 페이지를 나갈지 묻는 창은 나가기.
fn default_answer(kind: &str) -> Answer {
    Answer { accept: matches!(kind, "alert" | "beforeunload"), text: None }
}

/// `Page.handleJavaScriptDialog`에 넘길 인자.
pub fn dialog_params(dialog: &Dialog, answer: &Answer) -> Value {
    let mut params = json!({ "accept": answer.accept });
    if answer.accept && dialog.kind == "prompt" {
        params["promptText"] = json!(answer.text.as_deref().unwrap_or(&dialog.preset));
    }
    params
}

/// 창을 닫은 뒤 페이지에서 실행할 것(`Runtime.evaluate`의 인자). 나가지 않기로 답했으면 떠나는 중 표시를 지운다
/// (`beforeunload`에서 표시는 이미 섰는데 이동은 없으므로, 두면 `wait_loaded`가 한도까지 기다린다).
pub fn dialog_follow_up(dialog: &Dialog, answer: &Answer) -> Option<Value> {
    (dialog.kind == "beforeunload" && !answer.accept).then(|| json!({ "expression": CLEAR_LEAVING_JS }))
}

/// 페이지가 준 글을 알림 글 한 줄에 넣을 수 있게 다듬는다: 줄 바꿈·제어 문자는 빈칸으로, 큰따옴표는 작은따옴표로.
/// 그대로 넣으면 페이지가 쓴 글이 Cypher가 쓴 안내 줄처럼 보이는 줄을 만들 수 있다.
fn plain_line(text: &str) -> String {
    text.chars().map(|c| if c.is_control() || c == '\u{2028}' || c == '\u{2029}' { ' ' } else if c == '"' { '\'' } else { c }).collect()
}

/// 닫은 창을 Claude에게 알리는 글. `planned`는 `browser_dialog`로 정해 둔 답을 썼는지,
/// `waiting`은 도구가 시작할 때 이미 떠 있던 창인지.
fn dialog_note(dialog: &Dialog, answer: &Answer, planned: bool, waiting: bool) -> String {
    let done = |accepted: &str, dismissed: &str| (if answer.accept { accepted } else { dismissed }).to_string();
    let (name, did) = match dialog.kind.as_str() {
        "alert" => ("알림 창", done("확인을 눌렀습니다", "닫았습니다")),
        "confirm" => ("확인 창", done("수락했습니다", "취소했습니다")),
        "prompt" if answer.accept => {
            let typed: String = plain_line(answer.text.as_deref().unwrap_or(&dialog.preset)).chars().take(MAX_DIALOG_TEXT).collect();
            ("입력 창", format!("수락했습니다(입력값: \"{typed}\")"))
        }
        "prompt" => ("입력 창", "취소했습니다".to_string()),
        "beforeunload" => ("나가기 확인 창", done("나가기로 답했습니다", "머무르기로 답했습니다")),
        _ => ("창", done("수락했습니다", "취소했습니다")),
    };
    let head = if waiting { format!("페이지에 {name}이 떠 있었습니다") } else { format!("페이지가 {name}을 띄웠습니다") };
    let mut message: String = plain_line(&dialog.message).chars().take(MAX_DIALOG_TEXT).collect();
    if dialog.message.chars().count() > MAX_DIALOG_TEXT {
        message.push('…');
    }
    let quoted = if message.is_empty() { String::new() } else { format!(": \"{message}\"") };
    let tail = match dialog.kind.as_str() {
        _ if planned => " (browser_dialog로 정해 둔 답입니다)",
        "confirm" => " 수락하려면 browser_dialog로 다음 창을 수락하도록 정한 뒤 같은 조작을 다시 하세요.",
        "prompt" => " 값을 넣으려면 browser_dialog로 accept와 text를 정한 뒤 같은 조작을 다시 하세요.",
        "beforeunload" => " 머무르려면 browser_dialog로 다음 창을 취소하도록 정한 뒤 같은 조작을 다시 하세요.",
        _ => "",
    };
    format!("{head}({}){quoted} → {did}.{tail}", dialog.kind)
}

/// 브라우저 하나(사이트 항목, 또는 그 그룹의 Chrome)의 확인 창 상태.
#[derive(Default)]
struct Dialogs {
    /// 지금 이 브라우저를 쓰고 있는 도구의 수.
    using: usize,
    /// 떠 있는데 닫지 않은 창(도구가 쓰고 있지 않을 때 뜬 것).
    open: Option<Dialog>,
    /// `browser_dialog`로 정해 둔 답과 정한 때. 한 번 쓰이면 없어진다.
    planned: Option<(Answer, Instant)>,
    /// 이번 조작에서 닫은 창을 알리는 글(도구의 답 끝에 붙인다).
    notes: Vec<String>,
}

impl Dialogs {
    /// 이 종류의 창에 줄 답과, 그것이 정해 둔 답인지. 정해 둔 답은 여기서 쓰이고 없어진다(오래된 것은 버린다).
    fn answer_for(&mut self, kind: &str, now: Instant) -> (Answer, bool) {
        match self.planned.take() {
            Some((answer, at)) if now.duration_since(at) < PLAN_LIFE => (answer, true),
            _ => (default_answer(kind), false),
        }
    }

    /// 창이 떴다. 도구가 쓰는 중이면 줄 답을 돌려준다(부른 쪽이 그 답으로 바로 닫는다).
    /// 아니면 떠 있는 창으로 적어만 둔다.
    fn opened(&mut self, dialog: Dialog, now: Instant) -> Option<Answer> {
        if self.using == 0 {
            self.open = Some(dialog);
            return None;
        }
        let (answer, planned) = self.answer_for(&dialog.kind, now);
        self.notes.push(dialog_note(&dialog, &answer, planned, false));
        Some(answer)
    }

    /// 그 탭의 창이 닫혔다(사용자가 답했거나 우리가 닫았다). `tab`이 None이면 어느 탭의 것이든(브라우저가 새로 떴거나 끝났다).
    fn closed(&mut self, tab: Option<&str>) {
        if self.open.as_ref().is_some_and(|dialog| tab.is_none_or(|tab| dialog.tab == tab)) {
            self.open = None;
        }
    }

    /// 도구가 시작할 때 이미 떠 있던 창과, 거기에 줄 답(정해 둔 답인지도).
    fn take_open(&mut self, now: Instant) -> Option<(Dialog, Answer, bool)> {
        let dialog = self.open.take()?;
        let (answer, planned) = self.answer_for(&dialog.kind, now);
        Some((dialog, answer, planned))
    }

    /// 도구 하나가 끝났다. 닫은 창을 알리는 글을 꺼낸다. 조작하는 도구였으면 정해 둔 답은 쓰였든 아니든 지운다.
    fn finish(&mut self, operated: bool) -> Vec<String> {
        self.using = self.using.saturating_sub(1);
        if operated {
            self.planned = None;
        }
        std::mem::take(&mut self.notes)
    }

    /// 기억할 것이 남아 있지 않은지.
    fn idle(&self) -> bool {
        self.using == 0 && self.open.is_none() && self.planned.is_none() && self.notes.is_empty()
    }
}

/// 브라우저마다의 확인 창 상태. 열쇠는 `site_dialogs`·`chrome_dialogs`가 만든다.
static DIALOGS: LazyLock<Mutex<HashMap<String, Dialogs>>> = LazyLock::new(Default::default);

/// 그 브라우저의 상태를 고친다. 기억할 것이 없어지면 자리를 비운다.
fn with_dialogs<T>(key: &str, change: impl FnOnce(&mut Dialogs) -> T) -> T {
    let mut all = DIALOGS.lock().unwrap_or_else(PoisonError::into_inner);
    let state = all.entry(key.to_string()).or_default();
    let out = change(state);
    if state.idle() {
        all.remove(key);
    }
    out
}

/// 그 브라우저가 닫혔다: 남아 있던 상태(떠 있던 창, 쓰이지 않은 답)를 버린다. 도구가 쓰는 중이면 끝날 때 비워진다.
pub fn forget_dialogs(key: &str) {
    let mut all = DIALOGS.lock().unwrap_or_else(PoisonError::into_inner);
    if all.get(key).is_some_and(|state| state.using == 0) {
        all.remove(key);
    } else if let Some(state) = all.get_mut(key) {
        state.open = None;
        state.planned = None;
    }
}

/// 사이트 항목의 확인 창 상태를 가리키는 열쇠. 프로필까지 넣는다: 항목이 다른 그룹으로 옮겨지면 옛 그룹의
/// 세션이 정해 둔 답이 따라가지 않는다.
pub fn site_dialogs(id: u32, profile: &str) -> String {
    format!("site-{id}/{profile}")
}

/// "chrome" 엔진에서 그 프로필(그룹)의 브라우저의 확인 창 상태를 가리키는 열쇠.
pub fn chrome_dialogs(profile: &str) -> String {
    format!("chrome/{profile}")
}

/// 엔진이 알려 준다: 그 브라우저에 확인 창이 떴다. 답을 돌려주면 부른 쪽이 그 답으로 바로 닫는다
/// (`dialog_params`, 그리고 `dialog_follow_up`이 있으면 그것도 보낸다). None이면 그대로 둔다.
pub fn dialog_opened(key: &str, dialog: Dialog) -> Option<Answer> {
    with_dialogs(key, |state| state.opened(dialog, Instant::now()))
}

/// 엔진이 알려 준다: 그 브라우저의 그 탭에 떠 있던 확인 창이 닫혔다.
/// `tab`이 None이면 그 브라우저에 떠 있던 창은 더 없다(새로 떴거나 끝났다).
pub fn dialog_closed(key: &str, tab: Option<&str>) {
    with_dialogs(key, |state| state.closed(tab));
}

/// 도구가 그 브라우저를 쓰는 동안 들고 있는 것. 놓으면(도구가 패닉으로 끝나도) 쓰는 중이라는 표시가 풀린다.
struct Using<'a> {
    key: &'a str,
    finished: bool,
}

impl<'a> Using<'a> {
    fn begin(key: &'a str) -> Using<'a> {
        with_dialogs(key, |state| state.using += 1);
        Using { key, finished: false }
    }

    /// 도구가 끝났다: 이번 조작에서 닫은 창을 알리는 글을 돌려준다.
    fn finish(mut self, operated: bool) -> Vec<String> {
        self.finished = true;
        with_dialogs(self.key, |state| state.finish(operated))
    }
}

impl Drop for Using<'_> {
    fn drop(&mut self) {
        if !self.finished {
            with_dialogs(self.key, |state| state.finish(false));
        }
    }
}

/// 사이트 웹뷰가 띄우는 확인 창을 지켜보기 시작한다. 웹뷰를 만든 직후 한 번 부른다.
/// 처리기는 창 스레드에서 불린다. 그 안에서는 명령을 보내기만 하고 답을 기다리지 않는다.
pub fn watch_dialogs(webview: &tauri::Webview, key: String) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
    use webview2_com::{take_pwstr, CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler};
    use windows_core::{HSTRING, PWSTR};

    /// 명령을 보내기만 한다(답은 기다리지 않고, 실패하면 적어 둔다).
    fn post(core: &ICoreWebView2, method: &'static str, params: Value) {
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, _| {
            if let Err(problem) = status {
                crate::log(&format!("확인 창 처리 실패({method}): {}", problem.message()));
            }
            Ok(())
        }));
        // SAFETY: 창 스레드에서 살아 있는 웹뷰에 대해 부른다. 문자열은 호출 동안 유효하다.
        let started = unsafe { core.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params.to_string()), &handler) };
        if let Err(problem) = started {
            crate::log(&format!("확인 창 처리 실패({method}): {}", problem.message()));
        }
    }

    // 새로 만든 웹뷰다. 같은 항목의 옛 웹뷰에 떠 있던 창은 웹뷰와 함께 사라졌다.
    dialog_closed(&key, None);
    let attached = webview.with_webview(move |platform| {
        // SAFETY: 창 스레드에서 살아 있는 웹뷰에 대해 부른다.
        let Ok(core) = (unsafe { platform.controller().CoreWebView2() }) else { return };
        let for_opening = key.clone();
        let opening = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |core, args| {
            let (Some(core), Some(args)) = (core, args) else { return Ok(()) };
            let mut text = PWSTR::null();
            // SAFETY: 살아 있는 사건 인자에서 값을 읽는다. 받은 문자열은 take_pwstr가 풀어 준다.
            unsafe { args.ParameterObjectAsJson(&mut text)? };
            let event: Value = serde_json::from_str(&take_pwstr(text)).unwrap_or_default();
            let dialog = Dialog::from_event(&event, "");
            if let Some(answer) = dialog_opened(&for_opening, dialog.clone()) {
                post(&core, HANDLE_DIALOG, dialog_params(&dialog, &answer));
                if let Some(script) = dialog_follow_up(&dialog, &answer) {
                    post(&core, "Runtime.evaluate", script);
                }
            }
            Ok(())
        }));
        let closed = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, _| {
            dialog_closed(&key, Some(""));
            Ok(())
        }));
        for (event, handler) in [("Page.javascriptDialogOpening", opening), ("Page.javascriptDialogClosed", closed)] {
            let mut token = 0i64;
            // SAFETY: 창 스레드에서 살아 있는 웹뷰에 처리기를 단다. 처리기는 웹뷰가 들고 있으므로 받은 객체는 놓아도 된다.
            let added = unsafe {
                core.GetDevToolsProtocolEventReceiver(&HSTRING::from(event))
                    .and_then(|receiver| receiver.add_DevToolsProtocolEventReceived(&handler, &mut token))
            };
            if let Err(problem) = added {
                crate::log(&format!("확인 창 지켜보기 준비 실패({event}): {}", problem.message()));
            }
        }
        // 이 사건들은 Page 영역을 켜 둔 뒤에 뜬 창에 대해서만 온다.
        post(&core, "Page.enable", json!({}));
    });
    if let Err(problem) = attached {
        crate::log(&format!("확인 창 지켜보기 준비 실패: {problem}"));
    }
}

/// 웹뷰 하나에 CDP 명령을 보내고 답을 받는다.
/// 답은 창 스레드에서 돌아오므로, 창 스레드에서 부르면 안 된다(작업 스레드 전용).
pub fn webview_cdp(webview: &tauri::Webview, method: &str, params: Value) -> Result<Value, String> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows_core::HSTRING;

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let (method, params) = (HSTRING::from(method), HSTRING::from(params.to_string()));
    webview
        .with_webview(move |platform| {
            let tx_done = tx.clone();
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, json| {
                let _ = tx_done.send(status.map(|_| json).map_err(|e| e.message()));
                Ok(())
            }));
            // SAFETY: 창 스레드에서 살아 있는 웹뷰에 대해 부른다. 문자열은 호출 동안 유효하다.
            let started = unsafe {
                platform.controller().CoreWebView2().and_then(|core| core.CallDevToolsProtocolMethod(&method, &params, &handler))
            };
            if let Err(e) = started {
                let _ = tx.send(Err(e.message()));
            }
        })
        .map_err(err)?;
    let text = rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "브라우저가 응답하지 않습니다".to_string())??;
    serde_json::from_str(&text).map_err(err)
}

/// 페이지에서 JS를 실행하고 값을 받는다.
fn eval(target: &Target, script: &str) -> Result<Value, String> {
    let reply = target.cdp(
        "Runtime.evaluate",
        json!({ "expression": script, "returnByValue": true, "awaitPromise": true, "userGesture": true }),
    )?;
    if let Some(problem) = reply.get("exceptionDetails") {
        let detail = problem["exception"]["description"].as_str().or(problem["text"].as_str()).unwrap_or("알 수 없는 오류");
        return Err(format!("스크립트 오류: {detail}"));
    }
    Ok(reply["result"]["value"].clone())
}

/// 페이지를 떠나게 할 수 있는 조작(누르기, 키, 뒤로 가기, 스크립트 실행) 직전에 지금 문서에 심는 표시.
/// 그 조작으로 페이지를 떠나기 시작하면(`beforeunload`, `pagehide`) 그때를 `window.__cypherLeaving`에 적는다.
/// 새 문서가 들어오기 전까지는 옛 문서가 "다 읽힘" 상태라서, 이 표시가 없으면 서버의 답이 느릴 때
/// `wait_loaded`가 옛 페이지를 보고 바로 돌아온다. 같은 문서 안 이동(해시, pushState)에서는 표시가 서지 않는다.
/// 듣는 것은 문서마다 한 번만 단다(조작할 때마다 달면 쌓인다). 뒤로 가기로 되살아난 문서에는 떠날 때 세운
/// 표시가 남아 있으므로 다시 보일 때(`pageshow`) 지운다.
const MARK_LEAVING_JS: &str = r#"(() => {
  window.__cypherLeaving = 0;
  if (window.__cypherWatching) return;
  window.__cypherWatching = true;
  const leave = () => { window.__cypherLeaving = Date.now(); };
  window.addEventListener('beforeunload', leave, true);
  window.addEventListener('pagehide', leave, true);
  window.addEventListener('pageshow', () => { window.__cypherLeaving = 0; }, true);
})()"#;

/// 페이지가 다 읽혔는지. 떠나는 중인 문서(표시가 섰고 아직 새 문서가 들어오지 않았다)는 다 읽힌 것으로 치지 않는다.
/// 표시가 선 지 `limit`(밀리초)가 넘었으면 취소된 이동으로 본다(사용자가 내려받기 링크를 누른 뒤처럼,
/// 떠나려다 만 문서에 표시가 남아 있을 때 다음 조작이 한도까지 기다리지 않게 한다).
const LOADED_JS: &str = r#"((limit) => {
  const left = window.__cypherLeaving;
  return document.readyState === 'complete' && !(left > 0 && Date.now() - left < limit);
})"#;

/// 이동이 취소됐을 때(내려받기, 페이지가 떠나기를 막음) 남은 표시를 지운다.
const CLEAR_LEAVING_JS: &str = "(() => { if (window.__cypherLeaving) window.__cypherLeaving = 0; })()";

/// 페이지를 떠나게 할 수 있는 조작 직전에 부른다. 심지 못해도(문서가 바뀌는 중) 조작은 그대로 한다.
fn mark_leaving(target: &Target) {
    let _ = eval(target, MARK_LEAVING_JS);
}

/// 이동·클릭 뒤 페이지가 다 읽힐 때까지(최대 12초) 기다린다.
/// 조작 직전에 `mark_leaving`으로 표시를 심어 두었으면, 그 조작으로 떠나기 시작한 옛 문서는 다 읽힌 것으로 치지 않고
/// 새 문서(표시가 없다)가 다 읽힐 때까지 기다린다.
fn wait_loaded(target: &Target) {
    let deadline = Instant::now() + MAX_WAIT;
    let loaded = format!("({LOADED_JS})({})", MAX_WAIT.as_millis());
    thread::sleep(Duration::from_millis(300));
    while Instant::now() < deadline {
        // 이동 중에는 실행이 실패할 수 있다. 그때는 다시 물어본다.
        if eval(target, &loaded).is_ok_and(|v| v == true) {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
    // 시간이 다 됐다. 떠나려다 만 것일 수 있으니 표시를 지우고, 지금 상태로 답하게 한다.
    let _ = eval(target, CLEAR_LEAVING_JS);
}

fn text(content: impl Into<String>) -> Vec<Value> {
    vec![json!({ "type": "text", "text": content.into() })]
}

fn summary(target: &Target) -> String {
    let info = eval(target, "({ url: location.href, title: document.title })").unwrap_or_default();
    format!("주소: {}\n제목: {}", info["url"].as_str().unwrap_or("?"), info["title"].as_str().unwrap_or(""))
}

// ───────────────────────── Claude가 쓰는 도구 ─────────────────────────

/// 페이지 글자와, 누르거나 입력할 수 있는 요소 목록을 만든다. 요소는 `window.__cypherRefs`에 번호로 남긴다.
const SNAPSHOT_JS: &str = r#"(() => {
  const visible = (el) => {
    const r = el.getBoundingClientRect(), st = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && st.visibility !== 'hidden' && st.display !== 'none';
  };
  const selector = 'a[href], button, input, select, textarea, summary, [role=button], [role=link], [role=tab], ' +
    '[role=menuitem], [role=checkbox], [role=radio], [role=option], [contenteditable=""], [contenteditable=true]';
  const els = [...document.querySelectorAll(selector)].filter(visible).slice(0, 300);
  window.__cypherRefs = els;
  // SVG 안의 요소는 속성이 글자가 아니라 객체(SVGAnimatedString)이거나 아예 없다(href, innerText 등). 글자일 때만 쓴다.
  const clean = (s) => (typeof s === 'string' ? s : '').trim().replace(/\s+/g, ' ').slice(0, 80);
  const link = (el) => (typeof el.href === 'string' ? el.href : (el.href?.baseVal || el.getAttribute('href') || ''));
  const items = els.map((el, i) => {
    const tag = el.tagName.toLowerCase();
    const secret = el.type === 'password';
    const name = clean(el.getAttribute('aria-label') || (el.innerText ?? el.textContent) || el.placeholder || el.title || el.name || el.alt);
    const kind = tag === 'input' ? `input[${el.type}]` : tag;
    let extra = '';
    if (tag === 'a') extra = ` → ${link(el).slice(0, 100)}`;
    else if ((tag === 'input' || tag === 'textarea') && !secret && el.value) extra = ` = "${clean(el.value)}"`;
    else if (tag === 'select') extra = ` = "${clean(el.selectedOptions[0]?.text)}"`;
    if (el.type === 'checkbox' || el.type === 'radio') extra += el.checked ? ' (선택됨)' : ' (선택 안 됨)';
    return `[${i}] ${kind} "${name}"${extra}`;
  });
  const body = (document.body?.innerText || '').replace(/\n{3,}/g, '\n\n');
  return { url: location.href, title: document.title, text: body, elements: items };
})()"#;

fn tool_read(target: &Target) -> ToolResult {
    wait_loaded(target);
    let snap = eval(target, SNAPSHOT_JS)?;
    let mut body: String = snap["text"].as_str().unwrap_or("").chars().take(MAX_TEXT).collect();
    if snap["text"].as_str().is_some_and(|t| t.chars().count() > MAX_TEXT) {
        body.push_str("\n… (이하 생략. 아래로 스크롤한 뒤 다시 읽거나 browser_eval로 필요한 부분만 읽으세요)");
    }
    let elements: Vec<&str> = snap["elements"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    Ok(text(format!(
        "주소: {}\n제목: {}\n\n── 누르거나 입력할 수 있는 요소 (번호는 browser_click / browser_type의 ref) ──\n{}\n\n── 페이지 글자 ──\n{}",
        snap["url"].as_str().unwrap_or("?"),
        snap["title"].as_str().unwrap_or(""),
        elements.join("\n"),
        body
    )))
}

fn tool_navigate(target: &Target, args: &Value) -> ToolResult {
    let url = args["url"].as_str().ok_or("url이 필요합니다")?;
    let address = normalize_url(url)?.to_string();
    target.cdp("Page.navigate", json!({ "url": address }))?;
    wait_loaded(target);
    Ok(text(summary(target)))
}

/// 번호로 가리킨 요소의 화면 가운데 좌표(CSS 픽셀). 화면 밖이면 먼저 보이게 스크롤한다.
/// 스크롤은 바로 끝나게 한다(`instant`): 부드럽게 스크롤하는 페이지에서는 움직이기 시작만 한 채로 옛 좌표를 읽게 된다.
fn element_center(target: &Target, reference: u64) -> Result<(f64, f64), String> {
    let script = format!(
        "(() => {{ const el = window.__cypherRefs?.[{reference}]; if (!el || !el.isConnected) return null; \
         el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }}); const r = el.getBoundingClientRect(); \
         return {{ x: r.left + r.width / 2, y: r.top + r.height / 2 }}; }})()"
    );
    let point = eval(target, &script)?;
    match (point["x"].as_f64(), point["y"].as_f64()) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(format!("ref {reference}에 해당하는 요소가 없습니다. browser_read를 다시 불러 번호를 새로 받으세요.")),
    }
}

/// 실제 마우스처럼 그 좌표를 누른다.
fn click_point(target: &Target, x: f64, y: f64) -> Result<(), String> {
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        let buttons = if kind == "mousePressed" { 1 } else { 0 };
        target.cdp(
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": x, "y": y, "button": if kind == "mouseMoved" { "none" } else { "left" },
                    "buttons": buttons, "clickCount": 1 }),
        )?;
    }
    Ok(())
}

fn tool_click(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let reference = args["ref"].as_u64().ok_or("ref(요소 번호)가 필요합니다")?;
    let (x, y) = element_center(target, reference)?;
    mark_leaving(target);
    click_point(target, x, y)?;
    wait_loaded(target);
    Ok(text(format!("눌렀습니다.\n{}", summary(target))))
}

fn tool_click_at(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let (Some(x), Some(y)) = (args["x"].as_f64(), args["y"].as_f64()) else {
        return Err("x, y 좌표가 필요합니다".into());
    };
    mark_leaving(target);
    click_point(target, x, y)?;
    wait_loaded(target);
    Ok(text(format!("({x}, {y})를 눌렀습니다.\n{}", summary(target))))
}

/// 이름으로 받은 키를 누른다.
fn press_key(target: &Target, name: &str) -> Result<(), String> {
    let (key, code, vk, typed): (&str, &str, u32, Option<&str>) = match name.to_ascii_lowercase().as_str() {
        "enter" => ("Enter", "Enter", 13, Some("\r")),
        "tab" => ("Tab", "Tab", 9, None),
        "escape" | "esc" => ("Escape", "Escape", 27, None),
        "backspace" => ("Backspace", "Backspace", 8, None),
        "delete" => ("Delete", "Delete", 46, None),
        "space" => (" ", "Space", 32, Some(" ")),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, None),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, None),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, None),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, None),
        "pageup" => ("PageUp", "PageUp", 33, None),
        "pagedown" => ("PageDown", "PageDown", 34, None),
        "home" => ("Home", "Home", 36, None),
        "end" => ("End", "End", 35, None),
        other => return Err(format!("지원하지 않는 키입니다: {other}")),
    };
    let mut down = json!({ "type": if typed.is_some() { "keyDown" } else { "rawKeyDown" }, "key": key, "code": code,
                           "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk });
    if let Some(typed) = typed {
        down["text"] = json!(typed);
    }
    target.cdp("Input.dispatchKeyEvent", down)?;
    target.cdp(
        "Input.dispatchKeyEvent",
        json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk }),
    )?;
    Ok(())
}

fn tool_type(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let typed = args["text"].as_str().ok_or("text가 필요합니다")?;
    if let Some(reference) = args["ref"].as_u64() {
        // 요소를 실제로 눌러 초점을 준 뒤, 들어 있던 내용을 전부 고른다(새 글자로 바뀌게).
        let (x, y) = element_center(target, reference)?;
        click_point(target, x, y)?;
        eval(
            target,
            &format!(
                "(() => {{ const el = window.__cypherRefs?.[{reference}]; if (!el) return; el.focus(); \
                 if (el.select) el.select(); else document.execCommand('selectAll'); }})()"
            ),
        )?;
    }
    target.cdp("Input.insertText", json!({ "text": typed }))?;
    if args["submit"].as_bool().unwrap_or(false) {
        mark_leaving(target);
        press_key(target, "enter")?;
        wait_loaded(target);
    }
    Ok(text(format!("입력했습니다.\n{}", summary(target))))
}

fn tool_key(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let key = args["key"].as_str().ok_or("key가 필요합니다")?;
    mark_leaving(target);
    press_key(target, key)?;
    wait_loaded(target);
    Ok(text(format!("키를 눌렀습니다.\n{}", summary(target))))
}

fn tool_scroll(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let dy = args["dy"].as_f64().unwrap_or(600.0);
    // 부드럽게 스크롤하는 페이지에서도 바로 끝나게 한다(그러지 않으면 움직이기 전의 위치를 돌려준다).
    let position =
        eval(target, &format!("(window.scrollBy({{ top: {dy}, left: 0, behavior: 'instant' }}), Math.round(window.scrollY))"))?;
    Ok(text(format!("스크롤했습니다. 지금 세로 위치: {position}")))
}

/// 보이는 화면을 그림으로 준다. 그림의 픽셀 좌표가 곧 `browser_click_at`의 좌표가 되도록
/// 화면 배율만큼 줄여서 찍는다.
fn tool_screenshot(target: &Target) -> ToolResult {
    wait_loaded(target);
    let view = eval(target, "({ w: innerWidth, h: innerHeight, dpr: devicePixelRatio, x: scrollX, y: scrollY })")?;
    let (w, h) = (view["w"].as_f64().unwrap_or(1280.0), view["h"].as_f64().unwrap_or(800.0));
    let dpr = view["dpr"].as_f64().unwrap_or(1.0).max(0.1);
    let shot = target.cdp(
        "Page.captureScreenshot",
        json!({ "format": "jpeg", "quality": 70,
                "clip": { "x": view["x"], "y": view["y"], "width": w, "height": h, "scale": 1.0 / dpr } }),
    )?;
    let data = shot["data"].as_str().ok_or("화면을 찍지 못했습니다")?;
    Ok(vec![
        json!({ "type": "image", "data": data, "mimeType": "image/jpeg" }),
        json!({ "type": "text", "text": format!(
            "보이는 화면 {w}×{h}. 이 그림의 픽셀 좌표를 그대로 browser_click_at(x, y)에 쓰면 됩니다.\n{}", summary(target)) }),
    ])
}

fn tool_eval(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let script = args["script"].as_str().ok_or("script가 필요합니다")?;
    // 스크립트가 페이지를 옮기면(location 바꾸기, 양식 보내기) 다음 도구가 새 페이지를 기다리게 한다.
    mark_leaving(target);
    let value = eval(target, script)?;
    let mut out = serde_json::to_string_pretty(&value).unwrap_or_default();
    if out.len() > 2 * MAX_TEXT {
        let mut cut = 2 * MAX_TEXT;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n… (이하 생략)");
    }
    Ok(text(out))
}

fn tool_back(target: &Target) -> ToolResult {
    wait_loaded(target);
    mark_leaving(target);
    eval(target, "history.back()")?;
    wait_loaded(target);
    Ok(text(summary(target)))
}

/// Claude에게 알려 줄 도구 목록.
pub fn tools() -> Value {
    let none = json!({ "type": "object", "properties": {} });
    json!([
        { "name": "browser_read", "inputSchema": none,
          "description": "이 세션과 같은 그룹에 있는 Cypher 브라우저(사이드바의 사이트 항목)에 지금 열려 있는 페이지를 읽는다(사용자가 로그인해 둔 상태 그대로). 주소, 제목, 누르거나 입력할 수 있는 요소 목록(번호 포함), 페이지 글자를 돌려준다. 조작하기 전에 먼저 부른다." },
        { "name": "browser_navigate", "description": "브라우저를 그 주소로 이동시킨다. 같은 그룹에 브라우저(사이트 항목)가 없으면 새로 만들어 연다.",
          "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] } },
        { "name": "browser_click", "description": "browser_read가 준 번호(ref)의 요소를 누른다.",
          "inputSchema": { "type": "object", "properties": { "ref": { "type": "integer" } }, "required": ["ref"] } },
        { "name": "browser_type", "description": "글자를 입력한다. ref를 주면 그 입력칸의 내용을 바꾸고, 생략하면 지금 초점이 있는 곳에 입력한다. submit이 참이면 입력 뒤 Enter를 누른다.",
          "inputSchema": { "type": "object", "properties": { "ref": { "type": "integer" }, "text": { "type": "string" }, "submit": { "type": "boolean" } }, "required": ["text"] } },
        { "name": "browser_key", "description": "키를 누른다: Enter, Tab, Escape, Backspace, Delete, Space, ArrowUp/Down/Left/Right, PageUp, PageDown, Home, End.",
          "inputSchema": { "type": "object", "properties": { "key": { "type": "string" } }, "required": ["key"] } },
        { "name": "browser_scroll", "description": "세로로 스크롤한다. dy는 픽셀(양수는 아래로, 기본 600).",
          "inputSchema": { "type": "object", "properties": { "dy": { "type": "number" } } } },
        { "name": "browser_screenshot", "inputSchema": none,
          "description": "보이는 화면을 그림으로 받는다. 글자로 읽기 어려운 화면(그림, 캔버스, 복잡한 배치)을 눈으로 보고 판단할 때 쓴다. 그림의 픽셀 좌표는 browser_click_at에 그대로 쓸 수 있다." },
        { "name": "browser_click_at", "description": "화면 좌표(browser_screenshot 그림의 픽셀 좌표)를 누른다. 번호로 가리킬 수 없는 것을 누를 때 쓴다.",
          "inputSchema": { "type": "object", "properties": { "x": { "type": "number" }, "y": { "type": "number" } }, "required": ["x", "y"] } },
        { "name": "browser_back", "inputSchema": none, "description": "이전 페이지로 돌아간다." },
        { "name": "browser_eval", "description": "페이지에서 JavaScript 식을 실행하고 값을 받는다. 다른 도구로 안 되는 읽기·조작에만 쓴다.",
          "inputSchema": { "type": "object", "properties": { "script": { "type": "string" } }, "required": ["script"] } },
        { "name": "browser_tabs", "inputSchema": none,
          "description": "이 세션과 같은 그룹에 있는 브라우저(사이트 항목)의 목록과, 그중 지금 조작 대상인 것을 알려 준다. 브라우저가 여러 개일 때 어느 것을 조작할지 확인하는 데 쓴다." },
        { "name": "browser_select", "description": "조작 대상을 browser_tabs가 준 번호(id)의 브라우저로 바꾼다.",
          "inputSchema": { "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"] } },
        { "name": "browser_dialog",
          "description": "페이지가 다음에 띄우는 확인 창(alert·confirm·prompt, 페이지를 나갈지 묻는 창) 하나에 줄 답을 정해 둔다. 정해 두지 않으면 Cypher가 조작 중에 뜬 창을 바로 닫는다(alert는 확인, confirm·prompt는 취소, 나가기 확인은 나가기). 닫은 창의 종류·글·준 답은 그 도구의 답 끝에 적힌다. confirm을 수락하거나 prompt에 값을 넣어야 하면 이 도구로 답을 정한 뒤 그 창을 띄우는 조작을 다시 한다. 정해 둔 답은 한 번만 쓰이고, 다음 조작 한 번이 끝나면 쓰이지 않았어도 지워진다.",
          "inputSchema": { "type": "object", "properties": {
              "accept": { "type": "boolean", "description": "true면 수락(확인·나가기), false면 취소(머무르기)" },
              "text": { "type": "string", "description": "입력 창(prompt)을 수락할 때 넣을 값. 생략하면 들어 있던 값 그대로." } },
            "required": ["accept"] } },
    ])
}

fn run(target: &Target, name: &str, args: &Value) -> ToolResult {
    match name {
        "browser_read" => tool_read(target),
        "browser_navigate" => tool_navigate(target, args),
        "browser_click" => tool_click(target, args),
        "browser_click_at" => tool_click_at(target, args),
        "browser_type" => tool_type(target, args),
        "browser_key" => tool_key(target, args),
        "browser_scroll" => tool_scroll(target, args),
        "browser_screenshot" => tool_screenshot(target),
        "browser_back" => tool_back(target),
        "browser_eval" => tool_eval(target, args),
        other => Err(format!("없는 도구입니다: {other}")),
    }
}

/// 페이지를 조작하는 도구인지(읽기·스크린샷·스크롤은 아니다). `browser_dialog`로 정해 둔 답은 조작 한 번이 끝나면 지운다.
fn operates(name: &str) -> bool {
    matches!(
        name,
        "browser_navigate" | "browser_click" | "browser_click_at" | "browser_type" | "browser_key" | "browser_back" | "browser_eval"
    )
}

/// 닫은 확인 창을 알리는 글을 도구의 답 끝에 붙인다. 오류로 끝났어도 붙인다(Claude가 까닭을 알 수 있게).
fn with_notes(outcome: ToolResult, notes: &[String]) -> ToolResult {
    if notes.is_empty() {
        return outcome;
    }
    let notes = notes.join("\n");
    match outcome {
        Ok(mut content) => {
            content.push(json!({ "type": "text", "text": notes }));
            Ok(content)
        }
        Err(problem) => Err(format!("{problem}\n{notes}")),
    }
}

/// 도구 하나를 그 브라우저에서 실행한다. 실행하는 동안 페이지가 띄우는 확인 창은 바로 닫히고(`dialog_opened`),
/// 닫은 사실이 답 끝에 붙는다. `dialogs`는 그 브라우저의 확인 창 상태를 가리키는 열쇠.
fn operate(target: &Target, dialogs: &str, name: &str, args: &Value) -> ToolResult {
    let using = Using::begin(dialogs);
    // 도구가 쓰고 있지 않을 때 떠서 그대로 둔 창이 있으면 닫고 시작한다(두면 페이지가 멈춰 있어 도구도 멈춘다).
    if let Some((dialog, answer, planned)) = with_dialogs(dialogs, |state| state.take_open(Instant::now())) {
        match target.answer(&dialog, &answer) {
            Ok(()) => {
                with_dialogs(dialogs, |state| state.notes.push(dialog_note(&dialog, &answer, planned, true)));
                if let Some(script) = dialog_follow_up(&dialog, &answer) {
                    let _ = target.cdp("Runtime.evaluate", script);
                }
            }
            // 그 창이 이미 없다(탭이나 웹뷰가 닫혔다). 정해 둔 답을 썼다면 되돌려 놓는다.
            Err(problem) => {
                crate::log(&format!("떠 있던 확인 창을 닫지 못했습니다: {problem}"));
                if planned {
                    with_dialogs(dialogs, |state| state.planned = Some((answer, Instant::now())));
                }
            }
        }
    }
    let outcome = run(target, name, args);
    with_notes(outcome, &using.finish(operates(name)))
}

/// `browser_dialog`: 다음에 뜨는 확인 창 하나에 줄 답을 정해 둔다.
fn plan_dialog(dialogs: &str, args: &Value) -> ToolResult {
    let accept = args["accept"].as_bool().ok_or("accept(true 또는 false)가 필요합니다")?;
    let answer = Answer { accept, text: args["text"].as_str().map(str::to_string) };
    let waiting = with_dialogs(dialogs, |state| {
        state.planned = Some((answer.clone(), Instant::now()));
        state.open.clone()
    });
    Ok(text(plan_text(&answer, waiting.as_ref())))
}

/// `browser_dialog`의 답 글. `waiting`은 지금 떠 있는 창(도구가 쓰고 있지 않을 때 뜬 것).
fn plan_text(answer: &Answer, waiting: Option<&Dialog>) -> String {
    let what = match (answer.accept, &answer.text) {
        (true, Some(typed)) => {
            let typed: String = typed.chars().take(MAX_DIALOG_TEXT).collect();
            format!("수락합니다(입력 창이면 입력값: \"{typed}\", 나가기 확인 창이면 나가기)")
        }
        (true, None) => "수락합니다(입력 창이면 들어 있던 값 그대로, 나가기 확인 창이면 나가기)".to_string(),
        (false, _) => "취소합니다(나가기 확인 창이면 머무르기)".to_string(),
    };
    let now = match waiting {
        Some(dialog) => format!(" 지금 떠 있는 창({})에는 다음 도구를 부를 때 이 답을 줍니다.", dialog.kind),
        None => String::new(),
    };
    format!(
        "다음에 뜨는 확인 창 하나를 {what}. 이 답은 한 번만 쓰이고, 쓰이지 않아도 다음 조작(이동·누르기·입력·키·뒤로 가기·스크립트 실행) \
         한 번이 끝나거나 5분이 지나면 지워집니다. 이제 그 창을 띄우는 조작을 하세요.{now}"
    )
}

/// "chrome" 엔진에서, 그 프로필(그룹)의 브라우저에 도구 하나를 실행한다.
pub(crate) fn on_chrome(profile: &str, name: &str, args: &Value) -> ToolResult {
    let dialogs = chrome_dialogs(profile);
    if name == "browser_dialog" {
        return plan_dialog(&dialogs, args);
    }
    operate(&Target::Chrome(profile.to_string()), &dialogs, name, args)
}

/// 도구 하나를 실행한다. `session`은 도구를 부른 Claude 세션의 번호(모르면 None).
pub fn call(app: &AppHandle, session: Option<u32>, name: &str, args: &Value) -> ToolResult {
    let choosing = matches!(name, "browser_tabs" | "browser_select");
    if chrome_engine(app) {
        // 이 엔진의 브라우저는 사이드바 항목이 아니라 그룹마다 하나씩 따로 띄우는 Chrome이다. 부른 세션의 그룹이 쓰는
        // 프로필을 화면에 물어 그 브라우저만 조작한다(부른 세션을 알 수 없거나 그룹에 들지 않은 세션이면 화면이 거절한다).
        let scope = crate::mcp::ask_ui(app, "chrome_scope", json!({ "session": session }))?;
        let profile = crate::site::profile_key(scope["profile"].as_str().unwrap_or(""))?.to_string();
        if choosing {
            return Ok(text("브라우저 엔진이 chrome으로 설정되어 있어 이 그룹의 브라우저는 하나뿐입니다(이 그룹 전용 프로필의 Chrome)."));
        }
        // 확인 창에 줄 답을 정해 두는 것은 브라우저가 떠 있지 않아도 된다.
        if name != "browser_dialog" && !crate::chrome::is_open(&profile) {
            open_chrome(app, &profile, None)?;
            thread::sleep(Duration::from_millis(800));
        }
        return on_chrome(&profile, name, args);
    }
    if choosing {
        // 고를 브라우저의 번호는 `site`로 넘긴다(`id`는 화면에 묻는 요청의 번호로 쓰인다).
        let reply = crate::mcp::ask_ui(app, name, json!({ "session": session, "site": args["id"] }))?;
        return Ok(text(reply["text"].as_str().unwrap_or("")));
    }
    // 어느 사이트 항목을 조작할지는 화면이 정한다. 이동하라는 요청이면, 그룹에 브라우저가 없을 때 새로 만든다.
    let found = crate::mcp::ask_ui(app, "browser_target", json!({ "session": session, "create": name == "browser_navigate" }))?;
    let id = found["key"].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("조작할 브라우저를 찾지 못했습니다")?;
    let profile = found["profile"].as_str().unwrap_or("");
    let dialogs = site_dialogs(id, profile);
    let mut content = if name == "browser_dialog" {
        // 답을 정해 두기만 한다(웹뷰가 아직 없어도 된다).
        plan_dialog(&dialogs, args)?
    } else {
        // 그 사이트 항목의 그룹이 쓰는 프로필로 웹뷰를 만든다(화면이 알려 준다).
        let webview = crate::site::ensure(app, id, found["url"].as_str().unwrap_or("about:blank"), profile)?;
        // 방금 받은 그 웹뷰만 조작한다(이름으로 다시 찾으면, 그사이 항목이 옮겨져 다시 만들어진 다른 프로필의 웹뷰일 수 있다).
        crate::site::with_shown_on(&webview, |webview| operate(&Target::Site(Box::new(webview.clone())), &dialogs, name, args))?
    };
    if let Some(note) = found["note"].as_str().filter(|note| !note.is_empty()) {
        content.push(json!({ "type": "text", "text": note }));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 주소창_입력을_주소로_바꾼다() {
        let url = |s: &str| normalize_url(s).unwrap().to_string();
        assert_eq!(url("https://example.com/a?b=1"), "https://example.com/a?b=1");
        assert_eq!(url("example.com/path"), "https://example.com/path");
        assert_eq!(url("localhost:3000"), "http://localhost:3000/");
        assert_eq!(url(""), "about:blank");
        assert_eq!(url("러스트 터미널"), "https://www.google.com/search?q=%EB%9F%AC%EC%8A%A4%ED%8A%B8+%ED%84%B0%EB%AF%B8%EB%84%90");
    }

    #[test]
    fn 웹_주소가_아닌_것은_열지_않고_검색한다() {
        for input in ["javascript:alert(1)", "file:///C:/Windows/win.ini", "data:text/html,x", "tauri://localhost/index.html"] {
            let url = normalize_url(input).unwrap();
            assert_eq!(url.host_str(), Some("www.google.com"), "{input}");
        }
    }

    #[test]
    fn 도구_목록에_이름과_입력_형식이_있다() {
        let list = tools();
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 13);
        assert!(list.iter().all(|t| t["name"].is_string() && t["description"].is_string() && t["inputSchema"]["type"] == "object"));
    }

    fn dialog(kind: &str, message: &str) -> Dialog {
        Dialog { kind: kind.into(), message: message.into(), preset: String::new(), tab: String::new() }
    }

    fn accept(text: Option<&str>) -> Answer {
        Answer { accept: true, text: text.map(str::to_string) }
    }

    #[test]
    fn 확인_창의_기본_답과_넘기는_인자() {
        // 알림은 확인, 확인·입력 창은 취소, 나가기 확인은 나가기.
        assert!(default_answer("alert").accept);
        assert!(!default_answer("confirm").accept);
        assert!(!default_answer("prompt").accept);
        assert!(default_answer("beforeunload").accept);
        assert!(!default_answer("모르는 종류").accept);

        // 사건의 내용에서 종류·글·미리 든 값을 읽는다.
        let event = json!({ "type": "prompt", "message": "이름?", "defaultPrompt": "홍길동", "url": "https://example.com/" });
        let asked = Dialog::from_event(&event, "S1");
        assert_eq!(asked, Dialog { kind: "prompt".into(), message: "이름?".into(), preset: "홍길동".into(), tab: "S1".into() });

        // 입력값은 입력 창을 수락할 때만 넘긴다. 정한 값이 없으면 미리 든 값 그대로.
        assert_eq!(dialog_params(&asked, &accept(Some("김철수"))), json!({ "accept": true, "promptText": "김철수" }));
        assert_eq!(dialog_params(&asked, &accept(None)), json!({ "accept": true, "promptText": "홍길동" }));
        assert_eq!(dialog_params(&asked, &default_answer("prompt")), json!({ "accept": false }));
        assert_eq!(dialog_params(&dialog("confirm", "지울까요?"), &accept(Some("쓰이지 않음"))), json!({ "accept": true }));

        // 나가지 않기로 답했을 때만 떠나는 중 표시를 지운다.
        let stay = Answer { accept: false, text: None };
        assert!(dialog_follow_up(&dialog("beforeunload", ""), &stay).is_some());
        assert!(dialog_follow_up(&dialog("beforeunload", ""), &accept(None)).is_none());
        assert!(dialog_follow_up(&dialog("confirm", ""), &stay).is_none());
    }

    #[test]
    fn 조작_중에_뜬_창만_닫는다() {
        let now = Instant::now();
        let mut state = Dialogs::default();
        // 도구가 쓰고 있지 않을 때 뜬 창은 그대로 둔다(사용자가 답한다).
        assert_eq!(state.opened(dialog("confirm", "지울까요?"), now), None);
        assert_eq!(state.open, Some(dialog("confirm", "지울까요?")));
        assert!(state.notes.is_empty());
        // 사용자가 답해 닫히면 잊는다. 다른 탭의 창이 닫힌 것은 상관없다.
        state.closed(Some("다른 탭"));
        assert!(state.open.is_some());
        state.closed(Some(""));
        assert!(state.open.is_none() && state.idle());

        // 도구가 쓰는 중에 뜬 창은 기본 답으로 바로 닫고, 닫은 사실을 적어 둔다.
        state.using += 1;
        assert_eq!(state.opened(dialog("confirm", "지울까요?"), now), Some(default_answer("confirm")));
        assert_eq!(state.opened(dialog("alert", "지웠습니다"), now), Some(default_answer("alert")));
        assert!(state.open.is_none());
        let notes = state.finish(true);
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("(confirm): \"지울까요?\" → 취소했습니다."), "{}", notes[0]);
        assert!(notes[1].contains("(alert): \"지웠습니다\" → 확인을 눌렀습니다."), "{}", notes[1]);
        assert!(state.idle());

        // 떠 있는 채로 도구가 시작되면 그 창을 꺼내 닫는다.
        assert_eq!(state.opened(dialog("prompt", "이름?"), now), None);
        state.using += 1;
        assert_eq!(state.take_open(now), Some((dialog("prompt", "이름?"), default_answer("prompt"), false)));
        assert_eq!(state.take_open(now), None);
    }

    #[test]
    fn 정해_둔_답은_한_번만_쓰인다() {
        let now = Instant::now();
        let mut state = Dialogs { using: 1, planned: Some((accept(Some("김철수")), now)), ..Default::default() };
        // 처음 뜬 창에 정해 둔 답을 쓰고, 그다음 창은 기본 답이다.
        assert_eq!(state.opened(dialog("prompt", "이름?"), now), Some(accept(Some("김철수"))));
        assert_eq!(state.opened(dialog("confirm", "정말?"), now), Some(default_answer("confirm")));
        let notes = state.finish(true);
        assert!(notes[0].ends_with("→ 수락했습니다(입력값: \"김철수\"). (browser_dialog로 정해 둔 답입니다)"), "{}", notes[0]);
        assert!(notes[1].contains("→ 취소했습니다. 수락하려면 browser_dialog로"), "{}", notes[1]);

        // 쓰이지 않았어도 조작하는 도구 한 번이 끝나면 지운다. 읽기만 하는 도구는 지우지 않는다.
        state.planned = Some((accept(None), now));
        state.using = 1;
        assert!(state.finish(false).is_empty());
        assert!(state.planned.is_some());
        state.using = 1;
        state.finish(true);
        assert!(state.planned.is_none());

        // 오래된 답은 쓰지 않는다.
        state.planned = Some((accept(None), now));
        state.using = 1;
        assert_eq!(state.opened(dialog("confirm", "지울까요?"), now + PLAN_LIFE), Some(default_answer("confirm")));
        assert!(state.planned.is_none());

        // 떠 있던 창에도 정해 둔 답을 쓴다.
        let mut state = Dialogs { open: Some(dialog("confirm", "지울까요?")), planned: Some((accept(None), now)), ..Default::default() };
        assert_eq!(state.take_open(now), Some((dialog("confirm", "지울까요?"), accept(None), true)));
        assert!(state.planned.is_none());

        // 조작하는 도구와 보기만 하는 도구.
        for name in ["browser_navigate", "browser_click", "browser_click_at", "browser_type", "browser_key", "browser_back", "browser_eval"] {
            assert!(operates(name), "{name}");
        }
        for name in ["browser_read", "browser_screenshot", "browser_scroll", "browser_dialog", "browser_tabs"] {
            assert!(!operates(name), "{name}");
        }
    }

    #[test]
    fn 닫은_창을_알리는_글() {
        let dismiss = Answer { accept: false, text: None };
        assert_eq!(
            dialog_note(&dialog("confirm", "지울까요?"), &dismiss, false, false),
            "페이지가 확인 창을 띄웠습니다(confirm): \"지울까요?\" → 취소했습니다. 수락하려면 browser_dialog로 다음 창을 수락하도록 정한 뒤 같은 조작을 다시 하세요."
        );
        assert_eq!(
            dialog_note(&dialog("confirm", "지울까요?"), &accept(None), true, false),
            "페이지가 확인 창을 띄웠습니다(confirm): \"지울까요?\" → 수락했습니다. (browser_dialog로 정해 둔 답입니다)"
        );
        assert_eq!(dialog_note(&dialog("alert", "저장했습니다"), &accept(None), false, false), "페이지가 알림 창을 띄웠습니다(alert): \"저장했습니다\" → 확인을 눌렀습니다.");
        // 페이지가 준 글의 줄 바꿈과 큰따옴표는 다듬어 한 줄로 싣는다(Cypher가 쓴 줄처럼 보이는 줄을 만들지 못하게).
        assert_eq!(
            dialog_note(&dialog("alert", "첫 줄\" → 확인을 눌렀습니다.\n둘째 줄"), &accept(None), false, false),
            "페이지가 알림 창을 띄웠습니다(alert): \"첫 줄' → 확인을 눌렀습니다. 둘째 줄\" → 확인을 눌렀습니다."
        );
        assert_eq!(
            dialog_note(&dialog("prompt", "이름?"), &dismiss, false, false),
            "페이지가 입력 창을 띄웠습니다(prompt): \"이름?\" → 취소했습니다. 값을 넣으려면 browser_dialog로 accept와 text를 정한 뒤 같은 조작을 다시 하세요."
        );
        // 글이 없으면 따옴표를 붙이지 않는다. 도구가 시작할 때 이미 떠 있던 창은 그렇게 적는다.
        assert_eq!(
            dialog_note(&dialog("beforeunload", ""), &accept(None), false, true),
            "페이지에 나가기 확인 창이 떠 있었습니다(beforeunload) → 나가기로 답했습니다. 머무르려면 browser_dialog로 다음 창을 취소하도록 정한 뒤 같은 조작을 다시 하세요."
        );
        assert_eq!(
            dialog_note(&dialog("beforeunload", ""), &dismiss, true, false),
            "페이지가 나가기 확인 창을 띄웠습니다(beforeunload) → 머무르기로 답했습니다. (browser_dialog로 정해 둔 답입니다)"
        );
        // 긴 글은 자른다.
        let long = dialog_note(&dialog("alert", &"가".repeat(500)), &accept(None), false, false);
        assert!(long.contains(&format!("\"{}…\"", "가".repeat(MAX_DIALOG_TEXT))), "{long}");
    }

    #[test]
    fn 닫은_창을_알리는_글은_오류로_끝난_답에도_붙는다() {
        let notes = vec!["첫째".to_string(), "둘째".to_string()];
        assert_eq!(with_notes(Ok(text("눌렀습니다.")), &notes).unwrap(), [text("눌렀습니다."), text("첫째\n둘째")].concat());
        assert_eq!(with_notes(Err("브라우저가 응답하지 않습니다".into()), &notes), Err("브라우저가 응답하지 않습니다\n첫째\n둘째".to_string()));
        assert_eq!(with_notes(Ok(text("눌렀습니다.")), &[]).unwrap(), text("눌렀습니다."));

        // browser_dialog의 답 글.
        let planned = plan_text(&accept(Some("김철수")), None);
        assert!(planned.starts_with("다음에 뜨는 확인 창 하나를 수락합니다(입력 창이면 입력값: \"김철수\", 나가기 확인 창이면 나가기)."), "{planned}");
        let planned = plan_text(&Answer { accept: false, text: None }, Some(&dialog("confirm", "지울까요?")));
        assert!(planned.starts_with("다음에 뜨는 확인 창 하나를 취소합니다(나가기 확인 창이면 머무르기)."), "{planned}");
        assert!(planned.ends_with("지금 떠 있는 창(confirm)에는 다음 도구를 부를 때 이 답을 줍니다."), "{planned}");
    }
}
