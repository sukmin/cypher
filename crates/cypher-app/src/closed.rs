//! 닫은 항목 되살리기.
//!
//! 메모·브레인스토밍·이미지는 닫을 때 저장소의 `closed` 폴더로 옮겨 30일 동안 둔다
//! (`<닫은 시각>-<번호>.<확장자>`). 화면에서 "닫은 항목 다시 열기"를 고르면 그 파일을 원래 자리
//! (`<번호>.<확장자>`)로 되돌려 놓고, 화면이 그 번호로 항목을 다시 만든다.

use std::fs;
use std::path::Path;

/// 이미지 저장소가 다루는 형식(`image.rs`의 목록과 같다).
const IMAGE_FORMATS: &[&str] = &["png", "jpg", "gif", "webp", "bmp", "svg", "avif", "ico"];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 항목 번호는 화면이 건네준다. 경로로 쓰기 전에 영문·숫자뿐인지 확인한다.
fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// `dir\closed`에 보관된 `<닫은 시각>-<id>.<ext>`를 `dir\<id>.<ext>`로 되돌린다.
/// 되돌렸거나 이미 제자리에 있으면 true, 보관된 것이 없으면(30일이 지났거나 내용 없이 닫음) false.
/// 같은 번호가 여러 번 닫혔으면 가장 나중에 닫은 것을 되돌린다.
pub fn restore_in(dir: &Path, id: &str, ext: &str) -> Result<bool, String> {
    let live = dir.join(format!("{id}.{ext}"));
    if live.exists() {
        return Ok(true);
    }
    let suffix = format!("-{id}.{ext}");
    let Ok(entries) = fs::read_dir(dir.join("closed")) else { return Ok(false) };
    // 파일 이름이 닫은 시각(`2026-11-13-08-00-00`)으로 시작하므로 이름이 가장 큰 것이 가장 나중 것이다.
    let newest = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(&suffix))
        .max_by_key(|entry| entry.file_name());
    let Some(entry) = newest else { return Ok(false) };
    fs::rename(entry.path(), &live).map_err(err)?;
    Ok(true)
}

/// 닫은 항목의 내용을 저장소로 되돌린다. `kind`: "memo" | "mindmap" | "image"(이미지는 `ext`가 필요하다).
#[tauri::command]
pub fn closed_restore(kind: String, id: String, ext: Option<String>) -> Result<bool, String> {
    if !is_id(&id) {
        return Err("항목 번호가 올바르지 않습니다".into());
    }
    let (folder, ext) = match kind.as_str() {
        "memo" => ("memos", "txt".to_string()),
        "mindmap" => ("mindmaps", "md".to_string()),
        "image" => {
            let ext = ext.unwrap_or_default();
            if !IMAGE_FORMATS.contains(&ext.as_str()) {
                return Err("이미지 형식이 올바르지 않습니다".into());
            }
            ("images", ext)
        }
        _ => return Err("되살릴 수 없는 종류입니다".into()),
    };
    restore_in(&crate::user_dir().join(folder), &id, &ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 닫은_항목을_제자리로_되돌린다() {
        let dir = std::env::temp_dir().join("cypher-closed-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("closed")).unwrap();

        // 보관된 것이 없으면 false.
        assert_eq!(restore_in(&dir, "m1", "txt"), Ok(false));

        // 같은 번호가 두 번 닫혔으면 나중 것을 되돌린다. 번호가 다른 것(끝이 같은 것 포함)은 건드리지 않는다.
        fs::write(dir.join("closed").join("2026-11-12-09-00-00-m1.txt"), "예전").unwrap();
        fs::write(dir.join("closed").join("2026-11-13-09-00-00-m1.txt"), "나중").unwrap();
        fs::write(dir.join("closed").join("2026-11-14-09-00-00-xm1.txt"), "다른 메모").unwrap();
        fs::write(dir.join("closed").join("2026-11-14-09-00-00-m1.md"), "다른 종류").unwrap();
        assert_eq!(restore_in(&dir, "m1", "txt"), Ok(true));
        assert_eq!(fs::read_to_string(dir.join("m1.txt")).unwrap(), "나중");
        assert!(dir.join("closed").join("2026-11-12-09-00-00-m1.txt").exists());
        assert!(dir.join("closed").join("2026-11-14-09-00-00-xm1.txt").exists());

        // 이미 제자리에 있으면 그대로 두고 true(보관된 예전 것으로 덮어쓰지 않는다).
        assert_eq!(restore_in(&dir, "m1", "txt"), Ok(true));
        assert_eq!(fs::read_to_string(dir.join("m1.txt")).unwrap(), "나중");
    }

    #[test]
    fn 번호와_종류를_확인한다() {
        assert!(closed_restore("memo".into(), "..\\밖".into(), None).is_err());
        assert!(closed_restore("memo".into(), String::new(), None).is_err());
        assert!(closed_restore("session".into(), "a1".into(), None).is_err());
        assert!(closed_restore("image".into(), "a1".into(), Some("exe".into())).is_err());
        assert!(closed_restore("image".into(), "a1".into(), None).is_err());
    }
}
