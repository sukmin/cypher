//! 삭제한 워크스페이스의 기록.
//!
//! 워크스페이스를 삭제하면 그때의 모습을 파일 하나로 적어 3년 동안 둔다:
//! `%APPDATA%\cypher\history\<삭제한 시각>-<번호>.json`. 파일 이름이 삭제한 시각으로 시작하므로 이름순이 곧 시간순이고,
//! 기록 하나가 파일 하나라서 한 기록이 깨져도 다른 기록은 그대로 읽힌다(3년 동안 쌓여도 목록 파일을 통째로 다시 쓰지 않는다).
//!
//! 파일에 담기는 것(화면이 만들어 건넨다):
//! ```json
//! {
//!   "version": 1,
//!   "name": "결제 개편",            // 워크스페이스 이름
//!   "folder": "C:\\work\\pay",      // 워크스페이스 폴더(정하지 않았으면 빈 글)
//!   "deletedAt": 1795000000000,     // 삭제한 때(밀리초) — 여기서 적는다
//!   "keepUntil": 1889700000000,     // 이때까지 보관(삭제한 때 + 3년) — 여기서 적는다
//!   "summary": "Claude 2 · 메모 1", // 항목 구성
//!   "items": [                      // 삭제할 때 그 워크스페이스에 있던 항목
//!     { "kind": "memo", "label": "회의 메모", "kindLabel": "메모", "cwd": "…", "saved": { "text": "…" } },
//!     { "kind": "term", "label": "Claude · pay", "profile": "claude", "resume": "대화 번호", "cwd": "…" }
//!   ]
//! }
//! ```
//! 글로 보관할 수 있는 것(메모·할 일·마인드맵·ERD의 글, 명령 항목의 명령)은 내용째 담고, 세션은 프로필·폴더·이어 갈 대화 번호만,
//! 사이트·HTTP·문서·표·코드·헥사·이미지 같은 산출물은 이름과 가리키던 주소·경로만 담는다(파일 내용은 담지 않는다).

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 보관 기간: 3년(윤년 하루를 넣어 1096일).
const KEEP_MS: u64 = 1096 * 24 * 60 * 60 * 1000;
/// 기록 하나의 크기 한도(메모·할 일의 글이 통째로 들어가므로 넉넉히 둔다).
const MAX_BYTES: u64 = 16 << 20;
/// 기록 하나에 담는 항목 수 한도.
const MAX_ITEMS: usize = 2000;

/// 목록에 보일 기록 하나.
#[derive(Debug, Serialize, PartialEq)]
pub struct HistoryInfo {
    /// 기록 번호(파일 이름에서 `.json`을 뺀 것). 읽거나 지울 때 이것으로 가리킨다.
    id: String,
    /// 워크스페이스 이름.
    name: String,
    /// 워크스페이스 폴더(정하지 않았으면 빈 글).
    folder: String,
    /// 삭제한 때(밀리초).
    deleted: f64,
    /// 이때까지 보관한다(밀리초).
    until: f64,
    /// 담긴 항목 수.
    items: usize,
    /// 항목 구성을 적은 글("Claude 2 · 메모 1").
    summary: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn history_dir() -> Result<PathBuf, String> {
    let dir = crate::user_dir().join("history");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 기록 번호는 화면이 건네준다. 경로로 쓰기 전에 영문·숫자·`-`뿐인지 확인한다.
fn is_id(id: &str) -> bool {
    (1..=80).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !id.starts_with('-')
}

fn stored(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_id(id) {
        return Err("기록 번호가 올바르지 않습니다".into());
    }
    Ok(dir.join(format!("{id}.json")))
}

fn modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_record(path: &Path) -> Result<Value, String> {
    let size = fs::metadata(path).map_err(|_| "기록이 없습니다(보관 기간 3년이 지났거나 지워졌습니다)".to_string())?.len();
    if size > MAX_BYTES {
        return Err("기록 파일이 너무 큽니다".into());
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(|e| format!("기록 파일을 읽지 못했습니다: {e}"))?;
    if !value.is_object() || !value["items"].is_array() {
        return Err("기록 파일의 형식이 올바르지 않습니다(items가 없습니다)".into());
    }
    Ok(value)
}

/// 그 기록을 삭제한 때와 보관이 끝나는 때. 파일에 적힌 값이 먼저고, 없으면(손으로 고친 파일 등) 파일을 쓴 때부터 센다.
fn times_of(path: &Path, value: &Value) -> (u64, u64) {
    let deleted = value["deletedAt"].as_u64().filter(|&at| at > 0).unwrap_or_else(|| modified_ms(path));
    let until = value["keepUntil"].as_u64().filter(|&at| at > 0).unwrap_or_else(|| deleted.saturating_add(KEEP_MS));
    (deleted, until)
}

fn info_of(path: &Path) -> Option<HistoryInfo> {
    let id = path.file_stem()?.to_string_lossy().into_owned();
    if !is_id(&id) {
        return None;
    }
    let value = read_record(path).ok()?;
    let (deleted, until) = times_of(path, &value);
    Some(HistoryInfo {
        name: value["name"].as_str().unwrap_or("").to_string(),
        folder: value["folder"].as_str().unwrap_or("").to_string(),
        deleted: deleted as f64,
        until: until as f64,
        items: value["items"].as_array().map_or(0, Vec::len),
        summary: value["summary"].as_str().unwrap_or("").to_string(),
        id,
    })
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")))
        .collect()
}

/// 보관 기간(3년)이 지난 기록을 지운다. 읽지 못하는 파일은 건드리지 않는다(사용자가 고치다 만 것일 수 있다).
fn clean_in(dir: &Path, now: u64) -> usize {
    let mut removed = 0;
    for path in json_files(dir) {
        let Ok(value) = read_record(&path) else { continue };
        if now > times_of(&path, &value).1 && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 앱을 시작할 때 한 번: 보관 기간이 지난 기록을 지운다.
pub fn clean_expired() {
    if let Ok(dir) = history_dir() {
        clean_in(&dir, now_ms());
    }
}

/// 최근에 삭제한 것이 앞에 오는 목록. 보관 기간이 지난 것은 빼고(파일은 다음에 시작할 때 지운다) 읽지 못하는 파일도 뺀다.
fn list_in(dir: &Path, now: u64) -> Vec<HistoryInfo> {
    let mut list: Vec<HistoryInfo> = json_files(dir).iter().filter_map(|path| info_of(path)).filter(|info| info.until >= now as f64).collect();
    list.sort_by(|a, b| b.deleted.total_cmp(&a.deleted).then_with(|| b.id.cmp(&a.id)));
    list
}

/// 화면이 건넨 기록을 확인하고, 삭제한 때와 보관이 끝나는 때를 적어 파일로 둔다. 기록 번호를 돌려준다.
fn save_in(dir: &Path, mut record: Value, now: u64, stamp: &str, token: &str) -> Result<String, String> {
    let Some(body) = record.as_object_mut() else { return Err("기록의 형식이 올바르지 않습니다".into()) };
    if body.get("name").and_then(Value::as_str).is_none_or(|name| name.trim().is_empty()) {
        return Err("워크스페이스 이름이 없습니다".into());
    }
    match body.get("items").and_then(Value::as_array) {
        Some(items) if items.len() <= MAX_ITEMS && items.iter().all(Value::is_object) => {}
        Some(items) if items.len() > MAX_ITEMS => return Err(format!("항목이 너무 많습니다(기록 하나에 {MAX_ITEMS}개까지)")),
        _ => return Err("기록의 형식이 올바르지 않습니다(items가 없습니다)".into()),
    }
    body.insert("version".into(), Value::from(1));
    body.insert("deletedAt".into(), Value::from(now));
    body.insert("keepUntil".into(), Value::from(now.saturating_add(KEEP_MS)));
    let bytes = serde_json::to_vec_pretty(&record).map_err(err)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("기록이 너무 큽니다({}MB · 한도 {}MB)", bytes.len() >> 20, MAX_BYTES >> 20));
    }
    // 같은 초에 여러 워크스페이스를 지워도 겹치지 않게 번호를 붙인다. 그래도 겹치면 뒤에 숫자를 더한다.
    let token: String = token.chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    let base = format!("{stamp}-{}", if token.is_empty() { "0" } else { &token });
    let mut id = base.clone();
    let mut again = 1;
    while stored(dir, &id)?.exists() {
        again += 1;
        id = format!("{base}{again}");
    }
    crate::memo::write_safely(&stored(dir, &id)?, &bytes)?;
    Ok(id)
}

/// 삭제하는 워크스페이스의 기록을 적어 둔다(3년 보관). 기록 번호를 돌려준다.
/// 기록을 읽고 쓰는 명령은 화면이 멈추지 않게 뒤에서 돈다(메모 글이 통째로 든 큰 파일일 수 있다).
#[tauri::command(async)]
pub fn history_save(record: Value) -> Result<String, String> {
    let stamp = crate::http::now_text().replace([':', ' '], "-");
    save_in(&history_dir()?, record, now_ms(), &stamp, &crate::random_token())
}

/// 삭제한 워크스페이스의 기록 목록(최근에 삭제한 것이 앞).
#[tauri::command(async)]
pub fn history_list() -> Result<Vec<HistoryInfo>, String> {
    Ok(list_in(&history_dir()?, now_ms()))
}

/// 기록 하나의 내용(항목과 보관해 둔 글).
#[tauri::command(async)]
pub fn history_read(id: String) -> Result<Value, String> {
    read_record(&stored(&history_dir()?, &id)?)
}

/// 기록 하나를 지운다(워크스페이스를 끝내 삭제하지 않았을 때, 또는 사용자가 목록에서 지울 때).
#[tauri::command]
pub fn history_remove(id: String) -> Result<(), String> {
    let path = stored(&history_dir()?, &id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 기록을_적고_목록과_내용으로_읽는다() {
        let dir = fresh("cypher-history-test");
        let day = 24 * 60 * 60 * 1000;
        let old = save_in(&dir, json!({ "name": "예전 업무", "folder": "C:\\work\\old", "summary": "메모 1",
            "items": [{ "kind": "memo", "label": "회의", "saved": { "text": "한글 메모\n둘째 줄" } }] }), 10 * day, "2026-01-01-09-00-00", "ab12").unwrap();
        let new = save_in(&dir, json!({ "name": "새 업무", "items": [] }), 20 * day, "2026-01-11-09-00-00", "cd34").unwrap();
        assert_eq!(old, "2026-01-01-09-00-00-ab12");

        // 최근에 삭제한 것이 앞이다. 삭제한 때와 보관이 끝나는 때(3년 뒤)는 저장할 때 적힌다.
        let list = list_in(&dir, 21 * day);
        assert_eq!(list.iter().map(|info| info.id.as_str()).collect::<Vec<_>>(), [new.as_str(), old.as_str()]);
        assert_eq!(list[1], HistoryInfo {
            id: old.clone(), name: "예전 업무".into(), folder: "C:\\work\\old".into(),
            deleted: (10 * day) as f64, until: (10 * day + KEEP_MS) as f64, items: 1, summary: "메모 1".into(),
        });
        assert_eq!(list[0].folder, "");

        // 보관해 둔 글은 그대로 돌아온다.
        let record = read_record(&stored(&dir, &old).unwrap()).unwrap();
        assert_eq!(record["items"][0]["saved"]["text"], "한글 메모\n둘째 줄");
        assert_eq!(record["version"], 1);

        // 같은 초에 같은 번호가 또 나와도 앞의 기록을 덮지 않는다.
        let twin = save_in(&dir, json!({ "name": "쌍둥이", "items": [] }), 20 * day, "2026-01-11-09-00-00", "cd34").unwrap();
        assert_ne!(twin, new);
        assert_eq!(list_in(&dir, 21 * day).len(), 3);
    }

    #[test]
    fn 삼_년이_지난_기록만_지운다() {
        let dir = fresh("cypher-history-clean-test");
        let day = 24 * 60 * 60 * 1000;
        let old = save_in(&dir, json!({ "name": "오래된 것", "items": [] }), day, "2020-01-01-00-00-00", "a").unwrap();
        let new = save_in(&dir, json!({ "name": "최근 것", "items": [] }), 400 * day, "2021-02-04-00-00-00", "b").unwrap();
        fs::write(dir.join("broken.json"), "{ 깨진 파일").unwrap();
        fs::write(dir.join("notes.txt"), "기록이 아닌 파일").unwrap();

        // 3년이 되기 전에는 아무것도 지우지 않는다.
        assert_eq!(clean_in(&dir, day + KEEP_MS), 0);
        assert_eq!(list_in(&dir, day + KEEP_MS).len(), 2);

        // 3년이 지난 것은 목록에서 빠지고, 정리하면 그 파일만 지워진다(읽지 못하는 파일과 다른 파일은 그대로 둔다).
        let later = day + KEEP_MS + 1;
        assert_eq!(list_in(&dir, later).iter().map(|info| info.id.clone()).collect::<Vec<_>>(), [new.clone()]);
        assert_eq!(clean_in(&dir, later), 1);
        assert!(!stored(&dir, &old).unwrap().exists());
        assert!(stored(&dir, &new).unwrap().exists());
        assert!(dir.join("broken.json").exists() && dir.join("notes.txt").exists());
    }

    #[test]
    fn 형식과_번호를_확인한다() {
        let dir = fresh("cypher-history-check-test");
        assert!(save_in(&dir, json!([1, 2]), 1, "s", "t").is_err());
        assert!(save_in(&dir, json!({ "name": "  ", "items": [] }), 1, "s", "t").is_err());
        assert!(save_in(&dir, json!({ "name": "이름", "items": "글" }), 1, "s", "t").is_err());
        assert!(save_in(&dir, json!({ "name": "이름", "items": [1] }), 1, "s", "t").is_err());
        assert!(save_in(&dir, json!({ "name": "이름" }), 1, "s", "t").is_err());
        assert!(json_files(&dir).is_empty());

        for bad in ["", "..", "..\\밖", "a/b", "a.json", "-a", "한글", &"a".repeat(81)] {
            assert!(stored(&dir, bad).is_err(), "{bad:?}");
        }
        assert!(stored(&dir, "2026-11-25-10-00-00-ab12").is_ok());
        assert!(read_record(&stored(&dir, "nothing").unwrap()).is_err());
    }
}
