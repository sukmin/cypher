//! 내장 브라우저: 주소 줄이 달린 별도 창과, Claude 세션이 쓰는 조작 도구.
//!
//! 창 하나에 웹뷰가 둘 들어간다. 위는 주소 줄(앱 화면), 아래는 웹 페이지.
//! 웹 페이지 쪽 웹뷰에는 어떤 앱 명령 권한도 주지 않는다(capabilities 참고).
//! 조작은 포트를 열지 않고 WebView2의 개발자 도구 프로토콜(CDP)을 앱 안에서 직접 부른다.
//!
//! 설정에서 엔진을 "chrome"으로 바꾸면 이 창 대신 설치된 Chrome·Edge를 띄워 조작한다(`chrome.rs`).
//! Claude가 쓰는 도구는 두 엔진에서 똑같이 동작한다.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, WebviewBuilder};
use tauri::window::WindowBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Url, WebviewUrl, WindowEvent};

const WINDOW: &str = "browser";
const BAR: &str = "browser-bar";
const PAGE: &str = "browser-page";
/// 주소 줄 웹뷰의 높이(화면 배율 100% 기준). 즐겨찾기 줄이 있으면 화면 쪽에서 키운다.
static BAR_HEIGHT: AtomicU32 = AtomicU32::new(44);

fn bar_height() -> f64 {
    f64::from(BAR_HEIGHT.load(Ordering::Relaxed))
}
const MAX_TEXT: usize = 15_000;

type ToolResult = Result<Vec<Value>, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ───────────────────────── 창 ─────────────────────────

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

/// 주소 줄과 페이지 웹뷰를 창 크기에 맞춘다.
fn layout(app: &AppHandle) {
    let (Some(window), Some(bar), Some(page)) = (app.get_window(WINDOW), app.get_webview(BAR), app.get_webview(PAGE))
    else {
        return;
    };
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else { return };
    let bar_height = (bar_height() * scale).round() as u32;
    let _ = bar.set_position(PhysicalPosition::new(0, 0));
    let _ = bar.set_size(PhysicalSize::new(size.width, bar_height));
    let _ = page.set_position(PhysicalPosition::new(0, bar_height as i32));
    let _ = page.set_size(PhysicalSize::new(size.width, size.height.saturating_sub(bar_height)));
}

/// 설정에서 고른 엔진이 "chrome"(설치된 브라우저를 띄워 조작)인지. 아니면 내장 웹뷰 창이다.
fn chrome_engine(app: &AppHandle) -> bool {
    app.state::<crate::AppState>().config.browser.engine.eq_ignore_ascii_case("chrome")
}

/// 브라우저가 떠 있는지.
fn is_open(app: &AppHandle) -> bool {
    if chrome_engine(app) { crate::chrome::is_open() } else { app.get_webview(PAGE).is_some() }
}

/// 브라우저 창을 띄운다. 이미 떠 있으면 앞으로 가져오고, 주소가 있으면 그리로 간다.
pub fn open(app: &AppHandle, url: Option<&str>) -> Result<(), String> {
    open_engine(app, url)?;
    // 엔진의 Chromium 버전이 뒤처져 있지 않은지 가끔 뒤에서 확인한다(단추로 열든 Claude가 열든).
    crate::engine::check_in_background(app);
    Ok(())
}

fn open_engine(app: &AppHandle, url: Option<&str>) -> Result<(), String> {
    if chrome_engine(app) {
        let settings = &app.state::<crate::AppState>().config.browser;
        return crate::chrome::open(&settings.chrome_path, &settings.home, url);
    }
    if let Some(window) = app.get_window(WINDOW) {
        if let Some(page) = app.get_webview(PAGE) {
            let _ = window.unminimize();
            let _ = window.set_focus();
            if let Some(url) = url {
                page.navigate(normalize_url(url)?).map_err(err)?;
            }
            return Ok(());
        }
        // 창만 있고 페이지가 없는 반쯤 만들어진 상태: 치우고 새로 만든다.
        let _ = window.destroy();
        thread::sleep(Duration::from_millis(300));
    }
    let home = app.state::<crate::AppState>().config.browser.home.clone();
    let start = normalize_url(url.unwrap_or(&home))?;

    let window = WindowBuilder::new(app, WINDOW)
        .title(concat!("Cypher 브라우저 ", env!("CARGO_PKG_VERSION")))
        .inner_size(1280.0, 860.0)
        .build()
        .map_err(err)?;
    if let Err(problem) = add_webviews(app, &window, start) {
        // 실패한 창을 남겨 두면 다음 시도가 "이미 열려 있음"으로 넘어가 버린다.
        let _ = window.destroy();
        return Err(problem);
    }
    // 브라우저 창에도 자석을 단다(창 스레드에서 달아야 한다).
    let settings = &app.state::<crate::AppState>().config.window;
    if settings.magnet {
        if let Ok(hwnd) = window.hwnd() {
            let (hwnd, distance) = (hwnd.0 as isize, settings.magnet_distance);
            let _ = app.run_on_main_thread(move || crate::magnet::attach(hwnd, distance));
        }
    }
    Ok(())
}

/// 브라우저 창에 주소 줄과 페이지 웹뷰를 넣는다.
fn add_webviews(app: &AppHandle, window: &tauri::Window, start: Url) -> Result<(), String> {
    let size = window.inner_size().map_err(err)?;
    let bar_height = (bar_height() * window.scale_factor().map_err(err)?).round() as u32;

    window
        .add_child(
            WebviewBuilder::new(BAR, WebviewUrl::App("browser.html".into())),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(size.width, bar_height),
        )
        .map_err(err)?;

    let app_for_new_window = app.clone();
    let page = WebviewBuilder::new(PAGE, WebviewUrl::External(start))
        // 웹 주소가 아닌 곳(file:, 앱 내부 주소 등)으로는 가지 못하게 한다.
        .on_navigation(|url| matches!(url.scheme(), "http" | "https" | "about"))
        // 새 창으로 열리는 링크는 같은 창에서 연다(통제되지 않는 창이 생기지 않게).
        .on_new_window(move |url, _features| {
            if let Some(page) = app_for_new_window.get_webview(PAGE) {
                let _ = page.navigate(url);
            }
            NewWindowResponse::Deny
        });
    window
        .add_child(
            page,
            PhysicalPosition::new(0, bar_height as i32),
            PhysicalSize::new(size.width, size.height.saturating_sub(bar_height)),
        )
        .map_err(err)?;

    let app_for_layout = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            layout(&app_for_layout);
        }
    });
    Ok(())
}

fn page(app: &AppHandle) -> Result<tauri::Webview, String> {
    app.get_webview(PAGE).ok_or_else(|| "브라우저가 열려 있지 않습니다".to_string())
}

// ───────────────────────── 화면에서 부르는 명령 ─────────────────────────

#[tauri::command]
pub async fn browser_open(app: AppHandle, url: Option<String>) -> Result<(), String> {
    open(&app, url.as_deref()).inspect_err(|problem| crate::log(&format!("브라우저 열기 실패: {problem}")))
}

#[tauri::command]
pub async fn browser_navigate(app: AppHandle, url: String) -> Result<String, String> {
    let target = normalize_url(&url)?;
    page(&app)?.navigate(target.clone()).map_err(err)?;
    Ok(target.to_string())
}

#[tauri::command]
pub fn browser_back(app: AppHandle) -> Result<(), String> {
    page(&app)?.eval("history.back()").map_err(err)
}

#[tauri::command]
pub fn browser_forward(app: AppHandle) -> Result<(), String> {
    page(&app)?.eval("history.forward()").map_err(err)
}

#[tauri::command]
pub fn browser_reload(app: AppHandle) -> Result<(), String> {
    page(&app)?.eval("location.reload()").map_err(err)
}

/// 주소 줄 높이를 바꾼다(즐겨찾기 줄을 보이거나 숨길 때).
#[tauri::command]
pub fn browser_bar_height(app: AppHandle, height: u32) {
    BAR_HEIGHT.store(height.clamp(44, 120), Ordering::Relaxed);
    layout(&app);
}

/// 지금 페이지의 주소와 제목(즐겨찾기에 넣을 때 쓴다).
#[tauri::command]
pub async fn browser_page(app: AppHandle) -> Result<Value, String> {
    eval(&app, "({ url: location.href, title: document.title })")
}

/// 지금 열려 있는 페이지 주소(주소 줄 표시용).
#[tauri::command]
pub fn browser_current(app: AppHandle) -> Option<String> {
    app.get_webview(PAGE)?.url().ok().map(|u| u.to_string())
}

/// 본창에서 묻는, 지금 브라우저에 열려 있는 페이지 주소(엔진과 무관). 브라우저가 닫혀 있으면 None.
/// 본창의 "Claude에게 넘기기"가 쓴다.
#[tauri::command]
pub async fn browser_where(app: AppHandle) -> Option<String> {
    if chrome_engine(&app) { crate::chrome::current_url() } else { browser_current(app) }
}

/// "Claude에게 넘기기": 본창으로 돌아가 Claude 세션 입력창에 이어서 할 일의 머리말을 넣는다.
/// `fresh`가 참이면 새 Claude 세션을 열어 거기에 넘긴다.
#[tauri::command]
pub fn browser_handoff(app: AppHandle, fresh: Option<bool>) -> Result<(), String> {
    let url = browser_current(app.clone()).unwrap_or_default();
    let payload = json!({ "url": url, "fresh": fresh.unwrap_or(false) });
    app.emit_to("main", "browser-handoff", payload).map_err(err)?;
    if let Some(main) = app.get_window("main") {
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
    Ok(())
}

// ───────────────────────── 개발자 도구 프로토콜 ─────────────────────────

/// 페이지 웹뷰에 CDP 명령을 보내고 답을 받는다.
/// 답은 창 스레드에서 돌아오므로, 창 스레드에서 부르면 안 된다(작업 스레드 전용).
pub fn cdp(app: &AppHandle, method: &str, params: Value) -> Result<Value, String> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows_core::HSTRING;

    if chrome_engine(app) {
        return crate::chrome::cdp(method, params);
    }

    let webview = page(app)?;
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
fn eval(app: &AppHandle, script: &str) -> Result<Value, String> {
    let reply = cdp(
        app,
        "Runtime.evaluate",
        json!({ "expression": script, "returnByValue": true, "awaitPromise": true, "userGesture": true }),
    )?;
    if let Some(problem) = reply.get("exceptionDetails") {
        let detail = problem["exception"]["description"].as_str().or(problem["text"].as_str()).unwrap_or("알 수 없는 오류");
        return Err(format!("스크립트 오류: {detail}"));
    }
    Ok(reply["result"]["value"].clone())
}

/// 브라우저가 없으면 띄우고, 페이지가 다 읽힐 때까지 기다린다.
fn ensure_ready(app: &AppHandle) -> Result<(), String> {
    if !is_open(app) {
        open(app, None)?;
        thread::sleep(Duration::from_millis(800));
    }
    wait_loaded(app);
    Ok(())
}

/// 이동·클릭 뒤 페이지가 다 읽힐 때까지(최대 12초) 기다린다.
fn wait_loaded(app: &AppHandle) {
    let deadline = Instant::now() + Duration::from_secs(12);
    thread::sleep(Duration::from_millis(300));
    while Instant::now() < deadline {
        // 이동 중에는 실행이 실패할 수 있다. 그때는 다시 물어본다.
        if eval(app, "document.readyState").is_ok_and(|v| v == "complete") {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn text(content: impl Into<String>) -> Vec<Value> {
    vec![json!({ "type": "text", "text": content.into() })]
}

fn summary(app: &AppHandle) -> String {
    let info = eval(app, "({ url: location.href, title: document.title })").unwrap_or_default();
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
  const clean = (s) => (s || '').trim().replace(/\s+/g, ' ').slice(0, 80);
  const items = els.map((el, i) => {
    const tag = el.tagName.toLowerCase();
    const secret = el.type === 'password';
    const name = clean(el.getAttribute('aria-label') || el.innerText || el.placeholder || el.title || el.name || el.alt);
    const kind = tag === 'input' ? `input[${el.type}]` : tag;
    let extra = '';
    if (tag === 'a') extra = ` → ${el.href.slice(0, 100)}`;
    else if ((tag === 'input' || tag === 'textarea') && !secret && el.value) extra = ` = "${clean(el.value)}"`;
    else if (tag === 'select') extra = ` = "${clean(el.selectedOptions[0]?.text)}"`;
    if (el.type === 'checkbox' || el.type === 'radio') extra += el.checked ? ' (선택됨)' : ' (선택 안 됨)';
    return `[${i}] ${kind} "${name}"${extra}`;
  });
  const body = (document.body?.innerText || '').replace(/\n{3,}/g, '\n\n');
  return { url: location.href, title: document.title, text: body, elements: items };
})()"#;

fn tool_read(app: &AppHandle) -> ToolResult {
    ensure_ready(app)?;
    let snap = eval(app, SNAPSHOT_JS)?;
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

fn tool_navigate(app: &AppHandle, args: &Value) -> ToolResult {
    let url = args["url"].as_str().ok_or("url이 필요합니다")?;
    let target = normalize_url(url)?.to_string();
    if is_open(app) {
        cdp(app, "Page.navigate", json!({ "url": target }))?;
    } else if chrome_engine(app) {
        // 먼저 띄운 뒤에 이동시킨다. 띄우면서 주소를 넘기면 첫 문서가 바뀌기 전의 빈 페이지를 "다 읽힘"으로 볼 수 있다.
        open(app, None)?;
        cdp(app, "Page.navigate", json!({ "url": target }))?;
    } else {
        open(app, Some(url))?;
        thread::sleep(Duration::from_millis(800));
    }
    wait_loaded(app);
    Ok(text(summary(app)))
}

/// 번호로 가리킨 요소의 화면 가운데 좌표(CSS 픽셀). 화면 밖이면 먼저 보이게 스크롤한다.
fn element_center(app: &AppHandle, reference: u64) -> Result<(f64, f64), String> {
    let script = format!(
        "(() => {{ const el = window.__cypherRefs?.[{reference}]; if (!el || !el.isConnected) return null; \
         el.scrollIntoView({{ block: 'center', inline: 'center' }}); const r = el.getBoundingClientRect(); \
         return {{ x: r.left + r.width / 2, y: r.top + r.height / 2 }}; }})()"
    );
    let point = eval(app, &script)?;
    match (point["x"].as_f64(), point["y"].as_f64()) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(format!("ref {reference}에 해당하는 요소가 없습니다. browser_read를 다시 불러 번호를 새로 받으세요.")),
    }
}

/// 실제 마우스처럼 그 좌표를 누른다.
fn click_point(app: &AppHandle, x: f64, y: f64) -> Result<(), String> {
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        let buttons = if kind == "mousePressed" { 1 } else { 0 };
        cdp(
            app,
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": x, "y": y, "button": if kind == "mouseMoved" { "none" } else { "left" },
                    "buttons": buttons, "clickCount": 1 }),
        )?;
    }
    Ok(())
}

fn tool_click(app: &AppHandle, args: &Value) -> ToolResult {
    ensure_ready(app)?;
    let reference = args["ref"].as_u64().ok_or("ref(요소 번호)가 필요합니다")?;
    let (x, y) = element_center(app, reference)?;
    click_point(app, x, y)?;
    wait_loaded(app);
    Ok(text(format!("눌렀습니다.\n{}", summary(app))))
}

fn tool_click_at(app: &AppHandle, args: &Value) -> ToolResult {
    ensure_ready(app)?;
    let (Some(x), Some(y)) = (args["x"].as_f64(), args["y"].as_f64()) else {
        return Err("x, y 좌표가 필요합니다".into());
    };
    click_point(app, x, y)?;
    wait_loaded(app);
    Ok(text(format!("({x}, {y})를 눌렀습니다.\n{}", summary(app))))
}

/// 이름으로 받은 키를 누른다.
fn press_key(app: &AppHandle, name: &str) -> Result<(), String> {
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
    cdp(app, "Input.dispatchKeyEvent", down)?;
    cdp(
        app,
        "Input.dispatchKeyEvent",
        json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk }),
    )?;
    Ok(())
}

fn tool_type(app: &AppHandle, args: &Value) -> ToolResult {
    ensure_ready(app)?;
    let typed = args["text"].as_str().ok_or("text가 필요합니다")?;
    if let Some(reference) = args["ref"].as_u64() {
        // 요소를 실제로 눌러 초점을 준 뒤, 들어 있던 내용을 전부 고른다(새 글자로 바뀌게).
        let (x, y) = element_center(app, reference)?;
        click_point(app, x, y)?;
        eval(
            app,
            &format!(
                "(() => {{ const el = window.__cypherRefs?.[{reference}]; if (!el) return; el.focus(); \
                 if (el.select) el.select(); else document.execCommand('selectAll'); }})()"
            ),
        )?;
    }
    cdp(app, "Input.insertText", json!({ "text": typed }))?;
    if args["submit"].as_bool().unwrap_or(false) {
        press_key(app, "enter")?;
        wait_loaded(app);
    }
    Ok(text(format!("입력했습니다.\n{}", summary(app))))
}

fn tool_key(app: &AppHandle, args: &Value) -> ToolResult {
    ensure_ready(app)?;
    press_key(app, args["key"].as_str().ok_or("key가 필요합니다")?)?;
    wait_loaded(app);
    Ok(text(format!("키를 눌렀습니다.\n{}", summary(app))))
}

fn tool_scroll(app: &AppHandle, args: &Value) -> ToolResult {
    ensure_ready(app)?;
    let dy = args["dy"].as_f64().unwrap_or(600.0);
    let position = eval(app, &format!("(window.scrollBy(0, {dy}), Math.round(window.scrollY))"))?;
    Ok(text(format!("스크롤했습니다. 지금 세로 위치: {position}")))
}

/// 보이는 화면을 그림으로 준다. 그림의 픽셀 좌표가 곧 `browser_click_at`의 좌표가 되도록
/// 화면 배율만큼 줄여서 찍는다.
fn tool_screenshot(app: &AppHandle) -> ToolResult {
    ensure_ready(app)?;
    let view = eval(app, "({ w: innerWidth, h: innerHeight, dpr: devicePixelRatio, x: scrollX, y: scrollY })")?;
    let (w, h) = (view["w"].as_f64().unwrap_or(1280.0), view["h"].as_f64().unwrap_or(800.0));
    let dpr = view["dpr"].as_f64().unwrap_or(1.0).max(0.1);
    let shot = cdp(
        app,
        "Page.captureScreenshot",
        json!({ "format": "jpeg", "quality": 70,
                "clip": { "x": view["x"], "y": view["y"], "width": w, "height": h, "scale": 1.0 / dpr } }),
    )?;
    let data = shot["data"].as_str().ok_or("화면을 찍지 못했습니다")?;
    Ok(vec![
        json!({ "type": "image", "data": data, "mimeType": "image/jpeg" }),
        json!({ "type": "text", "text": format!(
            "보이는 화면 {w}×{h}. 이 그림의 픽셀 좌표를 그대로 browser_click_at(x, y)에 쓰면 됩니다.\n{}", summary(app)) }),
    ])
}

fn tool_eval(app: &AppHandle, args: &Value) -> ToolResult {
    ensure_ready(app)?;
    let value = eval(app, args["script"].as_str().ok_or("script가 필요합니다")?)?;
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

fn tool_back(app: &AppHandle) -> ToolResult {
    ensure_ready(app)?;
    eval(app, "history.back()")?;
    wait_loaded(app);
    Ok(text(summary(app)))
}

/// Claude에게 알려 줄 도구 목록.
pub fn tools() -> Value {
    let none = json!({ "type": "object", "properties": {} });
    json!([
        { "name": "browser_read", "inputSchema": none,
          "description": "Cypher 내장 브라우저에 지금 열려 있는 페이지를 읽는다(사용자가 로그인해 둔 상태 그대로). 주소, 제목, 누르거나 입력할 수 있는 요소 목록(번호 포함), 페이지 글자를 돌려준다. 조작하기 전에 먼저 부른다." },
        { "name": "browser_navigate", "description": "브라우저를 그 주소로 이동시킨다(브라우저가 닫혀 있으면 띄운다).",
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
    ])
}

/// 도구 하나를 실행한다.
pub fn call(app: &AppHandle, name: &str, args: &Value) -> ToolResult {
    match name {
        "browser_read" => tool_read(app),
        "browser_navigate" => tool_navigate(app, args),
        "browser_click" => tool_click(app, args),
        "browser_click_at" => tool_click_at(app, args),
        "browser_type" => tool_type(app, args),
        "browser_key" => tool_key(app, args),
        "browser_scroll" => tool_scroll(app, args),
        "browser_screenshot" => tool_screenshot(app),
        "browser_back" => tool_back(app),
        "browser_eval" => tool_eval(app, args),
        other => Err(format!("없는 도구입니다: {other}")),
    }
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
        assert_eq!(list.len(), 10);
        assert!(list.iter().all(|t| t["name"].is_string() && t["description"].is_string() && t["inputSchema"]["type"] == "object"));
    }
}
