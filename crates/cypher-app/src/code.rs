//! 코드 보기 항목: 프로젝트 폴더의 파일을 늘어놓고, 정의·쓰는 곳·기호·글을 프로젝트 전체에서 찾는다.
//!
//! 코드 보기가 폴더를 열면 그 아래의 파일을 한 번 훑어 색인(파일마다의 정의와 낱말)을 디스크에 만들어 두고, 정의·쓰는 곳·기호는
//! 색인이 가리키는 파일만 지금 내용으로 다시 읽어 답한다(Claude 세션이 파일을 계속 고쳐도 줄 번호가 맞다). 색인은 바뀐 파일만
//! 다시 읽어 맞추고, 앱을 다시 켜도 그대로 쓴다. 글 찾기와, 색인을 두지 않은 폴더는 물을 때마다 파일을 여러 스레드로 나눠 훑는다. 대상 파일은 git이 아는
//! 파일(추적하는 것과 무시하지 않는 새 파일)이고, git 저장소가 아니면 숨김·빌드 폴더를 빼고 직접 돌아다닌다.
//! 무엇이 정의이고 쓰는 곳인지는 `cypher_core::code`가 가린다.
//! 파일을 고치지는 않는다(보기 전용).

use cypher_core::code::{self, Def, Hit, Lang};
use cypher_core::textfile;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

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
                let dim = is_dir && is_plumbing(&name) || name == ".git";
                // 폴더 하나만 품은 폴더는 그 안의 폴더까지 한 줄로 잇는다(`main/java/com/example`): Java의 패키지 길을 한 칸씩 펼치지 않게.
                let mut name = name;
                if is_dir && !dim && !kind.is_symlink() {
                    let mut path = entry.path();
                    for _ in 0..16 {
                        let Some(only) = only_dir(&path) else { break };
                        path.push(&only);
                        name.push('/');
                        name.push_str(&only);
                    }
                }
                Some(Entry { dim, name, dir: is_dir })
            })
            .take(MAX_ENTRIES)
            .collect();
        entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(entries)
    })
    .await
    .map_err(err)?
}

/// 그 폴더에 든 것이 폴더 하나뿐이면 그 이름(숨김·빌드 폴더와 바로가기는 치지 않는다).
fn only_dir(dir: &Path) -> Option<String> {
    let mut entries = std::fs::read_dir(dir).ok()?;
    let first = entries.next()?.ok()?;
    if entries.next().is_some() {
        return None;
    }
    let name = first.file_name().to_string_lossy().into_owned();
    (first.file_type().ok()?.is_dir() && !is_plumbing(&name)).then_some(name)
}

/// git이 아는 파일(추적하는 것과, 무시하지 않는 새 파일)을 프로젝트 폴더 기준 경로로. git 저장소가 아니면 None.
fn git_files(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        // `-t`는 항목 앞에 종류를 붙인다(`H 경로`, `? 경로`, 작업 폴더에서 지운 것은 `R 경로`가 한 번 더).
        .args(["-c", "core.quotepath=false", "ls-files", "-z", "-t", "--cached", "--others", "--deleted", "--exclude-standard"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(tracked_files(&String::from_utf8_lossy(&output.stdout)))
}

/// `git ls-files -z -t`의 출력에서 지금 있는 파일만 고른다.
/// - 작업 폴더에서 지웠지만 아직 커밋하지 않은 파일(`R`)은 뺀다(열 수 없다).
/// - 추적하지 않는 중첩 저장소는 `폴더/`로 나오므로 뺀다(파일이 아니다).
/// - 충돌 난 파일은 단계마다 한 번씩(세 번까지) 나오므로 하나로 줄인다.
fn tracked_files(text: &str) -> Vec<String> {
    let mut gone: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut files: Vec<&str> = Vec::new();
    for entry in text.split('\0') {
        let Some((tag, path)) = entry.split_once(' ') else { continue };
        if path.is_empty() || path.ends_with('/') {
            continue;
        }
        if tag == "R" {
            gone.insert(path);
        } else {
            files.push(path);
        }
    }
    files.dedup();
    files.into_iter().filter(|path| !gone.contains(path)).map(str::to_string).collect()
}

/// git 없이 돌아다니며 찾은 파일(프로젝트 폴더 기준 경로, `/`로 구분). 숨김·빌드 폴더에는 들어가지 않는다.
fn walked_files(root: &Path) -> Vec<String> {
    walked(root, false)
}

/// `only_code`이면 코드·글자 파일만 모은다(프로젝트를 훑을 때).
fn walked(root: &Path, only_code: bool) -> Vec<String> {
    walked_up_to(root, only_code, MAX_FILES)
}

/// `limit`개까지만 모은다.
fn walked_up_to(root: &Path, only_code: bool, limit: usize) -> Vec<String> {
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
                if found.len() >= limit {
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
    // git 저장소가 아니면(또는 git이 아는 파일이 없으면) 코드·글자 파일만 골라 돌아다닌다.
    let mut files = git_files(root).filter(|files| !files.is_empty()).unwrap_or_else(|| walked(root, true));
    // 코드·글자 파일만 남긴 뒤에 자른다(그림·자료 파일이 많은 저장소에서 뒤쪽 코드 파일이 잘려 나가지 않게).
    files.retain(|path| code::lang_of(path).is_some());
    files.truncate(MAX_FILES);
    scan_files(root, &files, limit, visit)
}

/// 정해 준 파일들만 여러 스레드로 나눠 훑는다(색인이 "이 파일들에만 있다"고 알려 줄 때는 그 파일들만 읽으면 된다).
fn scan_files<T: Send>(root: &Path, files: &[String], limit: usize, visit: impl Fn(&str, Lang, &str) -> Vec<T> + Sync) -> Vec<T> {
    let total = AtomicUsize::new(0);
    let total = &total;
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
    /// 서명: 정의한 줄부터 몸통 앞까지를 한 줄로 이은 것(반환형·인자·상위 자료형을 읽는 데 쓴다). 기호 찾기에서는 비워 둔다.
    sig: String,
    /// 자료형이면 그 상위 자료형들(물려받는 클래스, 구현하는 인터페이스).
    supers: Vec<String>,
}

/// 찾은 정의에 그 줄의 글과 서명을 붙인다.
fn found_of(file: &str, lang: Lang, text: &str, signer: &code::Signer, def: Def) -> Found {
    let sig = signer.of(&def);
    let supers = if matches!(def.kind, "class" | "interface" | "enum" | "struct" | "trait") { code::supertypes(lang, &sig, &def.name) } else { Vec::new() };
    Found { file: file.to_string(), text: line_of(text, def.line), sig, supers, def }
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

/// 그 이름의 정의를 프로젝트 전체에서 찾는다(정의로 가기). `within`을 주면 감싼 이름(클래스, impl의 자료형)이 그 가운데 하나인 것만 내놓는다:
/// 받는 쪽의 자료형을 아는 호출(`orderMapper.insert(…)`)에서 흔한 이름의 정의 수백 개에 묻히지 않게 한다.
#[tauri::command]
pub async fn code_defs(root: String, name: String, within: Option<Vec<String>>, fresh: Option<bool>) -> Result<Vec<Found>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        if name.trim().is_empty() {
            return Ok(Vec::new());
        }
        let visit = |file: &str, lang: Lang, text: &str| -> Vec<Found> {
            if !text.contains(name.as_str()) {
                return Vec::new();
            }
            let defs: Vec<Def> = code::definitions(lang, text)
                .into_iter()
                .filter(|def| def.name == name && within.as_ref().is_none_or(|owners| owners.contains(&def.container)))
                .collect();
            if defs.is_empty() {
                return Vec::new();
            }
            let signer = code::Signer::new(lang, text);
            defs.into_iter().map(|def| found_of(file, lang, text, &signer, def)).collect()
        };
        // 색인이 있으면 그 이름의 정의가 든 파일만 읽는다(그 파일들은 지금 내용으로 다시 읽으니 줄 번호는 늘 맞다).
        let hash = word_hash(name.as_bytes());
        let holders = |index: &Index| index.files.iter().filter(|(_, entry)| entry.names.binary_search(&hash).is_ok()).map(|(file, _)| file.clone()).collect::<Vec<_>>();
        let mut found = match indexed(&root, holders) {
            Some(files) => {
                let mut found = scan_files(&root, &files, MAX_SYMBOLS * 4, visit);
                // 색인에서 못 찾았는데 색인이 좀 묵었으면(방금 만든 파일의 정의일 수 있다) 색인을 새로 맞추고 한 번 더 찾는다.
                // (`fresh`가 거짓이거나 감싼 이름으로 거른 물음이면 하지 않는다: 정의로 가기는 원래 없는 것 — 라이브러리의 자료형,
                // 그 자료형에 없는 멤버 — 도 여러 번 묻는데, 그때마다 프로젝트를 다시 훑으면 안 된다.)
                if found.is_empty() && fresh.unwrap_or(true) && within.is_none() && index_is_older(&root, Duration::from_secs(3)) {
                    refresh_index(&root, None);
                    // (맞추다 디스크에 쓰지 못해 색인이 내려갔으면 파일을 모두 읽어 찾는다.)
                    found = match indexed(&root, holders) {
                        Some(files) => scan_files(&root, &files, MAX_SYMBOLS * 4, visit),
                        None => scan(&root, MAX_SYMBOLS * 4, visit),
                    };
                }
                found
            }
            None => scan(&root, MAX_SYMBOLS * 4, visit),
        };
        found.truncate(MAX_SYMBOLS);
        Ok(found)
    })
    .await
    .map_err(err)?
}

/// 그 자료형을 물려받거나 구현하는 것들을 프로젝트 전체에서 찾는다(구현으로 가기): 클래스·인터페이스·트레이트의 정의와, Rust의
/// `impl 트레이트 for 자료형`(종류가 "impl", 이름이 구현하는 자료형).
#[tauri::command]
pub async fn code_subtypes(root: String, name: String) -> Result<Vec<Found>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        if name.trim().is_empty() {
            return Ok(Vec::new());
        }
        let visit = |file: &str, lang: Lang, text: &str| -> Vec<Found> {
            code::subtypes(lang, text, &name)
                .into_iter()
                .map(|(def, sig)| Found { file: file.to_string(), text: line_of(text, def.line), supers: code::supertypes(lang, &sig, &def.name), sig, def })
                .collect()
        };
        // 색인이 있으면 그 이름이 든 파일만 읽는다.
        let holders = simple_name(&name)
            .then(|| word_hash(name.as_bytes()))
            .and_then(|hash| indexed(&root, |index| word_holders(index, hash)).flatten());
        let mut found = match holders {
            Some(files) => scan_files(&root, &files, MAX_SYMBOLS * 4, visit),
            None => scan(&root, MAX_SYMBOLS * 4, visit),
        };
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
        let visit = |file: &str, lang: Lang, text: &str| -> Vec<FoundHit> {
            code::occurrences(lang, text, &name).into_iter().map(|hit| FoundHit { file: file.to_string(), hit }).collect()
        };
        // 색인이 있고 이름이 영문·숫자·`_`·`$`뿐이면 그 낱말이 든 파일만 읽는다. 그 밖의 이름(한글, `-`가 든 이름)은 모두 훑는다.
        let holders = simple_name(&name)
            .then(|| word_hash(name.as_bytes()))
            .and_then(|hash| indexed(&root, |index| word_holders(index, hash)).flatten());
        Ok(capped(match holders {
            Some(files) => scan_files(&root, &files, MAX_HITS, visit),
            None => scan(&root, MAX_HITS, visit),
        }))
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
        // 색인이 있으면 파일을 읽지 않고 색인의 정의에서 고른다. 그 줄의 글만 고른 것들의 파일에서 읽는다.
        let picked = indexed(&root, |index| ranked_symbols(index, &query)).flatten();
        if let Some(picked) = picked {
            // 고른 파일이 색인에 적은 뒤로 바뀌었으면(고친 때나 크기가 다르다) 그 파일의 정의를 지금 내용에서 다시 읽는다: 줄이 밀렸거나
            // 이름이 없어졌을 수 있다. 그런 파일은 고른 이름들의 지금 정의로 바꿔 넣는다.
            let mut wanted: HashMap<String, Vec<String>> = HashMap::new();
            for (file, _, def) in &picked {
                wanted.entry(file.clone()).or_default().push(def.name.clone());
            }
            // 파일 → (지금의 글, 바뀌었으면 지금의 정의들)
            let mut sources: HashMap<String, (Option<String>, Option<Vec<Def>>)> = HashMap::new();
            let mut found = Vec::with_capacity(picked.len());
            for (file, stamp, def) in picked {
                let first = !sources.contains_key(&file);
                let (text, fresh) = sources.entry(file.clone()).or_insert_with(|| {
                    let moved = std::fs::metadata(root.join(&file)).map_or(true, |meta| (modified_of(&meta), meta.len()) != stamp);
                    match read_source(&root, &file) {
                        Some((lang, text)) => {
                            let defs = moved.then(|| code::definitions(lang, &text));
                            (Some(text), defs)
                        }
                        None => (None, moved.then(Vec::new)),
                    }
                });
                let text = text.as_deref().unwrap_or_default();
                match fresh {
                    None => found.push(Found { text: line_of(text, def.line), sig: String::new(), supers: Vec::new(), file, def }),
                    Some(defs) if first => {
                        let names = &wanted[&file];
                        found.extend(
                            defs.iter()
                                .filter(|def| !matches!(def.kind, "heading" | "section") && names.contains(&def.name))
                                .map(|def| Found { file: file.clone(), text: line_of(text, def.line), sig: String::new(), supers: Vec::new(), def: def.clone() }),
                        );
                    }
                    Some(_) => {}
                }
            }
            found.truncate(MAX_SYMBOLS);
            return Ok(found);
        }
        let mut ranked: Vec<(u32, Found)> = scan(&root, MAX_RANKED, |file, lang, text| {
            code::definitions(lang, text)
                .into_iter()
                // 제목·구역·선택자는 기호로 치지 않는다(문서와 스타일의 것이 코드의 기호를 묻어 버린다).
                .filter(|def| !matches!(def.kind, "heading" | "section"))
                .filter_map(|def| Some((code::rank(&query, &def.name)?, Found { file: file.to_string(), text: line_of(text, def.line), sig: String::new(), supers: Vec::new(), def })))
                .collect()
        });
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.def.name.len().cmp(&b.1.def.name.len())).then_with(|| a.1.file.cmp(&b.1.file)));
        ranked.truncate(MAX_SYMBOLS);
        Ok(ranked.into_iter().map(|(_, found)| found).collect())
    })
    .await
    .map_err(err)?
}

// ───────────────────────── 색인 ─────────────────────────
//
// 정의로 가기·쓰는 곳 찾기·기호 찾기를 할 때마다 프로젝트의 파일을 모두 읽으면, 파일이 수천 개인 프로젝트에서는 한 번에 몇 초씩 걸린다.
// 코드 보기가 프로젝트 폴더를 열 때 한 번 훑어 파일마다 "정의들"과 "들어 있는 낱말들(해시)"을 적어 두고(색인), 찾을 때는 색인이
// 가리키는 파일만 지금 내용으로 다시 읽는다.
//
// 색인은 디스크에 둔다(`%LOCALAPPDATA%\cypher\code-index\<폴더의 해시>\`). 앱을 다시 켜도 다시 만들지 않고, 아주 큰 프로젝트에서도
// 메모리를 많이 쓰지 않는다. 파일은 셋이다:
// - `blocks.bin`: 파일마다의 정의들. 뒤에 덧붙이기만 한다(바뀐 파일은 새 덩이를 덧붙이고 옛 덩이는 버려진 채 둔다).
// - `words.bin`: 파일마다의 낱말 해시들(정렬). 역시 덧붙이기만 한다.
// - `table.bin`: 파일 목록 — 경로, 고친 때, 크기, 두 파일에서의 자리, 정의 이름의 해시들. 메모리에는 이것만 올린다.
// 버려진 덩이가 많아지면 두 파일을 새로 써서 줄인다. 색인이 깨졌거나 판이 다르면 버리고 다시 만든다.

/// 색인하는 파일 수의 한도(프로젝트를 통째로 훑는 찾기보다 훨씬 크다: 색인은 파일마다 메모리를 조금만 쓴다).
const INDEX_MAX_FILES: usize = 300_000;
/// 파일 목록(table.bin)을 디스크에 다시 적는 가장 짧은 간격. 그사이에 바뀐 것은 다음에 켤 때 다시 읽으면 된다.
const TABLE_SAVE_EVERY: Duration = Duration::from_secs(30);
/// 버려진 덩이가 이만큼을 넘고 살아 있는 것보다 많으면 두 파일을 새로 써서 줄인다.
const COMPACT_OVER: u64 = 16 << 20;
/// 쓰지 않은 지 이만큼 된 색인 폴더는 지운다(열어 보지 않는 프로젝트의 색인이 쌓이지 않게).
const INDEX_KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
const TABLE_MAGIC: &[u8; 8] = b"CYIDX\0\x04\0";
/// 정의 하나의 기록 크기(blocks.bin).
const ITEM_BYTES: usize = 24;

/// 색인에 적어 둔 파일 하나(메모리와 table.bin에 있는 것).
#[derive(Clone)]
struct FileEntry {
    /// 고친 때(1970년부터의 나노초, 모르면 0)와 크기. 같으면 다시 읽지 않는다.
    modified: u64,
    size: u64,
    /// blocks.bin에서의 자리와 길이, 정의의 수.
    defs_at: u64,
    defs_len: u32,
    def_count: u32,
    /// words.bin에서의 자리와 낱말의 수.
    words_at: u64,
    word_count: u32,
    /// 이 파일에 든 정의 이름들의 해시(정렬, 겹친 것 없음). 정의로 가기는 이것만으로 파일을 고른다.
    names: Box<[u32]>,
}

struct Index {
    /// 색인 파일들을 둔 폴더.
    dir: PathBuf,
    /// 프로젝트 폴더 기준 경로(`/`로 구분) → 그 파일.
    files: BTreeMap<String, FileEntry>,
    /// 마지막으로 파일들과 맞춰 본 때.
    refreshed: Option<Instant>,
    /// table.bin에 적은 뒤로 바뀐 것이 있는지와, 마지막으로 적은 때.
    dirty: bool,
    saved: Option<Instant>,
    /// 아직 다 만들지 못한 색인인지(만드는 중에 화면이 닫혔다). 그런 색인은 디스크에만 두고 찾는 데 쓰지 않는다: 다음에 이어서 만든다.
    partial: bool,
}

/// 프로젝트 폴더 하나의 색인과 그것을 만드는 중의 상태.
#[derive(Default)]
struct Slot {
    data: RwLock<Option<Index>>,
    /// 색인을 만들거나 맞추는 일은 한 번에 하나만 한다(두 파일에 덧붙이는 것도 이 잠금 안에서만 한다).
    working: Mutex<()>,
    busy: AtomicBool,
    done: AtomicUsize,
    total: AtomicUsize,
    /// 이 색인을 쓰는 코드 보기의 수. 0이 되면 메모리에서 내린다(디스크의 것은 남는다).
    users: AtomicUsize,
    /// 쓰는 화면이 없어졌다: 돌고 있는 "모두 맞추기"는 읽은 데까지만 적고 그만둔다(다시 열리면 꺼진다).
    cancel: AtomicBool,
    /// 마지막으로 본 "색인을 쓸 수 있는지"(맞추기가 쓰기 잠금을 쥔 동안에는 이것으로 답한다).
    ready: AtomicBool,
}

fn slots() -> &'static Mutex<HashMap<String, Arc<Slot>>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, Arc<Slot>>>> = OnceLock::new();
    SLOTS.get_or_init(Default::default)
}

/// 프로젝트 폴더를 가리키는 값(대소문자와 구분자, 끝의 구분자를 가리지 않는다).
fn slot_key(root: &Path) -> String {
    root.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn slot_of(root: &Path) -> Option<Arc<Slot>> {
    slots().lock().unwrap_or_else(|e| e.into_inner()).get(&slot_key(root)).cloned()
}

/// 색인 파일들을 두는 맨 위 폴더(`%LOCALAPPDATA%\cypher\code-index`).
fn index_home() -> Option<PathBuf> {
    // 시험은 사용자의 데이터 폴더를 건드리지 않게 빌드 폴더 아래에 둔다.
    #[cfg(test)]
    return Some(std::env::current_exe().ok()?.parent()?.join("test-files").join("code-index-store"));
    #[cfg(not(test))]
    Some(crate::data_dir()?.join("code-index"))
}

/// 그 프로젝트 폴더의 색인을 두는 폴더(경로의 해시로 이름을 짓는다. 해시는 판이 바뀌어도 같다).
fn index_dir(root: &Path) -> Option<PathBuf> {
    let hash = slot_key(root).bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3));
    Some(index_home()?.join(format!("{hash:016x}")))
}

/// 색인이 있으면 그것으로 `pick`을 부른다. 처음 만드는 중이면 다 만들 때까지 기다린다. 색인을 두지 않은 폴더면 None.
/// `pick`이 도는 동안 색인 파일은 바뀌지 않는다(덧붙기만 한다): 그 안에서 색인 파일을 읽어도 된다.
fn indexed<T>(root: &Path, pick: impl FnOnce(&Index) -> T) -> Option<T> {
    let slot = slot_of(root)?;
    if slot.data.read().unwrap_or_else(|e| e.into_inner()).is_none() {
        drop(slot.working.lock().unwrap_or_else(|e| e.into_inner()));
    }
    let guard = slot.data.read().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().map(pick)
}

/// 색인을 마지막으로 맞춘 지 `age`가 넘었는지.
fn index_is_older(root: &Path, age: Duration) -> bool {
    indexed(root, |index| index.refreshed.is_none_or(|at| at.elapsed() > age)).unwrap_or(false)
}

/// 낱말의 해시(FNV-1a).
fn word_hash(word: &[u8]) -> u32 {
    word.iter().fold(0x811c_9dc5u32, |hash, byte| (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193))
}

/// 낱말을 이루는 글자: 영문·숫자·`_`·`$`. 쓰는 곳 찾기(`code::occurrences`)가 영문 이름의 경계로 보는 것과 같다.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// 이름이 낱말 글자로만 되어 있는지(그런 이름만 낱말 색인으로 찾을 수 있다).
fn simple_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(is_word_byte)
}

/// 글에 든 낱말들의 해시(정렬, 겹친 것 없음).
fn words_of(text: &str) -> Vec<u32> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut start = None;
    for (at, byte) in bytes.iter().enumerate() {
        match (is_word_byte(*byte), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                found.push(word_hash(&bytes[from..at]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        found.push(word_hash(&bytes[from..]));
    }
    found.sort_unstable();
    found.dedup();
    found
}

// ── 덩이: 파일 하나의 정의들을 촘촘히 적은 것 ──
//
// [종류 이름의 수 u8] [종류 이름들: 길이 u8 + 글] [글의 길이 u32] [정의의 수 u32] [글: 이름과 감싼 것의 이름을 이어 붙인 것]
// [정의마다 24바이트: 이름의 자리 u32, 감싼 것의 자리 u32, 줄 u32, 칸 u32, 이름의 길이 u16, 감싼 것의 길이 u16, 종류 u8, 빈칸 3]
// 정의마다 문자열을 따로 두지 않는다(파일이 수만 개인 프로젝트에서 수백 MB가 된다). 종류는 덩이 안의 순번으로 적는다.

/// 정의들을 덩이로 적는다. (덩이, 정의의 수, 이름들의 해시)
fn encode_defs<'a>(defs: &'a [Def]) -> (Vec<u8>, u32, Box<[u32]>) {
    let mut kinds: Vec<&str> = Vec::new();
    let mut text = String::new();
    let mut placed: HashMap<&'a str, u32> = HashMap::new();
    let mut items: Vec<u8> = Vec::with_capacity(defs.len() * ITEM_BYTES);
    let mut names: Vec<u32> = Vec::with_capacity(defs.len());
    let mut count = 0u32;
    for def in defs {
        // 이름이 아주 길거나 정의가 너무 많은 파일(만든 코드)은 담을 수 있는 데까지만 담는다.
        if def.name.len() > usize::from(u16::MAX) || def.container.len() > usize::from(u16::MAX) || text.len() > (u32::MAX / 2) as usize || def.kind.len() > 255 {
            continue;
        }
        let kind = match kinds.iter().position(|kind| *kind == def.kind) {
            Some(at) => at,
            None if kinds.len() < 255 => {
                kinds.push(def.kind);
                kinds.len() - 1
            }
            None => continue,
        } as u8;
        // 같은 글(클래스 이름 등)은 한 번만 적는다.
        let mut place = |word: &'a str| -> u32 {
            *placed.entry(word).or_insert_with(|| {
                let at = text.len() as u32;
                text.push_str(word);
                at
            })
        };
        let name_at = place(def.name.as_str());
        let container_at = place(def.container.as_str());
        items.extend_from_slice(&name_at.to_le_bytes());
        items.extend_from_slice(&container_at.to_le_bytes());
        items.extend_from_slice(&def.line.to_le_bytes());
        items.extend_from_slice(&def.col.to_le_bytes());
        items.extend_from_slice(&(def.name.len() as u16).to_le_bytes());
        items.extend_from_slice(&(def.container.len() as u16).to_le_bytes());
        items.extend_from_slice(&[kind, 0, 0, 0]);
        names.push(word_hash(def.name.as_bytes()));
        count += 1;
    }
    let mut block = Vec::with_capacity(16 + kinds.len() * 12 + text.len() + items.len());
    block.push(kinds.len() as u8);
    for kind in &kinds {
        block.push(kind.len() as u8);
        block.extend_from_slice(kind.as_bytes());
    }
    block.extend_from_slice(&(text.len() as u32).to_le_bytes());
    block.extend_from_slice(&count.to_le_bytes());
    block.extend_from_slice(text.as_bytes());
    block.extend_from_slice(&items);
    names.sort_unstable();
    names.dedup();
    (block, count, names.into_boxed_slice())
}

/// 덩이를 읽은 것: 종류 이름들, 글, 정의 기록들.
struct Block<'a> {
    kinds: Vec<&'a str>,
    text: &'a str,
    items: &'a [u8],
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

impl<'a> Block<'a> {
    /// 덩이를 읽는다. 모양이 맞지 않으면(깨진 색인) None.
    fn read(bytes: &'a [u8]) -> Option<Self> {
        let mut at = 1;
        let mut kinds = Vec::new();
        for _ in 0..*bytes.first()? {
            let len = usize::from(*bytes.get(at)?);
            kinds.push(std::str::from_utf8(bytes.get(at + 1..at + 1 + len)?).ok()?);
            at += 1 + len;
        }
        let head = bytes.get(at..at + 8)?;
        let (text_len, count) = (le_u32(head, 0) as usize, le_u32(head, 4) as usize);
        let text = std::str::from_utf8(bytes.get(at + 8..at + 8 + text_len)?).ok()?;
        let items = bytes.get(at + 8 + text_len..at + 8 + text_len + count.checked_mul(ITEM_BYTES)?)?;
        Some(Block { kinds, text, items })
    }

    fn items(&self) -> impl Iterator<Item = &'a [u8]> {
        self.items.as_chunks::<ITEM_BYTES>().0.iter().map(|item| item.as_slice())
    }

    fn name(&self, item: &[u8]) -> &'a str {
        let (at, len) = (le_u32(item, 0) as usize, usize::from(u16::from_le_bytes([item[16], item[17]])));
        self.text.get(at..at + len).unwrap_or("")
    }

    fn kind(&self, item: &[u8]) -> &'a str {
        self.kinds.get(usize::from(item[20])).copied().unwrap_or("")
    }

    /// 화면에 내보낼 정의로 되돌린다(몸통의 끝과 깊이는 담지 않았다).
    fn def(&self, item: &[u8]) -> Def {
        let (at, len) = (le_u32(item, 4) as usize, usize::from(u16::from_le_bytes([item[18], item[19]])));
        let line = le_u32(item, 8);
        Def { name: self.name(item).to_string(), kind: static_kind(self.kind(item)), line, col: le_u32(item, 12), container: self.text.get(at..at + len).unwrap_or("").to_string(), depth: 0, end: line }
    }
}

/// 덩이에서 읽은 종류 이름을 프로그램이 끝날 때까지 사는 글로 바꾼다(`Def::kind`가 그런 글이다). 종류는 몇 가지뿐이라 한 번씩만 남긴다.
fn static_kind(kind: &str) -> &'static str {
    static KINDS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    let mut kinds = KINDS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = kinds.iter().find(|known| **known == kind) {
        return found;
    }
    if kinds.len() >= 255 {
        return "";
    }
    let kept: &'static str = Box::leak(kind.to_string().into_boxed_str());
    kinds.push(kept);
    kept
}

// ── table.bin: 파일 목록 ──

/// 파일 목록을 디스크에 적는다(임시 파일에 쓴 뒤 이름을 바꿔, 쓰다 만 파일이 남지 않게 한다).
fn save_table(index: &Index, root: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let key = slot_key(root);
    let mut out = std::io::BufWriter::new(std::fs::File::create(index.dir.join("table.tmp"))?);
    out.write_all(TABLE_MAGIC)?;
    out.write_all(&u32::from(index.partial).to_le_bytes())?;
    out.write_all(&(key.len() as u32).to_le_bytes())?;
    out.write_all(key.as_bytes())?;
    out.write_all(&(index.files.len() as u32).to_le_bytes())?;
    for (path, entry) in &index.files {
        out.write_all(&(path.len() as u32).to_le_bytes())?;
        out.write_all(path.as_bytes())?;
        out.write_all(&entry.modified.to_le_bytes())?;
        out.write_all(&entry.size.to_le_bytes())?;
        out.write_all(&entry.defs_at.to_le_bytes())?;
        out.write_all(&entry.defs_len.to_le_bytes())?;
        out.write_all(&entry.def_count.to_le_bytes())?;
        out.write_all(&entry.words_at.to_le_bytes())?;
        out.write_all(&entry.word_count.to_le_bytes())?;
        out.write_all(&(entry.names.len() as u32).to_le_bytes())?;
        for name in entry.names.iter() {
            out.write_all(&name.to_le_bytes())?;
        }
    }
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    std::fs::rename(index.dir.join("table.tmp"), index.dir.join("table.bin"))
}

/// 디스크의 파일 목록을 읽는다. 없거나, 다른 폴더의 것이거나, 깨졌거나, 가리키는 자리가 두 파일의 끝을 넘으면 None(다시 만든다).
/// 함께 돌려주는 값은 "다 만들지 못한 색인"인지다.
fn load_table(dir: &Path, root: &Path) -> Option<(BTreeMap<String, FileEntry>, bool)> {
    let bytes = std::fs::read(dir.join("table.bin")).ok()?;
    let (blocks_len, words_len) = (std::fs::metadata(dir.join("blocks.bin")).ok()?.len(), std::fs::metadata(dir.join("words.bin")).ok()?.len());
    /// 앞에서부터 `len`바이트를 떼어 준다. 모자라면 None.
    fn take<'a>(bytes: &'a [u8], at: &mut usize, len: usize) -> Option<&'a [u8]> {
        let part = bytes.get(*at..at.checked_add(len)?)?;
        *at += len;
        Some(part)
    }
    let (bytes, at) = (bytes.as_slice(), &mut 0usize);
    if take(bytes, at, 8)? != TABLE_MAGIC {
        return None;
    }
    let partial = le_u32(take(bytes, at, 4)?, 0) != 0;
    let key_len = le_u32(take(bytes, at, 4)?, 0) as usize;
    if take(bytes, at, key_len)? != slot_key(root).as_bytes() {
        return None;
    }
    let count = le_u32(take(bytes, at, 4)?, 0) as usize;
    let mut files = BTreeMap::new();
    for _ in 0..count {
        let path_len = le_u32(take(bytes, at, 4)?, 0) as usize;
        let path = std::str::from_utf8(take(bytes, at, path_len)?).ok()?.to_string();
        // 고친 때 8, 크기 8, 정의의 자리 8, 정의의 길이 4, 정의의 수 4, 낱말의 자리 8, 낱말의 수 4, 이름 해시의 수 4 (save_table과 같은 차례)
        let fixed = take(bytes, at, 48)?;
        let wide = |from: usize| u64::from_le_bytes(fixed[from..from + 8].try_into().unwrap_or_default());
        let name_count = le_u32(fixed, 44) as usize;
        let names: Box<[u32]> = take(bytes, at, name_count.checked_mul(4)?)?.as_chunks::<4>().0.iter().map(|part| u32::from_le_bytes(*part)).collect();
        let entry = FileEntry { modified: wide(0), size: wide(8), defs_at: wide(16), defs_len: le_u32(fixed, 24), def_count: le_u32(fixed, 28), words_at: wide(32), word_count: le_u32(fixed, 40), names };
        if entry.defs_at.checked_add(u64::from(entry.defs_len))? > blocks_len || entry.words_at.checked_add(u64::from(entry.word_count) * 4)? > words_len {
            return None;
        }
        files.insert(path, entry);
    }
    Some((files, partial))
}

/// 오래 쓰지 않은 색인 폴더를 지운다(색인을 열 때 한 번씩).
fn prune_indexes(keep: &Path) {
    let Some(home) = index_home() else { return };
    let Ok(entries) = std::fs::read_dir(&home) else { return };
    // 지금 열려 있는 색인(만드는 중이라 파일 목록이 아직 없는 것도)은 지우지 않는다.
    let open: Vec<PathBuf> = slots().lock().unwrap_or_else(|e| e.into_inner()).keys().filter_map(|key| index_dir(Path::new(key))).collect();
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || open.contains(&path) || !path.is_dir() {
            continue;
        }
        // 그 색인을 마지막으로 쓴 때: 파일 목록이나 덩이를 적은 때(만들다 만 색인에는 목록이 없다). 알 수 없으면 그대로 둔다.
        let written = |name: &str| std::fs::metadata(path.join(name)).and_then(|meta| meta.modified()).ok();
        let used = [written("table.bin"), written("blocks.bin")].into_iter().flatten().max().or_else(|| written(""));
        if used.and_then(|at| SystemTime::now().duration_since(at).ok()).is_some_and(|age| age > INDEX_KEEP) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// 파일의 "고친 때"(1970년부터의 나노초, 모르면 0).
fn modified_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|at| at.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |since| since.as_nanos() as u64)
}

/// 색인을 맞춘 결과.
#[derive(Serialize, Clone, Copy, Default)]
pub struct IndexInfo {
    /// 색인에 든 파일과 정의의 수.
    files: usize,
    defs: usize,
    /// 이번에 새로 읽은 파일 수(처음에는 전부, 그 뒤로는 바뀐 것만).
    parsed: usize,
    /// 디스크에 있던 색인을 읽어 왔는지(앱을 다시 켰을 때).
    cached: bool,
    /// 색인 파일들의 크기(바이트).
    bytes: u64,
    ms: u64,
}

/// 새로 읽은 파일 하나(색인에 넣을 것).
struct Fresh {
    path: String,
    modified: u64,
    size: u64,
    block: Vec<u8>,
    def_count: u32,
    names: Box<[u32]>,
    words: Vec<u32>,
}

/// 파일 하나를 읽어 색인에 넣을 것으로 만든다. 읽지 못하거나 너무 크면 None.
fn read_fresh(root: &Path, relative: &str, known: Option<(u64, u64)>) -> Option<Result<Fresh, ()>> {
    let path = root.join(relative);
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SCAN_BYTES {
        return None;
    }
    let stamp = (modified_of(&meta), meta.len());
    if known == Some(stamp) && stamp.0 != 0 {
        return Some(Err(())); // 그대로다
    }
    let lang = code::lang_of(relative)?;
    let text = textfile::decode(&std::fs::read(&path).ok()?).ok()?.text;
    let (block, def_count, names) = encode_defs(&code::definitions(lang, &text));
    Some(Ok(Fresh { path: relative.to_string(), modified: stamp.0, size: stamp.1, block, def_count, names, words: words_of(&text) }))
}

/// 두 색인 파일의 끝에 덧붙이는 것.
struct Appender {
    blocks: std::io::BufWriter<std::fs::File>,
    words: std::io::BufWriter<std::fs::File>,
    blocks_at: u64,
    words_at: u64,
}

impl Appender {
    fn open(dir: &Path) -> std::io::Result<Self> {
        let open = |name: &str| std::fs::OpenOptions::new().create(true).append(true).open(dir.join(name));
        let (blocks, words) = (open("blocks.bin")?, open("words.bin")?);
        let (blocks_at, words_at) = (blocks.metadata()?.len(), words.metadata()?.len());
        Ok(Appender { blocks: std::io::BufWriter::with_capacity(1 << 20, blocks), words: std::io::BufWriter::with_capacity(1 << 20, words), blocks_at, words_at })
    }

    /// 새로 읽은 파일을 덧붙이고, 색인에 적을 것을 돌려준다.
    fn push(&mut self, fresh: Fresh) -> std::io::Result<(String, FileEntry)> {
        use std::io::Write;
        self.blocks.write_all(&fresh.block)?;
        for word in &fresh.words {
            self.words.write_all(&word.to_le_bytes())?;
        }
        let entry = FileEntry {
            modified: fresh.modified,
            size: fresh.size,
            defs_at: self.blocks_at,
            defs_len: fresh.block.len() as u32,
            def_count: fresh.def_count,
            words_at: self.words_at,
            word_count: fresh.words.len() as u32,
            names: fresh.names,
        };
        self.blocks_at += fresh.block.len() as u64;
        self.words_at += fresh.words.len() as u64 * 4;
        Ok((fresh.path, entry))
    }

    fn finish(mut self) -> std::io::Result<()> {
        use std::io::Write;
        self.blocks.flush()?;
        self.words.flush()
    }
}

/// 색인 파일들의 크기와, 그 가운데 살아 있는(색인이 가리키는) 크기.
fn index_sizes(index: &Index) -> (u64, u64) {
    let on_disk: u64 = ["blocks.bin", "words.bin", "table.bin"].iter().filter_map(|name| std::fs::metadata(index.dir.join(name)).ok()).map(|meta| meta.len()).sum();
    let live: u64 = index.files.values().map(|entry| u64::from(entry.defs_len) + u64::from(entry.word_count) * 4).sum();
    (on_disk, live)
}

/// 버려진 덩이가 많으면 두 파일을 살아 있는 것만으로 새로 쓴다. 색인을 읽는 쪽이 없을 때(쓰기 잠금 안에서) 부른다.
fn compact(index: &mut Index) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut from_blocks = std::fs::File::open(index.dir.join("blocks.bin"))?;
    let mut from_words = std::fs::File::open(index.dir.join("words.bin"))?;
    let mut to_blocks = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(index.dir.join("blocks.tmp"))?);
    let mut to_words = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(index.dir.join("words.tmp"))?);
    let (mut blocks_at, mut words_at) = (0u64, 0u64);
    let mut moved: Vec<(u64, u64)> = Vec::with_capacity(index.files.len());
    let mut buffer = Vec::new();
    for entry in index.files.values() {
        buffer.resize(entry.defs_len as usize, 0);
        from_blocks.seek(SeekFrom::Start(entry.defs_at))?;
        from_blocks.read_exact(&mut buffer)?;
        to_blocks.write_all(&buffer)?;
        buffer.resize(entry.word_count as usize * 4, 0);
        from_words.seek(SeekFrom::Start(entry.words_at))?;
        from_words.read_exact(&mut buffer)?;
        to_words.write_all(&buffer)?;
        moved.push((blocks_at, words_at));
        blocks_at += u64::from(entry.defs_len);
        words_at += u64::from(entry.word_count) * 4;
    }
    to_blocks.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    to_words.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    drop((from_blocks, from_words));
    // 옛 파일 목록이 새 두 파일을 가리키는 일이 없게, 목록을 먼저 지우고 바꾼 뒤 새로 적는다(도중에 멈추면 색인을 다시 만든다).
    let _ = std::fs::remove_file(index.dir.join("table.bin"));
    std::fs::rename(index.dir.join("blocks.tmp"), index.dir.join("blocks.bin"))?;
    std::fs::rename(index.dir.join("words.tmp"), index.dir.join("words.bin"))?;
    for (entry, (blocks_at, words_at)) in index.files.values_mut().zip(moved) {
        entry.defs_at = blocks_at;
        entry.words_at = words_at;
    }
    index.dirty = true;
    Ok(())
}

/// 색인을 지금의 파일들과 맞춘다(색인을 두는 폴더일 때만).
/// `only`가 있으면 그 파일들만 본다(Claude 세션이 방금 쓴 파일). 없으면 파일 목록을 다시 읽어 모두 본다: 고친 때나 크기가 달라진
/// 파일만 다시 읽고, 없어진 파일은 뺀다. 처음에는 디스크에 있던 색인을 읽어 오고(없으면 처음부터 만든다), 끝에 파일 목록을 디스크에 적는다.
fn refresh_index(root: &Path, only: Option<&[String]>) -> Option<IndexInfo> {
    refresh_with(root, only, false)
}

/// `rebuild`면 있던 색인을 지우고 처음부터 만든다(화면의 [재색인]).
/// 디스크에 쓰지 못하면(자리가 없다, 파일이 잠겼다 등) None을 돌려주고 메모리의 색인을 내린다: 찾는 쪽은 파일을 모두 읽는 길로
/// 물러나고, 다음에 맞출 때 디스크의 것부터 다시 읽어 본다.
fn refresh_with(root: &Path, only: Option<&[String]>, rebuild: bool) -> Option<IndexInfo> {
    let slot = slot_of(root)?;
    let _working = slot.working.lock().unwrap_or_else(|e| e.into_inner());
    // 기다리는 사이에 이 자리가 치워졌으면(쓰는 화면이 없어졌다) 하지 않는다: 색인 파일에 덧붙이는 일은 지금 놓여 있는 자리의
    // 잠금을 쥔 쪽만 한다(같은 폴더가 다시 열려 새 자리가 생겼을 수 있다).
    if !slots().lock().unwrap_or_else(|e| e.into_inner()).get(&slot_key(root)).is_some_and(|now| Arc::ptr_eq(now, &slot)) {
        return None;
    }
    let started = Instant::now();
    let dir = index_dir(root)?;
    slot.done.store(0, Ordering::Relaxed);
    slot.total.store(0, Ordering::Relaxed);
    slot.busy.store(true, Ordering::Relaxed);
    let finish = |info: Option<IndexInfo>| {
        if info.is_none() {
            *slot.data.write().unwrap_or_else(|e| e.into_inner()) = None;
        }
        slot.busy.store(false, Ordering::Relaxed);
        info
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return finish(None);
    }
    let wipe = || {
        for name in ["table.bin", "blocks.bin", "words.bin", "table.tmp", "blocks.tmp", "words.tmp"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
    };
    if rebuild {
        // 메모리의 색인을 비워 두면 찾는 쪽이 새 색인을 기다린다.
        *slot.data.write().unwrap_or_else(|e| e.into_inner()) = None;
        wipe();
    }
    // 처음: 디스크에 있던 색인을 읽어 온다. 읽어 왔으면 바로 쓸 수 있게 내놓고(찾을 때 가리킨 파일은 지금 내용으로 다시 읽으니
    // 조금 묵어도 된다), 없으면 다 만들 때까지 비워 둔다(찾는 쪽이 기다린다). 만들다 만 색인이면 내놓지 않고 이어서 만든다.
    let mut cached = false;
    let mut pending: Option<Index> = None;
    if slot.data.read().unwrap_or_else(|e| e.into_inner()).is_none() {
        prune_indexes(&dir);
        match load_table(&dir, root) {
            Some((files, partial)) => {
                cached = true;
                let index = Index { dir: dir.clone(), files, refreshed: None, dirty: false, saved: Some(Instant::now()), partial };
                if partial {
                    pending = Some(index);
                } else {
                    *slot.data.write().unwrap_or_else(|e| e.into_inner()) = Some(index);
                }
            }
            // 쓸 수 없는 색인(없음, 깨짐, 다른 판)은 지우고 처음부터 만든다.
            None => wipe(),
        }
    }
    let known: HashMap<String, (u64, u64)> = {
        let guard = slot.data.read().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().or(pending.as_ref()).map(|index| index.files.iter().map(|(file, entry)| (file.clone(), (entry.modified, entry.size))).collect()).unwrap_or_default()
    };
    let whole = only.is_none();
    let files: Vec<String> = match only {
        Some(files) => files.iter().filter(|file| code::lang_of(file).is_some()).cloned().collect(),
        None => {
            let mut files = git_files(root).filter(|files| !files.is_empty()).unwrap_or_else(|| walked_up_to(root, true, INDEX_MAX_FILES));
            files.retain(|file| code::lang_of(file).is_some());
            files.truncate(INDEX_MAX_FILES);
            files
        }
    };
    slot.total.store(files.len(), Ordering::Relaxed);
    // 여러 스레드가 파일을 읽어 넘기고, 이 스레드가 받는 대로 디스크에 덧붙인다(읽은 것을 메모리에 쌓아 두지 않는다).
    let Ok(mut appender) = Appender::open(&dir) else { return finish(None) };
    let mut fresh_entries: Vec<(String, FileEntry)> = Vec::new();
    let mut seen: Vec<String> = Vec::new(); // 그대로인 파일들
    let mut failed = false;
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let chunk = files.len().div_ceil(workers).max(1);
    // 이번 맞추기가 도중에 그만뒀는지(끝에서 `cancel`을 다시 읽으면, 그사이 다시 열려 꺼졌을 때 "다 봤다"로 잘못 안다).
    let gave_up = AtomicBool::new(false);
    let (known_ref, slot_ref, gave_up_ref) = (&known, &slot, &gave_up);
    std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::sync_channel::<Result<Fresh, String>>(64);
        for part in files.chunks(chunk) {
            let send = send.clone();
            scope.spawn(move || {
                for relative in part {
                    // 이 폴더를 보던 화면이 없어졌다: 읽은 데까지만 적고 그만둔다(정해 준 파일만 볼 때는 금방 끝나니 끝까지 한다).
                    if whole && (slot_ref.cancel.load(Ordering::Relaxed) || gave_up_ref.load(Ordering::Relaxed)) {
                        gave_up_ref.store(true, Ordering::Relaxed);
                        break;
                    }
                    match read_fresh(root, relative, known_ref.get(relative).copied()) {
                        Some(Ok(fresh)) => drop(send.send(Ok(fresh))),
                        Some(Err(())) => drop(send.send(Err(relative.clone()))),
                        None => {}
                    }
                    slot_ref.done.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        drop(send);
        for message in receive {
            match message {
                Ok(fresh) => match appender.push(fresh) {
                    Ok(entry) => fresh_entries.push(entry),
                    Err(_) => failed = true,
                },
                Err(same) => seen.push(same),
            }
        }
    });
    if failed || appender.finish().is_err() {
        return finish(None); // 디스크에 쓰지 못했다(자리가 없다 등)
    }
    let stopped = gave_up.load(Ordering::Relaxed);
    let parsed = fresh_entries.len();
    let mut guard = slot.data.write().unwrap_or_else(|e| e.into_inner());
    let mut index = guard.take().or(pending).unwrap_or(Index { dir: dir.clone(), files: BTreeMap::new(), refreshed: None, dirty: false, saved: None, partial: true });
    let before = index.files.len();
    if stopped {
        // 도중에 그만뒀다: 읽은 것만 넣어 둔다. 다 보지 못했으니 없어진 파일은 가리지 않는다(다음에 모두 맞출 때 빠진다).
    } else if whole {
        // 목록에 없는 파일(지웠거나 이제 읽을 수 없는 것)은 빠진다.
        let mut old = std::mem::take(&mut index.files);
        for file in seen {
            if let Some(entry) = old.remove(&file) {
                index.files.insert(file, entry);
            }
        }
        index.refreshed = Some(Instant::now());
        index.partial = false;
    } else {
        // 정해 준 파일만 봤다: 읽지 못한 것(지웠다)은 빼고, 나머지는 그대로 둔다.
        for file in &files {
            if !seen.contains(file) {
                index.files.remove(file);
            }
        }
    }
    index.files.extend(fresh_entries);
    if parsed > 0 || index.files.len() != before {
        index.dirty = true;
    }
    // 버려진 덩이가 많아졌으면 줄인다.
    let (on_disk, live) = index_sizes(&index);
    let mut compacted = false;
    if on_disk.saturating_sub(live) > COMPACT_OVER && on_disk > live * 2 {
        if compact(&mut index).is_err() {
            // 줄이다 실패했다: 디스크의 색인을 믿을 수 없으니 버린다(다음에 맞출 때 처음부터 만든다).
            let _ = std::fs::remove_file(dir.join("table.bin"));
            drop(guard);
            return finish(None);
        }
        compacted = true;
    }
    // 파일 목록을 디스크에 적는다. 자주 바뀌는 동안에는 가끔만 적는다(적지 못한 것은 다음에 켤 때 다시 읽으면 된다).
    // 디스크에서 읽어 온 때에는 바뀐 것이 없어도 적는다(파일 목록을 적은 때가 "이 색인을 마지막으로 쓴 때"다). 두 파일을 줄인
    // 뒤(옛 목록을 지웠다)와 만들다 그만둔 때(메모리에 두지 않는다)에는 바로 적는다.
    let due = index.dirty && index.saved.is_none_or(|at| at.elapsed() > TABLE_SAVE_EVERY);
    if (due || cached || compacted || (index.partial && index.dirty)) && save_table(&index, root).is_ok() {
        index.dirty = false;
        index.saved = Some(Instant::now());
    }
    let info = IndexInfo { files: index.files.len(), defs: index.files.values().map(|entry| entry.def_count as usize).sum(), parsed, cached, bytes: index_sizes(&index).0, ms: started.elapsed().as_millis() as u64 };
    // 다 만들지 못한 색인은 찾는 데 쓰지 않는다(디스크에만 두고, 다음에 열 때 이어서 만든다).
    if !index.partial {
        *guard = Some(index);
    }
    drop(guard);
    finish(Some(info))
}

/// 그 낱말이 든 파일들(쓰는 곳 찾기가 읽을 파일). words.bin을 앞에서부터 한 번 훑는다.
fn word_holders(index: &Index, hash: u32) -> Option<Vec<String>> {
    use std::io::{BufReader, Read};
    let mut entries: Vec<(&String, &FileEntry)> = index.files.iter().filter(|(_, entry)| entry.word_count > 0).collect();
    entries.sort_by_key(|(_, entry)| entry.words_at);
    let mut reader = BufReader::with_capacity(1 << 20, std::fs::File::open(index.dir.join("words.bin")).ok()?);
    let mut at = 0u64;
    let mut buffer: Vec<u8> = Vec::new();
    let mut found = Vec::new();
    for (file, entry) in entries {
        reader.seek_relative(entry.words_at as i64 - at as i64).ok()?;
        buffer.resize(entry.word_count as usize * 4, 0);
        reader.read_exact(&mut buffer).ok()?;
        at = entry.words_at + buffer.len() as u64;
        // 정렬된 해시들에서 둘로 나눠 찾는다.
        let (mut low, mut high) = (0usize, entry.word_count as usize);
        while low < high {
            let middle = (low + high) / 2;
            match le_u32(&buffer, middle * 4).cmp(&hash) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    found.push(file.clone());
                    break;
                }
            }
        }
    }
    found.sort();
    Some(found)
}

/// 기호 찾기가 색인에서 고른 것 하나: (파일, 색인에 적을 때의 고친 때와 크기, 정의).
type Picked = (String, (u64, u64), Def);

/// 이름이 적은 글과 맞는 정의를 색인에서 고른다(기호 찾기). blocks.bin을 여러 스레드로 나눠 훑어 순위를 매기고 앞의 것만 돌려준다.
/// 색인 파일을 읽지 못하면 None(파일을 모두 읽는 길로 물러난다).
fn ranked_symbols(index: &Index, query: &str) -> Option<Vec<Picked>> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    let mut entries: Vec<(&String, &FileEntry)> = index.files.iter().filter(|(_, entry)| entry.def_count > 0).collect();
    entries.sort_by_key(|(_, entry)| entry.defs_at);
    // (순위, 이름의 길이, 파일, 그 덩이에서 몇째 정의인지)
    type Ranked<'a> = (u32, usize, &'a String, &'a FileEntry, usize);
    let order = |a: &Ranked, b: &Ranked| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(b.2)).then_with(|| a.4.cmp(&b.4));
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let chunk = entries.len().div_ceil(workers).max(1);
    let path = index.dir.join("blocks.bin");
    let path = &path;
    let ranked: Option<Vec<Vec<Ranked>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || -> Option<Vec<Ranked>> {
                    let mut reader = BufReader::with_capacity(1 << 20, std::fs::File::open(path).ok()?);
                    let mut at = part.first()?.1.defs_at;
                    reader.seek(SeekFrom::Start(at)).ok()?;
                    let mut buffer = Vec::new();
                    let mut found: Vec<Ranked> = Vec::new();
                    for (file, entry) in part {
                        reader.seek_relative(entry.defs_at as i64 - at as i64).ok()?;
                        buffer.resize(entry.defs_len as usize, 0);
                        reader.read_exact(&mut buffer).ok()?;
                        at = entry.defs_at + u64::from(entry.defs_len);
                        let Some(block) = Block::read(&buffer) else { continue };
                        for (nth, item) in block.items().enumerate() {
                            // 제목·구역은 기호로 치지 않는다(문서와 스타일의 것이 코드의 기호를 묻어 버린다).
                            if matches!(block.kind(item), "heading" | "section") {
                                continue;
                            }
                            let name = block.name(item);
                            if let Some(rank) = code::rank(query, name) {
                                found.push((rank, name.len(), *file, *entry, nth));
                            }
                        }
                        if found.len() > MAX_SYMBOLS * 8 {
                            found.sort_by(order);
                            found.truncate(MAX_SYMBOLS);
                        }
                    }
                    Some(found)
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().ok().flatten()).collect()
    });
    let mut ranked: Vec<Ranked> = ranked?.into_iter().flatten().collect();
    ranked.sort_by(order);
    ranked.truncate(MAX_SYMBOLS);
    // 고른 것들의 정의를 덩이에서 다시 읽어 낸다.
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = Vec::new();
    let mut picked = Vec::with_capacity(ranked.len());
    for (_, _, name, entry, nth) in ranked {
        buffer.resize(entry.defs_len as usize, 0);
        file.seek(SeekFrom::Start(entry.defs_at)).ok()?;
        file.read_exact(&mut buffer).ok()?;
        if let Some(def) = Block::read(&buffer).and_then(|block| block.items().nth(nth).map(|item| block.def(item))) {
            picked.push((name.clone(), (entry.modified, entry.size), def));
        }
    }
    Some(picked)
}

/// 프로젝트 폴더의 색인을 연다(처음: 디스크에 있으면 읽어 오고 없으면 만든다) 또는 지금의 파일들과 맞춘다(그 뒤).
/// `open`이면 이 색인을 쓰는 화면이 하나 늘었다는 뜻이다. 코드 보기가 폴더를 열 때와, 파일이 바뀌었을 때·가끔씩 부른다.
#[tauri::command]
pub async fn code_index(root: String, open: bool) -> Result<IndexInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        if open {
            open_slot(&root);
        }
        refresh_index(&root, None).ok_or_else(|| INDEX_FAILED.to_string())
    })
    .await
    .map_err(err)?
}

const INDEX_FAILED: &str = "색인을 디스크에 쓰지 못했습니다";

/// 그 폴더의 색인 자리를 마련하고 쓰는 화면을 하나 늘린다(닫히는 중이었다면 그만두던 일을 잇는다).
/// 수를 세는 일은 자리들의 잠금 안에서 한다: 닫는 쪽이 "쓰는 화면이 없다"고 보고 자리를 치우는 것과 엇갈리지 않게.
/// 자리는 여기서만 생긴다: 열지 않은(또는 이미 닫은) 폴더를 맞추거나 다시 만들라는 말은 아무 일도 하지 않는다.
fn open_slot(root: &Path) {
    let mut slots = slots().lock().unwrap_or_else(|e| e.into_inner());
    let slot = slots.entry(slot_key(root)).or_default();
    slot.users.fetch_add(1, Ordering::Relaxed);
    slot.cancel.store(false, Ordering::Relaxed);
}

/// 색인을 지우고 처음부터 다시 만든다(화면의 [재색인]: 색인이 어긋난 것 같을 때). 다시 만드는 동안 찾는 쪽은 기다린다.
#[tauri::command]
pub async fn code_index_rebuild(root: String) -> Result<IndexInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        refresh_with(&root, None, true).ok_or_else(|| INDEX_FAILED.to_string())
    })
    .await
    .map_err(err)?
}

/// 경로를 가지런히 한다: `/`를 `\`로, 겹친 구분자를 하나로, `.`과 `..`을 풀어서(같은 파일이 여러 표기로 오지 않게).
fn tidy_path(path: &str) -> String {
    let path = path.replace('/', "\\");
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                // 드라이브(첫 마디) 위로는 올라가지 않는다.
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            part => parts.push(part),
        }
    }
    format!("{}{}", if path.starts_with("\\\\") { "\\\\" } else { "" }, parts.join("\\"))
}

/// 색인에 아직 없는 파일들 가운데, 모두 맞출 때에도 색인에 들어갈 것만 고른다: git이 아는 파일(무시하지 않는 것)이거나,
/// git을 쓰지 않는 폴더면 숨김·빌드 폴더 밖의 것. (Claude 세션이 빌드 폴더에 쓴 파일이 찾기 결과에 끼지 않게.)
fn listed_files(root: &Path, files: Vec<String>) -> Vec<String> {
    if files.is_empty() {
        return files;
    }
    let outside_plumbing = |files: Vec<String>| -> Vec<String> { files.into_iter().filter(|file| file.rsplit_once('/').is_none_or(|(dirs, _)| !dirs.split('/').any(is_plumbing))).collect() };
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        command.arg("-C").arg(root).args(args).env("GIT_OPTIONAL_LOCKS", "0").stdin(Stdio::null()).stderr(Stdio::null()).creation_flags(CREATE_NO_WINDOW);
        command
    };
    // (경로에 든 `*`나 `[`를 무늬로 읽지 않게 글자 그대로 견준다.)
    let listing = git(&["-c", "core.quotepath=false", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--"]).args(&files).env("GIT_LITERAL_PATHSPECS", "1").output();
    let Ok(output) = listing else { return outside_plumbing(files) };
    if !output.status.success() {
        return outside_plumbing(files); // git 저장소가 아니다
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let known: Vec<String> = text.split('\0').filter(|path| !path.is_empty()).map(str::to_string).collect();
    if known.is_empty() {
        // git이 무시하는 폴더를 프로젝트로 열었으면 git이 아는 파일이 하나도 없다: 그런 폴더는 직접 돌아다니는 쪽의 규칙을 따른다.
        let ignored_root = git(&["check-ignore", "-q", "."]).stdout(Stdio::null()).status().is_ok_and(|status| status.success());
        return if ignored_root { outside_plumbing(files) } else { Vec::new() };
    }
    known
}

/// 그 파일들만 색인에 다시 넣는다(Claude 세션이 방금 쓴 파일: 파일 목록을 다시 읽지 않아 금방 끝난다). 색인을 두지 않은 폴더면 아무 일도 없다.
#[tauri::command]
pub async fn code_index_touch(root: String, paths: Vec<String>) -> Result<Option<IndexInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root_of(&root)?;
        // 프로젝트 폴더 기준 경로(`/`로 구분)로 바꾼다. 폴더 밖의 파일은 뺀다.
        let prefix = format!("{}\\", tidy_path(&slot_key(&root)));
        let mut files: Vec<String> = paths
            .iter()
            .filter_map(|path| {
                let full = tidy_path(path);
                let lower = full.to_lowercase();
                // (소문자로 바꾸면 길이가 달라지는 글자가 든 경로는 자리를 믿을 수 없어 뺀다. 다음에 모두 맞출 때 들어간다.)
                (lower.len() == full.len() && lower.starts_with(&prefix)).then(|| full.get(prefix.len()..).map(|rest| rest.replace('\\', "/"))).flatten()
            })
            .filter(|file| !file.is_empty())
            .collect();
        files.sort();
        files.dedup();
        if files.is_empty() || slot_of(&root).is_none() {
            return Ok(None);
        }
        // 색인에 이미 있는 파일이면 그 표기(대소문자)를 따른다: 같은 파일이 두 이름으로 들어가지 않게.
        // 아직 없는 파일은 모두 맞출 때에도 들어갈 것만 넣는다.
        let split = |index: &Index| -> (Vec<String>, Vec<String>) {
            let mut known = Vec::new();
            let mut fresh = Vec::new();
            for file in &files {
                match index.files.keys().find(|known| known.eq_ignore_ascii_case(file)) {
                    Some(key) => known.push(key.clone()),
                    None => fresh.push(file.clone()),
                }
            }
            (known, fresh)
        };
        let (mut known, fresh) = indexed(&root, split).unwrap_or_else(|| (Vec::new(), files.clone()));
        known.extend(listed_files(&root, fresh.into_iter().filter(|file| code::lang_of(file).is_some()).collect()));
        known.sort();
        known.dedup();
        if known.is_empty() {
            return Ok(None);
        }
        refresh_index(&root, Some(&known)).map(Some).ok_or_else(|| INDEX_FAILED.to_string())
    })
    .await
    .map_err(err)?
}

/// 색인을 만드는 중인지와 얼마나 읽었는지(화면이 진행을 보여 준다).
#[derive(Serialize, Default)]
pub struct IndexState {
    ready: bool,
    busy: bool,
    done: usize,
    total: usize,
}

#[tauri::command]
pub fn code_index_state(root: String) -> IndexState {
    let Some(slot) = slot_of(Path::new(&root)) else { return IndexState::default() };
    // 맞추기가 쓰기 잠금을 쥐고 있으면(끝에서 파일 목록을 적는 동안) 기다리지 않고 마지막으로 본 값으로 답한다.
    let ready = match slot.data.try_read() {
        Ok(data) => {
            slot.ready.store(data.is_some(), Ordering::Relaxed);
            data.is_some()
        }
        Err(_) => slot.ready.load(Ordering::Relaxed),
    };
    IndexState {
        ready,
        busy: slot.busy.load(Ordering::Relaxed),
        done: slot.done.load(Ordering::Relaxed),
        total: slot.total.load(Ordering::Relaxed),
    }
}

/// 코드 보기가 그 폴더를 더는 보지 않는다: 쓰는 화면이 없으면 색인을 메모리에서 내린다(디스크의 색인은 남아 다음에 바로 쓴다).
#[tauri::command]
pub async fn code_index_close(root: String) {
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(root);
        let key = slot_key(&root);
        let slot = {
            let slots = slots().lock().unwrap_or_else(|e| e.into_inner());
            let Some(slot) = slots.get(&key).cloned() else { return };
            let users = slot.users.load(Ordering::Relaxed).saturating_sub(1);
            slot.users.store(users, Ordering::Relaxed);
            if users > 0 {
                return;
            }
            // 돌고 있는 맞추기가 있으면 읽은 데까지만 적고 그만두게 한다.
            slot.cancel.store(true, Ordering::Relaxed);
            slot
        };
        // 돌던 일이 끝난 뒤에 자리를 치운다. 일이 도는 중에 자리부터 치우면, 같은 폴더를 다시 열 때 새 자리가 생겨 두 일이 같은
        // 색인 파일에 함께 덧붙인다(자리가 어긋나 색인이 깨진다). 기다리는 사이에 다시 열렸으면 치우지 않는다.
        let _working = slot.working.lock().unwrap_or_else(|e| e.into_inner());
        // 적지 못한 바뀐 것이 있으면 내리기 전에 파일 목록을 적어 둔다(자리를 치우기 전에: 치운 뒤에 적으면 그사이 다시 열린
        // 새 자리의 일과 같은 파일을 함께 쓸 수 있다).
        if slots().lock().unwrap_or_else(|e| e.into_inner()).get(&key).is_some_and(|now| Arc::ptr_eq(now, &slot)) {
            let mut data = slot.data.write().unwrap_or_else(|e| e.into_inner());
            if let Some(index) = data.as_mut().filter(|index| index.dirty) {
                if save_table(index, &root).is_ok() {
                    index.dirty = false;
                    index.saved = Some(Instant::now());
                }
            }
        }
        let mut slots = slots().lock().unwrap_or_else(|e| e.into_inner());
        if slot.users.load(Ordering::Relaxed) > 0 || !slots.get(&key).is_some_and(|now| Arc::ptr_eq(now, &slot)) {
            return;
        }
        slots.remove(&key);
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 색인으로_찾는다() {
        let dir = project("cypher-code-index-test");
        let root = dir.to_string_lossy().into_owned();
        let store = index_dir(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&store);
        let run = |name: &str| -> Vec<String> {
            let mut found: Vec<String> = tauri::async_runtime::block_on(code_defs(root.clone(), name.into(), None, None)).unwrap().into_iter().map(|f| format!("{}:{}:{}", f.file, f.def.line, f.text)).collect();
            found.sort();
            found
        };
        let usages = |name: &str| -> Vec<String> {
            let mut found: Vec<String> = tauri::async_runtime::block_on(code_usages(root.clone(), name.into())).unwrap().hits.into_iter().map(|f| format!("{}:{}:{}", f.file, f.hit.line, f.hit.role)).collect();
            found.sort();
            found
        };
        let symbols = |query: &str| -> Vec<String> {
            tauri::async_runtime::block_on(code_symbols(root.clone(), query.into())).unwrap().into_iter().map(|f| format!("{}:{}:{}:{}:{}", f.file, f.def.kind, f.def.container, f.def.name, f.text)).collect()
        };
        let index = |open: bool| tauri::async_runtime::block_on(code_index(root.clone(), open)).unwrap();
        let close = || tauri::async_runtime::block_on(code_index_close(root.clone()));
        // 색인이 없을 때(모두 훑음)와 있을 때의 답이 같다.
        let before = (run("load_user"), run("name"), usages("load_user"), usages("User"));
        let info = index(true);
        assert_eq!((info.files, info.parsed, info.cached), (3, 3, false));
        assert!(info.defs >= 5 && info.bytes > 0, "{} {}", info.defs, info.bytes);
        assert!(tauri::async_runtime::block_on(async { code_index_state(root.clone()) }).ready);
        assert_eq!((run("load_user"), run("name"), usages("load_user"), usages("User")), before);
        assert_eq!(before.0, ["src/model/user.rs:6:pub fn load_user(id: u32) -> User {"]);
        // 낱말 색인: 다른 이름의 일부로만 든 파일은 고르지 않고, 한글 이름은 모두 훑어 찾는다.
        assert_eq!(usages("user"), ["src/main.rs:3:use", "src/main.rs:4:use"]);
        assert_eq!(usages("사용자를"), ["src/model/user.rs:5:comment"]);
        assert!(symbols("lu").contains(&"src/model/user.rs:function::load_user:pub fn load_user(id: u32) -> User {".to_string()), "{:?}", symbols("lu"));
        assert!(symbols("name").contains(&"src/model/user.rs:field:User:name:pub name: String,".to_string()), "{:?}", symbols("name"));
        // 바뀐 파일만 다시 읽는다. 새 파일의 정의는 색인을 맞춘 뒤에 찾는다(못 찾으면 스스로 맞춘다).
        let again = index(false);
        assert_eq!((again.files, again.parsed), (3, 0));
        std::fs::write(dir.join("src").join("extra.rs"), "pub fn brand_new() {}\n").unwrap();
        std::thread::sleep(Duration::from_millis(3100));
        assert_eq!(run("brand_new"), ["src/extra.rs:1:pub fn brand_new() {}"]);
        std::fs::write(dir.join("src").join("extra.rs"), "\n\npub fn brand_new() {}\npub fn another() {}\n").unwrap();
        // 색인이 가리킨 파일은 지금 내용으로 다시 읽으므로 줄 번호가 맞다.
        assert_eq!(run("brand_new"), ["src/extra.rs:3:pub fn brand_new() {}"]);
        // 그 파일만 다시 넣는다(Claude 세션이 방금 쓴 파일): 파일 목록을 다시 읽지 않고도 새 정의를 찾는다.
        let touch = |path: &Path| tauri::async_runtime::block_on(code_index_touch(root.clone(), vec![path.to_string_lossy().into_owned()])).unwrap();
        let touched = touch(&dir.join("SRC").join("extra.rs")).unwrap(); // 표기(대소문자)가 달라도 같은 파일이다
        assert_eq!((touched.files, touched.parsed), (4, 1));
        assert_eq!(run("another"), ["src/extra.rs:4:pub fn another() {}"]);
        assert!(touch(Path::new(r"C:\다른 폴더\x.rs")).is_none());
        // 앱을 다시 켠 것처럼: 메모리에서 내렸다가 다시 열면 디스크의 색인을 읽어 오고, 그사이 바뀐 파일만 다시 읽는다.
        close();
        assert!(slot_of(&dir).is_none());
        assert!(store.join("table.bin").is_file() && store.join("blocks.bin").is_file() && store.join("words.bin").is_file());
        std::fs::write(dir.join("src").join("main.rs"), "mod model;\nfn main() {}\nfn later_added() {}\n").unwrap();
        let reopened = index(true);
        assert_eq!((reopened.files, reopened.parsed, reopened.cached), (4, 1, true));
        assert_eq!(run("later_added"), ["src/main.rs:3:fn later_added() {}"]);
        assert_eq!(run("another"), ["src/extra.rs:4:pub fn another() {}"]);
        assert_eq!(usages("load_user"), ["src/model/user.rs:5:comment", "src/model/user.rs:6:def", "src/한글 파일.js:2:string"]);
        // 지운 파일은 빠진다(그 파일만 다시 볼 때도, 모두 맞출 때도).
        std::fs::remove_file(dir.join("src").join("extra.rs")).unwrap();
        assert_eq!(touch(&dir.join("src").join("extra.rs")).unwrap().files, 3);
        assert_eq!(index(false).files, 3);
        assert!(run("another").is_empty());
        // 버려진 덩이를 줄여도(두 파일을 새로 써도) 답이 같다.
        let sizes = |name: &str| std::fs::metadata(store.join(name)).unwrap().len();
        let (blocks, words) = (sizes("blocks.bin"), sizes("words.bin"));
        {
            let slot = slot_of(&dir).unwrap();
            let mut guard = slot.data.write().unwrap();
            compact(guard.as_mut().unwrap()).unwrap();
            save_table(guard.as_ref().unwrap(), &dir).unwrap();
        }
        assert!(sizes("blocks.bin") < blocks && sizes("words.bin") < words);
        assert_eq!((run("load_user"), usages("User")), (before.0.clone(), before.3.clone()));
        assert!(symbols("lu").contains(&"src/model/user.rs:function::load_user:pub fn load_user(id: u32) -> User {".to_string()));
        // 깨진 색인은 버리고 처음부터 만든다.
        close();
        std::fs::write(store.join("table.bin"), b"\x00\x01\x02 broken").unwrap();
        let rebuilt = index(true);
        assert_eq!((rebuilt.files, rebuilt.parsed, rebuilt.cached), (3, 3, false));
        assert_eq!(run("load_user"), before.0);
        // [재색인]: 있던 색인을 지우고 처음부터 다시 만든다.
        let again = tauri::async_runtime::block_on(code_index_rebuild(root.clone())).unwrap();
        assert_eq!((again.files, again.parsed, again.cached), (3, 3, false));
        assert_eq!(run("load_user"), before.0);
        // 색인을 맞추기 전에 줄이 밀린 파일: 기호 찾기도 지금의 줄을 준다(없어진 이름은 빠진다).
        let user_file = dir.join("src").join("model").join("user.rs");
        let user_text = std::fs::read_to_string(&user_file).unwrap();
        std::fs::write(&user_file, format!("\n\n{}", user_text.replace("pub name", "pub title"))).unwrap();
        assert!(symbols("lu").contains(&"src/model/user.rs:function::load_user:pub fn load_user(id: u32) -> User {".to_string()), "{:?}", symbols("lu"));
        assert!(!symbols("name").iter().any(|found| found.starts_with("src/model/user.rs:field")), "{:?}", symbols("name"));
        std::fs::write(&user_file, &user_text).unwrap();
        // 쓰는 화면이 없어지면 돌던 "모두 맞추기"는 읽은 데까지만 적고 그만둔다(색인은 그대로 쓴다).
        let slot = slot_of(&dir).unwrap();
        slot.cancel.store(true, Ordering::Relaxed);
        let stopped = refresh_index(&dir, None).unwrap();
        assert_eq!((stopped.files, stopped.parsed), (3, 0));
        slot.cancel.store(false, Ordering::Relaxed);
        assert_eq!(index(false).parsed, 1); // 되돌린 user.rs
        drop(slot);
        close();
        // 만들다 그만둔 색인: 디스크에만 두고 찾는 데 쓰지 않으며(모두 훑어 찾는다), 다음에 이어서 만든다.
        let _ = std::fs::remove_dir_all(&store);
        open_slot(&dir);
        let part = touch(&dir.join("src").join("main.rs")).unwrap();
        assert_eq!((part.files, part.parsed), (1, 1));
        assert!(slot_of(&dir).unwrap().data.read().unwrap().is_none());
        assert_eq!(load_table(&store, &dir).map(|(files, partial)| (files.len(), partial)), Some((1, true)));
        assert_eq!(run("load_user"), before.0);
        let resumed = index(false);
        assert_eq!((resumed.files, resumed.parsed, resumed.cached), (3, 2, true));
        assert_eq!(load_table(&store, &dir).map(|(files, partial)| (files.len(), partial)), Some((3, false)));
        assert_eq!(run("load_user"), before.0);
        // 같은 파일을 여러 표기로 일러 줘도 한 번만 들어간다. 빌드 폴더 안의 파일은 넣지 않는다.
        let odd = format!(r"{root}\\src\.\model\..\main.rs");
        let twice = tauri::async_runtime::block_on(code_index_touch(root.clone(), vec![odd, format!("{root}/src//main.rs"), format!(r"{root}\node_modules\lib\index.js")])).unwrap().unwrap();
        assert_eq!((twice.files, twice.parsed), (3, 0));
        assert_eq!(tidy_path(r"C:\a\\b/./c\..\d.rs"), r"C:\a\b\d.rs");
        assert_eq!(tidy_path(r"\\server\share\..\..\x"), r"\\server\x");
        // 오래 쓰지 않은 색인 폴더만 지운다: 열려 있는 것과 만드는 중인 것(파일 목록이 아직 없다)은 둔다.
        let home = index_home().unwrap();
        let old = SystemTime::now() - INDEX_KEEP - Duration::from_secs(3600);
        let make = |folder: &Path, file: &str, at: Option<SystemTime>| {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join(file), b"x").unwrap();
            if let Some(at) = at {
                std::fs::File::options().write(true).open(folder.join(file)).unwrap().set_modified(at).unwrap();
            }
        };
        let (stale, building, abandoned) = (home.join("0000000000000001"), home.join("0000000000000002"), home.join("0000000000000003"));
        make(&stale, "table.bin", Some(old));
        make(&building, "blocks.bin", None);
        make(&abandoned, "blocks.bin", Some(old));
        let elsewhere = Path::new(r"C:\cypher 시험\열려 있는 폴더");
        open_slot(elsewhere);
        let opened = index_dir(elsewhere).unwrap();
        make(&opened, "table.bin", Some(old));
        prune_indexes(&store);
        assert_eq!((stale.exists(), building.exists(), abandoned.exists(), opened.exists(), store.exists()), (false, true, false, true, true));
        slots().lock().unwrap().remove(&slot_key(elsewhere));
        let _ = std::fs::remove_dir_all(&building);
        let _ = std::fs::remove_dir_all(&opened);
        close();
        // 덩이를 적고 읽기.
        let defs = code::definitions(Lang::Rust, "pub struct User {\n    pub name: String,\n}\nimpl User {\n    fn name(&self) {}\n}\n");
        let (block, count, names) = encode_defs(&defs);
        let read = Block::read(&block).unwrap();
        let back: Vec<Def> = read.items().map(|item| read.def(item)).collect();
        assert_eq!(count as usize, defs.len());
        assert_eq!(back.iter().map(|def| (def.name.as_str(), def.kind, def.line, def.col, def.container.as_str())).collect::<Vec<_>>(), defs.iter().map(|def| (def.name.as_str(), def.kind, def.line, def.col, def.container.as_str())).collect::<Vec<_>>());
        assert_eq!(names.len(), 2); // User, name(둘이 한 해시)
        assert!(Block::read(&block[..block.len() - 3]).is_none() && Block::read(&[]).is_none());
        assert_eq!(words_of("fn load_user(id: u32) -> $a 한글b"), {
            let mut expected: Vec<u32> = ["fn", "load_user", "id", "u32", "$a", "b"].iter().map(|word| word_hash(word.as_bytes())).collect();
            expected.sort_unstable();
            expected
        });
        assert!(simple_name("load_user") && simple_name("$scope") && !simple_name("card-title") && !simple_name("사용자") && !simple_name(""));
        let _ = std::fs::remove_dir_all(&store);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
    fn git이_아는_파일에서_지금_있는_파일만_고른다() {
        // 추적하지 않는 중첩 저장소(폴더), 새 파일, 충돌 난 파일(단계마다 한 번), 작업 폴더에서 지운 파일.
        let raw = "? nested/\0? new file.txt\0M a.rs\0M a.rs\0M a.rs\0H b.rs\0H gone.rs\0R gone.rs\0H 한글 파일.txt\0";
        assert_eq!(tracked_files(raw), ["new file.txt", "a.rs", "b.rs", "한글 파일.txt"]);
        assert!(tracked_files("").is_empty());
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

    #[test]
    fn 자료형을_따라_정의를_고른다() {
        let dir = project("cypher-code-nav-test");
        let java = dir.join("app").join("src").join("main").join("java").join("shop");
        std::fs::create_dir_all(java.join("impl")).unwrap();
        std::fs::write(java.join("OrderService.java"), "package shop;\npublic interface OrderService extends BaseService<Order> {\n    Order find(Long id);\n}\n").unwrap();
        std::fs::write(java.join("CartService.java"), "package shop;\npublic interface CartService {\n    Cart find(Long id);\n}\n").unwrap();
        std::fs::write(
            java.join("impl").join("OrderServiceImpl.java"),
            "package shop.impl;\npublic class OrderServiceImpl implements OrderService {\n    @Override\n    public Order find(Long id) {\n        return null;\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("shape.rs"), "pub trait Shape {\n    fn area(&self) -> f64;\n}\npub struct Circle;\nimpl Shape for Circle {\n    fn area(&self) -> f64 { 1.0 }\n}\n").unwrap();
        let root = dir.to_string_lossy().into_owned();
        let defs = |name: &str, within: Option<&[&str]>| -> Vec<Found> {
            let within = within.map(|owners| owners.iter().map(|owner| owner.to_string()).collect());
            let mut found = tauri::async_runtime::block_on(code_defs(root.clone(), name.into(), within, Some(false))).unwrap();
            found.sort_by(|a, b| a.file.cmp(&b.file));
            found
        };
        let shown = |found: &[Found]| -> Vec<String> { found.iter().map(|f| format!("{}.{}@{}", f.def.container, f.def.name, f.file.rsplit('/').next().unwrap())).collect() };
        // 감싼 이름으로 거른다: 받는 쪽의 자료형을 알면 그 자료형의 것만 본다.
        assert_eq!(shown(&defs("find", None)).len(), 3);
        assert_eq!(shown(&defs("find", Some(&["OrderService"]))), ["OrderService.find@OrderService.java"]);
        assert!(defs("find", Some(&["Nothing"])).is_empty());
        // 서명과 상위 자료형
        let service = &defs("OrderService", None)[0];
        assert_eq!((service.sig.as_str(), service.supers.as_slice()), ("public interface OrderService extends BaseService<Order>", ["BaseService".to_string()].as_slice()));
        assert_eq!(defs("find", Some(&["OrderServiceImpl"]))[0].sig, "public Order find(Long id)");
        assert_eq!(defs("area", Some(&["Circle"]))[0].sig, "fn area(&self) -> f64");
        // 구현 찾기: 인터페이스를 구현한 클래스, 트레이트를 구현한 자료형
        let subtypes = |name: &str| -> Vec<String> {
            tauri::async_runtime::block_on(code_subtypes(root.clone(), name.into())).unwrap().iter().map(|f| format!("{}:{}@{}:{}", f.def.kind, f.def.name, f.file.rsplit('/').next().unwrap(), f.def.line)).collect()
        };
        assert_eq!(subtypes("OrderService"), ["class:OrderServiceImpl@OrderServiceImpl.java:2"]);
        assert_eq!(subtypes("Shape"), ["impl:Circle@shape.rs:5"]);
        assert!(subtypes("CartService").is_empty());
        // 나무: 폴더 하나만 품은 폴더는 한 줄로 잇는다(빌드·숨김 폴더는 잇지 않는다).
        let names: Vec<String> = tauri::async_runtime::block_on(code_list(root.clone())).unwrap().into_iter().map(|entry| entry.name).collect();
        assert!(names.contains(&"app/src/main/java/shop".to_string()), "{names:?}");
        assert!(names.contains(&"node_modules".to_string()) && names.contains(&"src".to_string()), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
