//! 메모 항목: 쓰던 내용의 임시 저장과, 글자 파일 열기·저장.
//!
//! 메모마다 쓰던 내용을 `%APPDATA%\cypher\memos\<번호>.txt`에 계속 적어 둔다(파일로 저장하지 않은
//! 메모도 다시 시작하면 그대로 돌아온다). 닫은 메모는 바로 지우지 않고 `closed` 폴더로 옮겨
//! 30일 동안 둔다.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use cypher_core::textfile;

/// 편집기로 열 수 있는 파일 크기.
const MAX_BYTES: u64 = 8 << 20;
const KEEP_CLOSED: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// 화면이 마지막으로 본 뒤에 디스크의 파일이 바뀌어 저장하지 않았을 때 `text_save`가 돌려주는 오류 글의 머리.
/// 화면(메모)은 이 글로 "다른 곳에서 바뀌었다"를 알아보고 덮어쓸지 다시 읽을지 묻는다.
const CHANGED_ON_DISK: &str = "changed-on-disk";

#[derive(Serialize)]
pub struct TextFile {
    path: String,
    text: String,
    encoding: &'static str,
    eol: &'static str,
    lossy: bool,
    /// 읽을 때의 고친 시각(1970년부터의 밀리초). 화면이 저장할 때 `expected`로 되돌려 준다.
    modified: Option<f64>,
}

/// `text_save`가 돌려주는 것: 보통은 저장한 경로만, `stamped`를 청했으면 저장한 뒤의 고친 시각도 함께.
#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Saved {
    Path(String),
    Stamped { path: String, modified: Option<f64> },
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
pub(crate) fn write_safely(path: &Path, bytes: &[u8]) -> Result<(), String> {
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

/// 파일의 고친 시각(1970년부터의 밀리초). 파일이 없거나 파일이 아니면 None.
/// 문서 항목의 `reader_stamp`와 같은 기준이라, 화면은 그 명령으로 지금 시각을 물어 이 값과 견줄 수 있다.
fn stamp_of(path: &Path) -> Option<f64> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0))
}

/// 화면이 마지막으로 본 판(`expected`: 그때의 고친 시각, 본 적이 없으면 0)과 디스크의 지금 파일이 다른지.
/// `expected`를 주지 않았거나(견주지 않는 저장) 파일이 지금 없으면(저장하면 새로 생긴다) 다르지 않은 것으로 본다.
fn changed_since(path: &Path, expected: Option<f64>) -> bool {
    match (expected, stamp_of(path)) {
        (Some(seen), Some(now)) => now != seen,
        _ => false,
    }
}

/// 글을 그 인코딩과 줄 끝 형식으로 파일에 쓰고, 쓴 뒤의 고친 시각을 돌려준다.
/// `expected`를 주었는데 그 뒤로 파일이 바뀌어 있으면 쓰지 않고 `changed-on-disk`로 시작하는 오류를 돌려준다
/// (다른 곳에서 고친 내용을 말없이 덮어쓰지 않게).
fn save_text_file(path: &Path, text: &str, encoding: &str, eol: &str, expected: Option<f64>) -> Result<Option<f64>, String> {
    if changed_since(path, expected) {
        return Err(format!("{CHANGED_ON_DISK}: {}", path.display()));
    }
    write_safely(path, &textfile::encode(text, encoding, eol)?)?;
    Ok(stamp_of(path))
}

/// 메모의 쓰던 내용(`dir\<id>.txt`)을 `dir\closed`로 옮기고 옮긴 경로를 돌려준다. 옮길 파일이 없으면 None.
///
/// 닫은 메모는 `<시각>-<id>.txt`로 둔다("닫은 항목 다시 열기"가 이 이름으로 되찾아 간다).
/// 바꾸기 전의 글(`before`: Claude가 통째로 바꾸기 전, 디스크의 내용으로 다시 읽기 전)은 `<시각>-<id>.before.txt`로 두어
/// 되찾기에 걸리지 않게 한다. 닫은 메모를 다시 열 때 이 보관본이 쓰던 내용 자리로 옮겨져 사라지면 안 되기 때문이다.
/// 같은 초에 두 번 바꿔 이름이 겹치면 번호를 붙인다. 어느 쪽이든 30일 뒤에 지운다(`clean_closed`).
fn archive(dir: &Path, id: &str, stamp: &str, before: bool) -> Result<Option<String>, String> {
    let path = dir.join(format!("{id}.txt"));
    if !path.exists() {
        return Ok(None);
    }
    let closed = dir.join("closed");
    fs::create_dir_all(&closed).map_err(err)?;
    let mut target = closed.join(if before { format!("{stamp}-{id}.before.txt") } else { format!("{stamp}-{id}.txt") });
    if before {
        for n in 2..1000 {
            if !target.exists() {
                break;
            }
            target = closed.join(format!("{stamp}-{id}.before-{n}.txt"));
        }
    }
    fs::rename(&path, &target).map_err(err)?;
    // 보관 기간은 옮긴 때부터 센다.
    if let Ok(file) = fs::File::options().write(true).open(&target) {
        let _ = file.set_modified(SystemTime::now());
    }
    Ok(Some(target.to_string_lossy().into_owned()))
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

/// 메모의 쓰던 내용을 치운다. `keep`이면 내용을 `closed` 폴더로 옮기고 그 경로를 돌려준다.
/// `before`이면 닫은 메모가 아니라 "바꾸기 전의 글"로 보관한다(닫은 항목 다시 열기가 되찾아 가지 않는 이름).
#[tauri::command]
pub fn memo_remove(id: String, keep: bool, before: Option<bool>) -> Result<Option<String>, String> {
    let path = memo_path(&id)?;
    if !path.exists() {
        return Ok(None);
    }
    if !keep {
        fs::remove_file(&path).map_err(err)?;
        return Ok(None);
    }
    let stamp = crate::http::now_text().replace([':', ' '], "-");
    archive(&memo_dir()?, &id, &stamp, before.unwrap_or(false))
}

fn read_text_file(path: &Path) -> Result<TextFile, String> {
    let size = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if size > MAX_BYTES {
        return Err(format!("파일이 너무 큽니다({}MB). 8MB까지 열 수 있습니다.", size >> 20));
    }
    // 시각을 먼저 적어 둔다. 읽는 사이에 파일이 또 바뀌면 시각이 달라져, 저장할 때 바뀐 것으로 알아본다.
    let modified = stamp_of(path);
    let decoded = textfile::decode(&fs::read(path).map_err(err)?)?;
    Ok(TextFile {
        path: path.to_string_lossy().into_owned(),
        text: decoded.text,
        encoding: decoded.encoding,
        eol: decoded.eol,
        lossy: decoded.lossy,
        modified,
    })
}

/// 글자 파일을 연다. `path`가 없으면 고르는 창을 띄운다(취소하면 None).
#[tauri::command]
pub async fn text_open(path: Option<String>, start: Option<String>, kind: Option<String>) -> Result<Option<TextFile>, String> {
    let mindmap = kind.as_deref() == Some("mindmap");
    tauri::async_runtime::spawn_blocking(move || {
        let path = match path {
            Some(path) => PathBuf::from(path),
            None => {
                let mut dialog = rfd::FileDialog::new().set_title(if mindmap { "브레인스토밍으로 열 개요 파일" } else { "메모로 열 파일" });
                dialog = if mindmap {
                    dialog.add_filter("개요 글(Markdown·글자 파일)", &["md", "markdown", "txt"])
                } else {
                    dialog.add_filter("글자 파일", &["txt", "md", "json", "log", "http", "toml", "yaml", "yml", "csv", "sql"])
                };
                dialog = dialog.add_filter("모든 파일", &["*"]);
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
///
/// `expected`(화면이 마지막으로 본 판의 고친 시각, 본 적이 없으면 0)를 주면, 그 뒤로 파일이 바뀌어 있을 때
/// 쓰지 않고 `changed-on-disk`로 시작하는 오류를 돌려준다. `stamped`이면 경로 대신 `{ path, modified }`를 돌려준다.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn text_save(
    path: Option<String>,
    name: String,
    start: Option<String>,
    text: String,
    encoding: String,
    eol: String,
    kind: Option<String>,
    expected: Option<f64>,
    stamped: Option<bool>,
) -> Result<Option<Saved>, String> {
    let mindmap = kind.as_deref() == Some("mindmap");
    tauri::async_runtime::spawn_blocking(move || {
        let path = match path {
            Some(path) => PathBuf::from(path),
            None => {
                let mut dialog = rfd::FileDialog::new().set_title(if mindmap { "브레인스토밍 저장" } else { "메모 저장" }).set_file_name(name);
                dialog = if mindmap {
                    dialog.add_filter("Markdown 개요", &["md"]).add_filter("글자 파일", &["txt"])
                } else {
                    dialog.add_filter("글자 파일", &["txt"]).add_filter("Markdown", &["md"])
                };
                dialog = dialog.add_filter("모든 파일", &["*"]);
                if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.save_file() {
                    Some(path) => path,
                    None => return Ok(None),
                }
            }
        };
        let modified = save_text_file(&path, &text, &encoding, &eol, expected)?;
        let path = path.to_string_lossy().into_owned();
        Ok(Some(if stamped.unwrap_or(false) { Saved::Stamped { path, modified } } else { Saved::Path(path) }))
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
    fn 다른_곳에서_바뀐_파일은_덮어쓰지_않는다() {
        let dir = std::env::temp_dir().join("cypher-memo-changed-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("계획.md");
        fs::write(&path, "처음\n").unwrap();
        let seen = read_text_file(&path).unwrap().modified;
        assert!(seen.is_some_and(|ms| ms > 0.0));
        // 문서 항목의 reader_stamp와 같은 기준이어야 화면이 그 명령으로 견줄 수 있다.
        assert_eq!(seen, tauri::async_runtime::block_on(crate::reader::reader_stamp(path.to_string_lossy().into_owned())));

        // 본 그대로면 저장되고, 저장한 뒤의 시각을 돌려준다(읽어서 얻는 시각과 같다).
        let saved = save_text_file(&path, "내가 고침\n", textfile::UTF8, textfile::LF, seen).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "내가 고침\n");
        assert_eq!(saved, read_text_file(&path).unwrap().modified);

        // 그 뒤에 다른 곳에서 고쳤다(고친 시각이 달라졌다): 쓰지 않고 구분되는 오류를 돌려준다.
        fs::write(&path, "다른 곳에서 고침\n").unwrap();
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        let refused = save_text_file(&path, "덮어쓰려는 글\n", textfile::UTF8, textfile::LF, saved).unwrap_err();
        assert!(refused.starts_with(CHANGED_ON_DISK), "{refused}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "다른 곳에서 고침\n");
        // 본 적이 없는(0) 파일이 이미 있어도 쓰지 않는다.
        assert!(save_text_file(&path, "x", textfile::UTF8, textfile::LF, Some(0.0)).unwrap_err().starts_with(CHANGED_ON_DISK));
        assert_eq!(fs::read_to_string(&path).unwrap(), "다른 곳에서 고침\n");

        // 견주지 않는 저장(덮어쓰기를 골랐거나 다른 이름으로 저장)은 그대로 쓴다.
        let forced = save_text_file(&path, "덮어씀\n", textfile::UTF8, textfile::LF, None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "덮어씀\n");
        assert_eq!(forced, read_text_file(&path).unwrap().modified);
        // 파일이 없어졌으면 본 시각이 있어도 새로 만든다(잃을 내용이 없다).
        fs::remove_file(&path).unwrap();
        assert!(save_text_file(&path, "다시 만듦\n", textfile::UTF8, textfile::LF, forced).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "다시 만듦\n");

        // 화면에 돌려주는 모양: 보통은 경로 글, 시각을 청했으면 묶음.
        assert_eq!(serde_json::to_value(Saved::Path("a.txt".into())).unwrap(), serde_json::json!("a.txt"));
        assert_eq!(
            serde_json::to_value(Saved::Stamped { path: "a.txt".into(), modified: Some(5.0) }).unwrap(),
            serde_json::json!({ "path": "a.txt", "modified": 5.0 })
        );
    }

    #[test]
    fn 바꾸기_전의_글은_닫은_메모와_다른_이름으로_보관한다() {
        let dir = std::env::temp_dir().join("cypher-memo-archive-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let stamp = "2026-11-13-09-00-00";
        // 보관할 것이 없으면 None.
        assert_eq!(archive(&dir, "m1", stamp, true), Ok(None));

        // 바꾸기 전의 글: 같은 초에 두 번 바꿔도 앞의 보관본을 덮어쓰지 않는다.
        fs::write(dir.join("m1.txt"), "처음 글").unwrap();
        let first = archive(&dir, "m1", stamp, true).unwrap().unwrap();
        fs::write(dir.join("m1.txt"), "Claude가 바꾼 글").unwrap();
        let second = archive(&dir, "m1", stamp, true).unwrap().unwrap();
        assert!(first.ends_with("2026-11-13-09-00-00-m1.before.txt"), "{first}");
        assert!(second.ends_with("2026-11-13-09-00-00-m1.before-2.txt"), "{second}");
        assert_eq!(fs::read_to_string(&first).unwrap(), "처음 글");
        assert_eq!(fs::read_to_string(&second).unwrap(), "Claude가 바꾼 글");
        assert!(!dir.join("m1.txt").exists());

        // 닫은 메모를 다시 열 때(쓰던 내용 없이 닫은, 파일에 연결된 메모 등) 이 보관본을 되찾아 가지 않는다.
        assert_eq!(crate::closed::restore_in(&dir, "m1", "txt"), Ok(false));
        assert!(Path::new(&first).exists() && Path::new(&second).exists() && !dir.join("m1.txt").exists());

        // 닫은 메모는 예전 이름 그대로 보관되고 되찾아진다. 바꾸기 전의 글은 그대로 남는다.
        fs::write(dir.join("m1.txt"), "닫을 때의 글").unwrap();
        let closed = archive(&dir, "m1", "2026-11-13-08-00-00", false).unwrap().unwrap();
        assert!(closed.ends_with("2026-11-13-08-00-00-m1.txt"), "{closed}");
        assert_eq!(crate::closed::restore_in(&dir, "m1", "txt"), Ok(true));
        assert_eq!(fs::read_to_string(dir.join("m1.txt")).unwrap(), "닫을 때의 글");
        assert!(Path::new(&first).exists() && Path::new(&second).exists());
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
