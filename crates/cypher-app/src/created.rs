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
use crate::paths::{is_within, parts, same_place, strip_root};
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

/// 그 파일들 가운데 git이 무시하는 것(같은 순서의 참·거짓). 프로젝트의 `.gitignore`(와 git의 다른 무시 규칙)에
/// 걸리는 파일은 그 프로젝트가 스스로 "저장소에 두지 않는 산출물"이라고 적어 둔 것이다: 빌드 폴더의 이름이 무엇이든
/// (`out`, `bin`, `generated`, 따로 정한 출력 폴더) 빌드 도구를 몰라도 가려진다.
///
/// 파일이 든 폴더마다 `git check-ignore`를 한 번 부른다(보기만 하는 명령). 저장소가 아니거나 git을 실행할 수 없으면
/// 무시하지 않는 것으로 친다. 요청 파일을 두는 자리(`.cypher\http`)는 숨김 폴더라 무시해 두는 일이 많지만 HTTP 항목이
/// 보는 곳이므로 묻지 않는다.
fn git_ignored(paths: &[&Path]) -> Vec<bool> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// 한 번에 묻는 이름 수(입력과 출력이 한꺼번에 차서 서로 기다리는 일이 없게 작게 나눈다).
    const AT_ONCE: usize = 100;

    let mut ignored = vec![false; paths.len()];
    // 폴더 → 그 폴더에 든 파일들의 (자리, 이름)
    let mut by_dir: Vec<(&Path, Vec<(usize, String)>)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { continue };
        if parts(dir).is_some_and(|names| matches!(names.as_slice(), [.., a, b] if a == ".cypher" && b == "http")) {
            continue;
        }
        let name = name.to_string_lossy().into_owned();
        match by_dir.iter_mut().find(|(known, _)| *known == dir) {
            Some((_, list)) => list.push((index, name)),
            None => by_dir.push((dir, vec![(index, name)])),
        }
    }
    for (dir, list) in by_dir {
        for chunk in list.chunks(AT_ONCE) {
            let spawned = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["check-ignore", "-z", "--stdin"])
                .env("GIT_OPTIONAL_LOCKS", "0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            let Ok(mut child) = spawned else { return ignored }; // git이 없다.
            if let Some(mut stdin) = child.stdin.take() {
                let asked: Vec<u8> = chunk.iter().flat_map(|(_, name)| name.bytes().chain(std::iter::once(0))).collect();
                let _ = stdin.write_all(&asked);
            }
            let Ok(output) = child.wait_with_output() else { continue };
            // 0: 무시하는 것이 있다, 1: 없다, 그 밖(128 등): 저장소가 아니다.
            if output.status.code() != Some(0) {
                continue;
            }
            let said = String::from_utf8_lossy(&output.stdout);
            let said: HashSet<&str> = said.split('\0').filter(|name| !name.is_empty()).collect();
            for (index, name) in chunk {
                ignored[*index] = said.contains(name.as_str());
            }
        }
    }
    ignored
}

/// 빌드 도구가 정한 소스 폴더에 든 파일인지: 그 파일에서 위로 올라가며 처음 만나는 프로젝트 파일(`Cargo.toml`,
/// `pom.xml`, `build.gradle`·`build.gradle.kts`)이 있는 폴더를 그 프로젝트(모듈)의 뿌리로 보고, 그 바로 아래의
/// 소스 폴더(Cargo: `src`·`tests`·`benches`·`examples`, Maven·Gradle: `src`) 안에 있으면 참.
/// Maven·Gradle은 빌드 파일 없이 뿌리의 설정만으로 꾸린 하위 프로젝트가 있으므로, 뿌리 아래 어디든 표준 소스 자리
/// (`…/src/main/…`, `…/src/test/…`)에 든 파일도 친다.
///
/// 그런 파일(`src/main/resources/templates/*.html`, `src/test/resources/*.csv`, `tests/data/*.txt` …)은 프로그램의
/// 일부(자원·시험 자료)이지 사람이 볼 결과물이 아니다. `base`(세션이 일하는 폴더) 위로는 올라가지 않는다. 세션 폴더
/// 밖에 만든 파일이거나 세션 폴더를 모르면 몇 층만 올라가 본다.
fn in_source_tree(path: &Path, base: Option<&Path>) -> bool {
    const CARGO: [&str; 4] = ["src", "tests", "benches", "examples"];
    const JVM: [&str; 1] = ["src"];
    /// 세션 폴더로 끝을 정할 수 없을 때 올라가 보는 층 수.
    const LEVELS: usize = 6;
    let base = base.filter(|base| is_within(path, base));
    let mut trail: Vec<String> = Vec::new(); // 지금 보는 폴더에서 파일까지 가는 폴더 이름들(아래에서 위로 쌓는다)
    for (level, dir) in path.ancestors().skip(1).enumerate() {
        let has = |name: &str| dir.join(name).is_file();
        let roots: Option<&[&str]> = if has("Cargo.toml") {
            Some(&CARGO)
        } else if has("pom.xml") || has("build.gradle") || has("build.gradle.kts") {
            Some(&JVM)
        } else {
            None
        };
        // 가장 가까운 프로젝트 파일이 정한다(그 위의 것은 보지 않는다). 프로젝트 파일과 같은 폴더에 바로 놓인 파일
        // (README 등)은 소스 폴더 안이 아니다.
        if let Some(roots) = roots {
            let first = trail.last().is_some_and(|first| roots.iter().any(|root| first.eq_ignore_ascii_case(root)));
            // trail은 아래에서 위로 쌓였다: [.., "main", "src", ..]이면 `src/main`이다.
            let standard = roots.len() == JVM.len()
                && trail.windows(2).any(|pair| pair[1].eq_ignore_ascii_case("src") && ["main", "test"].iter().any(|set| pair[0].eq_ignore_ascii_case(set)));
            return first || standard;
        }
        match base {
            Some(base) if same_place(dir, base) => break,
            None if level >= LEVELS => break,
            _ => {}
        }
        match dir.file_name() {
            Some(name) => trail.push(name.to_string_lossy().into_owned()),
            None => break, // 드라이브의 뿌리
        }
    }
    false
}

/// 그 폴더가 속한 git 저장소의 뿌리(작업 폴더의 맨 위). 저장소가 아니거나 git을 실행할 수 없으면 None.
fn git_top(dir: &Path) -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let top = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !top.is_empty()).then(|| PathBuf::from(top))
}

/// 파일마다의 판정: "final"(사이드바에 놓을 최종 파일), "later"(있지만 아직 빈 파일 — 다음에 다시 본다), "skip"(놓지 않는다).
///
/// 중간 파일(`final_files`)에 더해, git이 무시하는 파일과 (`keep_sources`가 아니면) 소스 폴더에 든 파일도 놓지 않는다.
/// `base`는 그 세션이 일하는 폴더, `keep_sources`는 설정이 "모든 글자 파일을 놓기"(`show_created = "all"`)일 때 참이다:
/// 그때는 코드 파일도 놓아 달라는 것이므로 소스 폴더라고 가리지 않는다.
pub fn verdicts(paths: &[PathBuf], base: Option<&Path>, keep_sources: bool) -> Vec<&'static str> {
    let mut said: Vec<&'static str> = final_files(paths)
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
        .collect();
    // 놓을 것과 나중에 다시 볼 것만 더 가린다(이미 놓지 않기로 한 것 때문에 git을 부르지 않는다).
    let open: Vec<usize> = (0..paths.len()).filter(|index| said[*index] != "skip").collect();
    if open.is_empty() {
        return said;
    }
    // 세션 폴더 안의 파일은 그 파일이 속한 저장소가 어디서 시작하는지를 보고 git 규칙을 쓸지 정한다
    // (숨김·빌드 폴더 규칙을 세션 폴더 아래에만 거는 것과 같은 까닭):
    // - 저장소의 뿌리가 세션 폴더이거나 그 안이면 이 프로젝트의 저장소다: 그 규칙을 쓴다.
    // - 저장소의 뿌리가 세션 폴더보다 위면, 세션 폴더 자체가 그 저장소에서 무시되는 곳인지 본다. 무시되는 곳이면
    //   (위쪽 저장소가 무시하는 폴더 안의 프로젝트, 홈 폴더를 `*`로 무시하는 저장소 아래의 폴더 등) 그 규칙은 이
    //   프로젝트의 산출물을 가리는 것이 아니므로 쓰지 않는다.
    // 세션 폴더 밖의 파일(이나 세션 폴더를 모를 때)은 그대로 묻는다.
    let mut tops: Vec<(&Path, Option<PathBuf>)> = Vec::new(); // 폴더 → 저장소의 뿌리(같은 폴더는 한 번만 묻는다)
    let mut base_ignored: Option<bool> = None;
    let asked: Vec<usize> = open
        .iter()
        .copied()
        .filter(|index| {
            let path = &paths[*index];
            let (Some(base), Some(dir)) = (base.filter(|base| is_within(path, base)), path.parent()) else { return true };
            let top = match tops.iter().find(|(known, _)| *known == dir) {
                Some((_, top)) => top.clone(),
                None => {
                    let top = git_top(dir);
                    tops.push((dir, top.clone()));
                    top
                }
            };
            match top {
                None => false, // 저장소가 아니다: 물어볼 것이 없다.
                Some(top) if is_within(&top, base) => true,
                Some(_) => !*base_ignored.get_or_insert_with(|| git_ignored(&[base])[0]),
            }
        })
        .collect();
    let ignored = git_ignored(&asked.iter().map(|index| paths[*index].as_path()).collect::<Vec<_>>());
    for (index, ignored) in asked.into_iter().zip(ignored) {
        if ignored {
            said[index] = "skip";
        }
    }
    for index in open {
        if !keep_sources && in_source_tree(&paths[index], base) {
            said[index] = "skip";
        }
    }
    said
}

/// 화면이 묻는다: 요청이 끝났을 때, 그동안 만든 파일 가운데 어느 것이 사이드바에 놓을 최종 파일인지.
#[tauri::command]
pub async fn created_final(paths: Vec<String>, base: Option<String>, keep_sources: Option<bool>) -> Vec<&'static str> {
    // 파일을 읽고 git을 부르는 일이라 작업 스레드에서 한다. 가리지 못하면 모두 놓는다.
    let count = paths.len();
    tauri::async_runtime::spawn_blocking(move || {
        let base = base.filter(|base| !base.is_empty()).map(PathBuf::from);
        verdicts(&paths.iter().map(PathBuf::from).collect::<Vec<_>>(), base.as_deref(), keep_sources.unwrap_or(false))
    })
    .await
    .unwrap_or_else(|_| vec!["final"; count])
}

/// 화면이 묻는다: 앞서 사이드바에 놓아 준 파일들 가운데 지금은 없는 것(같은 순서의 참·거짓). 없어진 파일의 항목은
/// 화면이 걷는다. 그 파일은 없는데 그 위의 폴더는 보일 때만 참이다: 드라이브가 빠졌거나(외장 디스크, 끊긴 네트워크
/// 드라이브) 읽을 권한이 없는 것은 없어진 것이 아니다. Windows는 드라이브가 없을 때도 "찾을 수 없음"이라고 답하므로,
/// 그 드라이브(공유 폴더)의 뿌리가 보이는지를 함께 본다(끊긴 네트워크 드라이브에서 층마다 기다리지 않게 뿌리만 본다).
#[tauri::command]
pub async fn created_gone(paths: Vec<String>) -> Vec<bool> {
    let count = paths.len();
    tauri::async_runtime::spawn_blocking(move || paths.iter().map(|path| gone(Path::new(path))).collect())
        .await
        .unwrap_or_else(|_| vec![false; count])
}

fn gone(path: &Path) -> bool {
    path.is_absolute()
        && matches!(std::fs::metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        && path.ancestors().last().is_some_and(|drive| drive != path && drive.is_dir())
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
        let said = verdicts(&paths, Some(&dir), false);
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
    fn git이_무시하는_파일은_놓지_않는다() {
        let dir = test_dir("cypher-created-ignore-test");
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().is_ok_and(|output| output.status.success());
        let file = |name: &str| {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "내용").unwrap();
            path
        };
        let paths = vec![
            file("out/report.html"),         // 이름이 낯선 빌드 폴더
            file("notes.md"),
            file("docs/a.gen.md"),           // 이름 규칙으로 무시한 것
            file("docs/guide.md"),
            file(".cypher/http/req.http"),   // 숨김 폴더를 무시해 두었어도 요청 파일 자리는 놓는다
            file("generated/표 1.csv"),
            dir.join("docs").join("없는 파일.md"),
        ];
        // 저장소가 아니면 아무것도 가리지 않는다.
        assert_eq!(verdicts(&paths, Some(&dir), false), ["final", "final", "final", "final", "final", "final", "skip"]);
        if !git(&["init", "-q"]) {
            return; // git이 없는 곳에서는 여기까지만 본다.
        }
        std::fs::write(dir.join(".gitignore"), "out/\n*.gen.md\n.cypher/\n/generated/\n").unwrap();
        assert_eq!(verdicts(&paths, Some(&dir), false), ["skip", "final", "skip", "final", "final", "skip", "skip"]);
        // 놓지 않기로 한 것만 물어도, 폴더가 달라도 자리가 어긋나지 않는다.
        let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        assert_eq!(git_ignored(&refs[..6]), [true, false, true, false, false, true]);
        // 모든 글자 파일을 놓는 설정이어도 무시하는 파일은 놓지 않는다.
        assert_eq!(verdicts(&paths[..2], Some(&dir), true), ["skip", "final"]);
        // 한 폴더에 한 번에 묻는 수보다 많아도, 이름에 특별한 글자가 있어도 자리가 어긋나지 않는다.
        std::fs::create_dir_all(dir.join("many")).unwrap();
        let many: Vec<PathBuf> = (0..230).map(|n| dir.join("many").join(if n % 7 == 0 { format!("표 [{n}] #!;.gen.md") } else { format!(" 문서 {n}.md") })).collect();
        let many_refs: Vec<&Path> = many.iter().map(PathBuf::as_path).collect();
        assert_eq!(git_ignored(&many_refs), (0..230).map(|n| n % 7 == 0).collect::<Vec<_>>());
        // 세션 폴더가 통째로 무시되는 곳이면(위쪽 저장소가 무시하는 폴더 안에서 일한다) 그 규칙으로 가리지 않는다.
        // 세션 폴더 밖의 파일은 그대로 가린다.
        let inside = file("out/sub/보고서.md");
        assert_eq!(verdicts(&[inside.clone(), paths[2].clone()], Some(&dir.join("out")), false), ["final", "skip"]);
        assert_eq!(verdicts(std::slice::from_ref(&inside), Some(&dir), false), ["skip"]);
        // 그 안에 따로 있는 저장소(여러 저장소를 담은 폴더에서 일할 때)의 규칙은 쓴다.
        let inner = dir.join("out").join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        assert!(std::process::Command::new("git").arg("-C").arg(&inner).args(["init", "-q"]).output().is_ok_and(|output| output.status.success()));
        std::fs::write(inner.join(".gitignore"), "build2/
").unwrap();
        let (made, doc) = (file("out/inner/build2/r.md"), file("out/inner/doc.md"));
        assert_eq!(verdicts(&[made.clone(), doc.clone(), inside.clone()], Some(&dir.join("out")), false), ["skip", "final", "final"]);
        assert_eq!(verdicts(&[made, doc], Some(&inner), false), ["skip", "final"]);
        // 프로젝트 자신의 규칙이 넓어도(허용한 것만 두는 꼴, 확장자 없는 파일을 무시하는 꼴) 그 규칙을 그대로 쓴다.
        std::fs::write(inner.join(".gitignore"), "/*
!/docs/
").unwrap();
        assert_eq!(verdicts(&[file("out/inner/x.md"), file("out/inner/docs/y.md")], Some(&inner), false), ["skip", "final"]);
        std::fs::write(inner.join(".gitignore"), "*
!*.*
!*/
bin2/
").unwrap();
        assert_eq!(verdicts(&[file("out/inner/bin2/r.md"), file("out/inner/z.md")], Some(&inner), false), ["skip", "final"]);
        // 무시되는 곳의 빈 파일은 나중에 다시 보지도 않는다.
        std::fs::write(dir.join("out").join("빈.md"), "").unwrap();
        assert_eq!(verdicts(&[dir.join("out").join("빈.md"), dir.join("빈.md")], Some(&dir), false), ["skip", "skip"]);
        std::fs::write(dir.join("빈.md"), "").unwrap();
        assert_eq!(verdicts(&[dir.join("빈.md")], Some(&dir), false), ["later"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 소스_폴더에_든_파일은_놓지_않는다() {
        let dir = test_dir("cypher-created-source-test");
        let file = |name: &str| {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "내용").unwrap();
            path
        };
        // Rust: 뿌리에 Cargo.toml, 그 안에 딸린 프로젝트(examples/demo)와 워크스페이스의 크레이트(crates/core).
        file("rust/Cargo.toml");
        file("rust/examples/demo/Cargo.toml");
        file("rust/crates/core/Cargo.toml");
        let rust = dir.join("rust");
        for (name, source) in [
            ("rust/src/templates/page.html", true),
            ("rust/tests/data/rows.csv", true),
            ("rust/benches/input.txt", true),
            ("rust/examples/notes.md", true),
            ("rust/README.md", false),
            ("rust/docs/guide.md", false),
            ("rust/SRC2/x.md", false),
            // 딸린 프로젝트의 뿌리에 놓인 파일은 그 프로젝트의 문서다(위 프로젝트의 examples 안이어도).
            ("rust/examples/demo/README.md", false),
            ("rust/examples/demo/src/x.txt", true),
            ("rust/crates/core/src/fixture.md", true),
            ("rust/crates/core/CHANGELOG.md", false),
            ("rust/crates/notes.md", false),
        ] {
            assert_eq!(in_source_tree(&file(name), Some(&rust)), source, "{name}");
        }
        // Java: Maven 모듈과 Gradle 모듈. `tests`·`examples`는 소스 폴더로 치지 않는다.
        file("java/pom.xml");
        file("java/web/pom.xml");
        file("java/app/build.gradle.kts");
        file("java/lib/build.gradle");
        let java = dir.join("java");
        for (name, source) in [
            ("java/web/src/main/resources/templates/index.html", true),
            ("java/web/src/test/resources/data.csv", true),
            ("java/app/src/main/resources/static/logo.svg", true),
            ("java/lib/Src/test/x.txt", true),
            ("java/src/site/markdown/intro.md", true),
            ("java/web/README.md", false),
            ("java/docs/설계.md", false),
            ("java/web/tests/x.md", false),
            ("java/examples/x.md", false),
            // 빌드 파일 없이 뿌리의 설정만으로 꾸린 하위 프로젝트: 표준 소스 자리면 소스다.
            ("java/batch/src/main/resources/job.html", true),
            ("java/batch/SRC/Test/resources/rows.csv", true),
            ("java/batch/src/docs/guide.md", false),
            ("java/batch/main/src/x.md", false),
        ] {
            assert_eq!(in_source_tree(&file(name), Some(&java)), source, "{name}");
        }
        // 세션 폴더 위의 프로젝트 파일은 보지 않는다(세션 폴더가 남의 프로젝트 안의 한 폴더일 때).
        let inner = file("rust/docs/src/a.md");
        assert!(!in_source_tree(&inner, Some(&rust.join("docs"))));
        assert!(in_source_tree(&inner.with_file_name("b.md"), Some(&dir)) == false && in_source_tree(&file("rust/src/a.md"), Some(&dir)));
        // 세션 폴더를 모르거나 그 밖의 파일이면 몇 층만 올라가 본다.
        assert!(in_source_tree(&file("rust/src/a/b/c.md"), None));
        assert!(in_source_tree(&file("rust/src/a/b/c.md"), Some(&dir.join("다른 곳"))));
        assert!(!in_source_tree(&file("rust/src/1/2/3/4/5/6/7/8/deep.md"), None));
        // 프로젝트 파일이 없으면 `src` 안이어도 가리지 않는다.
        assert!(!in_source_tree(&file("plain/src/notes.md"), Some(&dir.join("plain"))));

        // 판정: 소스 폴더의 파일은 놓지 않되, 모든 글자 파일을 놓는 설정이면 놓는다.
        let paths = vec![file("rust/src/templates/page.html"), file("rust/docs/guide.md")];
        assert_eq!(verdicts(&paths, Some(&rust), false), ["skip", "final"]);
        assert_eq!(verdicts(&paths, Some(&rust), true), ["final", "final"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 없어진_파일만_없어졌다고_답한다() {
        let dir = test_dir("cypher-created-gone-test");
        let there = dir.join("있는 파일.md");
        std::fs::write(&there, "x").unwrap();
        assert!(!gone(&there));
        assert!(gone(&dir.join("지운 파일.md")));
        assert!(gone(&dir.join("없는 폴더").join("a.md")));
        // 폴더가 된 것은 없어진 것이 아니고, 상대 경로는 판단하지 않는다.
        assert!(!gone(&dir));
        assert!(!gone(Path::new("상대.md")));
        // 드라이브가 빠진 것은 없어진 것이 아니다(없는 드라이브 글자를 찾아 물어본다).
        if let Some(letter) = ('D'..='Z').rev().find(|letter| !Path::new(&format!("{letter}:\\")).exists()) {
            assert!(!gone(Path::new(&format!("{letter}:\\work\\a.md"))), "{letter}:");
        }
        let _ = std::fs::remove_dir_all(&dir);
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
