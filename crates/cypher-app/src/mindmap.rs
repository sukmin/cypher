//! 마인드맵 항목: 내용의 자동 저장과 내보내기.
//!
//! 내용은 개요 글(첫 줄 `# 중심 주제`, 그 아래 `- 가지`)이다. 항목마다 `%APPDATA%\cypher\mindmaps\<번호>.md`에
//! UTF-8로 계속 적어 둔다(파일로 따로 저장하지 않아도 다시 시작하면 그대로 돌아온다). 같은 그룹의 Claude 세션이
//! 이 파일을 읽을 수 있다. 닫은 항목은 바로 지우지 않고 `closed` 폴더로 옮겨 30일 동안 둔다.
//!
//! 그림(PNG·SVG)은 화면이 만들어 보내고, PDF는 본창 웹뷰의 인쇄 기능으로 만든다(글자가 그대로 살아 있는 PDF).

use base64::Engine;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Manager};

const KEEP_CLOSED: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// PDF 한 쪽의 한 변 길이 한도(인치). Chromium 인쇄의 한도 안쪽으로 잡는다.
const MAX_PAGE_INCHES: f64 = 190.0;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn dir() -> Result<PathBuf, String> {
    let dir = crate::user_dir().join("mindmaps");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 항목 번호는 화면이 만든다. 경로로 쓰기 전에 영문·숫자뿐인지 확인한다.
fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn stored(id: &str) -> Result<PathBuf, String> {
    if !is_id(id) {
        return Err("항목 번호가 올바르지 않습니다".into());
    }
    Ok(dir()?.join(format!("{id}.md")))
}

/// 닫은 지 30일이 넘은 것을 지운다(시작할 때 한 번).
pub fn clean_closed() {
    let Ok(entries) = dir().and_then(|dir| fs::read_dir(dir.join("closed")).map_err(err)) else { return };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > KEEP_CLOSED);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// 적어 둔 내용. 아직 없으면 None. (Claude 세션이 다른 인코딩으로 고쳐 써도 읽을 수 있는 만큼 읽는다.)
#[tauri::command]
pub fn mindmap_read(id: String) -> Result<Option<String>, String> {
    let path = stored(&id)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(err)?;
    Ok(Some(match cypher_core::textfile::decode(&bytes) {
        Ok(decoded) => decoded.text,
        Err(_) => String::from_utf8_lossy(&bytes).into_owned(),
    }))
}

#[tauri::command]
pub fn mindmap_write(id: String, text: String) -> Result<(), String> {
    crate::memo::write_safely(&stored(&id)?, text.as_bytes())
}

/// 항목을 닫는다. `keep`이면 내용을 `closed` 폴더로 옮기고 그 경로를 돌려준다.
#[tauri::command]
pub fn mindmap_remove(id: String, keep: bool) -> Result<Option<String>, String> {
    let path = stored(&id)?;
    if !path.exists() {
        return Ok(None);
    }
    if !keep {
        fs::remove_file(&path).map_err(err)?;
        return Ok(None);
    }
    let closed = dir()?.join("closed");
    fs::create_dir_all(&closed).map_err(err)?;
    let stamp = crate::http::now_text().replace([':', ' '], "-");
    let target = closed.join(format!("{stamp}-{id}.md"));
    fs::rename(&path, &target).map_err(err)?;
    if let Ok(file) = fs::File::options().write(true).open(&target) {
        let _ = file.set_modified(SystemTime::now()); // 보관 기간은 옮긴 때부터 센다.
    }
    Ok(Some(target.to_string_lossy().into_owned()))
}

fn safe_name(name: &str) -> String {
    let safe: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let safe: String = safe.trim().chars().take(80).collect();
    if safe.is_empty() { "마인드맵".into() } else { safe }
}

/// 저장할 곳을 묻는다. 취소하면 None.
fn ask_target(title: &str, name: &str, ext: &str, start: Option<&str>) -> Option<PathBuf> {
    let label = match ext {
        "png" => "PNG 이미지",
        "svg" => "SVG 그림",
        "pdf" => "PDF 문서",
        _ => "파일",
    };
    let mut dialog = rfd::FileDialog::new()
        .set_title(title)
        .set_file_name(format!("{}.{ext}", safe_name(name)))
        .add_filter(label, &[ext]);
    if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%' && i + 2 < bytes.len()).then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok()).flatten();
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(byte) => { out.push(byte); i += 3; }
            None => { out.push(bytes[i]); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 화면이 만든 그림(PNG·SVG)을 파일로 내보낸다. 내용은 요청 본문으로, 이름·확장자·시작 폴더는 머리글로 온다
/// (이름과 폴더는 URL 인코딩). 취소하면 None, 저장했으면 그 경로.
#[tauri::command]
pub async fn mindmap_export(request: tauri::ipc::Request<'_>) -> Result<Option<String>, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("내보낼 내용이 없습니다".into());
    };
    let header = |key: &str| request.headers().get(key).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (name, start, ext) = (percent_decode(&header("x-name")), percent_decode(&header("x-start")), header("x-ext"));
    // 저장 창의 제목. 화면이 주지 않으면 마인드맵의 것이다(ERD 항목도 이 명령으로 그림을 내보낸다).
    let title = Some(percent_decode(&header("x-title"))).filter(|title| !title.is_empty()).unwrap_or_else(|| "마인드맵 내보내기".into());
    if !matches!(ext.as_str(), "png" | "svg") {
        return Err("내보낼 수 없는 형식입니다".into());
    }
    let bytes = bytes.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(target) = ask_target(&title, &name, &ext, Some(start.as_str())) else { return Ok(None) };
        fs::write(&target, bytes).map_err(err)?;
        Ok(Some(target.to_string_lossy().into_owned()))
    })
    .await
    .map_err(err)?
}

/// 본창에 지금 인쇄용으로 펼쳐 둔 그림을 PDF로 내보낸다(화면 쪽이 인쇄용 영역에 그림만 보이게 해 둔다).
/// `width`·`height`는 그림의 크기(CSS 픽셀)이고, 그 크기 그대로 한 쪽짜리 PDF를 만든다.
/// `target`이 있으면 저장할 곳을 묻지 않고 그 경로에 쓴다(자동 검증용).
#[tauri::command]
pub async fn mindmap_pdf(
    app: AppHandle,
    name: String,
    start: Option<String>,
    width: f64,
    height: f64,
    target: Option<String>,
) -> Result<Option<String>, String> {
    let webview = app.get_webview("main").ok_or("본창이 없습니다")?;
    let inches = |px: f64| (px / 96.0).clamp(1.0, MAX_PAGE_INCHES);
    tauri::async_runtime::spawn_blocking(move || {
        let reply = crate::browser::webview_cdp(
            &webview,
            "Page.printToPDF",
            serde_json::json!({
                "printBackground": true, "preferCSSPageSize": false,
                "paperWidth": inches(width), "paperHeight": inches(height),
                "marginTop": 0, "marginBottom": 0, "marginLeft": 0, "marginRight": 0,
                "pageRanges": "1",
            }),
        )
        .map_err(|problem| format!("PDF를 만들지 못했습니다: {problem}"))?;
        let data = reply["data"].as_str().ok_or("PDF를 만들지 못했습니다")?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(err)?;
        let Some(target) = target.map(PathBuf::from).or_else(|| ask_target("마인드맵 내보내기", &name, "pdf", start.as_deref())) else { return Ok(None) };
        fs::write(&target, bytes).map_err(err)?;
        Ok(Some(target.to_string_lossy().into_owned()))
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 항목_번호와_파일_이름을_검사한다() {
        assert!(is_id("b1a2c3"));
        assert!(!is_id("..\\밖") && !is_id("") && !is_id("a/b"));
        assert_eq!(safe_name("결제: 개편/안?"), "결제_ 개편_안_");
        assert_eq!(safe_name("  "), "마인드맵");
        assert_eq!(percent_decode("%EA%B0%80%20b%"), "가 b%");
    }
}
