//! 결과 확인용 명령: 변경 내용(git), 출력 저장. Claude 설정 폴더 위치도 여기서 구한다.

use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_DIFF_BYTES: usize = 400_000;

// ───────────────────────── 변경 내용 ─────────────────────────

#[derive(Debug, PartialEq, Serialize)]
pub struct Change {
    /// 화면에 보일 상태 이름(수정, 새 파일 …).
    status: &'static str,
    /// 저장소 맨 위 폴더 기준 경로.
    path: String,
    untracked: bool,
}

#[derive(Serialize)]
pub struct Changes {
    root: String,
    branch: String,
    files: Vec<Change>,
}

/// git을 실행해 표준 출력을 돌려준다. `ok_codes`에 없는 종료 코드는 오류로 본다.
fn git(dir: &Path, args: &[&str], ok_codes: &[i32]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0") // 보기만 하므로 색인 잠금을 잡지 않는다.
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("git을 실행하지 못했습니다: {e}"))?;
    if !ok_codes.contains(&output.status.code().unwrap_or(-1)) {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `git status --porcelain=v1 -z` 출력을 읽는다. 항목은 NUL로 구분되고,
/// 이름 변경·복사는 원래 경로가 한 항목 더 따라온다.
fn parse_status(raw: &str) -> Vec<Change> {
    let mut files = Vec::new();
    let mut entries = raw.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        let (Some(code), Some(path)) = (entry.get(..2), entry.get(3..)) else { continue };
        let status = match code {
            "??" => "새 파일",
            c if c.contains('R') => "이름 변경",
            c if c.contains('C') => "복사",
            c if c.contains('D') => "삭제",
            c if c.contains('A') => "추가",
            c if c.contains('U') => "충돌",
            _ => "수정",
        };
        if code.contains('R') || code.contains('C') {
            entries.next(); // 원래 경로
        }
        files.push(Change { status, path: path.to_string(), untracked: code == "??" });
    }
    files
}

/// 세션 폴더가 속한 저장소의 바뀐 파일 목록.
#[tauri::command]
pub async fn git_changes(cwd: String) -> Result<Changes, String> {
    let cwd = Path::new(&cwd);
    let root = git(cwd, &["rev-parse", "--show-toplevel"], &[0])
        .map_err(|_| "git 저장소가 아닌 폴더입니다".to_string())?;
    let root = root.trim().to_string();
    let branch = git(cwd, &["branch", "--show-current"], &[0]).unwrap_or_default().trim().to_string();
    let status = git(Path::new(&root), &["status", "--porcelain=v1", "-z", "--untracked-files=all"], &[0])?;
    Ok(Changes { root, branch, files: parse_status(&status) })
}

/// 파일 하나의 차이. 새 파일은 전체가 추가된 것으로 보여 준다.
#[tauri::command]
pub async fn git_diff(root: String, path: String, untracked: bool) -> Result<String, String> {
    let root = Path::new(&root);
    let mut text = if untracked {
        // --no-index는 차이가 있으면 종료 코드 1을 낸다.
        git(root, &["diff", "--no-color", "--no-index", "--", "/dev/null", &path], &[0, 1])?
    } else {
        // 커밋이 하나도 없는 저장소에서는 HEAD가 없으므로 색인과 비교한다.
        git(root, &["diff", "--no-color", "HEAD", "--", &path], &[0])
            .or_else(|_| git(root, &["diff", "--no-color", "--", &path], &[0]))?
    };
    if text.len() > MAX_DIFF_BYTES {
        let mut cut = MAX_DIFF_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n… (너무 길어서 여기까지만 보여 줍니다)\n");
    }
    Ok(text)
}

// ───────────────────────── 출력 저장 ─────────────────────────

/// 저장할 곳을 물어 글자를 파일로 쓴다. 취소하면 None, 저장하면 경로.
#[tauri::command]
pub async fn save_text(name: String, text: String) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_title("세션 출력 저장")
            .set_file_name(name)
            .add_filter("글자 파일", &["txt", "log"])
            .save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked else { return Ok(None) };
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

// ───────────────────────── Claude 설정 폴더 ─────────────────────────

/// Claude Code가 폴더별 대화 기록을 두는 폴더 이름: 영문·숫자가 아닌 글자를 모두 `-`로 바꾼다.
/// `C:\workspace\cypher` → `C--workspace-cypher`
pub(crate) fn project_dir_name(cwd: &str) -> String {
    cwd.trim_end_matches(['\\', '/'])
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub(crate) fn claude_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    Some(PathBuf::from(std::env::var_os("USERPROFILE")?).join(".claude"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 상태_출력을_읽는다() {
        let raw = " M src/main.rs\0?? 새 폴더/메모.txt\0R  new.rs\0old.rs\0D  gone.rs\0A  added.rs\0";
        let files = parse_status(raw);
        let got: Vec<_> = files.iter().map(|c| (c.status, c.path.as_str(), c.untracked)).collect();
        assert_eq!(
            got,
            vec![
                ("수정", "src/main.rs", false),
                ("새 파일", "새 폴더/메모.txt", true),
                ("이름 변경", "new.rs", false),
                ("삭제", "gone.rs", false),
                ("추가", "added.rs", false),
            ]
        );
    }

    #[test]
    fn 대화_기록_폴더_이름() {
        assert_eq!(project_dir_name(r"C:\workspace\cypher"), "C--workspace-cypher");
        // 한글·공백·구두점도 글자 하나당 '-' 하나가 된다.
        assert_eq!(project_dir_name(r"C:\내 폴더\my.app\"), "C-------my-app");
    }
}
