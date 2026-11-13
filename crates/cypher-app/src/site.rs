//! 사이트 항목(사이드바의 브라우저): 웹 페이지를 본창 안(그 항목의 창 자리)에 띄운다.
//!
//! 많은 사이트가 다른 페이지 안에 끼워 넣는 것(iframe)을 막아 두었기 때문에, 화면 요소로 넣지 않고
//! 본창에 웹뷰를 하나 더 붙여 그 항목의 창 자리에 겹쳐 놓는다. 화면 쪽이 자리(위치·크기)를 알려 주면
//! 그대로 옮기고, 그 창이 화면에 없거나 메뉴·대화상자가 열려 있는 동안에는 숨긴다.
//!
//! 같은 그룹의 Claude 세션은 이 웹뷰를 브라우저 도구로 조작할 수 있다(`browser.rs`). 화면에 보이지 않는
//! 사이트를 조작할 때는 웹뷰를 창 밖 자리에 잠깐 꺼내 놓는다(숨긴 웹뷰는 화면을 그리지 않아 찍을 수 없다).
//!
//! 이 웹뷰가 여는 웹 페이지에는 어떤 앱 명령 권한도 주지 않는다. 로그인 상태(쿠키)는 사이트 항목끼리 함께 쓴다.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// 화면 쪽이 마지막으로 알려 준 자리.
#[derive(Clone, Copy)]
struct Place {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
}

static PLACES: LazyLock<Mutex<HashMap<u32, Place>>> = LazyLock::new(Default::default);
/// Claude가 지금 조작하고 있는 사이트(조작 중에는 숨기지 않는다).
static BUSY: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(Default::default);
/// 같은 사이트의 웹뷰를 화면과 도구가 동시에 만들지 않게 한다.
static CREATE: Mutex<()> = Mutex::new(());

/// 웹뷰 엔진(WebView2)에 넘기는 실행 인자. 엔진이 사용자의 요청과 무관하게 뒤에서 하는 통신(구성 요소 갱신,
/// 안정성 보고 등)을 끈다. 앞의 `--disable-features=…`는 웹뷰 라이브러리의 기본값이다.
/// 같은 저장소를 쓰는 웹뷰는 인자가 같아야 하므로 본창(`tauri.conf.json`의 additionalBrowserArgs)과 똑같이 둔다.
pub const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,msImplicitSignin --disable-sync --disable-background-networking --disable-component-update --disable-domain-reliability";

/// 화면에 없는 사이트를 조작할 때 웹뷰를 꺼내 놓는 창 밖 자리와, 자리를 모를 때의 크기.
const PARKED_X: f64 = -30000.0;
const PARKED_SIZE: (f64, f64) = (1280.0, 860.0);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn label(id: u32) -> String {
    format!("site-{id}")
}

fn place_of(id: u32) -> Option<Place> {
    PLACES.lock().unwrap().get(&id).copied()
}

fn is_visible(id: u32) -> bool {
    place_of(id).is_some_and(|place| place.visible)
}

/// 사이트 웹뷰의 주소·제목이 바뀌었을 때 화면에 알리는 내용.
#[derive(Clone, Serialize)]
struct Changed {
    id: u32,
    url: Option<String>,
    title: Option<String>,
}

fn apply(webview: &tauri::Webview, place: Place) {
    if place.visible {
        let _ = webview.set_position(LogicalPosition::new(place.x, place.y));
        let _ = webview.set_size(LogicalSize::new(place.width.max(1.0), place.height.max(1.0)));
        let _ = webview.show();
    } else if !BUSY.lock().unwrap().contains(&id_of(webview)) {
        let _ = webview.hide();
    }
}

fn id_of(webview: &tauri::Webview) -> u32 {
    webview.label().strip_prefix("site-").and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// 웹뷰가 없으면 만든다. `place`가 없으면(Claude가 먼저 연 경우) 숨긴 채로 만든다.
fn create(app: &AppHandle, id: u32, url: &str, place: Option<Place>) -> Result<tauri::Webview, String> {
    let _guard = CREATE.lock().unwrap();
    if let Some(existing) = app.get_webview(&label(id)) {
        return Ok(existing);
    }
    let target = crate::browser::normalize_url(url)?;
    let window = app.get_window("main").ok_or("본창이 없습니다")?;
    let (for_title, for_load, for_new) = (app.clone(), app.clone(), app.clone());
    let builder = WebviewBuilder::new(label(id), WebviewUrl::External(target))
        .additional_browser_args(BROWSER_ARGS)
        // 파일을 끌어다 놓는 것은 페이지가 직접 받게 한다(사이트의 끌어서 올리기가 동작하도록).
        .disable_drag_drop_handler()
        // 웹 주소가 아닌 곳(file:, 앱 내부 주소 등)으로는 가지 못하게 한다.
        .on_navigation(|url| matches!(url.scheme(), "http" | "https" | "about"))
        // 새 창으로 열리는 링크는 같은 자리에서 연다(통제되지 않는 창이 생기지 않게).
        .on_new_window(move |url, _features| {
            if let Some(webview) = for_new.get_webview(&label(id)) {
                let _ = webview.navigate(url);
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |_webview, title| {
            let _ = for_title.emit_to("main", "site-changed", Changed { id, url: None, title: Some(title) });
        })
        .on_page_load(move |webview, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let url = webview.url().ok().map(|u| u.to_string());
                let _ = for_load.emit_to("main", "site-changed", Changed { id, url, title: None });
            }
        });
    let shown = place.filter(|p| p.visible);
    let (x, y) = shown.map_or((PARKED_X, 0.0), |p| (p.x, p.y));
    let (width, height) = shown.map_or(PARKED_SIZE, |p| (p.width.max(1.0), p.height.max(1.0)));
    let webview = window.add_child(builder, LogicalPosition::new(x, y), LogicalSize::new(width, height)).map_err(err)?;
    if shown.is_none() {
        let _ = webview.hide();
    }
    Ok(webview)
}

/// Claude의 브라우저 도구가 쓴다: 그 사이트 항목의 웹뷰가 없으면 화면에 보이지 않는 채로 만든다.
pub fn ensure(app: &AppHandle, id: u32, url: &str) -> Result<tauri::Webview, String> {
    create(app, id, url, None)
}

/// 사이트를 조작하는 동안 웹뷰가 화면을 그리게 한다. 화면에 없는 사이트는 창 밖 자리에 잠깐 꺼내 놓았다가
/// 끝나면 다시 숨긴다. 작업 스레드 전용.
pub fn with_shown<T>(app: &AppHandle, id: u32, run: impl FnOnce(&tauri::Webview) -> T) -> Result<T, String> {
    let webview = app.get_webview(&label(id)).ok_or("사이트가 아직 열리지 않았습니다")?;
    BUSY.lock().unwrap().insert(id);
    if !is_visible(id) {
        let (width, height) = place_of(id)
            .map(|p| (p.width, p.height))
            .filter(|(w, h)| *w >= 320.0 && *h >= 240.0)
            .unwrap_or(PARKED_SIZE);
        let _ = webview.set_position(LogicalPosition::new(PARKED_X, 0.0));
        let _ = webview.set_size(LogicalSize::new(width, height));
        let _ = webview.show();
        thread::sleep(Duration::from_millis(150));
    }
    let out = run(&webview);
    BUSY.lock().unwrap().remove(&id);
    if !is_visible(id) {
        let _ = webview.hide();
    }
    Ok(out)
}

/// 사이트 웹뷰를 만들어 그 자리(본창 안의 위치·크기, 화면 배율 100% 기준)에 놓는다. 이미 있으면 자리만 옮긴다.
/// 지금 열려 있는 주소를 돌려준다.
#[tauri::command]
pub async fn site_open(app: AppHandle, id: u32, url: String, x: f64, y: f64, width: f64, height: f64) -> Result<String, String> {
    let place = Place { x, y, width, height, visible: true };
    PLACES.lock().unwrap().insert(id, place);
    let webview = create(&app, id, &url, Some(place))?;
    // 만드는 사이에 화면이 자리를 바꿨을 수 있다. 마지막으로 알려 준 자리에 놓는다.
    apply(&webview, place_of(id).unwrap_or(place));
    Ok(webview.url().map(|u| u.to_string()).unwrap_or(url))
}

/// 자리를 옮기거나, 보이기·숨기기를 바꾼다. 아직 만들지 않은 웹뷰면 자리만 기억해 둔다.
#[tauri::command]
pub fn site_bounds(app: AppHandle, id: u32, x: f64, y: f64, width: f64, height: f64, visible: bool) {
    let place = Place { x, y, width, height, visible };
    PLACES.lock().unwrap().insert(id, place);
    if let Some(webview) = app.get_webview(&label(id)) {
        apply(&webview, place);
    }
}

#[tauri::command]
pub fn site_navigate(app: AppHandle, id: u32, url: String) -> Result<String, String> {
    let target = crate::browser::normalize_url(&url)?;
    app.get_webview(&label(id)).ok_or("사이트가 아직 열리지 않았습니다")?.navigate(target.clone()).map_err(err)?;
    Ok(target.to_string())
}

/// 뒤로·앞으로·새로 고침.
#[tauri::command]
pub fn site_action(app: AppHandle, id: u32, action: String) -> Result<(), String> {
    let webview = app.get_webview(&label(id)).ok_or("사이트가 아직 열리지 않았습니다")?;
    match action.as_str() {
        "back" => webview.eval("history.back()"),
        "forward" => webview.eval("history.forward()"),
        "reload" => webview.reload(),
        _ => return Err("모르는 동작입니다".into()),
    }
    .map_err(err)
}

/// 사이트에 보이는 화면(또는 `full`이면 페이지 전체)을 찍어 이미지 저장소에 넣는다.
/// 화면 쪽이 돌려받은 그림으로 이 사이트 항목과 같은 그룹에 이미지 항목을 만든다.
#[tauri::command]
pub async fn site_capture(app: AppHandle, id: u32, full: Option<bool>) -> Result<crate::image::Captured, String> {
    with_shown(&app, id, |webview| {
        crate::image::capture(&|method, params| crate::browser::webview_cdp(webview, method, params), full.unwrap_or(false))
    })?
    .inspect_err(|problem| crate::log(&format!("사이트 캡처 실패: {problem}")))
}

#[tauri::command]
pub fn site_close(app: AppHandle, id: u32) {
    PLACES.lock().unwrap().remove(&id);
    if let Some(webview) = app.get_webview(&label(id)) {
        let _ = webview.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 사이트_웹뷰와_본창의_엔진_실행_인자가_같다() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["app"]["windows"][0]["additionalBrowserArgs"], BROWSER_ARGS);
        // 엔진이 뒤에서 스스로 하는 통신을 끄는 인자가 들어 있다.
        for flag in ["--disable-background-networking", "--disable-component-update", "--disable-domain-reliability"] {
            assert!(BROWSER_ARGS.contains(flag), "{flag}");
        }
    }
}
