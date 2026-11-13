//! 그룹 틀: 그룹 하나에 든 항목(세션 폴더, 메모, 사이트 주소, HTTP 폴더 …)을 파일로 저장해 두고,
//! 같은 종류의 업무를 시작할 때 한 번에 펼친다.
//!
//! 틀은 `%APPDATA%\cypher\templates\<이름>.json`에 둔다(파일을 복사해 다른 PC로 옮길 수 있다).
//! 틀의 내용(항목 목록)은 화면이 만들고 읽는다. 여기서는 파일로 두고 꺼내는 일만 한다.

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 틀 파일 하나의 크기 한도(메모 글이 통째로 들어가므로 넉넉히 둔다).
const MAX_BYTES: u64 = 4 << 20;

/// 목록에 보일 틀 하나.
#[derive(Debug, Serialize)]
pub struct TemplateInfo {
    /// 파일 이름(`결제 개편.json`). 읽거나 지울 때 이것으로 가리킨다.
    file: String,
    /// 틀 이름.
    name: String,
    /// 담긴 항목 수.
    items: usize,
    /// 항목 구성을 적은 글("Claude 2 · 메모 1"). 화면이 저장할 때 적어 둔다.
    summary: String,
    /// 저장한 때(밀리초).
    saved: f64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn template_dir() -> Result<PathBuf, String> {
    let dir = crate::user_dir().join("templates");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 틀 이름을 파일 이름으로 바꾼다. 파일 이름에 쓸 수 없는 글자는 `_`로 바꾸고, 쓸 수 있는 글자가 없으면 None.
fn file_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let cleaned: String = cleaned.trim().trim_end_matches(['.', ' ']).chars().take(80).collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '_' || c == '.') {
        return None;
    }
    // Windows가 장치 이름으로 읽는 이름(CON, NUL, COM1 …)은 앞에 `_`를 붙인다.
    let upper = cleaned.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    let device = matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit()));
    Some(if device { format!("_{cleaned}.json") } else { format!("{cleaned}.json") })
}

/// 화면이 건넨 파일 이름으로 틀 폴더 안의 경로를 만든다. 폴더 밖을 가리키는 이름은 받지 않는다.
fn stored_path(file: &str) -> Result<PathBuf, String> {
    let plain = !file.is_empty()
        && file.ends_with(".json")
        && !file.contains(['\\', '/', ':'])
        && file != ".json"
        && Path::new(file).file_name().is_some_and(|name| name == file);
    if !plain {
        return Err("틀 파일 이름이 올바르지 않습니다".into());
    }
    Ok(template_dir()?.join(file))
}

fn modified_ms(path: &Path) -> f64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn read_template(path: &Path) -> Result<Value, String> {
    let size = fs::metadata(path).map_err(|_| "틀 파일이 없습니다".to_string())?.len();
    if size > MAX_BYTES {
        return Err("틀 파일이 너무 큽니다".into());
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(|e| format!("틀 파일을 읽지 못했습니다: {e}"))?;
    if !value["items"].is_array() {
        return Err("틀 파일의 형식이 올바르지 않습니다(items가 없습니다)".into());
    }
    Ok(value)
}

fn info_of(path: &Path) -> Option<TemplateInfo> {
    let file = path.file_name()?.to_string_lossy().into_owned();
    let value = read_template(path).ok()?;
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    Some(TemplateInfo {
        name: value["name"].as_str().filter(|n| !n.trim().is_empty()).map(str::to_string).unwrap_or(stem),
        items: value["items"].as_array().map_or(0, Vec::len),
        summary: value["summary"].as_str().unwrap_or("").to_string(),
        saved: value["savedAt"].as_f64().unwrap_or_else(|| modified_ms(path)),
        file,
    })
}

fn list_in(dir: &Path) -> Vec<TemplateInfo> {
    let mut list: Vec<TemplateInfo> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")))
        .filter_map(|path| info_of(&path))
        .collect();
    // 최근에 저장한 것이 앞.
    list.sort_by(|a, b| b.saved.total_cmp(&a.saved).then_with(|| a.name.cmp(&b.name)));
    list
}

fn save_in(dir: &Path, name: &str, body: &Value) -> Result<PathBuf, String> {
    if !body["items"].is_array() {
        return Err("틀에 담을 항목이 없습니다".into());
    }
    let file = file_name(name).ok_or("틀 이름에 쓸 수 있는 글자가 없습니다")?;
    let text = serde_json::to_string_pretty(body).map_err(err)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("틀이 너무 큽니다({}MB). 메모에 든 글을 줄이거나 파일로 저장한 뒤 다시 해 보세요.", text.len() >> 20));
    }
    let path = dir.join(file);
    // 이름은 다른데 파일 이름이 같아지는 틀(`결제/개편`과 `결제_개편`, 앞 80자가 같은 이름)을 말없이 덮어쓰지 않는다.
    // 화면은 틀 이름으로 겹치는지 묻기 때문에, 이 경우는 묻지 않고 여기까지 온다.
    if let Some(other) = info_of(&path).filter(|other| other.name.to_lowercase() != name.to_lowercase()) {
        return Err(format!("다른 틀 \"{}\"과(와) 파일 이름이 같아집니다. 틀 이름을 다르게 지어 주세요.", other.name));
    }
    crate::memo::write_safely(&path, text.as_bytes())?;
    Ok(path)
}

/// 저장해 둔 틀의 목록(최근에 저장한 것이 앞). 읽을 수 없는 파일은 건너뛴다.
#[tauri::command]
pub async fn template_list() -> Result<Vec<TemplateInfo>, String> {
    Ok(list_in(&template_dir()?))
}

/// 틀을 저장한다. 같은 이름의 틀이 있으면 바꿔 쓴다(바꿔 써도 되는지는 화면이 먼저 묻는다). 저장한 경로를 돌려준다.
#[tauri::command]
pub async fn template_save(name: String, body: Value) -> Result<String, String> {
    save_in(&template_dir()?, &name, &body).map(|path| path.to_string_lossy().into_owned())
}

/// 틀 하나의 내용.
#[tauri::command]
pub async fn template_read(file: String) -> Result<Value, String> {
    read_template(&stored_path(&file)?)
}

#[tauri::command]
pub fn template_remove(file: String) -> Result<(), String> {
    fs::remove_file(stored_path(&file)?).map_err(err)
}

/// 틀을 둔 폴더를 탐색기에서 연다(파일을 복사해 옮기거나 지울 때).
#[tauri::command]
pub fn template_folder() -> Result<String, String> {
    let dir = template_dir()?;
    std::process::Command::new("explorer.exe").arg(&dir).spawn().map_err(err)?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 폴더들이 지금도 있는지. 틀을 펼치기 전에, 없어진 폴더의 항목을 다른 폴더에서 열려고 확인한다.
/// 네트워크 경로(`\\서버\…`)는 확인하느라 멈출 수 있으므로 보지 않고 없는 것으로 친다.
#[tauri::command]
pub async fn dirs_exist(paths: Vec<String>) -> Vec<bool> {
    paths.iter().map(|path| !path.starts_with(r"\\") && !path.starts_with("//") && Path::new(path).is_dir()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 틀_이름을_파일_이름으로_바꾼다() {
        assert_eq!(file_name("결제 개편").as_deref(), Some("결제 개편.json"));
        assert_eq!(file_name(" a/b:c ").as_deref(), Some("a_b_c.json"));
        assert_eq!(file_name("끝에 점...").as_deref(), Some("끝에 점.json"));
        assert_eq!(file_name("con").as_deref(), Some("_con.json"));
        assert_eq!(file_name("COM1").as_deref(), Some("_COM1.json"));
        assert_eq!(file_name("command").as_deref(), Some("command.json"));
        assert_eq!(file_name(""), None);
        assert_eq!(file_name("  ..  "), None);
        assert_eq!(file_name("///"), None);
        assert_eq!(file_name(&"가".repeat(200)).map(|f| f.chars().count()), Some(85));
    }

    #[test]
    fn 틀_폴더_밖을_가리키는_이름은_받지_않는다() {
        for bad in ["", ".json", "..\\밖.json", "a/b.json", "C:x.json", "틀.txt", "..", "a\\..\\b.json"] {
            assert!(stored_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn 저장하고_목록에서_다시_읽는다() {
        let dir = std::env::temp_dir().join("cypher-template-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let body = json!({ "version": 1, "name": "결제 개편", "summary": "Claude 1 · 메모 1", "savedAt": 2000.0,
                           "items": [{ "kind": "term", "profile": "claude", "cwd": "C:\\ws" }, { "kind": "memo", "saved": { "text": "할 일" } }] });
        let path = save_in(&dir, "결제 개편", &body).unwrap();
        assert_eq!(path.file_name().unwrap().to_string_lossy(), "결제 개편.json");
        save_in(&dir, "예전 것", &json!({ "name": "예전 것", "savedAt": 1000.0, "items": [] })).unwrap();
        // 틀이 아닌 파일과 깨진 파일은 목록에 나오지 않는다.
        fs::write(dir.join("깨짐.json"), "{").unwrap();
        fs::write(dir.join("다른 것.json"), r#"{"name":"x"}"#).unwrap();
        fs::write(dir.join("메모.txt"), "x").unwrap();

        let list = list_in(&dir);
        let got: Vec<_> = list.iter().map(|t| (t.name.as_str(), t.items, t.summary.as_str())).collect();
        assert_eq!(got, vec![("결제 개편", 2, "Claude 1 · 메모 1"), ("예전 것", 0, "")]);
        assert_eq!(read_template(&path).unwrap()["items"][1]["saved"]["text"], "할 일");
        // 같은 이름으로 다시 저장하면 바꿔 쓴다(임시 파일이 남지 않는다).
        save_in(&dir, "결제 개편", &json!({ "name": "결제 개편", "items": [] })).unwrap();
        assert_eq!(list_in(&dir).iter().find(|t| t.name == "결제 개편").unwrap().items, 0);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 5);
        assert!(save_in(&dir, "항목 없음", &json!({ "name": "x" })).is_err());
        // 이름은 다른데 파일 이름이 같아지는 틀은 덮어쓰지 않는다. 대/소문자만 다른 이름은 같은 틀이다.
        save_in(&dir, "a/b", &json!({ "name": "a/b", "items": [1] })).unwrap();
        assert!(save_in(&dir, "a_b", &json!({ "name": "a_b", "items": [] })).is_err());
        assert_eq!(list_in(&dir).iter().find(|t| t.name == "a/b").unwrap().items, 1);
        save_in(&dir, "A/B", &json!({ "name": "A/B", "items": [] })).unwrap();
    }
}
