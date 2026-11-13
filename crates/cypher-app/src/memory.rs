//! Claude Code 기억 파일 내보내기·가져오기.
//!
//! Claude Code는 폴더별 기억을 `~/.claude/projects/<폴더 이름>/memory/`에 둔다
//! (`MEMORY.md` 색인 + 기억 하나당 파일 하나). 이 폴더를 다른 곳으로 복사해 두거나,
//! 다른 곳의 기억을 이 폴더로 들여온다.
//!
//! `<폴더 이름>`은 git 저장소 기준이다: 같은 저장소의 하위 폴더나 워크트리에서 시작한 세션은 저장소 뿌리의
//! 기억 폴더를 함께 쓴다. 저장소가 아니면 세션을 시작한 폴더 기준이다.
//!
//! 가져오기는 기존 기억을 잃지 않게 한다: 먼저 통째로 백업하고, 색인은 합친다.

use crate::extras::{claude_home, project_dir_name};
use serde::Serialize;
use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const INDEX: &str = "MEMORY.md";
const MAX_DEPTH: usize = 8;
/// 가져올 폴더에 기억 파일(.md)이 이보다 많으면 기억 폴더가 아니라고 본다(저장소 폴더를 잘못 골랐을 때).
const MAX_IMPORT_FILES: usize = 500;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Serialize)]
pub struct MemoryInfo {
    /// 이 폴더의 기억이 있는 위치.
    dir: String,
    files: usize,
}

#[derive(Debug, Serialize)]
pub struct Transfer {
    copied: usize,
    /// 복사한 곳.
    target: String,
    /// 가져오기 전에 기존 기억을 백업해 둔 곳(기존 기억이 있었을 때만).
    backup: Option<String>,
}

/// 그 폴더가 든 git 저장소의 뿌리(워크트리면 본 저장소의 뿌리). 저장소가 아니거나 git이 없으면 None.
fn repo_root(cwd: &str) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    root_of_common_dir(String::from_utf8_lossy(&output.stdout).trim())
}

/// 저장소의 공용 git 폴더(`…/.git`)에서 저장소 뿌리를 얻는다. 그 모양이 아니면(하위 모듈, 따로 둔 git 폴더) None.
fn root_of_common_dir(common: &str) -> Option<PathBuf> {
    let common = PathBuf::from(common.replace('/', "\\"));
    (common.file_name()? == ".git").then(|| common.parent().map(Path::to_path_buf))?
}

fn memory_dir(cwd: &str) -> Result<PathBuf, String> {
    let home = claude_home().ok_or("Claude 설정 폴더를 찾지 못했습니다")?;
    let of = |folder: &str| home.join("projects").join(project_dir_name(folder)).join("memory");
    let here = of(cwd);
    let Some(root) = repo_root(cwd) else { return Ok(here) };
    let shared = of(&root.to_string_lossy());
    // 저장소 뿌리 쪽에는 없고 이 폴더 쪽에만 기억이 쌓여 있으면 그것을 쓴다(있는 기억을 못 찾는 일이 없게).
    Ok(if list_files(&shared).is_empty() && !list_files(&here).is_empty() { here } else { shared })
}

/// 폴더 안의 일반 파일 전체(상대 경로). 바로가기·연결 파일은 따라가지 않는다.
fn list_files(root: &Path) -> Vec<PathBuf> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.is_file() {
                if let Ok(relative) = path.strip_prefix(root) {
                    out.push(relative.to_path_buf());
                }
            } else if meta.is_dir() && depth < MAX_DEPTH {
                walk(root, &path, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out.sort();
    out
}

/// 색인 두 개를 합친다: 기존 줄은 그대로 두고, 들어온 것 중 없는 줄만 뒤에 붙인다.
fn merge_index(existing: &str, incoming: &str) -> String {
    let mut merged: Vec<&str> = existing.lines().collect();
    let known: std::collections::HashSet<&str> = merged.iter().map(|l| l.trim()).collect();
    merged.extend(incoming.lines().filter(|l| !l.trim().is_empty() && !known.contains(l.trim())));
    let mut text = merged.join("\n");
    text.push('\n');
    text
}

/// 기억 파일인지(.md).
fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

/// `from`의 파일을 모두 `to`로 복사한다(같은 이름은 덮어쓴다).
/// `merge`가 참이면(가져오기) 기억 파일(.md)만 복사하고, 색인 파일은 덮어쓰지 않고 합친다.
fn copy_tree(from: &Path, to: &Path, merge: bool) -> io::Result<usize> {
    let mut files = list_files(from);
    if merge {
        files.retain(|relative| is_markdown(relative));
    }
    for relative in &files {
        let (source, target) = (from.join(relative), to.join(relative));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if merge && relative == Path::new(INDEX) && target.is_file() {
            let merged = merge_index(&fs::read_to_string(&target)?, &fs::read_to_string(&source)?);
            fs::write(&target, merged)?;
        } else {
            fs::copy(&source, &target)?;
        }
    }
    Ok(files.len())
}

/// 한쪽이 다른 쪽 안에 있으면(같은 폴더 포함) 복사하면 안 된다.
fn overlaps(a: &Path, b: &Path) -> bool {
    // 아직 없는 폴더도 비교할 수 있게, 실제로 있는 가장 가까운 윗폴더를 기준으로 정리한다.
    fn real(path: &Path) -> PathBuf {
        let mut missing = Vec::new();
        let mut current = path;
        loop {
            if let Ok(found) = fs::canonicalize(current) {
                return missing.iter().rev().fold(found, |acc, name| acc.join(name));
            }
            match (current.file_name(), current.parent()) {
                (Some(name), Some(parent)) => {
                    missing.push(name.to_os_string());
                    current = parent;
                }
                _ => return path.to_path_buf(),
            }
        }
    }
    let (a, b) = (real(a), real(b));
    a.starts_with(&b) || b.starts_with(&a)
}

fn export(memory: &Path, dest: &Path) -> Result<Transfer, String> {
    if list_files(memory).is_empty() {
        return Err("이 폴더에는 내보낼 기억이 없습니다".into());
    }
    if overlaps(memory, dest) {
        return Err("기억 폴더 자신이나 그 안으로는 내보낼 수 없습니다".into());
    }
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let copied = copy_tree(memory, dest, false).map_err(|e| e.to_string())?;
    Ok(Transfer { copied, target: dest.to_string_lossy().into_owned(), backup: None })
}

fn import(memory: &Path, source: &Path) -> Result<Transfer, String> {
    // 기억 파일(.md)만 가져온다. 저장소 폴더처럼 엉뚱한 폴더를 골랐을 때 그 안의 모든 파일이 기억 폴더로 들어가지 않게 한다.
    let markdown = list_files(source).iter().filter(|p| is_markdown(p)).count();
    if markdown == 0 {
        return Err("기억 파일(.md)이 없는 폴더입니다".into());
    }
    if markdown > MAX_IMPORT_FILES {
        return Err(format!("기억 폴더로 보이지 않습니다(.md 파일 {markdown}개). 내보낸 기억이 든 폴더를 골라 주세요"));
    }
    if overlaps(memory, source) {
        return Err("기억 폴더 자신에서는 가져올 수 없습니다".into());
    }
    // 기존 기억이 있으면 통째로 백업해 둔다.
    let mut backup = None;
    if !list_files(memory).is_empty() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let dir = memory.with_file_name(format!("memory.backup-{stamp}"));
        copy_tree(memory, &dir, false).map_err(|e| format!("백업 실패: {e}"))?;
        backup = Some(dir.to_string_lossy().into_owned());
    }
    fs::create_dir_all(memory).map_err(|e| e.to_string())?;
    let copied = copy_tree(source, memory, true).map_err(|e| e.to_string())?;
    Ok(Transfer { copied, target: memory.to_string_lossy().into_owned(), backup })
}

#[tauri::command]
pub async fn memory_info(cwd: String) -> Result<MemoryInfo, String> {
    let dir = memory_dir(&cwd)?;
    Ok(MemoryInfo { files: list_files(&dir).len(), dir: dir.to_string_lossy().into_owned() })
}

/// 이 폴더의 기억을 `dest`로 복사한다.
#[tauri::command]
pub async fn memory_export(cwd: String, dest: String) -> Result<Transfer, String> {
    export(&memory_dir(&cwd)?, Path::new(&dest))
}

/// `source`의 기억을 이 폴더의 기억으로 들여온다.
#[tauri::command]
pub async fn memory_import(cwd: String, source: String) -> Result<Transfer, String> {
    import(&memory_dir(&cwd)?, Path::new(&source))
}

/// 기억 폴더를 탐색기로 연다(없으면 만든다).
#[tauri::command]
pub async fn memory_open(cwd: String) -> Result<(), String> {
    let dir = memory_dir(&cwd)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Command::new("explorer.exe").arg(dir).spawn().map(drop).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cypher-memory-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 내보낸_것을_다른_곳에서_가져온다() {
        let root = fresh("roundtrip");
        let (a, out, b) = (root.join("a").join("memory"), root.join("out"), root.join("b").join("memory"));
        fs::create_dir_all(a.join("sub")).unwrap();
        fs::write(a.join(INDEX), "- [가](ga.md) — 첫째\n").unwrap();
        fs::write(a.join("ga.md"), "가").unwrap();
        fs::write(a.join("sub").join("na.md"), "나").unwrap();

        let exported = export(&a, &out).unwrap();
        assert_eq!(exported.copied, 3);
        assert_eq!(fs::read_to_string(out.join("sub").join("na.md")).unwrap(), "나");

        let imported = import(&b, &out).unwrap();
        assert_eq!((imported.copied, imported.backup.is_none()), (3, true));
        assert_eq!(fs::read_to_string(b.join("ga.md")).unwrap(), "가");
    }

    #[test]
    fn 가져올_때_기존_기억을_백업하고_색인을_합친다() {
        let root = fresh("merge");
        let (memory, source) = (root.join("p").join("memory"), root.join("in"));
        fs::create_dir_all(&memory).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(memory.join(INDEX), "- [옛](old.md) — 옛 기억\n- [공통](both.md) — 둘 다\n").unwrap();
        fs::write(memory.join("old.md"), "옛").unwrap();
        fs::write(memory.join("both.md"), "기존 내용").unwrap();
        fs::write(source.join(INDEX), "- [공통](both.md) — 둘 다\n- [새](new.md) — 새 기억\n").unwrap();
        fs::write(source.join("new.md"), "새").unwrap();
        fs::write(source.join("both.md"), "들어온 내용").unwrap();

        let result = import(&memory, &source).unwrap();
        assert_eq!(result.copied, 3);
        // 색인은 합쳐지고, 기존 파일은 남고, 같은 이름은 들어온 것으로 바뀐다.
        assert_eq!(
            fs::read_to_string(memory.join(INDEX)).unwrap(),
            "- [옛](old.md) — 옛 기억\n- [공통](both.md) — 둘 다\n- [새](new.md) — 새 기억\n"
        );
        assert_eq!(fs::read_to_string(memory.join("old.md")).unwrap(), "옛");
        assert_eq!(fs::read_to_string(memory.join("both.md")).unwrap(), "들어온 내용");
        // 바뀌기 전 내용은 백업에 남아 있다.
        let backup = PathBuf::from(result.backup.unwrap());
        assert_eq!(fs::read_to_string(backup.join("both.md")).unwrap(), "기존 내용");
    }

    #[test]
    fn 잘못된_대상은_거절한다() {
        let root = fresh("reject");
        let memory = root.join("memory");
        fs::create_dir_all(&memory).unwrap();
        assert!(export(&memory, &root.join("out")).is_err()); // 내보낼 것이 없음
        fs::write(memory.join("a.md"), "x").unwrap();
        assert!(export(&memory, &memory.join("inside")).is_err()); // 자기 안으로
        assert!(import(&memory, &memory).is_err()); // 자기 자신에서
        let empty = root.join("empty");
        fs::create_dir_all(&empty).unwrap();
        fs::write(empty.join("note.txt"), "x").unwrap();
        assert!(import(&memory, &empty).is_err()); // .md 없음
    }

    #[test]
    fn 가져올_때는_기억_파일만_복사한다() {
        let root = fresh("only-md");
        let (memory, source) = (root.join("p").join("memory"), root.join("repo"));
        fs::create_dir_all(source.join(".git")).unwrap();
        fs::write(source.join("README.MD"), "읽어 보기").unwrap();
        fs::write(source.join("main.rs"), "fn main() {}").unwrap();
        fs::write(source.join(".git").join("config"), "x").unwrap();
        let result = import(&memory, &source).unwrap();
        assert_eq!(result.copied, 1);
        assert!(memory.join("README.MD").is_file());
        assert!(!memory.join("main.rs").exists() && !memory.join(".git").exists());
    }

    #[test]
    fn 저장소의_공용_git_폴더에서_뿌리를_얻는다() {
        assert_eq!(root_of_common_dir("C:/work/proj/.git"), Some(PathBuf::from(r"C:\work\proj")));
        assert_eq!(root_of_common_dir(r"C:\work\proj\.git"), Some(PathBuf::from(r"C:\work\proj")));
        // 하위 모듈이나 따로 둔 git 폴더는 뿌리를 알 수 없다.
        assert_eq!(root_of_common_dir("C:/work/proj/.git/modules/sub"), None);
        assert_eq!(root_of_common_dir(""), None);
        // 이 저장소의 하위 폴더에서도 저장소 뿌리를 찾는다(git이 있을 때).
        let here = env!("CARGO_MANIFEST_DIR");
        if let Some(found) = repo_root(here) {
            assert!(Path::new(here).starts_with(&found) && found.join(".git").exists(), "{found:?}");
        }
    }
}
