//! 문서 항목: Markdown·글자 파일을 읽기 좋은 모습으로 보여 준다(고치지는 않는다).
//!
//! 여기서는 파일을 고르고, 읽고, 바뀌었는지(고친 시각) 알려 주는 일만 한다. 서식을 그리는 것은 화면 쪽이다.
//! 화면은 보이는 동안 고친 시각을 주기적으로 물어, 바뀌었으면 다시 읽어 그린다.

use serde::Serialize;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;
use cypher_core::textfile;

/// 문서로 열 수 있는 파일 크기.
const MAX_BYTES: u64 = 8 << 20;

#[derive(Debug, Serialize)]
pub struct ReaderFile {
    /// 읽은 파일의 전체 경로.
    path: String,
    /// 줄 끝을 `\n`으로 통일한 글.
    text: String,
    /// 고친 시각(1970년부터의 밀리초). `reader_stamp`가 돌려주는 값과 같은 기준이다.
    modified: f64,
    bytes: u64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 파일의 고친 시각(밀리초). 파일이 없거나 파일이 아니면 None.
fn stamp_of(path: &Path) -> Option<f64> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0))
}

fn read(path: &Path) -> Result<ReaderFile, String> {
    // 화면이 문서 폴더 기준으로 전체 경로를 만들어 보낸다. 상대 경로는 앱의 실행 위치 기준이 되어 버리므로 받지 않는다.
    if !path.is_absolute() {
        return Err(format!("전체 경로가 아닙니다: {}", path.display()));
    }
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("파일이 아닙니다: {}", path.display()));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!("파일이 너무 큽니다({}MB). 8MB까지 볼 수 있습니다.", meta.len() >> 20));
    }
    // 시각을 먼저 적어 둔다. 읽는 사이에 파일이 또 바뀌면 다음 확인 때 시각이 달라 다시 읽게 된다.
    let modified = stamp_of(path).unwrap_or(0.0);
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let decoded = textfile::decode(&bytes)?;
    Ok(ReaderFile { path: path.to_string_lossy().into_owned(), text: decoded.text, modified, bytes: bytes.len() as u64 })
}

/// 문서로 볼 파일을 고른다. 취소하면 None.
#[tauri::command]
pub async fn reader_pick(start: Option<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_title("문서로 볼 파일")
            .add_filter("문서(Markdown·글자 파일)", &["md", "markdown", "txt", "log", "json", "toml", "yaml", "yml", "csv"])
            .add_filter("모든 파일", &["*"]);
        if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_file().map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// 표 항목으로 볼 수 있는 확장자: 값을 구분자로 나눈 글(CSV·TSV)과 JSON(배열, 한 줄에 하나씩).
pub const TABLE_FORMATS: &[&str] = &["csv", "tsv", "json", "jsonl", "ndjson"];

/// 표 항목으로 볼 수 있는 파일인지(확장자로 가린다).
pub fn is_table(path: &Path) -> bool {
    path.extension().is_some_and(|ext| TABLE_FORMATS.iter().any(|format| ext.eq_ignore_ascii_case(format)))
}

/// 표로 볼 파일을 고른다. 취소하면 None. 읽기와 "바뀌었는지"는 문서와 같은 명령(`reader_read`, `reader_stamp`)을 쓴다.
#[tauri::command]
pub async fn table_pick(start: Option<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_title("표로 볼 파일")
            .add_filter("표(CSV·TSV·JSON)", TABLE_FORMATS)
            .add_filter("모든 파일", &["*"]);
        if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_file().map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// 파일을 글로 읽는다. 인코딩(UTF-8, UTF-16, CP949)은 내용으로 알아본다.
#[tauri::command]
pub async fn reader_read(path: String) -> Result<ReaderFile, String> {
    tauri::async_runtime::spawn_blocking(move || read(Path::new(&path))).await.map_err(err)?
}

/// 파일의 고친 시각(밀리초). 화면이 파일이 바뀌었는지 볼 때 쓴다. 파일이 없으면 None.
/// 화면이 주기적으로 부르므로 작업 스레드에서 돈다(느린 드라이브의 파일이어도 창이 멈추지 않게).
#[tauri::command]
pub async fn reader_stamp(path: String) -> Option<f64> {
    stamp(&path)
}

/// 전체 경로로 적은 파일의 고친 시각. 상대 경로는 받지 않는다.
fn stamp(path: &str) -> Option<f64> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return None;
    }
    stamp_of(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 인코딩을_알아보고_읽는다() {
        let dir = test_dir("cypher-reader-test");
        let path = dir.join("계획 문서.md");
        fs::write(&path, textfile::encode("# 제목\n둘째 줄", textfile::CP949, textfile::CRLF).unwrap()).unwrap();
        let file = read(&path).unwrap();
        assert_eq!(file.text, "# 제목\n둘째 줄");
        assert_eq!(file.path, path.to_string_lossy());
        // 읽을 때 준 시각과 따로 물은 시각이 같아야 화면이 "바뀌지 않았다"고 본다.
        assert_eq!(Some(file.modified), stamp(&path.to_string_lossy()));
        assert!(file.modified > 0.0 && file.bytes > 0);
    }

    #[test]
    fn 표로_볼_파일은_확장자로_가린다() {
        for table in [r"C:\a\결과.csv", r"C:\a\b.TSV", r"C:\a\rows.json", r"C:\a\log.jsonl", r"C:\a\x.ndjson"] {
            assert!(is_table(Path::new(table)), "{table}");
        }
        for other in [r"C:\a\notes.md", r"C:\a\csv", r"C:\a\data.csv.bak", r"C:\a\x.xlsx"] {
            assert!(!is_table(Path::new(other)), "{other}");
        }
    }

    #[test]
    fn 바뀌면_시각이_달라진다() {
        let dir = test_dir("cypher-reader-stamp-test");
        let path = dir.join("a.md");
        fs::write(&path, "처음").unwrap();
        let first = stamp_of(&path).unwrap();
        // 고친 시각을 뒤로 옮겨 바뀐 것을 흉내 낸다(같은 밀리초 안에 다시 쓰면 구별할 수 없다).
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        assert_ne!(stamp_of(&path).unwrap(), first);
    }

    #[test]
    fn 없는_파일과_폴더와_상대_경로는_받지_않는다() {
        let dir = test_dir("cypher-reader-missing-test");
        let gone = dir.join("없는 파일.md");
        assert_eq!(stamp(&gone.to_string_lossy()), None);
        assert!(read(&gone).is_err());
        // 폴더는 문서가 아니다.
        assert_eq!(stamp_of(&dir), None);
        assert!(read(&dir).is_err());
        assert_eq!(stamp("상대\\경로.md"), None);
        assert!(read(Path::new("상대\\경로.md")).is_err());
    }

    #[test]
    fn 글자_파일이_아니거나_너무_크면_읽지_않는다() {
        let dir = test_dir("cypher-reader-binary-test");
        let binary = dir.join("그림.md");
        fs::write(&binary, [0x89, b'P', b'N', b'G', 0, 0, 0, 1]).unwrap();
        assert!(read(&binary).is_err());
        let big = dir.join("큰 파일.md");
        fs::write(&big, vec![b'a'; (MAX_BYTES + 1) as usize]).unwrap();
        assert!(read(&big).unwrap_err().contains("너무 큽니다"));
    }
}
