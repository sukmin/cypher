//! 브라우저 엔진의 버전 확인: 지금 쓰는 Chromium이 최신 안정판에서 얼마나 뒤처졌는지 알려 준다.
//!
//! Cypher는 Chromium을 직접 싣지 않는다. 두 엔진 모두 바깥의 브라우저가 스스로 갱신한다.
//! - `webview`: WebView2 런타임(Edge와 함께 Windows가 갱신)
//! - `chrome`: 설치된 Chrome·Edge(브라우저 자체의 자동 업데이트)
//!
//! 그래서 여기서 하는 일은 "갱신이 멈춰 있지 않은지" 살피는 것이다. 도움말 메뉴에서 직접 확인할 수 있고,
//! 브라우저를 열 때 일주일에 한 번 뒤에서 확인해 두 단계 이상 뒤처졌으면 알린다.
//! 최신 버전은 Google이 공개하는 Chrome for Testing 목록에서 읽는다(보내는 정보는 없다).

use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

const LATEST_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json";
const CHECK_EVERY: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// 주 버전이 이만큼 이상 뒤처지면 알린다(안정판은 약 4주마다 한 단계 오른다).
const BEHIND_LIMIT: u32 = 2;

#[derive(Clone, Serialize)]
pub struct EngineInfo {
    /// "webview" 또는 "chrome"
    engine: String,
    /// 화면에 보일 이름(예: "WebView2 런타임", "Google Chrome").
    browser: String,
    /// Chromium 버전(예: 157.0.8103.52). 알 수 없으면 빈 글.
    version: String,
    /// 실행 파일(chrome 엔진일 때).
    path: String,
}

/// 버전 글자의 주 버전(맨 앞 숫자).
pub fn major(version: &str) -> Option<u32> {
    version.trim().split('.').next()?.parse().ok()
}

/// 실행 파일 옆의 버전 이름 폴더(`157.0.8103.52`)에서 설치된 버전을 읽는다. 여러 개면 가장 높은 것.
fn installed_version(program: &Path) -> Option<String> {
    let numbers = |name: &str| name.split('.').map(|part| part.parse::<u32>().ok()).collect::<Option<Vec<u32>>>();
    std::fs::read_dir(program.parent()?)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            numbers(&name).filter(|parts| parts.len() == 4).map(|parts| (parts, name))
        })
        .max()
        .map(|(_, name)| name)
}

fn browser_name(program: &Path) -> &'static str {
    match program.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("msedge") => "Microsoft Edge",
        Some("brave") => "Brave",
        Some("chrome") if program.to_string_lossy().to_ascii_lowercase().contains("chromium") => "Chromium",
        Some("chrome") => "Google Chrome",
        _ => "Chromium 계열 브라우저",
    }
}

/// 지금 설정된 엔진과 그 버전.
pub fn info(app: &AppHandle) -> EngineInfo {
    let settings = &app.state::<crate::AppState>().config.browser;
    if settings.engine.eq_ignore_ascii_case("chrome") {
        return match crate::chrome::find_browser(&settings.chrome_path) {
            Ok(program) => EngineInfo {
                engine: "chrome".into(),
                browser: browser_name(&program).into(),
                // 떠 있으면 실제로 돌고 있는 버전을, 아니면 설치된 버전을 알려 준다.
                version: crate::chrome::running_version().or_else(|| installed_version(&program)).unwrap_or_default(),
                path: program.to_string_lossy().into_owned(),
            },
            Err(problem) => EngineInfo { engine: "chrome".into(), browser: problem, version: String::new(), path: String::new() },
        };
    }
    EngineInfo {
        engine: "webview".into(),
        browser: "WebView2 런타임".into(),
        version: tauri::webview_version().unwrap_or_default(),
        path: String::new(),
    }
}

/// Chrome 최신 안정판의 버전(Google이 공개하는 목록에서 읽는다).
pub fn latest_stable() -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let body = agent
        .get(LATEST_URL)
        .set("User-Agent", "Cypher")
        .call()
        .map_err(|e| format!("최신 버전을 확인하지 못했습니다: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    json["channels"]["Stable"]["version"].as_str().map(str::to_string).ok_or_else(|| "최신 버전 정보의 형식이 바뀌었습니다".to_string())
}

#[tauri::command]
pub async fn browser_engine_info(app: AppHandle) -> EngineInfo {
    info(&app)
}

#[tauri::command]
pub async fn browser_latest_version() -> Result<String, String> {
    latest_stable()
}

/// 브라우저를 열 때 부른다: 마지막 확인에서 일주일이 지났으면 뒤에서 버전을 확인하고,
/// 두 단계 이상 뒤처졌으면 본창에 알린다. 설정에서 끌 수 있다.
pub fn check_in_background(app: &AppHandle) {
    // 한 번 실행하는 동안에는 한 번만 본다(브라우저를 여러 번 열어도 확인과 알림이 겹치지 않게).
    static CHECKED: AtomicBool = AtomicBool::new(false);
    if !app.state::<crate::AppState>().config.browser.update_check || CHECKED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(stamp) = crate::data_dir().map(|dir| dir.join("engine-check")) else { return };
    let recent = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .is_some_and(|age| age < CHECK_EVERY);
    if recent {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // 브라우저가 다 뜬 뒤에(실제로 돌고 있는 버전을 읽을 수 있을 때) 확인한다.
        std::thread::sleep(Duration::from_secs(5));
        let Ok(latest) = latest_stable() else { return }; // 다음에 열 때 다시 해 본다.
        let _ = std::fs::write(&stamp, &latest);
        let current = info(&app);
        if let (Some(now), Some(newest)) = (major(&current.version), major(&latest)) {
            if newest >= now + BEHIND_LIMIT {
                crate::log(&format!("브라우저 엔진이 오래됐습니다: {} {} (최신 안정판 {latest})", current.browser, current.version));
                let _ = app.emit_to("main", "browser-outdated", serde_json::json!({ "info": current, "latest": latest }));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 주_버전을_읽는다() {
        assert_eq!(major("157.0.8103.52"), Some(157));
        assert_eq!(major(" 99 "), Some(99));
        assert_eq!(major(""), None);
        assert_eq!(major("Chrome/157"), None);
    }

    #[test]
    fn 설치된_버전은_가장_높은_버전_폴더() {
        let dir = std::env::temp_dir().join("cypher-engine-test");
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["156.0.8000.99", "157.0.8103.52", "157.0.8103.7", "SetupMetrics", "1.2.3"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        let program = dir.join("chrome.exe");
        assert_eq!(installed_version(&program).as_deref(), Some("157.0.8103.52"));
        assert_eq!(browser_name(&program), "Google Chrome");
        assert_eq!(browser_name(Path::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")), "Microsoft Edge");
    }
}
