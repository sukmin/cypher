//! Claude 세션이 만든 파일을 사이드바의 어느 종류로 보여 줄지 정한다.
//!
//! Claude Code가 Write 도구로 파일을 쓰기 직전과 직후에 훅 신호가 온다(`status.rs`). 쓰기 직전에 그 파일이
//! 없었으면 "새로 만든 파일"이고, 그런 파일만 종류를 가려 화면에 알린다. 화면은 그 세션의 묶음에
//! 종류에 맞는 항목(문서, 이미지, HTTP)으로 놓는다.
//!
//! 전달 프로그램은 직전 신호를 보낸 뒤 앱이 받아 처리할 때까지 기다렸다가 끝나므로(`status.rs`의 답),
//! "그때 파일이 있었는지"는 Claude Code가 실제로 쓰기 전에 확인된다.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// 쓰기 직전에 없던 파일: (세션 번호, 소문자로 바꾼 경로). 쓴 직후의 신호가 오면 꺼낸다.
static PENDING: LazyLock<Mutex<HashSet<(u32, String)>>> = LazyLock::new(Default::default);
/// 직후 신호가 끝내 오지 않은 것(쓰기가 거절됨 등)이 쌓이지 않게 하는 한도.
const MAX_PENDING: usize = 500;

fn key(session: u32, path: &Path) -> (u32, String) {
    (session, path.to_string_lossy().to_lowercase())
}

/// 쓰기 직전: 그 파일이 아직 없으면 적어 둔다. 이미 있으면(고쳐 쓰기) 예전에 적어 둔 것도 지운다
/// (앞선 쓰기가 거절돼 남아 있던 기록 때문에, 고쳐 쓴 파일이 새 파일로 보이지 않게).
pub fn before_write(session: u32, path: &Path) {
    // 느린 드라이브에서 확인이 오래 걸려도 다른 세션의 신호가 줄을 서지 않게, 잠금을 잡기 전에 본다.
    let exists = path.exists();
    let mut pending = PENDING.lock().unwrap();
    if exists {
        pending.remove(&key(session, path));
        return;
    }
    if pending.len() >= MAX_PENDING {
        pending.clear();
    }
    pending.insert(key(session, path));
}

/// 쓴 직후: 새로 만든 파일이면 사이드바에 보일 종류를 돌려준다. 고쳐 쓴 파일이거나 보여 주지 않는 파일이면 None.
/// `base`는 그 세션이 일하고 있는 폴더(숨김 폴더·빌드 폴더 규칙은 이 폴더 아래의 경로에만 건다).
pub fn after_write(session: u32, path: &Path, base: Option<&Path>) -> Option<&'static str> {
    after_write_in(session, path, base, &skip_roots())
}

fn after_write_in(session: u32, path: &Path, base: Option<&Path>, skip: &[PathBuf]) -> Option<&'static str> {
    let fresh = PENDING.lock().unwrap().remove(&key(session, path));
    if !fresh || !path.is_file() {
        return None;
    }
    kind_of(path, base, skip)
}

/// 그 안의 파일은 사이드바에 놓지 않는 폴더: Claude Code의 설정·기억, Cypher의 저장소, 임시 폴더.
fn skip_roots() -> Vec<PathBuf> {
    let mut roots = vec![crate::user_dir(), std::env::temp_dir()];
    roots.extend(crate::extras::claude_home());
    roots.extend(crate::data_dir());
    roots
}

/// 경로를 견줄 수 있게 소문자 부분들로 나눈다(`C:\A\b` → ["c:", "a", "b"]).
fn parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|part| match part {
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().to_lowercase()),
            Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

/// `path`가 `root` 안에 있는지(대/소문자 무시).
pub fn is_within(path: &Path, root: &Path) -> bool {
    let (path, root) = (parts(path), parts(root));
    !root.is_empty() && path.len() >= root.len() && path[..root.len()] == root[..]
}

/// 두 경로가 같은 곳인지(대/소문자·끝의 구분자 무시).
pub fn same_place(a: &Path, b: &Path) -> bool {
    let (a, b) = (parts(a), parts(b));
    !a.is_empty() && a == b
}

/// 사람이 볼 결과물을 두지 않는 폴더 이름: 숨김 폴더(.git, .claude …), 의존성·빌드 폴더.
fn is_plumbing(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build" | "__pycache__")
}

/// 사이드바에 보일 종류: "doc"(문서), "image"(이미지), "http"(HTTP 요청), "other"(그 밖의 글자 파일).
/// 보여 주지 않는 파일(설정·저장소·임시 폴더 안, 숨김 폴더·의존성·빌드 폴더 안, 글자가 아닌 파일)은 None.
///
/// 숨김·빌드 폴더 규칙은 세션이 일하는 폴더(`base`) 아래의 부분에만 건다. 프로젝트 자체가 `D:\build\app`이나
/// `<저장소>\.claude\worktrees\<이름>`(워크트리 세션) 아래에 있어도 그 안에서 만든 파일은 놓여야 한다.
/// 세션 폴더 밖에 만든 파일은 경로 전체를 본다.
fn kind_of(path: &Path, base: Option<&Path>, skip: &[PathBuf]) -> Option<&'static str> {
    // `..`가 든 경로는 견주는 폴더와 실제 폴더가 달라질 수 있으므로 받지 않는다.
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir)) || skip.iter().any(|root| is_within(path, root)) {
        return None;
    }
    let all = parts(path.parent()?);
    let inside = base.map(parts).filter(|base| !base.is_empty() && all.len() >= base.len() && all[..base.len()] == base[..]);
    let mut folders: &[String] = match &inside {
        Some(base) => &all[base.len()..],
        None => all.get(1..).unwrap_or(&[]), // 드라이브 글자는 뺀다.
    };
    // 요청 파일을 두는 자리(`<폴더>\.cypher\http`)는 숨김 폴더지만 HTTP 항목이 보는 곳이다.
    if let [rest @ .., a, b] = folders {
        if a == ".cypher" && b == "http" {
            folders = rest;
        }
    }
    if folders.iter().any(|name| is_plumbing(name)) {
        return None;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if crate::image::format_of_extension(&ext).is_some() {
        return Some("image");
    }
    match ext.as_str() {
        "http" => Some("http"),
        "md" | "markdown" | "mdown" | "mkd" | "txt" => Some("doc"),
        _ => looks_like_text(path).then_some("other"),
    }
}

/// 앞부분에 0 바이트가 없으면 글자 파일로 본다.
fn looks_like_text(path: &Path) -> bool {
    let mut head = [0u8; 4096];
    let Ok(read) = std::fs::File::open(path).and_then(|mut file| file.read(&mut head)) else { return false };
    !head[..read].contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 종류를_가린다() {
        let dir = test_dir("cypher-created-kind-test");
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let none: [PathBuf; 0] = [];
        let kind = |name: &str, bytes: &[u8]| kind_of(&file(name, bytes), Some(&dir), &none);
        assert_eq!(kind("계획 문서.md", b"# x"), Some("doc"));
        assert_eq!(kind("notes.TXT", b"x"), Some("doc"));
        assert_eq!(kind("docs/그림.svg", b"<svg/>"), Some("image"));
        assert_eq!(kind("shot.PNG", b"\x89PNG"), Some("image"));
        assert_eq!(kind("api/요청.http", b"GET http://x"), Some("http"));
        assert_eq!(kind("src/main.rs", b"fn main() {}"), Some("other"));
        // .rest 파일은 HTTP 항목이 열지 못하므로 요청 파일로 치지 않는다.
        assert_eq!(kind("api/요청.rest", b"GET http://x"), Some("other"));
        assert_eq!(kind("data.bin", &[1, 0, 2]), None);
        // 숨김 폴더·의존성·빌드 폴더 안의 파일은 놓지 않는다.
        for hidden in [".git/x.md", ".claude/skills/a.md", "node_modules/p/README.md", "target/doc/x.md", "a/.cache/b.txt", ".cypher/x.md", ".cypher/http/sub/a.http"] {
            assert_eq!(kind(hidden, b"x"), None, "{hidden}");
        }
        // 요청 파일을 두는 자리는 숨김 폴더여도 놓는다.
        assert_eq!(kind(".cypher/http/requests.http", b"GET http://x"), Some("http"));
        // 점으로 시작하는 파일 이름 자체는 괜찮다.
        assert_eq!(kind(".notes.md", b"x"), Some("doc"));
        // 건너뛸 폴더 안(설정·저장소·임시)과 상대 경로.
        assert_eq!(kind_of(&file("계획 문서.md", b"# x"), Some(&dir), std::slice::from_ref(&dir)), None);
        assert_eq!(kind_of(Path::new("상대.md"), None, &none), None);
        // `..`로 돌아 나가는 경로는 놓지 않는다(건너뛸 폴더를 피해 가지 못하게).
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert_eq!(kind_of(&dir.join("sub").join("..").join("계획 문서.md"), Some(&dir), &none), None);
    }

    #[test]
    fn 숨김_폴더_규칙은_세션_폴더_아래에만_건다() {
        // 프로젝트가 숨김·빌드 폴더 아래에 있어도(워크트리 세션 등) 그 안에서 만든 파일은 놓는다.
        let root = test_dir("cypher-created-base-test");
        let project = root.join(".claude").join("worktrees").join("build");
        std::fs::create_dir_all(project.join("docs")).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        let doc = project.join("docs").join("a.md");
        let inner = project.join(".git").join("b.md");
        std::fs::write(&doc, "x").unwrap();
        std::fs::write(&inner, "x").unwrap();
        let none: [PathBuf; 0] = [];
        assert_eq!(kind_of(&doc, Some(&project), &none), Some("doc"));
        assert_eq!(kind_of(&inner, Some(&project), &none), None);
        // 세션 폴더를 모르거나 그 밖에 만든 파일은 경로 전체를 본다.
        assert_eq!(kind_of(&doc, None, &none), None);
        assert_eq!(kind_of(&doc, Some(&root.join("다른 곳")), &none), None);
    }

    #[test]
    fn 폴더_안인지는_대소문자를_가리지_않는다() {
        assert!(is_within(Path::new(r"C:\Work\Proj\a\b.md"), Path::new(r"c:\work\proj")));
        assert!(is_within(Path::new(r"C:\work\proj"), Path::new(r"C:\work\proj\")));
        assert!(!is_within(Path::new(r"C:\work\project2\a.md"), Path::new(r"C:\work\proj")));
        assert!(!is_within(Path::new(r"D:\work\proj\a.md"), Path::new(r"C:\work\proj")));
        assert!(!is_within(Path::new(r"C:\work"), Path::new(r"C:\work\proj")));
        assert!(!is_within(Path::new(r"C:\work\a.md"), Path::new("")));
        assert!(same_place(Path::new(r"C:\Work\Proj\"), Path::new(r"c:\work\proj")));
        assert!(!same_place(Path::new(r"C:\work\proj\a"), Path::new(r"c:\work\proj")));
        assert!(!same_place(Path::new(""), Path::new("")));
    }

    #[test]
    fn 쓰기_전에_없던_파일만_새로_만든_것으로_본다() {
        let dir = test_dir("cypher-created-write-test");
        let none: [PathBuf; 0] = [];
        let after_write = |session: u32, path: &Path| after_write_in(session, path, Some(&dir), &none);

        let fresh = dir.join("새 문서.md");
        before_write(7, &fresh);
        // 아직 쓰지 않았으면(쓰기가 거절됨) 보여 줄 것이 없다. 꺼낸 뒤라 다시 물어도 없다.
        assert_eq!(after_write(7, &fresh), None);
        before_write(7, &fresh);
        std::fs::write(&fresh, "# 새 문서").unwrap();
        // 다른 세션의 신호와 섞이지 않는다.
        assert_eq!(after_write(8, &fresh), None);
        assert_eq!(after_write(7, &fresh), Some("doc"));
        // 이미 있던 파일을 고쳐 쓴 것은 새로 만든 것이 아니다.
        before_write(7, &fresh);
        std::fs::write(&fresh, "# 고침").unwrap();
        assert_eq!(after_write(7, &fresh), None);
        // 직전 신호 없이 온 직후 신호는 받지 않는다.
        assert_eq!(after_write(7, &dir.join("신호 없음.md")), None);
        // 쓰기가 거절돼 기록이 남았어도, 나중에 다른 길로 생긴 그 파일을 고쳐 쓴 것은 새 파일이 아니다.
        let denied = dir.join("거절됨.md");
        before_write(7, &denied);
        std::fs::write(&denied, "다른 길로 생김").unwrap();
        before_write(7, &denied);
        assert_eq!(after_write(7, &denied), None);
        // 경로의 대/소문자가 달라도 같은 파일이다.
        let other = dir.join("B.TXT");
        before_write(7, &dir.join("b.txt"));
        std::fs::write(&other, "x").unwrap();
        assert_eq!(after_write(7, &other), Some("doc"));
    }
}
