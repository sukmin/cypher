#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod chrome;
mod claude;
mod clipboard;
mod commands;
mod drops;
mod engine;
mod extras;
mod http;
mod image;
mod magnet;
mod mcp;
mod memo;
mod memory;
mod site;
mod monitor;
mod status;

use std::collections::HashMap;
use std::ffi::OsString;
use std::hash::{BuildHasher, Hasher};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use cypher_core::config::Config;
use cypher_pty::PtySession;

pub struct AppState {
    pub sessions: Mutex<HashMap<u32, Arc<PtySession>>>,
    pub next_id: AtomicU32,
    /// 상태줄 전달 프로그램이 접속하는 로컬 포트와 확인용 값.
    pub status_port: u16,
    pub token: String,
    /// Claude 세션의 상태줄·훅에 넣을 전달 프로그램 호출문. 전달 프로그램이 없으면 None.
    pub bridge: Option<String>,
    /// Claude 세션에 `--mcp-config`로 넘길 파일(브라우저 조작 도구). 꺼져 있으면 None.
    pub mcp_config: Option<PathBuf>,
    pub config: Config,
    pub config_error: Option<String>,
    pub config_path: PathBuf,
}

/// `%LOCALAPPDATA%\cypher` — 임시 파일과 내부 상태.
pub fn data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("cypher"))
}

/// `%APPDATA%\cypher` — 사용자의 것(설정, 메모). 버전이 바뀌어도 그대로 둔다.
pub fn user_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("cypher")
}

/// `%APPDATA%\cypher\config.toml` — 사용자가 고치는 설정.
fn config_path() -> PathBuf {
    user_dir().join("config.toml")
}

/// 문제 확인용 기록을 `%LOCALAPPDATA%\cypher\cypher.log`에 한 줄 덧붙인다.
pub fn log(text: &str) {
    use std::io::Write;
    let Some(dir) = data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("cypher.log")) {
        let _ = writeln!(file, "{} {text}", http::now_text());
    }
}

/// 예전 실행이 남긴 연결 설정 파일(`claude-settings-<번호>.json`, `mcp-<번호>.json`)을 치운다.
/// 실행마다 파일을 따로 쓰는 것은 여러 창을 동시에 띄워도 서로의 접속 정보를 덮어쓰지 않게 하려는 것이다.
/// 다른 창이 아직 쓰고 있을 수 있으므로 이틀 넘은 것만 지운다.
fn clean_old_settings() {
    let Some(entries) = data_dir().and_then(|dir| std::fs::read_dir(dir).ok()) else { return };
    let limit = std::time::Duration::from_secs(2 * 24 * 60 * 60);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = (name.starts_with("claude-settings") || name.starts_with("mcp")) && name.ends_with(".json");
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > limit);
        if ours && old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn random_token() -> String {
    let a = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let b = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{a:016x}{b:016x}")
}

/// 공백 없는 짧은 경로(8.3 이름)를 얻는다. 드라이브에서 꺼져 있으면 원래 경로가 그대로 온다.
fn short_path(path: &Path) -> Option<PathBuf> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    // SAFETY: 두 버퍼 모두 유효하고, 길이를 함께 넘긴다.
    let len = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(OsString::from_wide(&buf[..len])))
}

/// 상태줄·훅 명령으로 쓸 전달 프로그램 호출문.
/// Claude Code가 어떤 셸로 실행하든 통하도록 슬래시 경로를 쓰고, 공백이 있으면
/// 짧은 경로로 바꾼다. 그래도 공백이 남으면 따옴표로 감싼다(bash·cmd에서 동작).
fn bridge_command(bridge: &Path) -> String {
    let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let mut command = slash(bridge);
    if command.contains(' ') {
        if let Some(short) = short_path(bridge) {
            command = slash(&short);
        }
    }
    if command.contains(' ') {
        command = format!("\"{command}\"");
    }
    command
}

/// 상태줄·훅 명령으로 쓸 전달 프로그램 호출문. 전달 프로그램이 실행 파일 옆에 없으면 None.
fn bridge_for_claude() -> Option<String> {
    let bridge = std::env::current_exe().ok()?.with_file_name("cypher-bridge.exe");
    bridge.exists().then(|| bridge_command(&bridge))
}

/// Claude 세션에 브라우저 조작 도구를 연결하는 MCP 설정 파일을 만든다(`--mcp-config`로 넘긴다).
/// 전달 프로그램이 MCP 서버 노릇을 하며 이 앱으로 요청을 나른다.
fn write_mcp_config(port: u16, token: &str) -> Option<PathBuf> {
    let bridge = std::env::current_exe().ok()?.with_file_name("cypher-bridge.exe");
    if !bridge.exists() {
        return None;
    }
    let json = serde_json::json!({
        "mcpServers": {
            "cypher": {
                "command": bridge.to_string_lossy(),
                "args": ["mcp"],
                "env": { "CYPHER_PORT": port.to_string(), "CYPHER_TOKEN": token }
            }
        }
    });
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("mcp-{}.json", std::process::id()));
    std::fs::write(&path, json.to_string()).ok()?;
    Some(path)
}

/// 본창 상단 메뉴. 사이드바에 두지 않는 기능을 모은다.
/// 단축키는 화면 쪽에서 처리하므로 여기서는 안내 글자로만 붙인다(등록하면 두 번 실행된다).
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    let item = |id: &str, text: &str| MenuItemBuilder::with_id(id, text).build(app);
    let tools = SubmenuBuilder::new(app, "도구")
        .item(&item("snippets", "자주 쓰는 지시\tCtrl+Shift+P")?)
        .item(&item("broadcast", "동시 입력 켜기/끄기\tCtrl+Shift+B")?)
        .item(&item("memory", "기억 내보내기·가져오기")?)
        .build()?;
    let settings = SubmenuBuilder::new(app, "설정")
        .item(&item("settings", "Cypher 설정 파일 열기")?)
        .separator()
        .item(&item("claude-settings", "Claude 공통 설정 열기 (settings.json)")?)
        .item(&item("claude-memory", "Claude 공통 지침 열기 (CLAUDE.md)")?)
        .build()?;
    let help = SubmenuBuilder::new(app, "도움말")
        .item(&item("help", "단축키\tF1")?)
        .item(&item("engine", "브라우저 엔진 정보·업데이트 확인")?)
        .build()?;
    MenuBuilder::new(app).items(&[&tools, &settings, &help]).build()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // 본창을 닫으면 브라우저 창이 남아 있어도 앱을 끝낸다.
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                chrome::shutdown(); // "chrome" 엔진으로 띄워 둔 브라우저도 함께 닫는다.
                window.app_handle().exit(0);
            }
        })
        .setup(|app| {
            let token = random_token();
            let status_port = status::start(app.handle().clone(), token.clone())?;
            clipboard::clean_old_pastes();
            clean_old_settings();
            memo::clean_closed();
            image::clean_closed();
            let config_path = config_path();
            let (config, config_error) = Config::load(&config_path);
            let mcp_config = if config.browser.enabled { write_mcp_config(status_port, &token) } else { None };
            // 어느 판을 쓰고 있는지 알 수 있게 창 제목에 버전을 붙인다.
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.set_title(concat!("Cypher ", env!("CARGO_PKG_VERSION")));
                main.set_menu(build_menu(app.handle())?)?;
            }
            // 메뉴에서 고른 항목은 본창 화면이 실행한다.
            app.on_menu_event(|app, event| {
                let _ = app.emit_to("main", "menu", event.id().as_ref());
            });
            if config.window.magnet {
                if let Some(Ok(hwnd)) = app.get_webview_window("main").map(|w| w.hwnd()) {
                    magnet::attach(hwnd.0 as isize, config.window.magnet_distance);
                }
            }
            app.manage(AppState {
                sessions: Mutex::new(HashMap::new()),
                next_id: AtomicU32::new(1),
                status_port,
                token,
                bridge: bridge_for_claude(),
                mcp_config,
                config,
                config_error,
                config_path,
            });
            monitor::start(app.handle().clone());
            drops::watch(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_config,
            commands::open_config,
            commands::open_claude_file,
            commands::home_dir,
            commands::autostart,
            commands::notify,
            commands::pick_folder,
            commands::session_create,
            commands::session_write,
            commands::session_resize,
            commands::session_pause,
            commands::session_close,
            commands::link_detect,
            commands::link_open,
            extras::git_changes,
            extras::git_diff,
            extras::save_text,
            memory::memory_info,
            memory::memory_export,
            memory::memory_import,
            memory::memory_open,
            memo::memo_read,
            memo::memo_write,
            memo::memo_remove,
            memo::text_open,
            memo::text_save,
            site::site_open,
            site::site_bounds,
            site::site_navigate,
            site::site_action,
            site::site_close,
            site::site_capture,
            image::image_import,
            image::image_pick,
            image::image_paste,
            image::image_store,
            image::image_fetch,
            image::image_read,
            image::image_info,
            image::image_remove,
            image::image_export,
            image::image_copy,
            drops::drop_classify,
            browser::browser_capture,
            clipboard::clipboard_read,
            clipboard::paste_preview,
            clipboard::clipboard_write,
            http::http_files,
            http::http_read,
            http::http_save,
            http::http_run_at,
            browser::browser_open,
            browser::browser_where,
            engine::browser_engine_info,
            engine::browser_latest_version,
            browser::browser_navigate,
            browser::browser_back,
            browser::browser_forward,
            browser::browser_reload,
            browser::browser_current,
            browser::browser_handoff,
            browser::browser_page,
            browser::browser_bar_height,
        ])
        .run(tauri::generate_context!())
        .expect("Cypher를 실행하지 못했습니다");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 전달_프로그램_호출문에는_역슬래시가_없다() {
        let command = bridge_command(Path::new(r"C:\tools\cypher\cypher-bridge.exe"));
        assert_eq!(command, "C:/tools/cypher/cypher-bridge.exe");
    }

    #[test]
    fn 공백이_남으면_따옴표로_감싼다() {
        // 존재하지 않는 경로라 짧은 경로로 바꿀 수 없다.
        let command = bridge_command(Path::new(r"C:\없는 폴더\cypher-bridge.exe"));
        assert_eq!(command, "\"C:/없는 폴더/cypher-bridge.exe\"");
    }
}
