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
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Url};

const MAX_TEXT: usize = 15_000;

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

/// Chrome에 보이는 화면(또는 `full`이면 페이지 전체)을 찍어 이미지 항목으로 만든다.
/// 찍은 그림은 저장소에 넣고 본창에 알린다. 본창이 지금 보고 있는 항목의 그룹에 이미지 항목을 만든다.
#[tauri::command]
pub async fn browser_capture(app: AppHandle, full: Option<bool>, profile: Option<String>) -> Result<(), String> {
    let profile = chrome_profile(profile)?;
    if !chrome_engine(&app) || !crate::chrome::is_open(&profile) {
        return Err("브라우저가 열려 있지 않습니다".into());
    }
    let captured = crate::image::capture(&|method, params| crate::chrome::cdp(&profile, method, params), full.unwrap_or(false))
        .inspect_err(|problem| crate::log(&format!("브라우저 캡처 실패: {problem}")))?;
    app.emit_to("main", "image-captured", captured).map_err(err)
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

/// 이동·클릭 뒤 페이지가 다 읽힐 때까지(최대 12초) 기다린다.
fn wait_loaded(target: &Target) {
    let deadline = Instant::now() + Duration::from_secs(12);
    thread::sleep(Duration::from_millis(300));
    while Instant::now() < deadline {
        // 이동 중에는 실행이 실패할 수 있다. 그때는 다시 물어본다.
        if eval(target, "document.readyState").is_ok_and(|v| v == "complete") {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
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
fn element_center(target: &Target, reference: u64) -> Result<(f64, f64), String> {
    let script = format!(
        "(() => {{ const el = window.__cypherRefs?.[{reference}]; if (!el || !el.isConnected) return null; \
         el.scrollIntoView({{ block: 'center', inline: 'center' }}); const r = el.getBoundingClientRect(); \
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
    click_point(target, x, y)?;
    wait_loaded(target);
    Ok(text(format!("눌렀습니다.\n{}", summary(target))))
}

fn tool_click_at(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let (Some(x), Some(y)) = (args["x"].as_f64(), args["y"].as_f64()) else {
        return Err("x, y 좌표가 필요합니다".into());
    };
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
        press_key(target, "enter")?;
        wait_loaded(target);
    }
    Ok(text(format!("입력했습니다.\n{}", summary(target))))
}

fn tool_key(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    press_key(target, args["key"].as_str().ok_or("key가 필요합니다")?)?;
    wait_loaded(target);
    Ok(text(format!("키를 눌렀습니다.\n{}", summary(target))))
}

fn tool_scroll(target: &Target, args: &Value) -> ToolResult {
    wait_loaded(target);
    let dy = args["dy"].as_f64().unwrap_or(600.0);
    let position = eval(target, &format!("(window.scrollBy(0, {dy}), Math.round(window.scrollY))"))?;
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
    let value = eval(target, args["script"].as_str().ok_or("script가 필요합니다")?)?;
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
        if !crate::chrome::is_open(&profile) {
            open_chrome(app, &profile, None)?;
            thread::sleep(Duration::from_millis(800));
        }
        return run(&Target::Chrome(profile), name, args);
    }
    if choosing {
        // 고를 브라우저의 번호는 `site`로 넘긴다(`id`는 화면에 묻는 요청의 번호로 쓰인다).
        let reply = crate::mcp::ask_ui(app, name, json!({ "session": session, "site": args["id"] }))?;
        return Ok(text(reply["text"].as_str().unwrap_or("")));
    }
    // 어느 사이트 항목을 조작할지는 화면이 정한다. 이동하라는 요청이면, 그룹에 브라우저가 없을 때 새로 만든다.
    let found = crate::mcp::ask_ui(app, "browser_target", json!({ "session": session, "create": name == "browser_navigate" }))?;
    let id = found["key"].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("조작할 브라우저를 찾지 못했습니다")?;
    // 그 사이트 항목의 그룹이 쓰는 프로필로 웹뷰를 만든다(화면이 알려 준다).
    let webview = crate::site::ensure(app, id, found["url"].as_str().unwrap_or("about:blank"), found["profile"].as_str().unwrap_or(""))?;
    // 방금 받은 그 웹뷰만 조작한다(이름으로 다시 찾으면, 그사이 항목이 옮겨져 다시 만들어진 다른 프로필의 웹뷰일 수 있다).
    let mut content = crate::site::with_shown_on(&webview, |webview| run(&Target::Site(Box::new(webview.clone())), name, args))?;
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
        assert_eq!(list.len(), 12);
        assert!(list.iter().all(|t| t["name"].is_string() && t["description"].is_string() && t["inputSchema"]["type"] == "object"));
    }
}
