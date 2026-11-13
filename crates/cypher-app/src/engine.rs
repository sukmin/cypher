//! 브라우저 엔진의 버전 확인: 지금 쓰는 Chromium이 최신 안정판에서 얼마나 뒤처졌는지 알려 준다.
//!
//! Cypher는 Chromium을 직접 싣지 않는다. 두 엔진 모두 바깥의 브라우저가 스스로 갱신한다.
//! - `webview`: WebView2 런타임(Edge와 함께 Windows가 갱신)
//! - `chrome`: 설치된 Chrome·Edge(브라우저 자체의 자동 업데이트)
//!
//! 그래서 여기서 하는 일은 "갱신이 멈춰 있지 않은지" 살피는 것이다. 사용자가 도움말 메뉴의 엔진 정보 창에서
//! [최신 버전 확인]을 눌렀을 때만 인터넷에서 최신 버전을 읽는다(Cypher가 스스로 확인하러 나가지 않는다).
//! 최신 버전은 Google이 공개하는 Chrome for Testing 목록에서 읽는다(보내는 정보는 없다).

use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const LATEST_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json";

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

#[cfg(test)]
mod tests {
    use super::*;

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
