//! 클립보드 읽기·쓰기. 붙여넣기 한 번으로 글자·파일·이미지를 구분해 처리한다.

use serde::Serialize;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct ClipPayload {
    /// "text" | "paths" | "image" | "empty"
    kind: &'static str,
    /// 붙여넣을 글자. 이미지면 저장된 파일 경로(필요하면 따옴표 포함).
    text: String,
    /// 이미지일 때만: 미리보기용 파일 경로와 크기.
    path: Option<String>,
    width: u32,
    height: u32,
}

fn payload(kind: &'static str, text: impl Into<String>) -> ClipPayload {
    ClipPayload { kind, text: text.into(), path: None, width: 0, height: 0 }
}

fn paste_dir() -> Option<PathBuf> {
    Some(crate::data_dir()?.join("paste"))
}

/// 공백이 든 경로는 따옴표로 감싼다.
fn quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.into_owned()
    }
}

/// 클립보드 내용을 읽는다.
/// - 탐색기에서 복사한 파일 → `paths`(경로 글자)
/// - 글자 → `text`
/// - 이미지 → PNG로 저장하고 `image`(경로와 크기 포함). 화면은 이 파일로 미리보기를 띄우고,
///   Claude 세션이면 경로 대신 Alt+V 신호를 보내 Claude Code가 클립보드를 직접 읽게 한다.
#[tauri::command]
pub async fn clipboard_read() -> Result<ClipPayload, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if let Ok(files) = cb.get().file_list() {
        if !files.is_empty() {
            let joined = files.iter().map(|p| quote(p)).collect::<Vec<_>>().join(" ");
            return Ok(payload("paths", joined));
        }
    }
    if let Ok(text) = cb.get_text() {
        if !text.is_empty() {
            return Ok(payload("text", text));
        }
    }
    if let Ok(img) = cb.get_image() {
        let (width, height) = (img.width as u32, img.height as u32);
        let path = save_png(width, height, &img.bytes)?;
        return Ok(ClipPayload {
            kind: "image",
            text: quote(&path),
            path: Some(path.to_string_lossy().into_owned()),
            width,
            height,
        });
    }
    Ok(payload("empty", ""))
}

/// 붙여넣기 폴더에 저장된 이미지의 내용을 돌려준다(화면 미리보기용).
/// 다른 위치의 파일은 읽어 주지 않는다.
#[tauri::command]
pub async fn paste_preview(path: String) -> Result<tauri::ipc::Response, String> {
    let path = PathBuf::from(path);
    let allowed = paste_dir().is_some_and(|dir| path.parent() == Some(dir.as_path()))
        && path.extension().is_some_and(|ext| ext == "png");
    if !allowed {
        return Err("미리보기를 만들 수 없는 파일입니다".into());
    }
    fs::read(&path).map(tauri::ipc::Response::new).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clipboard_write(text: String) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)).map_err(|e| e.to_string())
}

fn save_png(width: u32, height: u32, rgba: &[u8]) -> Result<PathBuf, String> {
    let dir = paste_dir().ok_or("저장 폴더를 찾지 못했습니다")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path = dir.join(format!("paste-{stamp}.png"));
    let file = fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast); // 붙여넣기가 늦어지지 않게 한다.
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(path)
}

/// 사흘 넘은 붙여넣기 이미지를 지운다.
pub fn clean_old_pastes() {
    let Some(dir) = paste_dir() else { return };
    let Ok(entries) = fs::read_dir(dir) else { return };
    let limit = Duration::from_secs(3 * 24 * 60 * 60);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > limit);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}
