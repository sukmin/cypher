//! 사이트 항목: 사이드바에 저장해 둔 웹 페이지를 본창 안(그 항목의 창 자리)에 띄운다.
//!
//! 많은 사이트가 다른 페이지 안에 끼워 넣는 것(iframe)을 막아 두었기 때문에, 화면 요소로 넣지 않고
//! 본창에 웹뷰를 하나 더 붙여 그 항목의 창 자리에 겹쳐 놓는다. 화면 쪽이 자리(위치·크기)를 알려 주면
//! 그대로 옮기고, 그 창이 화면에 없거나 메뉴·대화상자가 열려 있는 동안에는 숨긴다.
//!
//! 이 웹뷰가 여는 웹 페이지에는 내장 브라우저의 페이지와 마찬가지로 어떤 앱 명령 권한도 주지 않는다.
//! 로그인 상태(쿠키)는 내장 브라우저와 같은 저장소를 쓰므로 한쪽에서 로그인하면 다른 쪽에도 적용된다.

use serde::Serialize;
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn label(id: u32) -> String {
    format!("site-{id}")
}

/// 사이트 웹뷰의 주소·제목이 바뀌었을 때 화면에 알리는 내용.
#[derive(Clone, Serialize)]
struct Changed {
    id: u32,
    url: Option<String>,
    title: Option<String>,
}

/// 사이트 웹뷰를 만들어 그 자리(본창 안의 위치·크기, 화면 배율 100% 기준)에 놓는다. 이미 있으면 자리만 옮긴다.
#[tauri::command]
pub async fn site_open(app: AppHandle, id: u32, url: String, x: f64, y: f64, width: f64, height: f64) -> Result<String, String> {
    let target = crate::browser::normalize_url(&url)?;
    if let Some(existing) = app.get_webview(&label(id)) {
        place(&existing, x, y, width, height);
        return Ok(target.to_string());
    }
    let window = app.get_window("main").ok_or("본창이 없습니다")?;
    let (for_title, for_load, for_new) = (app.clone(), app.clone(), app.clone());
    let builder = WebviewBuilder::new(label(id), WebviewUrl::External(target.clone()))
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
    window.add_child(builder, LogicalPosition::new(x, y), LogicalSize::new(width.max(1.0), height.max(1.0))).map_err(err)?;
    Ok(target.to_string())
}

fn place(webview: &tauri::Webview, x: f64, y: f64, width: f64, height: f64) {
    let _ = webview.set_position(LogicalPosition::new(x, y));
    let _ = webview.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
}

/// 자리를 옮기거나, 보이기·숨기기를 바꾼다. 아직 만들지 않은 웹뷰면 아무 일도 하지 않는다.
#[tauri::command]
pub fn site_bounds(app: AppHandle, id: u32, x: f64, y: f64, width: f64, height: f64, visible: bool) {
    let Some(webview) = app.get_webview(&label(id)) else { return };
    if visible {
        place(&webview, x, y, width, height);
        let _ = webview.show();
    } else {
        let _ = webview.hide();
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
    let webview = app.get_webview(&label(id)).ok_or("사이트가 아직 열리지 않았습니다")?;
    crate::image::capture(&|method, params| crate::browser::webview_cdp(&webview, method, params), full.unwrap_or(false))
        .inspect_err(|problem| crate::log(&format!("사이트 캡처 실패: {problem}")))
}

#[tauri::command]
pub fn site_close(app: AppHandle, id: u32) {
    if let Some(webview) = app.get_webview(&label(id)) {
        let _ = webview.close();
    }
}
