//! 메모 항목: 쓰던 내용의 임시 저장과, 글자 파일 열기·저장.
//!
//! 메모마다 쓰던 내용을 `%APPDATA%\cypher\memos\<번호>.txt`에 계속 적어 둔다(파일로 저장하지 않은
//! 메모도 다시 시작하면 그대로 돌아온다). 닫은 메모는 바로 지우지 않고 `closed` 폴더로 옮겨
//! 30일 동안 둔다.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use cypher_core::textfile;

/// 편집기로 열 수 있는 파일 크기.
const MAX_BYTES: u64 = 8 << 20;
const KEEP_CLOSED: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Serialize)]
pub struct TextFile {
    path: String,
    text: String,
    encoding: &'static str,
    eol: &'static str,
    lossy: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn memo_dir() -> Result<PathBuf, String> {
    let dir = crate::user_dir().join("memos");
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

/// 메모 번호는 화면이 만든다. 경로로 쓰기 전에 영문·숫자뿐인지 확인한다.
fn is_memo_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn memo_path(id: &str) -> Result<PathBuf, String> {
    if !is_memo_id(id) {
        return Err("메모 번호가 올바르지 않습니다".into());
    }
    Ok(memo_dir()?.join(format!("{id}.txt")))
}

/// 이미 있는 파일의 내용만 바꿔치기한다(Win32 `ReplaceFileW`). 단순히 이름을 바꿔 덮는 것과 달리
/// 원래 파일의 속성(숨김 등), 권한, 대체 데이터 스트림, 만든 시각을 그대로 둔다.
fn replace_file(target: &Path, replacement: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }
    const REPLACEFILE_IGNORE_MERGE_ERRORS: u32 = 2;
    let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (target, replacement) = (wide(target), wide(replacement));
    // SAFETY: 널로 끝나는 유효한 경로 문자열 둘을 넘긴다. 나머지는 쓰지 않는 선택 인자다.
    unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ) != 0
    }
}

/// 임시 파일에 먼저 쓴 뒤 바꿔치기한다(쓰는 도중 꺼져도 원래 파일이 잘리지 않는다).
///
/// 이미 있는 파일은 속성·권한을 지키는 방식으로 바꿔치기하고, 그것이 안 되면(다른 프로그램이 열어 둔 파일 등)
/// 그 파일에 직접 쓴다. 어느 경우든 임시 파일은 남기지 않는다.
fn write_safely(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".cypher-tmp");
    let temp = PathBuf::from(temp);
    let written = fs::File::create(&temp).and_then(|mut file| {
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all() // 바꿔치기 전에 디스크까지 내려 쓴다.
    });
    if let Err(e) = written {
        let _ = fs::remove_file(&temp);
        // 임시 파일을 만들 수 없는 곳(쓰기 금지 폴더 안의 쓸 수 있는 파일 등)이면 직접 쓴다.
        return if path.exists() { fs::write(path, bytes).map_err(err) } else { Err(err(e)) };
    }
    let done = if path.exists() { replace_file(path, &temp) } else { fs::rename(&temp, path).is_ok() };
    if done {
        return Ok(());
    }
    let direct = fs::write(path, bytes).map_err(err);
    let _ = fs::remove_file(&temp);
    direct
}

/// 닫은 지 30일이 넘은 메모를 지운다(시작할 때 한 번).
pub fn clean_closed() {
    let Ok(entries) = memo_dir().and_then(|dir| fs::read_dir(dir.join("closed")).map_err(err)) else { return };
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

/// 쓰던 메모 내용. 아직 적어 둔 것이 없으면 None.
#[tauri::command]
pub fn memo_read(id: String) -> Result<Option<String>, String> {
    let path = memo_path(&id)?;
    if !path.exists() {
        return Ok(None);
    }
    fs::read(&path).map(|bytes| Some(String::from_utf8_lossy(&bytes).into_owned())).map_err(err)
}

#[tauri::command]
pub fn memo_write(id: String, text: String) -> Result<(), String> {
    write_safely(&memo_path(&id)?, text.as_bytes())
}

/// 메모를 닫는다. `keep`이면 내용을 `closed` 폴더로 옮기고 그 경로를 돌려준다.
#[tauri::command]
pub fn memo_remove(id: String, keep: bool) -> Result<Option<String>, String> {
    let path = memo_path(&id)?;
    if !path.exists() {
        return Ok(None);
    }
    if !keep {
        fs::remove_file(&path).map_err(err)?;
        return Ok(None);
    }
    let closed = memo_dir()?.join("closed");
    fs::create_dir_all(&closed).map_err(err)?;
    let stamp = crate::http::now_text().replace([':', ' '], "-");
    let target = closed.join(format!("{stamp}-{id}.txt"));
    fs::rename(&path, &target).map_err(err)?;
    // 보관 기간은 옮긴 때부터 센다.
    if let Ok(file) = fs::File::options().write(true).open(&target) {
        let _ = file.set_modified(SystemTime::now());
    }
    Ok(Some(target.to_string_lossy().into_owned()))
}

fn read_text_file(path: &Path) -> Result<TextFile, String> {
    let size = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if size > MAX_BYTES {
        return Err(format!("파일이 너무 큽니다({}MB). 8MB까지 열 수 있습니다.", size >> 20));
    }
    let decoded = textfile::decode(&fs::read(path).map_err(err)?)?;
    Ok(TextFile {
        path: path.to_string_lossy().into_owned(),
        text: decoded.text,
        encoding: decoded.encoding,
        eol: decoded.eol,
        lossy: decoded.lossy,
    })
}

/// 글자 파일을 연다. `path`가 없으면 고르는 창을 띄운다(취소하면 None).
#[tauri::command]
pub async fn text_open(path: Option<String>, start: Option<String>) -> Result<Option<TextFile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = match path {
            Some(path) => PathBuf::from(path),
            None => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("메모로 열 파일")
                    .add_filter("글자 파일", &["txt", "md", "json", "log", "http", "toml", "yaml", "yml", "csv", "sql"])
                    .add_filter("모든 파일", &["*"]);
                if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.pick_file() {
                    Some(path) => path,
                    None => return Ok(None),
                }
            }
        };
        read_text_file(&path).map(Some)
    })
    .await
    .map_err(err)?
}

/// 글을 파일로 저장한다. `path`가 없으면 저장할 곳을 묻는다(취소하면 None). 저장한 경로를 돌려준다.
#[tauri::command]
pub async fn text_save(
    path: Option<String>,
    name: String,
    start: Option<String>,
    text: String,
    encoding: String,
    eol: String,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = match path {
            Some(path) => PathBuf::from(path),
            None => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("메모 저장")
                    .set_file_name(name)
                    .add_filter("글자 파일", &["txt"])
                    .add_filter("Markdown", &["md"])
                    .add_filter("모든 파일", &["*"]);
                if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.save_file() {
                    Some(path) => path,
                    None => return Ok(None),
                }
            }
        };
        write_safely(&path, &textfile::encode(&text, &encoding, &eol)?)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 메모_번호는_영문과_숫자만() {
        assert!(is_memo_id("m1a2b3c"));
        assert!(!is_memo_id(""));
        assert!(!is_memo_id("..\\밖"));
        assert!(!is_memo_id("a/b"));
        assert!(!is_memo_id(&"a".repeat(41)));
    }

    #[test]
    fn 안전하게_쓰고_인코딩을_지켜_읽는다() {
        let dir = std::env::temp_dir().join("cypher-memo-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("한글 메모.txt");
        write_safely(&path, &textfile::encode("첫 줄\n둘째 줄", textfile::CP949, textfile::CRLF).unwrap()).unwrap();
        write_safely(&path, &textfile::encode("바꾼 줄\n둘째 줄", textfile::CP949, textfile::CRLF).unwrap()).unwrap();
        let file = read_text_file(&path).unwrap();
        assert_eq!((file.text.as_str(), file.encoding, file.eol), ("바꾼 줄\n둘째 줄", textfile::CP949, textfile::CRLF));
        // 임시 파일이 남지 않는다.
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn 저장해도_파일_속성이_그대로다() {
        use std::os::windows::fs::MetadataExt;
        let dir = std::env::temp_dir().join("cypher-memo-attr-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hidden.txt");
        fs::write(&path, "old").unwrap();
        let hide = std::process::Command::new("attrib").arg("+h").arg(&path).status().unwrap();
        assert!(hide.success());
        let created = fs::metadata(&path).unwrap().creation_time();
        std::thread::sleep(std::time::Duration::from_millis(30));

        write_safely(&path, b"new").unwrap();
        let after = fs::metadata(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_ne!(after.file_attributes() & 0x2, 0, "숨김 속성이 풀렸다");
        assert_eq!(after.creation_time(), created, "만든 시각이 바뀌었다");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

        // 읽기 전용 파일에는 쓰지 못하고, 임시 파일도 남기지 않는다.
        let readonly = dir.join("readonly.txt");
        fs::write(&readonly, "keep").unwrap();
        let mut permissions = fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&readonly, permissions.clone()).unwrap();
        assert!(write_safely(&readonly, b"x").is_err());
        assert_eq!(fs::read(&readonly).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&readonly, permissions).unwrap();
    }
}
