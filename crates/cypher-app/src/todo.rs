//! 할 일 항목: 그룹(업무)의 체크 목록을 담는 항목의 저장소.
//!
//! 내용은 Markdown 체크 목록(`- [ ] 할 일`, `- [x] 끝낸 일`) 글 그대로 `%APPDATA%\cypher\todos\<번호>.md`에
//! 자동 저장한다(UTF-8). 글을 읽어 목록으로 보여 주고 체크하는 일은 화면(`ui/todo.js`)이 직접 한다.
//! 닫은 항목의 내용은 바로 지우지 않고 `todos\closed`에 30일 동안 둔다.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const KEEP_CLOSED: Duration = Duration::from_secs(30 * 24 * 60 * 60);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn dir() -> Result<PathBuf, String> {
    let dir = crate::user_dir().join("todos");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 항목 번호는 영문·숫자만 받는다(경로를 벗어나지 못하게).
fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn stored(id: &str) -> Result<PathBuf, String> {
    if !is_id(id) {
        return Err("항목 번호가 올바르지 않습니다".into());
    }
    Ok(dir()?.join(format!("{id}.md")))
}

/// 보관 기간이 지난 닫은 할 일을 지운다(앱을 시작할 때 한 번).
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

/// 자동 저장해 둔 글. 아직 적어 둔 것이 없으면 None.
#[tauri::command]
pub fn todo_read(id: String) -> Result<Option<String>, String> {
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
pub fn todo_write(id: String, text: String) -> Result<(), String> {
    crate::memo::write_safely(&stored(&id)?, text.as_bytes())
}

/// 항목을 닫을 때: `keep`이면 내용을 `closed` 폴더로 옮겨 30일 동안 두고 그 경로를 돌려준다. 아니면 지운다.
#[tauri::command]
pub fn todo_remove(id: String, keep: bool) -> Result<Option<String>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 항목_번호는_영문과_숫자만_받는다() {
        assert!(is_id("t1abc") && is_id("A9"));
        for bad in ["", "../x", "a.b", "a b", "한글", &"a".repeat(41)] {
            assert!(!is_id(bad), "{bad:?}");
        }
    }
}
