//! 코드 보기 항목: 프로젝트 폴더의 파일을 늘어놓고, 정의·쓰는 곳·기호·글을 프로젝트 전체에서 찾는다.
//!
//! 색인을 만들어 두지 않고 물을 때마다 파일을 훑는다. Claude 세션이 파일을 계속 고치는 동안에도 늘 지금 내용으로 답하고,
//! 수천 개의 파일도 여러 스레드로 나눠 읽으면 금방이다. 훑을 파일은 git이 아는 파일(추적하는 것과 무시하지 않는 새 파일)이고,
//! git 저장소가 아니면 숨김·빌드 폴더를 빼고 직접 돌아다닌다. 무엇이 정의이고 쓰는 곳인지는 `cypher_core::code`가 가린다.
//! 파일을 고치지는 않는다(보기 전용).

use cypher_core::code::{self, Def, Hit, Lang};
use cypher_core::textfile;
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 한 번에 훑는 파일 수와, 훑는 파일 하나의 크기.
const MAX_FILES: usize = 40_000;
const MAX_SCAN_BYTES: u64 = 1 << 20;
/// 한 번에 돌려주는 결과 수.
const MAX_HITS: usize = 2000;
const MAX_SYMBOLS: usize = 300;
/// 기호 찾기에서 순위를 매겨 볼 후보 수(이보다 많이 맞으면 더 훑지 않는다. 글을 더 적어 좁히면 된다).
const MAX_RANKED: usize = 50_000;
/// 구조를 읽어 주는 파일의 크기.
const MAX_OUTLINE_BYTES: u64 = 4 << 20;
/// 폴더 하나에서 보여 주는 항목 수.
const MAX_ENTRIES: usize = 3000;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 프로젝트 폴더로 쓸 수 있는 경로인지(전체 경로이고 폴더여야 한다).
fn root_of(root: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(root);
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!("폴더가 없습니다: {root}"));
    }
    Ok(path)
}

/// 프로젝트 나무에서 흐리게 보이고, git 없이 돌아다닐 때 들어가지 않는 폴더: 숨김 폴더와 의존성·빌드 폴더.
fn is_plumbing(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build" | "out" | "__pycache__" | "vendor" | "obj" | "bin")
}

/// 폴더 안의 항목 하나.
#[derive(Serialize)]
pub struct Entry {
    name: String,
    dir: bool,
    /// 숨김·빌드 폴더(흐리게 보여 준다).
    dim: bool,
}

/// 폴더 안의 폴더와 파일(폴더가 먼저, 이름순).
#[tauri::command]
pub async fn code_list(dir: String) -> Result<Vec<Entry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = root_of(&dir)?;
        let mut entries: Vec<Entry> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let kind = entry.file_type().ok()?;
                // 바로가기(심볼릭 링크)는 가리키는 것이 폴더인지 본다.
                let is_dir = if kind.is_symlink() { entry.path().is_dir() } else { kind.is_dir() };
                Some(Entry { dim: is_dir && is_plumbing(&name) || name == ".git", name, dir: is_dir })
            })
            .take(MAX_ENTRIES)
            .collect();
        entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(entries)
    })
    .await
    .map_err(err)?
}

/// git이 아는 파일(추적하는 것과, 무시하지 않는 새 파일)을 프로젝트 폴더 기준 경로로. git 저장소가 아니면 None.
fn git_files(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=false", "ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.split('\0').filter(|path| !path.is_empty()).map(str::to_string).collect())
}

/// git 없이 돌아다니며 찾은 파일(프로젝트 폴더 기준 경로, `/`로 구분). 숨김·빌드 폴더에는 들어가지 않는다.
fn walked_files(root: &Path) -> Vec<String> {
    walked(root, false)
}

/// `only_code`이면 코드·글자 파일만 모은다(프로젝트를 훑을 때).
fn walked(root: &Path, only_code: bool) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new(), 0u32)];
    while let Some((dir, prefix, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            let relative = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if kind.is_dir() {
                if !is_plumbing(&name) && depth < 24 {
                    pending.push((entry.path(), relative, depth + 1));
                }
            } else if kind.is_file() {
                if only_code && code::lang_of(&name).is_none() {
                    continue;
                }
                found.push(relative);
                if found.len() >= MAX_FILES {
                    return found;
                }
            }
        }
    }
    found
}

/// 프로젝트의 파일들(프로젝트 폴더 기준 경로, `/`로 구분, 이름순).
fn project_files(root: &Path) -> Vec<String> {
    // git 저장소가 아니거나, git이 아는 파일이 하나도 없으면(무시하는 폴더를 프로젝트로 열었다) 직접 돌아다닌다.
    let mut files = git_files(root).filter(|files| !files.is_empty()).unwrap_or_else(|| walked_files(root));
    files.truncate(MAX_FILES);
    files.sort_by_key(|path| path.to_lowercase());
    files
}

/// 프로젝트의 파일 목록(파일 찾기에 쓴다).
#[tauri::command]
pub async fn code_files(root: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(project_files(&root_of(&root)?))).await.map_err(err)?
}

/// 훑을 파일을 읽는다: 코드·글자 파일이고 너무 크지 않을 때만. (언어, 글)
fn read_source(root: &Path, relative: &str) -> Option<(Lang, String)> {
    let lang = code::lang_of(relative)?;
    let path = root.join(relative);
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SCAN_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    Some((lang, textfile::decode(&bytes).ok()?.text))
}

/// 프로젝트의 파일을 여러 스레드로 나눠 훑는다. `visit`가 파일마다 결과를 내놓고, 파일 이름순으로 모아 돌려준다.
/// 모은 결과가 `limit`개를 넘으면 더 훑지 않는다(아주 흔한 이름을 찾을 때 결과가 끝없이 쌓이지 않게).
fn scan<T: Send>(root: &Path, limit: usize, visit: impl Fn(&str, Lang, &str) -> Vec<T> + Sync) -> Vec<T> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let total = AtomicUsize::new(0);
    let total = &total;
    // git 저장소가 아니면(또는 git이 아는 파일이 없으면) 코드·글자 파일만 골라 돌아다닌다.
    let mut files = git_files(root).filter(|files| !files.is_empty()).unwrap_or_else(|| walked(root, true));
    files.truncate(MAX_FILES);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let chunk = files.len().div_ceil(workers).max(1);
    let visit = &visit;
    std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    let mut found = Vec::new();
                    for relative in part {
                        if total.load(Ordering::Relaxed) > limit {
                            break;
                        }
                        if let Some((lang, text)) = read_source(root, relative) {
                            let before = found.len();
                            found.extend(visit(relative, lang, &text));
                            total.fetch_add(found.len() - before, Ordering::Relaxed);
                        }
                    }
                    found
                })
            })
            .collect();
        handles.into_iter().flat_map(|handle| handle.join().unwrap_or_default()).collect()
    })
}

/// 프로젝트에서 찾은 정의 하나.
#[derive(Serialize)]
pub struct Found {
    /// 프로젝트 폴더 기준 경로(`/`로 구분).
    file: String,
    #[serde(flatten)]
    def: Def,
    /// 그 줄의 글.
    text: String,
}

fn line_of(text: &str, line: u32) -> String {
    let shown: String = text.split('\n').nth(line.saturating_sub(1) as usize).unwrap_or("").trim().chars().take(200).collect();
    shown
}

/// 파일 하나의 정의들(구조 보기).
#[tauri::command]
pub async fn code_outline(path: String) -> Result<Vec<Def>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = Path::new(&path);
        if !file.is_absolute() {
            return Err(format!("전체 경로가 아닙니다: {path}"));
        }
        let Some(lang) = code::lang_of(&path).filter(|lang| *lang != Lang::Text) else { return Ok(Vec::new()) };
        // 아주 큰 파일(기록, 자료)은 통째로 읽지 않는다.
        if std::fs::metadata(file).map_or(true, |meta| !meta.is_file() || meta.len() > MAX_OUTLINE_BYTES) {
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(file).map_err(|e| format!("{path}: {e}"))?;
        Ok(code::definitions(lang, &textfile::decode(&bytes)?.text))
    })
    .await
    .map_err(err)?
}

/// 그 이름의 정의를 프로젝트 전체에서 찾는다(정의로 가기).
#[tauri::command]
pub async fn code_defs(root: String, name: String) -> Result<Vec<Found>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        if name.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut found = scan(&root, MAX_SYMBOLS * 4, |file, lang, text| {
            if !text.contains(name.as_str()) {
                return Vec::new();
            }
            code::definitions(lang, text)
                .into_iter()
                .filter(|def| def.name == name)
                .map(|def| Found { file: file.to_string(), text: line_of(text, def.line), def })
                .collect()
        });
        found.truncate(MAX_SYMBOLS);
        Ok(found)
    })
    .await
    .map_err(err)?
}

/// 찾은 자리들: 파일마다 묶지 않고 한 줄에 하나씩 돌려준다(화면이 파일별로 묶는다).
#[derive(Serialize)]
pub struct FoundHit {
    file: String,
    #[serde(flatten)]
    hit: Hit,
}

#[derive(Serialize)]
pub struct Hits {
    hits: Vec<FoundHit>,
    /// 결과가 많아 잘랐는지.
    truncated: bool,
}

fn capped(mut hits: Vec<FoundHit>) -> Hits {
    let truncated = hits.len() > MAX_HITS;
    hits.truncate(MAX_HITS);
    Hits { hits, truncated }
}

/// 그 이름이 쓰인 곳을 프로젝트 전체에서 찾는다(쓰는 곳 찾기). 정의·쓰는 곳·주석·문자열을 가려 준다.
#[tauri::command]
pub async fn code_usages(root: String, name: String) -> Result<Hits, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        if name.trim().is_empty() {
            return Ok(capped(Vec::new()));
        }
        Ok(capped(scan(&root, MAX_HITS, |file, lang, text| {
            code::occurrences(lang, text, &name).into_iter().map(|hit| FoundHit { file: file.to_string(), hit }).collect()
        })))
    })
    .await
    .map_err(err)?
}

/// 글을 프로젝트 전체에서 찾는다. `regex`가 아니면 적은 글자 그대로 찾는다.
#[tauri::command]
pub async fn code_search(root: String, query: String, case: bool, word: bool, regex: bool) -> Result<Hits, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        if query.is_empty() {
            return Ok(capped(Vec::new()));
        }
        let pattern = code::search_pattern(&query, case, word, regex)?;
        Ok(capped(scan(&root, MAX_HITS, |file, _, text| {
            code::search(text, &pattern, 200).into_iter().map(|hit| FoundHit { file: file.to_string(), hit }).collect()
        })))
    })
    .await
    .map_err(err)?
}

/// 이름이 적은 글과 맞는 정의를 프로젝트 전체에서 찾는다(기호 찾기). 잘 맞는 것이 앞에 온다.
#[tauri::command]
pub async fn code_symbols(root: String, query: String) -> Result<Vec<Found>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        let query = query.trim().to_string();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut ranked: Vec<(u32, Found)> = scan(&root, MAX_RANKED, |file, lang, text| {
            code::definitions(lang, text)
                .into_iter()
                // 제목·구역·선택자는 기호로 치지 않는다(문서와 스타일의 것이 코드의 기호를 묻어 버린다).
                .filter(|def| !matches!(def.kind, "heading" | "section"))
                .filter_map(|def| Some((code::rank(&query, &def.name)?, Found { file: file.to_string(), text: line_of(text, def.line), def })))
                .collect()
        });
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.def.name.len().cmp(&b.1.def.name.len())).then_with(|| a.1.file.cmp(&b.1.file)));
        ranked.truncate(MAX_SYMBOLS);
        Ok(ranked.into_iter().map(|(_, found)| found).collect())
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시험 폴더(빌드 폴더 아래). 끝나면 지운다.
    fn project(name: &str) -> PathBuf {
        let dir = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src").join("model")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules").join("lib")).unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "mod model;\nfn main() {\n    let user = model::load_user(1);\n    println!(\"{}\", user.name);\n}\n").unwrap();
        std::fs::write(
            dir.join("src").join("model").join("user.rs"),
            "pub struct User {\n    pub name: String,\n}\n\n/// load_user는 사용자를 읽는다.\npub fn load_user(id: u32) -> User {\n    User { name: id.to_string() }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("한글 파일.js"), "export function loadUser() { return 1; }\nconst name = 'load_user';\n").unwrap();
        std::fs::write(dir.join("node_modules").join("lib").join("index.js"), "function load_user() {}\n").unwrap();
        std::fs::write(dir.join("logo.png"), [0u8, 1, 2]).unwrap();
        dir
    }

    #[test]
    fn 프로젝트에서_정의와_쓰는_곳을_찾는다() {
        let dir = project("cypher-code-test");
        // git 저장소가 아닌 폴더: 숨김·빌드 폴더에는 들어가지 않는다.
        let files = walked_files(&dir);
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(sorted, ["logo.png", "src/main.rs", "src/model/user.rs", "src/한글 파일.js"]);

        let visit_defs = |name: &str| {
            let mut found: Vec<String> = files
                .iter()
                .filter_map(|file| Some((file, read_source(&dir, file)?)))
                .flat_map(|(file, (lang, text))| {
                    code::definitions(lang, &text).into_iter().filter(|def| def.name == name).map(|def| format!("{file}:{}:{}", def.line, def.kind)).collect::<Vec<_>>()
                })
                .collect();
            found.sort();
            found
        };
        assert_eq!(visit_defs("load_user"), ["src/model/user.rs:6:function"]);
        assert_eq!(visit_defs("User"), ["src/model/user.rs:1:struct"]);
        assert_eq!(visit_defs("name"), ["src/model/user.rs:2:field", "src/한글 파일.js:2:constant"]);

        // 그림 같은 이진 파일은 읽지 않는다.
        assert!(read_source(&dir, "logo.png").is_none());
        let mut roles: Vec<String> = files
            .iter()
            .filter_map(|file| Some((file, read_source(&dir, file)?)))
            .flat_map(|(file, (lang, text))| code::occurrences(lang, &text, "load_user").into_iter().map(move |hit| format!("{file}:{}:{}", hit.line, hit.role)))
            .collect();
        roles.sort();
        assert_eq!(roles, ["src/main.rs:3:use", "src/model/user.rs:5:comment", "src/model/user.rs:6:def", "src/한글 파일.js:2:string"]);
        assert_eq!(line_of("a\n  둘째 줄  \nc", 2), "둘째 줄");
        assert_eq!(line_of("a", 9), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 숨김_폴더와_빌드_폴더를_가린다() {
        for name in [".git", ".idea", "node_modules", "target", "dist", "build", "__pycache__"] {
            assert!(is_plumbing(name), "{name}");
        }
        for name in ["src", "docs", "targets", "builder"] {
            assert!(!is_plumbing(name), "{name}");
        }
        assert!(root_of("상대 경로").is_err());
        assert!(root_of(r"C:\이런 폴더는 없다 cypher").is_err());
    }
}
