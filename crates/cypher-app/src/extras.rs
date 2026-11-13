//! 결과 확인용 명령: 변경 내용(git) 보기·되돌리기·커밋, 출력 저장. Claude 설정 폴더 위치도 여기서 구한다.

use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_DIFF_BYTES: usize = 400_000;
/// 되돌리기 전에 보관해 둔 차이를 남겨 두는 기간.
const KEEP_REVERTED: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// git에 한 번에 넘길 수 있는 인자 길이(Windows 명령줄 한도보다 넉넉히 작게).
const MAX_ARGS_CHARS: usize = 24_000;

// ───────────────────────── 변경 내용 ─────────────────────────

#[derive(Debug, PartialEq, Serialize)]
pub struct Change {
    /// 화면에 보일 상태 이름(수정, 새 파일 …).
    status: &'static str,
    /// 저장소 맨 위 폴더 기준 경로.
    path: String,
    untracked: bool,
    /// 이름 변경·복사일 때의 원래 경로.
    from: Option<String>,
    /// git이 알려 준 상태 두 글자(색인 쪽, 작업 폴더 쪽). 화면에는 내보내지 않는다.
    #[serde(skip)]
    code: String,
}

#[derive(Serialize)]
pub struct Changes {
    root: String,
    branch: String,
    files: Vec<Change>,
}

/// git을 실행한다. `readonly`이면(보기만 하는 명령) 색인 잠금을 잡지 않는다.
fn run_git(dir: &Path, args: &[&str], readonly: bool) -> Result<std::process::Output, String> {
    let mut command = Command::new("git");
    // 경로 인자는 글자 그대로 받는다(--literal-pathspecs). 그러지 않으면 git이 `[ ]`·`*`·`?`를 글롭으로 풀어서,
    // `app/[id]/page.tsx`를 되돌리거나 커밋할 때 `app/i/page.tsx`까지 함께 처리된다.
    command.arg("--literal-pathspecs").arg("-C").arg(dir).args(["-c", "core.quotepath=false"]).args(args);
    if readonly {
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    command
        .stdin(Stdio::null()) // 입력을 기다리며 멈추지 않게 한다.
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("git을 실행하지 못했습니다: {e}"))
}

/// 보기만 하는 git 명령을 실행해 표준 출력을 돌려준다. `ok_codes`에 없는 종료 코드는 오류로 본다.
fn git(dir: &Path, args: &[&str], ok_codes: &[i32]) -> Result<String, String> {
    let output = run_git(dir, args, true)?;
    if !ok_codes.contains(&output.status.code().unwrap_or(-1)) {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 저장소를 바꾸는 git 명령을 실행한다. 실패하면 git이 한 말을 그대로 돌려준다(오류 출력이 비면 표준 출력).
fn git_write(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = run_git(dir, args, false)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let said = if stderr.is_empty() { stdout.trim().to_string() } else { stderr };
    Err(if said.is_empty() { format!("git이 실패했습니다(종료 코드 {:?})", output.status.code()) } else { said })
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
        // 이름 변경·복사는 원래 경로가 바로 뒤에 따라온다.
        let from = if code.contains('R') || code.contains('C') { entries.next().map(str::to_string) } else { None };
        files.push(Change { status, path: path.to_string(), untracked: code == "??", from, code: code.to_string() });
    }
    files
}

/// 저장소 전체의 바뀐 파일 목록.
fn status_of(root: &Path) -> Result<Vec<Change>, String> {
    git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], &[0]).map(|raw| parse_status(&raw))
}

/// 저장소 안의 파일을 가리키는 상대 경로인지(절대 경로나 `..`로 저장소 밖을 가리키지 않는지).
fn is_repo_path(path: &str) -> bool {
    use std::path::Component;
    !path.is_empty() && Path::new(path).components().all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// 세션 폴더가 속한 저장소의 바뀐 파일 목록.
#[tauri::command]
pub async fn git_changes(cwd: String) -> Result<Changes, String> {
    let cwd = Path::new(&cwd);
    let root = git(cwd, &["rev-parse", "--show-toplevel"], &[0])
        .map_err(|_| "git 저장소가 아닌 폴더입니다".to_string())?;
    let root = root.trim().to_string();
    let branch = git(cwd, &["branch", "--show-current"], &[0]).unwrap_or_default().trim().to_string();
    let files = status_of(Path::new(&root))?;
    Ok(Changes { root, branch, files })
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

// ───────────────────────── 되돌리기 ─────────────────────────

/// 파일 이름으로 쓸 수 없는 글자를 `_`로 바꾼다.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let cleaned: String = cleaned.trim().chars().take(80).collect();
    if cleaned.is_empty() { "file".into() } else { cleaned }
}

/// 보관한 지 30일이 넘은 차이 파일을 지운다(새로 보관할 때마다).
fn clean_reverted(keep: &Path) {
    let Ok(entries) = std::fs::read_dir(keep) else { return };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > KEEP_REVERTED);
        if old && entry.path().extension().is_some_and(|e| e == "patch") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// 파일 하나의 변경을 마지막 커밋의 내용으로 되돌린다(색인과 작업 폴더 모두).
/// 되돌리기 전의 차이를 `keep` 폴더에 `.patch` 파일로 보관하고 그 경로를 돌려준다
/// (되살리려면 저장소에서 `git apply <그 파일>`). 보관하지 못하면 되돌리지 않는다.
///
/// 화면이 알려 준 상태를 믿지 않고 지금 상태를 다시 읽는다. 새 파일은 지우지 않고,
/// 이름 변경·새로 추가·충돌처럼 한 파일만 안전하게 되돌리기 어려운 상태는 거절한다.
fn revert(root: &Path, path: &str, keep: &Path) -> Result<String, String> {
    if !is_repo_path(path) {
        return Err("저장소 안의 파일이 아닙니다".into());
    }
    let raw = git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", path], &[0])?;
    let changes = parse_status(&raw);
    let change = changes.iter().find(|c| c.path == path).ok_or("바뀐 것이 없는 파일입니다(이미 되돌렸거나 커밋했습니다)")?;
    if change.untracked {
        return Err("새 파일은 되돌릴 원본이 없습니다. 지우려면 탐색기에서 지우세요".into());
    }
    // 고쳤거나(M), 지웠거나(D), 종류가 바뀐(T) 파일만 되돌린다.
    if !change.code.chars().all(|c| matches!(c, ' ' | 'M' | 'D' | 'T')) {
        return Err(format!("이 상태({})의 파일은 되돌리기를 지원하지 않습니다. 터미널에서 git으로 되돌리세요", change.status));
    }
    // 하위 모듈은 checkout이 성공으로 끝나도 안의 변경이 그대로 남는다("되돌렸습니다"라고 하면 거짓이 된다).
    if root.join(path).is_dir() {
        return Err("하위 모듈(폴더)의 변경은 되돌리기를 지원하지 않습니다. 그 폴더 안에서 git으로 되돌리세요".into());
    }

    // 되돌리기 전의 차이를 보관한다(이진 파일도 되살릴 수 있게 --binary).
    let patch = run_git(root, &["diff", "--no-color", "--binary", "HEAD", "--", path], true)?;
    if !patch.status.success() {
        return Err(String::from_utf8_lossy(&patch.stderr).trim().to_string());
    }
    std::fs::create_dir_all(keep).map_err(|e| format!("되돌리기 전 차이를 보관할 폴더를 만들지 못했습니다: {e}"))?;
    clean_reverted(keep);
    let stamp = crate::http::now_text().replace([':', ' '], "-");
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut saved = keep.join(format!("{stamp}-{}.patch", safe_file_name(&name)));
    for n in 2..100 {
        if !saved.exists() {
            break;
        }
        saved = keep.join(format!("{stamp}-{}-{n}.patch", safe_file_name(&name)));
    }
    std::fs::write(&saved, &patch.stdout).map_err(|e| format!("되돌리기 전 차이를 보관하지 못해 되돌리지 않았습니다: {e}"))?;

    // `git restore`보다 오래된 git에도 있는 방식. 색인과 작업 폴더를 함께 마지막 커밋의 내용으로 돌린다.
    git_write(root, &["checkout", "HEAD", "--", path])?;
    Ok(saved.to_string_lossy().into_owned())
}

/// 파일 하나의 변경을 되돌린다. 되돌리기 전의 차이를 보관한 파일의 경로를 돌려준다.
/// `from`·`untracked`는 화면이 본 상태다. 실제 판단은 지금 상태를 다시 읽어서 한다.
#[tauri::command]
pub async fn git_revert(root: String, path: String, from: Option<String>, untracked: bool) -> Result<String, String> {
    let _ = (from, untracked);
    let keep = crate::data_dir().ok_or("되돌리기 전 차이를 보관할 폴더를 찾지 못했습니다")?.join("reverted");
    tauri::async_runtime::spawn_blocking(move || revert(Path::new(&root), &path, &keep)).await.map_err(|e| e.to_string())?
}

// ───────────────────────── 커밋 ─────────────────────────

/// 고른 파일만 커밋한다. 성공하면 만든 커밋을 한 줄로(짧은 번호와 제목) 돌려준다.
///
/// 아직 색인에 올리지 않은 변경(새 파일 포함)은 먼저 올린다. 이름 변경은 원래 경로도 함께 커밋해야
/// 지워진 쪽이 빠지지 않으므로, 지금 상태를 다시 읽어 원래 경로를 더한다. 고르지 않은 파일은
/// 색인에 올라 있어도 커밋하지 않는다. 훅과 서명은 저장소 설정대로 돈다.
fn commit(root: &Path, paths: &[String], message: &str) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("커밋 메시지가 비어 있습니다".into());
    }
    if paths.is_empty() {
        return Err("커밋할 파일을 고르지 않았습니다".into());
    }
    if let Some(bad) = paths.iter().find(|p| !is_repo_path(p)) {
        return Err(format!("저장소 안의 파일이 아닙니다: {bad}"));
    }
    let changes = status_of(root)?;
    let mut stage: Vec<&str> = Vec::new(); // 색인에 올려야 하는 것: 새 파일, 작업 폴더 쪽에 변경이 남은 것
    let mut all: Vec<&str> = Vec::new(); // 커밋할 경로: 고른 것과, 이름 변경의 원래 경로
    for path in paths {
        if all.contains(&path.as_str()) {
            continue;
        }
        all.push(path);
        let Some(change) = changes.iter().find(|c| &c.path == path) else { continue };
        if change.code.contains('U') || change.code == "AA" || change.code == "DD" {
            return Err(format!("충돌이 풀리지 않은 파일이 있습니다: {path}. 충돌을 푼 뒤에 커밋하세요"));
        }
        if change.untracked || !change.code.ends_with(' ') {
            stage.push(path);
        }
        if change.code.contains('R') {
            if let Some(from) = change.from.as_deref() {
                if !all.contains(&from) {
                    all.push(from);
                }
            }
        }
    }
    if all.iter().map(|p| p.len() + 3).sum::<usize>() + message.len() > MAX_ARGS_CHARS {
        return Err("한 번에 커밋하기에는 고른 파일이 너무 많습니다. 나눠서 커밋하거나 터미널에서 커밋하세요".into());
    }
    if !stage.is_empty() {
        let mut args = vec!["add", "-A", "--"];
        args.extend(&stage);
        git_write(root, &args)?;
    }
    let mut args = vec!["commit", "-m", message, "--"];
    args.extend(&all);
    git_write(root, &args)?;
    Ok(git(root, &["log", "-1", "--format=%h %s"], &[0]).unwrap_or_default().trim().to_string())
}

/// 고른 파일만 커밋한다(푸시는 하지 않는다).
#[tauri::command]
pub async fn git_commit(root: String, paths: Vec<String>, message: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || commit(Path::new(&root), &paths, &message)).await.map_err(|e| e.to_string())?
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
        let got: Vec<_> = files.iter().map(|c| (c.status, c.path.as_str(), c.untracked, c.from.as_deref(), c.code.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("수정", "src/main.rs", false, None, " M"),
                ("새 파일", "새 폴더/메모.txt", true, None, "??"),
                ("이름 변경", "new.rs", false, Some("old.rs"), "R "),
                ("삭제", "gone.rs", false, None, "D "),
                ("추가", "added.rs", false, None, "A "),
            ]
        );
    }

    #[test]
    fn 저장소_안의_상대_경로만_받는다() {
        assert!(is_repo_path("src/main.rs"));
        assert!(is_repo_path("-이름이 줄표로 시작.txt"));
        assert!(!is_repo_path(""));
        assert!(!is_repo_path("../밖.txt"));
        assert!(!is_repo_path("src/../../밖.txt"));
        assert!(!is_repo_path(r"C:\Windows\win.ini"));
        assert!(!is_repo_path("/etc/passwd"));
    }

    /// 시험용 저장소를 만든다. 사용자 정보·서명·훅 설정은 이 저장소에만 둔다(사용자 공통 설정의 영향을 받지 않게).
    fn test_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cypher-git-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git_write(&dir, &["init", "-q"]).unwrap();
        for (key, value) in [
            ("user.name", "Cypher 시험"),
            ("user.email", "test@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", ".git/no-hooks"),
        ] {
            git_write(&dir, &["config", key, value]).unwrap();
        }
        dir
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    /// 지금 바뀐 파일을 "상태 두 글자 + 경로"로.
    fn status_lines(dir: &Path) -> Vec<String> {
        status_of(dir).unwrap().iter().map(|c| format!("{} {}", c.code, c.path)).collect()
    }

    #[test]
    fn 고른_파일만_커밋한다() {
        let dir = test_repo("commit");
        write(&dir, "a.txt", "하나\n");
        write(&dir, "b.txt", "둘\n");
        write(&dir, "old name.txt", "이름을 바꿀 파일\n내용은 그대로\n");
        write(&dir, "gone.txt", "지울 파일\n");
        let all: Vec<String> = ["a.txt", "b.txt", "old name.txt", "gone.txt"].map(String::from).to_vec();
        // 커밋이 하나도 없는 저장소에서의 첫 커밋(새 파일뿐).
        let first = commit(&dir, &all, "첫 커밋").unwrap();
        assert!(first.ends_with(" 첫 커밋"), "{first}");
        assert!(status_lines(&dir).is_empty());

        // 고친 것, 색인에 올려 둔 고침, 새 파일, 줄표로 시작하는 새 파일, 이름 변경, 지움이 섞여 있다.
        write(&dir, "a.txt", "하나 고침\n");
        write(&dir, "b.txt", "둘 고침\n");
        git_write(&dir, &["add", "--", "b.txt"]).unwrap();
        write(&dir, "sub/새 파일.txt", "새로 만든 것\n");
        write(&dir, "-dash.txt", "줄표로 시작하는 이름\n");
        git_write(&dir, &["mv", "--", "old name.txt", "new name.txt"]).unwrap();
        git_write(&dir, &["rm", "-q", "--", "gone.txt"]).unwrap();

        // b.txt(색인에 올라 있음)는 고르지 않는다: 커밋에 들어가면 안 된다.
        let picked: Vec<String> = ["a.txt", "sub/새 파일.txt", "-dash.txt", "new name.txt", "gone.txt"].map(String::from).to_vec();
        let made = commit(&dir, &picked, "둘째 커밋\n\n본문 줄 \"따옴표\" $변수 `역따옴표`").unwrap();
        assert!(made.ends_with(" 둘째 커밋"), "{made}");
        assert_eq!(status_lines(&dir), vec!["M  b.txt"]);
        let body = git(&dir, &["log", "-1", "--format=%b"], &[0]).unwrap();
        assert_eq!(body.trim(), "본문 줄 \"따옴표\" $변수 `역따옴표`");
        let files = git(&dir, &["show", "--name-status", "--format=", "-M", "HEAD"], &[0]).unwrap();
        let mut files: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
        files.sort_unstable();
        assert_eq!(files, vec!["A\t-dash.txt", "A\tsub/새 파일.txt", "D\tgone.txt", "M\ta.txt", "R100\told name.txt\tnew name.txt"]);

        // 빈 메시지, 고른 파일 없음, 저장소 밖 경로는 거절한다.
        assert!(commit(&dir, &["b.txt".to_string()], "  \n").is_err());
        assert!(commit(&dir, &[], "메시지").is_err());
        assert!(commit(&dir, &["../밖.txt".to_string()], "메시지").is_err());
        // 바뀐 것이 없는 파일만 고르면 git의 말을 그대로 돌려준다(커밋은 생기지 않는다).
        let before = git(&dir, &["rev-parse", "HEAD"], &[0]).unwrap();
        let nothing = commit(&dir, &["a.txt".to_string()], "빈 커밋").unwrap_err();
        assert!(!nothing.is_empty());
        assert_eq!(git(&dir, &["rev-parse", "HEAD"], &[0]).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 변경을_되돌리고_되돌리기_전_차이를_보관한다() {
        let dir = test_repo("revert");
        let keep = dir.join(".git").join("cypher-reverted-test");
        write(&dir, "a.txt", "첫 줄\n둘째 줄\n");
        write(&dir, "staged.txt", "원래\n");
        write(&dir, "gone.txt", "지울 파일\n");
        write(&dir, "moved.txt", "옮길 파일\n");
        let all: Vec<String> = ["a.txt", "staged.txt", "gone.txt", "moved.txt"].map(String::from).to_vec();
        commit(&dir, &all, "처음").unwrap();

        // 고친 파일: 원래 내용으로 돌아가고, 보관한 차이로 고친 내용을 되살릴 수 있다.
        write(&dir, "a.txt", "첫 줄\n고친 둘째 줄\n셋째 줄\n");
        let saved = revert(&dir, "a.txt", &keep).unwrap();
        assert_eq!(read(&dir, "a.txt"), "첫 줄\n둘째 줄\n");
        let patch = std::fs::read_to_string(&saved).unwrap();
        assert!(patch.contains("+고친 둘째 줄") && patch.contains("-둘째 줄"), "{patch}");
        assert!(Path::new(&saved).starts_with(&keep) && saved.ends_with("-a.txt.patch"), "{saved}");
        git_write(&dir, &["apply", "--", &saved]).unwrap();
        assert_eq!(read(&dir, "a.txt"), "첫 줄\n고친 둘째 줄\n셋째 줄\n");
        revert(&dir, "a.txt", &keep).unwrap();

        // 색인에 올린 고침과 그 뒤의 고침이 함께 있어도 마지막 커밋으로 돌아간다.
        write(&dir, "staged.txt", "색인에 올린 고침\n");
        git_write(&dir, &["add", "--", "staged.txt"]).unwrap();
        write(&dir, "staged.txt", "그 뒤에 또 고침\n");
        revert(&dir, "staged.txt", &keep).unwrap();
        assert_eq!(read(&dir, "staged.txt"), "원래\n");

        // 지운 파일(작업 폴더에서만 지움, 색인에서도 지움)은 되살아난다.
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        revert(&dir, "gone.txt", &keep).unwrap();
        assert_eq!(read(&dir, "gone.txt"), "지울 파일\n");
        git_write(&dir, &["rm", "-q", "--", "gone.txt"]).unwrap();
        revert(&dir, "gone.txt", &keep).unwrap();
        assert_eq!(read(&dir, "gone.txt"), "지울 파일\n");
        assert!(status_lines(&dir).is_empty(), "{:?}", status_lines(&dir));

        // 새 파일은 지우지 않고 거절한다. 색인에 올린 새 파일, 이름 변경도 건드리지 않는다.
        write(&dir, "new.txt", "새 파일\n");
        assert!(revert(&dir, "new.txt", &keep).unwrap_err().contains("새 파일"));
        assert_eq!(read(&dir, "new.txt"), "새 파일\n");
        git_write(&dir, &["add", "--", "new.txt"]).unwrap();
        assert!(revert(&dir, "new.txt", &keep).unwrap_err().contains("지원하지 않습니다"));
        assert_eq!(read(&dir, "new.txt"), "새 파일\n");
        git_write(&dir, &["mv", "--", "moved.txt", "moved2.txt"]).unwrap();
        assert!(revert(&dir, "moved2.txt", &keep).unwrap_err().contains("지원하지 않습니다"));
        assert!(dir.join("moved2.txt").exists() && !dir.join("moved.txt").exists());

        // 바뀐 것이 없는 파일, 저장소 밖 경로.
        assert!(revert(&dir, "a.txt", &keep).is_err());
        assert!(revert(&dir, "../밖.txt", &keep).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 대화_기록_폴더_이름() {
        assert_eq!(project_dir_name(r"C:\workspace\cypher"), "C--workspace-cypher");
        // 한글·공백·구두점도 글자 하나당 '-' 하나가 된다.
        assert_eq!(project_dir_name(r"C:\내 폴더\my.app\"), "C-------my-app");
    }
}
