//! Claude 세션이 만든 파일을 사이드바의 어느 종류로 보여 줄지 정한다.
//!
//! Claude Code가 Write 도구로 파일을 쓰기 직전과 직후에 훅 신호가 온다(`status.rs`). 쓰기 직전에 그 파일이
//! 없었으면 "새로 만든 파일"이고, 그런 파일만 종류를 가려 화면에 알린다. 화면은 그 세션의 묶음에
//! 종류에 맞는 항목(문서, 이미지, HTTP, 표, 사이트)으로 놓는다.
//!
//! 전달 프로그램은 직전 신호를 보낸 뒤 앱이 받아 처리할 때까지 기다렸다가 끝나므로(`status.rs`의 답),
//! "그때 파일이 있었는지"는 Claude Code가 실제로 쓰기 전에 확인된다.

use std::collections::HashSet;
use std::io::Read;
use crate::paths::{is_within, parts, strip_root};
use std::path::{Path, PathBuf};
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
    // 환경 변수의 폴더는 짧은 이름(8.3: `C:\Users\LONGNA~1\AppData\Local\Temp`)으로 적혀 있을 수 있고, Claude는
    // 긴 이름으로 쓴다. 글자로 견주므로 긴 이름 꼴도 함께 넣는다.
    let long: Vec<PathBuf> = roots.iter().filter_map(|root| long_form(root)).collect();
    for root in long {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

/// 폴더의 긴 이름 꼴(운영체제가 알려 주는 실제 이름). 없는 폴더거나 이 컴퓨터의 드라이브 경로가 아니면 None.
fn long_form(dir: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(dir).ok()?;
    let text = real.to_str()?.strip_prefix(r"\\?\")?;
    (!text.starts_with("UNC")).then(|| PathBuf::from(text))
}

/// 사람이 볼 결과물을 두지 않는 폴더 이름: 숨김 폴더(.git, .claude …), 의존성·빌드 폴더.
fn is_plumbing(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build" | "__pycache__")
}

/// 사이드바에 보일 종류: "doc"(문서), "image"(이미지), "http"(HTTP 요청), "table"(표: CSV·TSV),
/// "html"(HTML 파일: 사이트 항목으로 연다), "other"(그 밖의 글자 파일).
/// 보여 주지 않는 파일(설정·저장소·임시 폴더 안, 숨김 폴더·의존성·빌드 폴더 안, 글자가 아닌 파일)은 None.
///
/// 숨김·빌드 폴더 규칙은 세션이 일하는 폴더(`base`) 아래의 부분에만 건다. 프로젝트 자체가 `D:\build\app`이나
/// `<저장소>\.claude\worktrees\<이름>`(워크트리 세션) 아래에 있어도 그 안에서 만든 파일은 놓여야 한다.
/// 세션 폴더 밖에 만든 파일은 경로 전체를 본다.
fn kind_of(path: &Path, base: Option<&Path>, skip: &[PathBuf]) -> Option<&'static str> {
    if !path.is_absolute() || skip.iter().any(|root| is_within(path, root)) {
        return None;
    }
    // 파일이 든 폴더의 이름들. `..`가 든 경로는 여기서 걸러진다(견주는 폴더와 실제 폴더가 달라질 수 있다).
    let dir = path.parent()?;
    let folders = match base.and_then(|base| strip_root(dir, base)) {
        Some(below) => below,
        None => parts(dir)?.split_off(1), // 세션 폴더 밖: 드라이브 글자만 빼고 경로 전체를 본다.
    };
    let mut folders = folders.as_slice();
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
        // 값을 구분자로 나눈 글은 표 항목으로 보여 준다(JSON은 설정 파일인 경우가 많아 표로 치지 않는다).
        "csv" | "tsv" => Some("table"),
        "md" | "markdown" | "mdown" | "mkd" | "txt" => Some("doc"),
        // HTML 결과물(보고서, 시안)은 브라우저로 봐야 하므로 사이트 항목으로 놓는다.
        "html" | "htm" => Some("html"),
        _ => looks_like_text(path).then_some("other"),
    }
}

/// 이름으로 보아 잠깐 쓰고 버릴 파일인지: `tmp`·`temp`·`scratch`·`임시`로 시작하거나, `…_tmp`·`….bak`·`…-old`처럼 끝나는 이름.
/// (`temperature.md`나 `oldest.csv`처럼 낱말의 일부인 것은 치지 않는다: 구분 글자나 이름의 끝이 이어져야 한다.)
fn looks_temporary(path: &Path) -> bool {
    let lower = |part: Option<&std::ffi::OsStr>| part.map(|text| text.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (stem, ext) = (lower(path.file_stem()), lower(path.extension()));
    if matches!(ext.as_str(), "tmp" | "temp" | "bak" | "part" | "partial" | "swp") {
        return true;
    }
    let is_break = |ch: Option<char>| ch.is_none_or(|ch| matches!(ch, '-' | '_' | '.' | ' '));
    let head = ["tmp", "temp", "scratch", "임시"].iter().any(|word| stem.strip_prefix(word).is_some_and(|rest| is_break(rest.chars().next())));
    let tail = ["tmp", "temp", "bak", "backup", "old", "wip", "scratch", "임시"]
        .iter()
        .any(|word| stem.strip_suffix(word).is_some_and(|rest| !rest.is_empty() && is_break(rest.chars().next_back())));
    head || tail
}

/// 그 문서(Markdown·HTML)가 그 이름의 파일을 가리키는지. 이름 그대로와 주소에 맞게 바꿔 적은 꼴(빈칸 → `%20` 등)을 찾는다.
fn mentions(text: &str, name: &str) -> bool {
    let encoded: String = name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02x}") }).collect();
    // 이름의 앞이 다른 이름의 일부면 안 된다(`bar-chart.png`를 넣은 문서가 `chart.png`를 넣은 것은 아니다).
    let whole = |needle: &str| {
        text.match_indices(needle).any(|(at, _)| {
            !text[..at].chars().next_back().is_some_and(|before| before.is_alphanumeric() || matches!(before, '-' | '_' | '.' | '%'))
        })
    };
    whole(name) || whole(&encoded)
}

/// 요청 하나에서 만든 파일들 가운데 사이드바에 놓을 "최종 파일"을 가린다(같은 순서의 참·거짓).
/// 놓지 않는 것(중간 파일):
/// - 요청이 끝난 지금은 없는 파일(만들었다가 지웠거나 다른 이름으로 옮겼다)과 빈 파일
/// - 이름으로 보아 잠깐 쓰는 파일(`tmp_…`, `….bak` 등)
/// - 같은 요청에서 만든 문서·HTML 파일 안에 들어간 그림(그 문서의 부품이다. 문서를 열면 함께 보인다)
pub fn final_files(paths: &[PathBuf]) -> Vec<bool> {
    const MAX_DOC: u64 = 4 << 20; // 이보다 큰 문서는 읽지 않는다
    let is_doc = |path: &Path| path.extension().is_some_and(|ext| ["md", "markdown", "mdown", "mkd", "html", "htm"].iter().any(|known| ext.eq_ignore_ascii_case(known)));
    let is_image = |path: &Path| path.extension().is_some_and(|ext| crate::image::format_of_extension(&ext.to_string_lossy().to_lowercase()).is_some());
    let alive: Vec<bool> = paths.iter().map(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)).collect();
    // 살아 있는 문서의 글(소문자). 그림이 있을 때만 읽는다.
    let any_image = paths.iter().zip(&alive).any(|(path, alive)| *alive && is_image(path));
    let docs: Vec<(usize, String)> = if any_image {
        paths
            .iter()
            .enumerate()
            .filter(|(index, path)| alive[*index] && is_doc(path) && std::fs::metadata(path).is_ok_and(|meta| meta.len() <= MAX_DOC))
            .filter_map(|(index, path)| Some((index, std::fs::read_to_string(path).ok()?.to_lowercase())))
            .collect()
    } else {
        Vec::new()
    };
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            if !alive[index] || looks_temporary(path) {
                return false;
            }
            if !is_image(path) {
                return true;
            }
            let Some(name) = path.file_name().map(|name| name.to_string_lossy().to_lowercase()) else { return true };
            !docs.iter().any(|(doc, text)| *doc != index && mentions(text, &name))
        })
        .collect()
}

/// 파일마다의 판정: "final"(사이드바에 놓을 최종 파일), "later"(있지만 아직 빈 파일 — 다음에 다시 본다), "skip"(놓지 않는다).
pub fn verdicts(paths: &[PathBuf]) -> Vec<&'static str> {
    final_files(paths)
        .into_iter()
        .zip(paths)
        .map(|(is_final, path)| {
            if is_final {
                "final"
            } else if !looks_temporary(path) && std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == 0) {
                // 빈 파일로 만들어 두고 다음 요청에서 채우는 경우가 있다.
                "later"
            } else {
                "skip"
            }
        })
        .collect()
}

/// 화면이 묻는다: 요청이 끝났을 때, 그동안 만든 파일 가운데 어느 것이 사이드바에 놓을 최종 파일인지.
#[tauri::command]
pub async fn created_final(paths: Vec<String>) -> Vec<&'static str> {
    // 파일을 읽는 일이라 작업 스레드에서 한다. 가리지 못하면 모두 놓는다.
    let count = paths.len();
    tauri::async_runtime::spawn_blocking(move || verdicts(&paths.iter().map(PathBuf::from).collect::<Vec<_>>()))
        .await
        .unwrap_or_else(|_| vec!["final"; count])
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
        assert_eq!(kind("out/결과.csv", b"a,b\n1,2"), Some("table"));
        assert_eq!(kind("out/rows.TSV", b"a\tb"), Some("table"));
        assert_eq!(kind("out/보고서.html", b"<!doctype html>"), Some("html"));
        assert_eq!(kind("index.HTM", b"<html></html>"), Some("html"));
        assert_eq!(kind("package.json", b"{}"), Some("other"));
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
    fn 중간_파일은_최종_파일로_치지_않는다() {
        let dir = test_dir("cypher-created-final-test");
        let file = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            path
        };
        let report = file("보고서.html", "<img src=\"chart%201.png\"><img src='logo.svg'>");
        let notes = file("정리.md", "# 정리\n![그림](Diagram.PNG)\n결과는 결과.csv에 있다");
        let paths = vec![
            report.clone(),                    // 최종 문서
            notes.clone(),                     // 최종 문서
            file("chart 1.png", "그림"),       // 보고서에 들어간 그림
            file("logo.svg", "<svg/>"),        // 보고서에 들어간 그림
            file("diagram.png", "그림"),       // 정리에 들어간 그림(대/소문자가 달라도)
            file("혼자 있는 그림.png", "그림"), // 어느 문서에도 들어가지 않은 그림
            file("결과.csv", "a,b"),           // 문서가 이름을 적었어도 그림이 아니면 최종 파일이다
            file("bar-chart 1.png", "그림"),   // 이름의 뒷부분만 같은 다른 그림은 문서에 든 것이 아니다
            file("tmp_data.csv", "a,b"),       // 이름이 임시 파일
            file("draft.md.bak", "x"),
            file("빈 파일.md", ""),
            dir.join("지운 파일.md"),          // 만들었다가 지웠다
        ];
        assert_eq!(final_files(&paths), [true, true, false, false, false, true, true, true, false, false, false, false]);
        // 빈 파일은 "나중에 다시 보기", 그 밖의 놓지 않는 것은 "건너뛰기"로 알려 준다.
        let said = verdicts(&paths);
        assert_eq!(said[0], "final");
        assert_eq!(said[paths.len() - 2], "later"); // 빈 파일.md
        assert_eq!(said[paths.len() - 1], "skip"); // 지운 파일.md
        assert_eq!(said[paths.len() - 4], "skip"); // tmp_data.csv
        // 문서 하나만 만들었으면 그대로 최종 파일이다. 자기 이름을 적은 문서도 마찬가지다.
        assert_eq!(final_files(std::slice::from_ref(&report)), [true]);
        let own = file("self.svg", "<svg><!-- self.svg --></svg>");
        assert_eq!(final_files(&[own]), [true]);

        for name in ["tmp.md", "TMP-1.csv", "temp notes.md", "scratch.txt", "임시_정리.md", "report_old.md", "data.bak.csv", "plan-wip.md", "x.tmp", "a.part"] {
            assert!(looks_temporary(Path::new(name)), "{name}");
        }
        for name in ["temperature.md", "template.html", "oldest.csv", "bold.md", "draft.md", "중간보고.md", "old.md", "scratchpad.md", "wip.md", "header-partial.html"] {
            assert!(!looks_temporary(Path::new(name)), "{name}");
        }
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
