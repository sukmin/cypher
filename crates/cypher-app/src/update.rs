//! Cypher 업데이트: GitHub의 최신 릴리즈를 확인하고, 새 버전이면 내려받아 실행 파일을 바꾼다.
//!
//! 사용자가 도움말 → "Cypher 업데이트"에서 단추를 눌렀을 때만 돈다. Cypher가 스스로 확인하러 나가지 않는다.
//! 저장소와 릴리즈가 공개라서 로그인이나 토큰이 필요 없다(보내는 정보도 없다).
//!
//! 순서: 최신 릴리즈 읽기 → 묶음(zip) 내려받기 → GitHub가 알려 준 SHA-256과 견주기 → 풀기 →
//! 실행 파일이 있는 폴더의 파일을 바꾸기. 실행 중인 파일은 덮어쓸 수 없지만 이름은 바꿀 수 있으므로,
//! 옛 파일의 이름을 바꿔 두고 새 파일을 놓는다. 옛 파일은 다음 실행 때 지운다. 새 버전은 다시 시작해야 뜬다.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const LATEST_URL: &str = "https://api.github.com/repos/sukmin/cypher/releases/latest";
/// 묶음은 이 저장소의 릴리즈에서만 받는다.
const DOWNLOAD_PREFIX: &str = "https://github.com/sukmin/cypher/releases/download/";
const ASSET_SUFFIX: &str = "-windows-x64.zip";
/// 묶음 크기의 한도(지금 묶음은 6MB쯤이다).
const MAX_SIZE: u64 = 300 << 20;
/// 바꿔 둔 옛 파일의 이름에 붙이는 꼬리.
const OLD_MARK: &str = ".old-";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 릴리즈에 올라 있는 묶음.
#[derive(Debug, Clone, PartialEq)]
struct Asset {
    name: String,
    url: String,
    size: u64,
    /// GitHub가 알려 준 SHA-256(소문자 16진수). 없는 릴리즈는 설치하지 않는다.
    sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Release {
    version: String,
    published: String,
    notes: String,
    page: String,
    asset: Option<Asset>,
}

/// 화면에 보여 줄 확인 결과.
#[derive(Serialize)]
pub struct UpdateInfo {
    current: String,
    latest: String,
    newer: bool,
    /// 올린 날짜(YYYY-MM-DD).
    published: String,
    notes: String,
    page: String,
    /// 묶음 크기(바이트). 묶음이 없으면 0.
    size: u64,
    /// 이 자리에서 설치할 수 없으면 그 까닭(없으면 설치할 수 있다).
    blocked: Option<String>,
}

/// `v0.15.0`, `0.15.0` 같은 글에서 숫자들을 꺼낸다. 숫자와 점만으로 된 버전이 아니면 None.
fn version_parts(text: &str) -> Option<Vec<u64>> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    if text.is_empty() {
        return None;
    }
    text.split('.').map(|part| part.parse().ok()).collect()
}

/// `latest`가 `current`보다 새 버전인지. 읽을 수 없는 버전은 새 버전으로 치지 않는다.
fn is_newer(latest: &str, current: &str) -> bool {
    match (version_parts(latest), version_parts(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

/// GitHub의 릴리즈 정보에서 쓸 것만 꺼낸다. 묶음은 이름이 `-windows-x64.zip`으로 끝나고 이 저장소의 릴리즈 주소에 있는 것만 받는다.
fn parse_release(json: &Value) -> Result<Release, String> {
    let tag = json["tag_name"].as_str().ok_or("릴리즈 정보의 형식이 바뀌었습니다(버전을 찾지 못함)")?;
    let version = tag.trim().trim_start_matches(['v', 'V']).to_string();
    if version_parts(&version).is_none() {
        return Err(format!("릴리즈의 버전을 읽지 못했습니다: {tag}"));
    }
    let asset = json["assets"].as_array().into_iter().flatten().find_map(|asset| {
        let name = asset["name"].as_str()?;
        let url = asset["browser_download_url"].as_str()?;
        // 이름에 경로가 섞여 있으면 받지 않는다(받은 파일을 그 이름으로 저장한다).
        let plain = !name.contains(['/', '\\']) && !name.contains("..");
        if !plain || !name.ends_with(ASSET_SUFFIX) || !url.starts_with(DOWNLOAD_PREFIX) {
            return None;
        }
        let sha256 = asset["digest"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase)
            .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
        Some(Asset { name: name.to_string(), url: url.to_string(), size: asset["size"].as_u64().unwrap_or(0), sha256 })
    });
    Ok(Release {
        version,
        published: json["published_at"].as_str().unwrap_or("").chars().take(10).collect(),
        notes: json["body"].as_str().unwrap_or("").to_string(),
        page: json["html_url"].as_str().unwrap_or("https://github.com/sukmin/cypher/releases").to_string(),
        asset,
    })
}

fn agent(seconds: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(seconds)).redirects(5).build()
}

fn latest() -> Result<Release, String> {
    let body = agent(15)
        .get(LATEST_URL)
        .set("User-Agent", "Cypher")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("최신 릴리즈를 확인하지 못했습니다: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    parse_release(&serde_json::from_str(&body).map_err(|e| e.to_string())?)
}

/// 실행 파일이 있는 폴더.
fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "실행 파일의 폴더를 찾지 못했습니다".to_string())
}

/// 이 자리에서 설치할 수 없는 까닭. 설치할 수 있으면 None.
fn blocked(release: &Release) -> Option<String> {
    if cfg!(debug_assertions) {
        return Some("개발용 빌드에서는 설치하지 않습니다(확인만 합니다).".into());
    }
    let Some(asset) = &release.asset else {
        return Some("이 릴리즈에는 Windows 64비트 묶음이 없습니다. 릴리즈 페이지에서 직접 받으세요.".into());
    };
    if asset.sha256.is_none() {
        return Some("이 릴리즈에는 묶음의 확인값(SHA-256)이 없어 설치하지 않습니다. 릴리즈 페이지에서 직접 받으세요.".into());
    }
    if asset.size == 0 || asset.size > MAX_SIZE {
        return Some("묶음의 크기가 이상해 설치하지 않습니다. 릴리즈 페이지에서 직접 받으세요.".into());
    }
    let dir = match install_dir() {
        Ok(dir) => dir,
        Err(text) => return Some(text),
    };
    // 실행 파일의 폴더에 쓸 수 있어야 한다(Program Files 같은 곳에 두었으면 쓸 수 없다).
    let probe = dir.join(format!(".cypher-update-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            None
        }
        Err(_) => Some(format!("실행 파일이 있는 폴더({})에 쓸 수 없습니다. 릴리즈 페이지에서 직접 받아 바꾸세요.", dir.display())),
    }
}

/// 설치가 어디까지 왔는지 화면에 알리는 내용(`update-progress`).
#[derive(Clone, Serialize)]
struct Progress {
    /// `check`(릴리즈 확인) · `download`(내려받기) · `unpack`(풀기) · `replace`(파일 바꾸기)
    stage: &'static str,
    /// 내려받은 바이트와 전체 바이트(내려받기에서만 뜻이 있다).
    done: u64,
    total: u64,
}

/// 설치의 단계를 알리는 함수: (단계, 받은 바이트, 전체 바이트).
type Report<'a> = &'a dyn Fn(&'static str, u64, u64);

/// 묶음을 받아 `to`에 저장한다. 받으면서 SHA-256을 구해 `asset`의 것과 견준다. 받는 양은 1%가 바뀔 때마다 알린다.
fn download(asset: &Asset, to: &Path, report: Report) -> Result<(), String> {
    let expected = asset.sha256.as_deref().ok_or("묶음의 확인값이 없습니다")?;
    let response = agent(300).get(&asset.url).set("User-Agent", "Cypher").call().map_err(|e| format!("묶음을 내려받지 못했습니다: {e}"))?;
    let mut reader = response.into_reader().take(MAX_SIZE + 1);
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    let percent = |total: u64| total.saturating_mul(100) / asset.size.max(1);
    report("download", 0, asset.size);
    loop {
        let read = reader.read(&mut buf).map_err(|e| format!("묶음을 내려받다가 끊겼습니다: {e}"))?;
        if read == 0 {
            break;
        }
        if percent(total + read as u64) != percent(total) {
            report("download", total + read as u64, asset.size);
        }
        total += read as u64;
        hasher.update(&buf[..read]);
        file.write_all(&buf[..read]).map_err(|e| e.to_string())?;
    }
    file.flush().map_err(|e| e.to_string())?;
    if total != asset.size {
        return Err(format!("내려받은 묶음의 크기가 다릅니다({total}바이트, 릴리즈에는 {}바이트).", asset.size));
    }
    let actual: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if actual != expected {
        return Err("내려받은 묶음의 확인값(SHA-256)이 릴리즈의 것과 다릅니다. 설치하지 않았습니다.".into());
    }
    Ok(())
}

/// zip을 `to` 폴더에 푼다. Windows에 든 tar(System32의 bsdtar)를 쓴다. PATH의 다른 tar(Git의 것 등)는 zip을 풀지 못한다.
///
/// `to`는 zip이 든 폴더 아래여야 한다. tar에는 전체 경로를 주지 않고, 그 폴더에서 띄워 상대 경로만 준다:
/// tar는 인자를 시스템 코드 페이지로 읽어서, 경로에 그 코드 페이지에 없는 글자(영어 Windows의 한글 사용자 이름 등)가
/// 있으면 전체 경로로는 폴더를 찾지 못한다.
fn unzip(zip: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let relative = zip.parent().zip(zip.file_name()).and_then(|(dir, name)| Some((dir, name, to.strip_prefix(dir).ok()?)));
    let (dir, zip, to) = relative.ok_or("묶음을 풀 폴더가 묶음이 든 폴더 아래에 있지 않습니다")?;
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let tar = root.join("System32").join("tar.exe");
    let done = Command::new(&tar)
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(to)
        .current_dir(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("묶음을 풀지 못했습니다({}): {e}", tar.display()))?;
    if !done.status.success() {
        return Err(format!("묶음을 풀지 못했습니다: {}", String::from_utf8_lossy(&done.stderr).trim()));
    }
    Ok(())
}

/// 푼 폴더에서 실행 파일이 든 곳을 찾는다: 그 폴더 자체이거나, 그 안의 폴더 하나(`cypher-<버전>-windows-x64`).
fn package_dir(unpacked: &Path) -> Result<PathBuf, String> {
    find_package(unpacked, &PROGRAMS)
}

/// 묶음에 반드시 들어 있어야 하는 실행 파일.
const PROGRAMS: [&str; 2] = ["cypher.exe", "cypher-bridge.exe"];

/// `names`의 파일이 모두 든 폴더를 찾는다: 푼 폴더 자신이거나 그 바로 아래의 폴더.
fn find_package(unpacked: &Path, names: &[&str]) -> Result<PathBuf, String> {
    let has = |dir: &Path| names.iter().all(|name| dir.join(name).is_file());
    if has(unpacked) {
        return Ok(unpacked.to_path_buf());
    }
    std::fs::read_dir(unpacked)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir() && has(path))
        .ok_or_else(|| "묶음 안에서 cypher.exe와 cypher-bridge.exe를 찾지 못했습니다. 설치하지 않았습니다.".to_string())
}

/// `from` 폴더의 파일들을 `to` 폴더에 놓는다. 이미 있는 파일은 이름을 바꿔 두고(실행 중이어도 된다) 새 파일을 놓는다.
/// 도중에 실패하면 놓은 것을 치우고 옛 파일의 이름을 되돌린다. `stamp`는 옛 파일 이름에 붙일 글.
fn replace_files(from: &Path, to: &Path, stamp: &str) -> Result<Vec<String>, String> {
    let names: Vec<std::ffi::OsString> = std::fs::read_dir(from)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name())
        .collect();
    // (새로 놓은 파일, 이름을 바꿔 둔 옛 파일)
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let mut put = |name: &std::ffi::OsString| -> Result<(), String> {
        let target = to.join(name);
        let mut kept = None;
        if target.exists() {
            let mut old = target.clone().into_os_string();
            old.push(format!("{OLD_MARK}{stamp}"));
            let old = PathBuf::from(old);
            std::fs::rename(&target, &old).map_err(|e| format!("{}의 이름을 바꾸지 못했습니다: {e}", target.display()))?;
            kept = Some(old);
        }
        done.push((target.clone(), kept));
        std::fs::copy(from.join(name), &target).map(drop).map_err(|e| format!("{}을(를) 놓지 못했습니다: {e}", target.display()))
    };
    let outcome = names.iter().try_for_each(&mut put);
    if let Err(text) = outcome {
        for (target, kept) in done.iter().rev() {
            let _ = std::fs::remove_file(target);
            if let Some(old) = kept {
                let _ = std::fs::rename(old, target);
            }
        }
        return Err(text);
    }
    Ok(names.iter().map(|name| name.to_string_lossy().into_owned()).collect())
}

/// 업데이트가 이름을 바꿔 둔 옛 파일의 이름인지: `<원래 이름>.old-<바꾼 시각(초)>`으로 끝난다.
/// 실행 파일 폴더에는 사용자의 다른 파일도 있을 수 있다. 이름에 `.old-`가 들었다고 지우면 안 된다(`보고서.old-2024.docx`).
fn is_leftover(name: &str) -> bool {
    name.rsplit_once(OLD_MARK)
        .is_some_and(|(base, stamp)| !base.is_empty() && stamp.len() >= 9 && stamp.bytes().all(|b| b.is_ascii_digit()))
}

/// 설치하고 있는 중인지(겹쳐 설치하지 않게 한다)와, 이번 실행에서 설치를 마친 버전.
static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static INSTALLED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 지난 업데이트가 남긴 것을 치운다: 이름을 바꿔 둔 옛 파일과 내려받은 묶음. 시작할 때 부른다.
pub fn clean_leftovers() {
    clean_old_files();
    // 내려받던 폴더는 프로세스마다 따로 둔다(`update\<번호>`). 다른 Cypher 창이 지금 내려받고 있을 수 있으므로
    // 오래된 것만 지운다(통째로 지우면 그 창의 설치가 "묶음을 풀지 못했습니다"로 실패한다).
    if let Some(dir) = crate::data_dir() {
        let update = dir.join("update");
        for entry in std::fs::read_dir(&update).into_iter().flatten().flatten() {
            let old = entry.metadata().and_then(|meta| meta.modified()).ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age > STALE_WORK);
            if old {
                let path = entry.path();
                let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            }
        }
        let _ = std::fs::remove_dir(&update); // 비었을 때만 지워진다.
    }
    // "다시 시작"으로 떴으면 앞선 창이 아직 떠 있어 옛 실행 파일이 지워지지 않는다. 그 창이 닫혔을 즈음 한 번 더 치운다.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        clean_old_files();
    });
}

/// 내려받던 폴더를 "남은 것"으로 보고 지우기까지의 시간.
const STALE_WORK: Duration = Duration::from_secs(60 * 60);

/// 실행 파일 폴더에서 업데이트가 이름을 바꿔 둔 옛 파일을 지운다.
fn clean_old_files() {
    // 이 프로세스가 지금 파일을 바꾸고 있으면 건드리지 않는다: 방금 이름을 바꿔 둔 옛 파일은 놓기에 실패했을 때 되돌릴 원본이다.
    if INSTALLING.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    if let Ok(dir) = install_dir() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if is_leftover(&entry.file_name().to_string_lossy()) && entry.path().is_file() {
                let _ = std::fs::remove_file(entry.path()); // 아직 쓰이고 있으면 지워지지 않는다. 다음에 다시 시도한다.
            }
        }
    }
}

/// 묶음을 받아 확인하고 `dir`의 파일을 바꾼다. 단계가 바뀔 때마다 `report`로 알린다.
fn install(asset: &Asset, dir: &Path, work: &Path, report: Report) -> Result<Vec<String>, String> {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
    let zip = work.join(&asset.name);
    download(asset, &zip, report)?;
    report("unpack", asset.size, asset.size);
    let unpacked = work.join("unpacked");
    unzip(&zip, &unpacked)?;
    report("replace", asset.size, asset.size);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
    let placed = replace_files(&package_dir(&unpacked)?, dir, &stamp)?;
    let _ = std::fs::remove_dir_all(work);
    Ok(placed)
}

/// 지금 버전(인터넷에 나가지 않는다).
#[tauri::command]
pub fn update_current() -> String {
    VERSION.to_string()
}

/// 최신 릴리즈를 확인한다.
#[tauri::command]
pub async fn update_check() -> Result<UpdateInfo, String> {
    let release = latest()?;
    let newer = is_newer(&release.version, VERSION);
    Ok(UpdateInfo {
        current: VERSION.to_string(),
        newer,
        blocked: if newer { blocked(&release) } else { None },
        size: release.asset.as_ref().map_or(0, |asset| asset.size),
        latest: release.version,
        published: release.published,
        notes: release.notes,
        page: release.page,
    })
}

/// 최신 릴리즈를 내려받아 설치한다. 설치한 버전을 돌려준다(다시 시작해야 그 버전으로 뜬다).
/// 받을 주소는 화면에서 받지 않고 여기서 다시 확인한다. 진행은 `update-progress`로 화면에 알린다.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<String, String> {
    // 내려받기는 오래 걸릴 수 있다(느린 회선이면 몇 분). 다른 명령의 답이 밀리지 않게 따로 떼어 돌린다.
    tauri::async_runtime::spawn_blocking(move || {
        install_latest(&|stage, done, total| {
            let _ = app.emit_to("main", "update-progress", Progress { stage, done, total });
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn install_latest(report: Report) -> Result<String, String> {
    report("check", 0, 0);
    let release = latest()?;
    if !is_newer(&release.version, VERSION) {
        return Err(format!("이미 최신 버전입니다({VERSION})."));
    }
    if let Some(reason) = blocked(&release) {
        return Err(reason);
    }
    let asset = release.asset.as_ref().ok_or("묶음이 없습니다")?;
    let work = crate::data_dir().ok_or("내려받을 폴더를 찾지 못했습니다")?.join("update").join(std::process::id().to_string());
    let target = install_dir()?;
    // 창을 닫았다 다시 열어 한 번 더 누르면 설치가 겹칠 수 있다. 둘은 같은 작업 폴더를 쓰므로 하나만 돌게 한다.
    use std::sync::atomic::Ordering;
    if INSTALLED.lock().unwrap().as_deref() == Some(release.version.as_str()) {
        return Err(format!("{}은(는) 이미 설치했습니다. Cypher를 다시 시작하면 새 버전으로 뜹니다.", release.version));
    }
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("이미 내려받아 설치하고 있습니다. 잠시 뒤에 다시 확인하세요.".into());
    }
    let outcome = install(asset, &target, &work, report);
    INSTALLING.store(false, Ordering::SeqCst);
    outcome?;
    *INSTALLED.lock().unwrap() = Some(release.version.clone());
    crate::log(&format!("업데이트: {VERSION} → {} 설치", release.version));
    Ok(release.version)
}

/// 다시 시작으로 띄운 프로세스에 넘기는 표시(환경 변수). 받은 쪽은 화면이 준비될 때까지 창을 숨겨 둔다.
const RESTART_MARK: &str = "CYPHER_RESTARTED";
/// 새 프로세스의 창이 뜨기를 기다리는 시간. 넘으면 이 창을 그냥 닫는다(새 프로세스는 돌고 있다).
const NEW_WINDOW_WAIT: Duration = Duration::from_secs(60);
/// 다시 시작으로 뜬 창을 화면이 준비되지 않아도 보여 주는 시간(화면이 뜨지 못해도 창은 보여야 한다).
const HIDDEN_START_LIMIT: Duration = Duration::from_secs(8);

/// 다시 시작으로 떠서 본창을 숨겨 둔 채인지.
static HIDDEN_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 이 프로세스가 "다시 시작"으로 띄워졌는지. 시작할 때 한 번 부른다(표시는 지워서 세션이 물려받지 않게 한다).
/// 그렇게 떴으면 본창을 숨긴 채 만들고 화면이 준비됐을 때 보여 준다: 창이 먼저 뜨면 화면이 그려질 때까지 하얗게 보인다.
pub fn take_restart_mark() -> bool {
    let marked = std::env::var_os(RESTART_MARK).is_some();
    std::env::remove_var(RESTART_MARK);
    HIDDEN_START.store(marked, std::sync::atomic::Ordering::SeqCst);
    marked
}

/// 숨겨 둔 본창을 보여 준다(한 번만).
fn reveal(app: &AppHandle) {
    if HIDDEN_START.swap(false, std::sync::atomic::Ordering::SeqCst) {
        if let Some(window) = app.get_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// 숨긴 채 띄웠으면, 화면이 준비됐다고 알려 오지 않아도 얼마 뒤에는 창을 보여 준다. 시작할 때 부른다.
pub fn reveal_later(app: &AppHandle) {
    if HIDDEN_START.load(std::sync::atomic::Ordering::SeqCst) {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(HIDDEN_START_LIMIT);
            reveal(&app);
        });
    }
}

/// 화면이 그려질 준비가 됐다: 다시 시작으로 떠서 숨겨 둔 본창을 보여 준다(평소에는 할 일이 없다).
#[tauri::command]
pub fn app_ready(app: AppHandle) {
    reveal(&app);
}

/// 새 실행 파일을 띄우고 그 창이 보일 때까지 기다린다.
///
/// 새 실행 파일을 처음 띄울 때 Windows(백신)가 그 파일을 검사하느라 띄우는 일 자체가 수십 초 걸릴 수 있다.
/// 창 스레드에서 하면 그동안 창이 답하지 않고 하얗게 된다. 그래서 따로 떼어 돌리고, 이 창은 안내를 띄운 채 기다린다.
/// `spawned`는 새 프로세스가 만들어진 직후(그 창이 뜨기 전)에 한 번 부른다.
fn launch_new(exe: &Path, folder: Option<PathBuf>, spawned: impl FnOnce()) -> Result<(), String> {
    let mut launch = Command::new(exe);
    launch.env(RESTART_MARK, "1");
    // 본창의 저장소를 환경 변수로 돌려 띄운 경우, 그 변수는 본창을 만든 뒤 이 프로세스에서 지워 두었다.
    // 새 프로세스에는 다시 넘겨야 같은 저장소(로그인·저장된 배치)로 뜬다.
    if let Some(folder) = folder {
        launch.env("WEBVIEW2_USER_DATA_FOLDER", folder);
    }
    let mut child = launch.spawn().map_err(|e| format!("새 버전을 띄우지 못했습니다({}): {e}", exe.display()))?;
    spawned();
    let started = std::time::Instant::now();
    while started.elapsed() < NEW_WINDOW_WAIT {
        if crate::magnet::has_visible_window(child.id()) {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("새 버전이 뜨지 못하고 끝났습니다({status}). 이 창은 계속 쓸 수 있습니다."));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// 새 버전으로 다시 시작한다: 새 실행 파일을 띄우고, 그 창이 뜨면 이 창을 닫는다. 화면은 쓰던 것을 저장한 뒤에 부른다.
/// 새 버전이 뜨지 못하면 오류를 돌려주고 이 창은 그대로 둔다.
#[tauri::command]
pub async fn update_restart(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_window_state::{AppHandleExt, StateFlags};
    let exe = install_dir()?.join("cypher.exe");
    let folder = crate::site::data_folder_override();
    // 새 창은 뜨자마자 저장된 창 위치·크기를 읽는다. 이 창은 그 뒤에야 닫히므로(위치는 닫힐 때 저장된다),
    // 지금 것을 먼저 적어 둔다. 그러지 않으면 새 창이 이번 실행을 시작할 때의 자리·크기로 뜬다.
    let _ = app.save_window_state(StateFlags::all());
    // 새 실행 파일을 띄우는 데 오래 걸리면(검사) 그사이에 화면이 바뀔 수 있다(세션이 만든 파일이 항목으로 놓이는 등).
    // 새 프로세스가 만들어진 직후에 화면에 알려, 새 화면이 읽기 전에 배치를 한 번 더 적게 한다.
    let notify = app.clone();
    let launch = move || launch_new(&exe, folder, || drop(notify.emit_to("main", "restart-spawned", ())));
    tauri::async_runtime::spawn_blocking(launch).await.map_err(|e| e.to_string())??;
    if let Some(window) = app.get_window("main") {
        let _ = window.destroy();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 시험용 폴더. 임시 폴더가 아니라 빌드 폴더(시험 프로그램이 있는 곳) 아래에 두고, 시험이 끝나면 `done`으로 지운다.
    ///
    /// 이 파일의 시험은 프로그램처럼 보이는 파일을 만들지 않는다: 이름은 `.txt`, 내용은 한 문장짜리 글로 쓴다.
    /// 임시 폴더에 5바이트짜리 글("새 a")이 든 `a.exe`를 만들었더니 Windows의 백신(Defender)이 악성 프로그램
    /// (`Trojan:Win32/Bearfoos.A!ml`)으로 잡아 격리했고, 이름을 `a.bin`으로 바꿔 빌드 폴더에 두어도 같은 내용이라 또 잡혔다
    /// (2026-11-15). 파일 이름이 정해진 함수는 이름을 인자로 받는 안쪽 함수(`find_package`)로 시험한다.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 시험이 끝났다: 만든 것을 남기지 않는다.
    fn done(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn 버전을_견준다() {
        assert!(is_newer("v0.16.0", "0.15.0"));
        assert!(is_newer("0.15.1", "0.15.0"));
        assert!(is_newer("0.100.0", "0.99.9")); // 글자가 아니라 숫자로 견준다
        assert!(is_newer("1.0", "0.99.9"));
        assert!(is_newer("0.15.0.1", "0.15.0"));
        assert!(!is_newer("0.15.0", "0.15.0"));
        assert!(!is_newer("v0.14.9", "0.15.0"));
        // 읽을 수 없는 버전은 새 버전으로 치지 않는다.
        for bad in ["", "v", "latest", "0.16.0-beta", "1..2"] {
            assert!(!is_newer(bad, "0.15.0"), "{bad}");
        }
    }

    #[test]
    fn 릴리즈_정보에서_묶음을_고른다() {
        let digest = format!("sha256:{}", "AB".repeat(32));
        let release = parse_release(&json!({
            "tag_name": "v0.16.0", "published_at": "2026-11-16T01:02:03Z", "body": "새 기능", "html_url": "https://github.com/sukmin/cypher/releases/tag/v0.16.0",
            "assets": [
                { "name": "notes.txt", "browser_download_url": "https://github.com/sukmin/cypher/releases/download/v0.16.0/notes.txt", "size": 3 },
                { "name": "cypher-0.16.0-windows-x64.zip", "browser_download_url": "https://github.com/sukmin/cypher/releases/download/v0.16.0/cypher-0.16.0-windows-x64.zip", "size": 6400000, "digest": digest },
            ],
        }))
        .unwrap();
        assert_eq!((release.version.as_str(), release.published.as_str(), release.notes.as_str()), ("0.16.0", "2026-11-16", "새 기능"));
        let asset = release.asset.unwrap();
        assert_eq!((asset.name.as_str(), asset.size), ("cypher-0.16.0-windows-x64.zip", 6400000));
        assert_eq!(asset.sha256.as_deref(), Some("ab".repeat(32).as_str())); // 소문자로 맞춘다

        // 다른 곳의 주소, 경로가 섞인 이름은 받지 않는다. 확인값이 이상하면 없는 것으로 친다.
        let odd = |name: &str, url: &str, digest: &str| {
            parse_release(&json!({ "tag_name": "v0.16.0", "assets": [{ "name": name, "browser_download_url": url, "size": 1, "digest": digest }] })).unwrap().asset
        };
        let good = "https://github.com/sukmin/cypher/releases/download/v0.16.0/cypher-0.16.0-windows-x64.zip";
        assert!(odd("cypher-0.16.0-windows-x64.zip", "https://example.com/cypher-0.16.0-windows-x64.zip", &digest).is_none());
        assert!(odd("cypher-0.16.0-windows-x64.zip", "https://github.com/other/cypher/releases/download/v1/cypher-0.16.0-windows-x64.zip", &digest).is_none());
        assert!(odd("..\\cypher-0.16.0-windows-x64.zip", good, &digest).is_none());
        assert_eq!(odd("cypher-0.16.0-windows-x64.zip", good, "sha256:1234").unwrap().sha256, None);
        assert_eq!(odd("cypher-0.16.0-windows-x64.zip", good, "md5:abcd").unwrap().sha256, None);
        // 버전을 읽을 수 없는 릴리즈는 받지 않는다.
        assert!(parse_release(&json!({ "tag_name": "nightly" })).is_err());
        assert!(parse_release(&json!({ "message": "Not Found" })).is_err());
    }

    #[test]
    fn 파일을_바꾸고_옛_파일은_이름을_바꿔_둔다() {
        let root = test_dir("cypher-update-replace-test");
        let (from, to) = (root.join("new"), root.join("app"));
        std::fs::create_dir_all(from.join("sub")).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        // 내용은 누가 봐도 글인 한 문장으로 쓴다(몇 바이트짜리 내용은 백신이 잡은 적이 있다).
        let text = |what: &str| format!("Cypher 단위 시험이 만든 글자 파일입니다: {what}\n");
        for (name, what) in [("main.txt", "새 본체"), ("bridge.txt", "새 전달"), ("README.md", "새 글")] {
            std::fs::write(from.join(name), text(what)).unwrap();
        }
        std::fs::write(from.join("sub").join("x.txt"), text("폴더 안의 것은 놓지 않는다")).unwrap();
        std::fs::write(to.join("main.txt"), text("옛 본체")).unwrap();
        std::fs::write(to.join("config.txt"), text("그대로")).unwrap();

        let mut placed = replace_files(&from, &to, "77").unwrap();
        placed.sort();
        assert_eq!(placed, ["README.md", "bridge.txt", "main.txt"]);
        let read = |name: &str| std::fs::read_to_string(to.join(name)).unwrap();
        assert_eq!((read("main.txt"), read("bridge.txt"), read("README.md")), (text("새 본체"), text("새 전달"), text("새 글")));
        assert_eq!(read("main.txt.old-77"), text("옛 본체")); // 옛 파일은 이름만 바뀌어 남는다
        assert_eq!(read("config.txt"), text("그대로"));
        assert!(!to.join("sub").exists());
        done(&root);
    }

    #[test]
    fn 바꾸다_실패하면_되돌린다() {
        let root = test_dir("cypher-update-rollback-test");
        let (from, to) = (root.join("new"), root.join("app"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        // 내용은 누가 봐도 글인 한 문장으로 쓴다. 5바이트짜리 "새 a"는 이름이 `.bin`이고 빌드 폴더에 있어도 백신이 잡았다.
        let text = |what: &str| format!("Cypher 단위 시험이 만든 글자 파일입니다: {what}\n");
        std::fs::write(from.join("a.txt"), text("새 a")).unwrap();
        std::fs::write(from.join("b.txt"), text("새 b")).unwrap();
        std::fs::write(to.join("a.txt"), text("옛 a")).unwrap();
        // 옛 b.txt가 옮겨 갈 이름의 자리에 폴더를 두어 이름 바꾸기가 실패하게 한다.
        std::fs::write(to.join("b.txt"), text("옛 b")).unwrap();
        std::fs::create_dir_all(to.join("b.txt.old-9").join("안")).unwrap();

        assert!(replace_files(&from, &to, "9").is_err());
        let read = |name: &str| std::fs::read_to_string(to.join(name)).unwrap();
        assert_eq!((read("a.txt"), read("b.txt")), (text("옛 a"), text("옛 b")));
        assert!(!to.join("a.txt.old-9").exists());
        done(&root);
    }

    #[test]
    fn 묶음을_풀어_실행_파일이_든_폴더를_찾는다() {
        // 실행 파일 자리에는 다른 이름의 글자 파일을 쓴다(찾는 이름만 바꿔 같은 길을 본다).
        let names = ["main.txt", "bridge.txt"];
        let root = test_dir("cypher-update-unzip-test");
        let inner = root.join("src").join("cypher-9.9.9-windows-x64");
        std::fs::create_dir_all(&inner).unwrap();
        let text = |what: &str| format!("Cypher 단위 시험이 만든 글자 파일입니다: {what}\n");
        std::fs::write(inner.join("main.txt"), text("본체")).unwrap();
        std::fs::write(inner.join("bridge.txt"), text("전달")).unwrap();
        // Windows에 든 tar로 zip을 만든다(-a: 이름의 확장자에 맞는 형식).
        let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("tar.exe");
        // 작업 폴더의 이름에 시스템 코드 페이지에 없을 글자를 넣는다(영어 Windows의 한글 사용자 이름 같은 경우).
        // tar에 전체 경로를 주면 이런 폴더를 찾지 못한다.
        let work = root.join("작업 tést ทดสอบ");
        std::fs::create_dir_all(&work).unwrap();
        let zip = work.join("pack.zip");
        let made = Command::new(&tar).current_dir(&work).arg("-a").arg("-cf").arg("pack.zip").arg("-C").arg(root.join("src")).arg("cypher-9.9.9-windows-x64").output().unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));

        let unpacked = work.join("unpacked");
        unzip(&zip, &unpacked).unwrap();
        let found = find_package(&unpacked, &names).unwrap();
        assert_eq!(found, unpacked.join("cypher-9.9.9-windows-x64"));
        assert_eq!(std::fs::read_to_string(found.join("main.txt")).unwrap(), text("본체"));
        // 파일이 든 폴더를 바로 가리켜도 찾는다.
        assert_eq!(find_package(&found, &names).unwrap(), found);
        // 하나라도 빠진 묶음, 실행 파일이 없는 묶음은 받지 않는다(진짜 묶음이 찾는 이름은 실행 파일 둘이다).
        assert!(find_package(&unpacked, &["main.txt", "없는 것.txt"]).is_err());
        assert!(package_dir(&unpacked).is_err());
        assert_eq!(PROGRAMS, ["cypher.exe", "cypher-bridge.exe"]);
        let empty = root.join("empty");
        std::fs::create_dir_all(empty.join("x")).unwrap();
        assert!(package_dir(&empty).is_err());
        // zip이 아닌 파일은 풀지 못한다.
        std::fs::write(root.join("bad.zip"), text("zip이 아닌 파일")).unwrap();
        assert!(unzip(&root.join("bad.zip"), &root.join("bad")).is_err());
        // 묶음이 든 폴더 밖에는 풀지 않는다.
        assert!(unzip(&zip, &root.join("밖")).is_err());
        done(&root);
    }

    #[test]
    fn 업데이트가_남긴_옛_파일만_치울_것으로_본다() {
        for ours in ["cypher.exe.old-1794800000", "README.md.old-1794800000", "a.old-1.txt.old-1794800000"] {
            assert!(is_leftover(ours), "{ours}");
        }
        // 실행 파일 폴더에 함께 있을 수 있는 사용자의 파일.
        for theirs in ["보고서.old-2024.docx", "config.old-1.json", "notes.old-2024", ".old-1794800000", "cypher.exe.old-", "cypher.exe.old-17948x0000", "cypher.exe"] {
            assert!(!is_leftover(theirs), "{theirs}");
        }
    }

    /// 실제 GitHub에서 최신 릴리즈를 받아 시험용 폴더에 설치해 본다(인터넷이 필요해 평소에는 돌리지 않는다).
    #[test]
    #[ignore]
    fn 실제_릴리즈를_받아_시험용_폴더에_설치한다() {
        let release = latest().unwrap();
        let asset = release.asset.expect("묶음이 있어야 한다");
        assert!(asset.sha256.is_some(), "확인값이 있어야 한다");
        let root = test_dir("cypher-update-real-test");
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        // 옛 본체 자리에는 진짜 실행 파일(이 시험 프로그램의 사본)을 둔다. 내용이 실행 파일이 아닌 `.exe`는 만들지 않는다.
        std::fs::copy(std::env::current_exe().unwrap(), app.join("cypher.exe")).unwrap();
        // 내려받는 동안 받은 양을 알려 오고, 마지막에는 전체 크기와 같아야 한다.
        let seen = std::cell::RefCell::new(Vec::new());
        let report = |stage: &'static str, done: u64, total: u64| seen.borrow_mut().push((stage, done, total));
        let placed = install(&asset, &app, &root.join("work"), &report).unwrap();
        let seen = seen.into_inner();
        assert_eq!(seen.first(), Some(&("download", 0, asset.size)));
        assert!(seen.contains(&("download", asset.size, asset.size)), "{:?}", seen.last());
        assert_eq!(seen[seen.len() - 2..].iter().map(|step| step.0).collect::<Vec<_>>(), ["unpack", "replace"]);
        assert!(placed.iter().any(|name| name == "cypher.exe") && placed.iter().any(|name| name == "cypher-bridge.exe"), "{placed:?}");
        assert!(std::fs::metadata(app.join("cypher.exe")).unwrap().len() > 1_000_000);
        assert!(std::fs::read_dir(&app).unwrap().flatten().any(|entry| entry.file_name().to_string_lossy().starts_with("cypher.exe.old-")));
        // 확인값이 다르면 설치하지 않는다.
        let wrong = Asset { sha256: Some("0".repeat(64)), ..asset };
        let other = root.join("app2");
        std::fs::create_dir_all(&other).unwrap();
        assert!(install(&wrong, &other, &root.join("work2"), &|_, _, _| {}).unwrap_err().contains("확인값"));
        assert!(std::fs::read_dir(&other).unwrap().next().is_none());
        done(&root);
    }
}
