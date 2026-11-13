//! 이미지 항목: 사이드바에 놓는 그림(화면 캡처, 파일에서 가져온 것, 붙여넣은 것, 끌어다 놓은 것).
//!
//! 그림은 `%APPDATA%\cypher\images\<번호>.<형식>`에 복사해 둔다(원본 파일을 옮기거나 지워도 항목은 남는다).
//! 닫은 이미지는 바로 지우지 않고 `closed` 폴더로 옮겨 30일 동안 둔다.

use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 이미지 하나의 크기 한도.
const MAX_BYTES: u64 = 64 << 20;
const KEEP_CLOSED: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// 다룰 수 있는 형식(저장할 때의 확장자).
const FORMATS: &[&str] = &["png", "jpg", "gif", "webp", "bmp", "svg", "avif", "ico"];

/// 저장소에 넣은 이미지 하나.
#[derive(Clone, Serialize)]
pub struct Stored {
    /// 저장소 안의 번호(영문·숫자).
    pub id: String,
    /// 형식(확장자).
    pub ext: String,
    /// 사이드바에 보일 이름(파일 이름에서 확장자를 뺀 것 등). 없으면 빈 글.
    pub name: String,
    /// 저장된 파일의 전체 경로(Claude 세션에 넘기거나 탐색기에서 열 때 쓴다).
    pub path: String,
    pub bytes: u64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn image_dir() -> Result<PathBuf, String> {
    let dir = crate::user_dir().join("images");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 번호와 형식은 화면이 다시 건네준다. 경로로 쓰기 전에 확인한다.
fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn stored_path(id: &str, ext: &str) -> Result<PathBuf, String> {
    if !is_id(id) || !FORMATS.contains(&ext) {
        return Err("이미지 번호가 올바르지 않습니다".into());
    }
    Ok(image_dir()?.join(format!("{id}.{ext}")))
}

fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let random = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("i{stamp:x}{:06x}", random & 0xff_ffff)
}

/// 확장자를 저장 형식으로 바꾼다. 다루지 않는 확장자면 None.
pub fn format_of_extension(ext: &str) -> Option<&'static str> {
    let ext = ext.to_ascii_lowercase();
    let ext = match ext.as_str() {
        "jpeg" | "jpe" | "jfif" => "jpg",
        other => other,
    };
    FORMATS.iter().copied().find(|f| *f == ext)
}

/// 내용의 앞부분으로 형식을 알아낸다.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("jpg");
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some("webp");
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" && (&head[8..12] == b"avif" || &head[8..12] == b"avis") {
        return Some("avif");
    }
    // BMP는 머리글의 예약 칸(6~9번째 바이트)이 0이다. "BM"으로 시작하는 글자 파일을 그림으로 보지 않게 함께 확인한다.
    if head.starts_with(b"BM") && head.len() >= 14 && head[6..10] == [0; 4] {
        return Some("bmp");
    }
    if head.starts_with(&[0, 0, 1, 0]) {
        return Some("ico");
    }
    // SVG는 글자 파일이다. 앞쪽에 <svg가 보이면 SVG로 본다.
    let text = String::from_utf8_lossy(&head[..head.len().min(1024)]).to_ascii_lowercase();
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with("<svg") || ((text.starts_with("<?xml") || text.starts_with("<!--") || text.starts_with("<!doctype svg")) && text.contains("<svg")) {
        return Some("svg");
    }
    None
}

/// 사이드바에 보일 이름으로 다듬는다(경로 구분자·제어 문자를 빼고 길이를 줄인다).
fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    cleaned.trim().chars().take(120).collect()
}

/// 내용을 저장소에 넣는다. 형식은 내용으로 정하고, 내용으로 알 수 없으면 `hint`(확장자)를 따른다.
pub fn store(bytes: &[u8], hint: Option<&str>, name: &str) -> Result<Stored, String> {
    if bytes.is_empty() {
        return Err("빈 이미지입니다".into());
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("이미지가 너무 큽니다({}MB). {}MB까지 넣을 수 있습니다.", bytes.len() >> 20, MAX_BYTES >> 20));
    }
    let ext = sniff(bytes).or_else(|| hint.and_then(format_of_extension)).ok_or("이미지 파일이 아닙니다")?;
    let id = new_id();
    let path = stored_path(&id, ext)?;
    fs::write(&path, bytes).map_err(err)?;
    Ok(Stored { id, ext: ext.into(), name: clean_name(name), path: path.to_string_lossy().into_owned(), bytes: bytes.len() as u64 })
}

/// 파일을 저장소로 복사해 넣는다.
pub fn import(path: &Path) -> Result<Stored, String> {
    let size = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if size > MAX_BYTES {
        return Err(format!("이미지가 너무 큽니다({}MB). {}MB까지 넣을 수 있습니다.", size >> 20, MAX_BYTES >> 20));
    }
    let bytes = fs::read(path).map_err(err)?;
    let hint = path.extension().map(|e| e.to_string_lossy().into_owned());
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    store(&bytes, hint.as_deref(), &name)
}

/// 여러 개를 가져온 결과에서 된 것만 남긴다(안 된 것은 건너뛴다). 하나도 되지 않았으면 첫 오류를 돌려준다.
fn only_done<T>(results: impl IntoIterator<Item = Result<T, String>>) -> Result<Vec<T>, String> {
    let mut done = Vec::new();
    let mut problem = None;
    for result in results {
        match result {
            Ok(one) => done.push(one),
            Err(first) => {
                problem.get_or_insert(first);
            }
        }
    }
    match problem {
        Some(problem) if done.is_empty() => Err(problem),
        _ => Ok(done),
    }
}

/// 닫은 지 30일이 넘은 이미지를 지운다(시작할 때 한 번).
pub fn clean_closed() {
    let Ok(entries) = image_dir().and_then(|dir| fs::read_dir(dir.join("closed")).map_err(err)) else { return };
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

// ───────────────────────── 화면에서 부르는 명령 ─────────────────────────

/// 파일을 이미지 항목으로 가져온다.
#[tauri::command]
pub async fn image_import(path: String) -> Result<Stored, String> {
    import(Path::new(&path))
}

/// 파일을 골라 이미지 항목으로 가져온다(여러 개 고를 수 있다). 취소하면 빈 목록.
#[tauri::command]
pub async fn image_pick(start: Option<String>) -> Result<Vec<Stored>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_title("이미지 항목으로 가져올 파일")
            .add_filter("이미지", &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "avif", "ico"])
            .add_filter("모든 파일", &["*"]);
        if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let picked = dialog.pick_files().unwrap_or_default();
        // 되는 것만 넣는다: 하나가 안 된다고 전부 버리면, 앞서 저장소에 복사된 그림이 항목 없이 남는다.
        only_done(picked.iter().take(20).map(|path| {
            import(path).inspect_err(|problem| crate::log(&format!("이미지 가져오기 실패({}): {problem}", path.display())))
        }))
    })
    .await
    .map_err(err)?
}

/// 클립보드의 이미지를 이미지 항목으로 넣는다. 탐색기에서 복사한 이미지 파일도 받는다. 이미지가 없으면 빈 목록.
#[tauri::command]
pub async fn image_paste() -> Result<Vec<Stored>, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(err)?;
    if let Ok(files) = clipboard.get().file_list() {
        let images: Vec<Stored> = files.iter().take(20).filter_map(|path| import(path).ok()).collect();
        if !images.is_empty() {
            return Ok(images);
        }
    }
    let Ok(image) = clipboard.get_image() else { return Ok(Vec::new()) };
    let png = encode_png(image.width as u32, image.height as u32, &image.bytes)?;
    Ok(vec![store(&png, Some("png"), "")?])
}

/// 화면이 가진 내용(경로 없는 파일, 그림 요소)을 그대로 받아 저장소에 넣는다.
/// 내용은 요청 본문으로, 이름과 확장자는 머리글로 온다(이름은 URL 인코딩).
#[tauri::command]
pub async fn image_store(request: tauri::ipc::Request<'_>) -> Result<Stored, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("이미지 내용이 없습니다".into());
    };
    let header = |key: &str| request.headers().get(key).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let name = percent_decode(&header("x-name"));
    let hint = header("x-ext");
    store(bytes, Some(hint.as_str()).filter(|h| !h.is_empty()), &name)
}

/// 웹 주소(또는 data: 주소)의 그림을 받아 저장소에 넣는다(브라우저에서 그림을 끌어다 놓았을 때).
#[tauri::command]
pub async fn image_fetch(url: String) -> Result<Stored, String> {
    tauri::async_runtime::spawn_blocking(move || fetch(&url)).await.map_err(err)?
}

fn fetch(url: &str) -> Result<Stored, String> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',').ok_or("올바르지 않은 data 주소입니다")?;
        let bytes = if meta.ends_with(";base64") {
            use base64::Engine;
            let compact: String = data.chars().filter(|c| !c.is_whitespace()).collect();
            base64::engine::general_purpose::STANDARD.decode(compact).map_err(err)?
        } else {
            percent_decode(data).into_bytes()
        };
        return store(&bytes, None, "");
    }
    let parsed = tauri::Url::parse(url).map_err(|_| "올바르지 않은 주소입니다".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("웹 주소(http, https)의 그림만 받을 수 있습니다".into());
    }
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).redirects(5).build();
    let response = agent.get(url).call().map_err(|e| format!("그림을 받지 못했습니다: {e}"))?;
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(err)?;
    // 주소의 마지막 부분을 이름과 형식의 실마리로 쓴다.
    let last = parsed.path_segments().and_then(|mut s| s.next_back()).unwrap_or("").to_string();
    let last = percent_decode(&last);
    let (stem, ext) = match last.rsplit_once('.') {
        Some((stem, ext)) if format_of_extension(ext).is_some() => (stem.to_string(), Some(ext.to_string())),
        _ => (last.clone(), None),
    };
    store(&bytes, ext.as_deref(), &stem)
}

/// `%EC%9D%B4` 같은 URL 인코딩을 푼다.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((high * 16 + low) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 저장된 이미지의 내용(화면에 그릴 때 쓴다).
#[tauri::command]
pub async fn image_read(id: String, ext: String) -> Result<tauri::ipc::Response, String> {
    let path = stored_path(&id, &ext)?;
    fs::read(&path).map(tauri::ipc::Response::new).map_err(|_| "이미지 파일이 없습니다".to_string())
}

/// 저장된 이미지의 경로와 크기. 파일이 없으면 None.
#[tauri::command]
pub fn image_info(id: String, ext: String) -> Result<Option<Stored>, String> {
    let path = stored_path(&id, &ext)?;
    let Ok(meta) = fs::metadata(&path) else { return Ok(None) };
    Ok(Some(Stored { id, ext, name: String::new(), path: path.to_string_lossy().into_owned(), bytes: meta.len() }))
}

/// 이미지 항목을 닫는다. 그림은 `closed` 폴더로 옮겨 30일 동안 두고, 옮긴 경로를 돌려준다.
#[tauri::command]
pub fn image_remove(id: String, ext: String) -> Result<Option<String>, String> {
    let path = stored_path(&id, &ext)?;
    if !path.exists() {
        return Ok(None);
    }
    let closed = image_dir()?.join("closed");
    fs::create_dir_all(&closed).map_err(err)?;
    let stamp = crate::http::now_text().replace([':', ' '], "-");
    let target = closed.join(format!("{stamp}-{id}.{ext}"));
    fs::rename(&path, &target).map_err(err)?;
    // 보관 기간은 옮긴 때부터 센다.
    if let Ok(file) = fs::File::options().write(true).open(&target) {
        let _ = file.set_modified(SystemTime::now());
    }
    Ok(Some(target.to_string_lossy().into_owned()))
}

/// 이미지를 다른 곳에 파일로 저장한다. 취소하면 None, 저장했으면 그 경로.
#[tauri::command]
pub async fn image_export(id: String, ext: String, name: String, start: Option<String>) -> Result<Option<String>, String> {
    let source = stored_path(&id, &ext)?;
    tauri::async_runtime::spawn_blocking(move || {
        let safe: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
        let safe = if safe.trim().is_empty() { "image".to_string() } else { safe.trim().to_string() };
        let mut dialog = rfd::FileDialog::new()
            .set_title("이미지 저장")
            .set_file_name(format!("{safe}.{ext}"))
            .add_filter("이미지", &[ext.as_str()]);
        if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(target) = dialog.save_file() else { return Ok(None) };
        fs::copy(&source, &target).map_err(err)?;
        Ok(Some(target.to_string_lossy().into_owned()))
    })
    .await
    .map_err(err)?
}

/// 이미지를 클립보드에 복사한다. 화면이 PNG로 바꿔 요청 본문으로 보낸다(형식과 무관하게 복사할 수 있게).
#[tauri::command]
pub async fn image_copy(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("이미지 내용이 없습니다".into());
    };
    let (width, height, rgba) = decode_png(bytes)?;
    let image = arboard::ImageData { width: width as usize, height: height as usize, bytes: rgba.into() };
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_image(image)).map_err(err)
}

// ───────────────────────── PNG ─────────────────────────

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(err)?;
        writer.write_image_data(rgba).map_err(err)?;
        writer.finish().map_err(err)?;
    }
    Ok(out)
}

/// PNG를 풀어 가로·세로와 RGBA 픽셀을 돌려준다.
fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // 색상표·16비트·회색조를 8비트 색으로 편다.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(err)?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer).map_err(err)?;
    let pixels = &buffer[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return Err("이 PNG는 복사할 수 없습니다".into()),
    };
    Ok((frame.width, frame.height, rgba))
}

// ───────────────────────── 화면 캡처 ─────────────────────────

/// 캡처한 그림 한 장과 그 페이지.
#[derive(Clone, Serialize)]
pub struct Captured {
    pub image: Stored,
    /// 캡처한 페이지의 주소와 제목.
    pub source: String,
    pub title: String,
    /// 전체 페이지를 찍었는지, 너무 길어 아래쪽이 잘렸는지.
    pub full: bool,
    pub cut: bool,
}

/// 한 변의 픽셀 수 한도(브라우저가 한 장으로 그릴 수 있는 크기).
const MAX_SIDE: f64 = 16_000.0;
/// 전체 페이지를 찍을 때의 세로 한도(CSS 픽셀).
const MAX_PAGE_HEIGHT: f64 = 30_000.0;

/// 페이지를 찍어 저장소에 넣는다. `cdp`는 그 페이지에 개발자 도구 명령을 보내는 함수다
/// (사이트 항목, 설치된 Chrome 어느 것이든 같은 방법으로 찍는다).
/// `full`이면 화면에 보이는 부분이 아니라 페이지 전체를 찍는다.
pub fn capture(cdp: &dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value, String>, full: bool) -> Result<Captured, String> {
    use serde_json::json;
    let script = "(() => { const d = document.documentElement, b = document.body; return { \
        url: location.href, title: document.title, dpr: devicePixelRatio, \
        width: Math.max(d.scrollWidth, b ? b.scrollWidth : 0, innerWidth), \
        height: Math.max(d.scrollHeight, b ? b.scrollHeight : 0, innerHeight) }; })()";
    let reply = cdp("Runtime.evaluate", json!({ "expression": script, "returnByValue": true }))?;
    let page = &reply["result"]["value"];
    let source = page["url"].as_str().unwrap_or("").to_string();
    let title = page["title"].as_str().unwrap_or("").to_string();

    let mut params = json!({ "format": "png" });
    let mut cut = false;
    if full {
        let dpr = page["dpr"].as_f64().unwrap_or(1.0).max(0.1);
        let width = page["width"].as_f64().unwrap_or(1280.0).max(1.0);
        let mut height = page["height"].as_f64().unwrap_or(800.0).max(1.0);
        if height > MAX_PAGE_HEIGHT {
            height = MAX_PAGE_HEIGHT;
            cut = true;
        }
        // 그림이 한도를 넘지 않게 줄인다.
        let scale = (MAX_SIDE / (width.max(height) * dpr)).min(1.0);
        params["captureBeyondViewport"] = json!(true);
        params["clip"] = json!({ "x": 0, "y": 0, "width": width, "height": height, "scale": scale });
    }
    let shot = cdp("Page.captureScreenshot", params)?;
    let data = shot["data"].as_str().ok_or("화면을 찍지 못했습니다")?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(err)?;
    let name = if title.trim().is_empty() {
        tauri::Url::parse(&source).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
    } else {
        title.clone()
    };
    Ok(Captured { image: store(&bytes, Some("png"), &name)?, source, title, full, cut })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 번호와_형식을_확인한다() {
        assert!(is_id("i18f3a2bc0a1b2c"));
        assert!(!is_id(""));
        assert!(!is_id("..\\밖"));
        assert!(stored_path("a/b", "png").is_err());
        assert!(stored_path("abc", "exe").is_err());
        assert_eq!(format_of_extension("JPEG"), Some("jpg"));
        assert_eq!(format_of_extension("txt"), None);
    }

    #[test]
    fn 내용으로_형식을_알아낸다() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n0000"), Some("png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(sniff(b"GIF89a...."), Some("gif"));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Some("svg"));
        assert_eq!(sniff(b"<svg width=\"1\"/>"), Some("svg"));
        assert_eq!(sniff("그냥 글자".as_bytes()), None);
        assert_eq!(sniff(b"<html><body>svg</body></html>"), None);
    }

    #[test]
    fn bm으로_시작하는_글자_파일은_bmp가_아니다() {
        // BMP 머리글: "BM", 파일 크기(4), 예약(4, 모두 0), 그림 내용이 시작하는 자리(4).
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&70u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        assert_eq!(sniff(&bmp), Some("bmp"));
        assert_eq!(sniff(b"BMW 320d, BMW X5 - price list"), None);
        assert_eq!(sniff("BM 회의록: 다음 주 일정".as_bytes()), None);
        // 머리글이 다 들어 있지 않으면 BMP로 보지 않는다.
        assert_eq!(sniff(b"BM\x46\0\0\0\0\0\0\0"), None);
    }

    #[test]
    fn 여러_개를_가져올_때_안_되는_것은_건너뛴다() {
        let bad = |text: &str| Err::<u32, String>(text.to_string());
        // 하나가 안 돼도 나머지는 넣는다(순서 그대로).
        assert_eq!(only_done([Ok(1), bad("이미지 파일이 아닙니다"), Ok(3)]), Ok(vec![1, 3]));
        // 전부 안 되면 첫 오류를 돌려준다.
        assert_eq!(only_done([bad("첫째"), bad("둘째")]), Err("첫째".to_string()));
        // 고른 것이 없으면(취소) 빈 목록이다.
        assert_eq!(only_done(Vec::<Result<u32, String>>::new()), Ok(Vec::new()));
    }

    #[test]
    fn png를_만들고_다시_푼다() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 9, 9, 9];
        let png = encode_png(2, 2, &rgba).unwrap();
        assert_eq!(sniff(&png), Some("png"));
        let (width, height, back) = decode_png(&png).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(back, rgba);
    }

    #[test]
    fn url_인코딩을_푼다() {
        assert_eq!(percent_decode("%ED%99%94%EB%A9%B4%20a.png"), "화면 a.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%41"), "A");
    }

    #[test]
    fn data_주소의_그림을_받는다() {
        // 저장소(사용자 폴더)에 쓰지 않도록 형식 판별까지만 확인한다.
        assert!(fetch("data:text/plain,hello").is_err());
        assert!(fetch("file:///C:/Windows/win.ini").is_err());
        assert!(fetch("javascript:alert(1)").is_err());
    }
}
