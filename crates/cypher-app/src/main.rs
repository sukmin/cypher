#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod chrome;
mod claude;
mod clipboard;
mod closed;
mod commands;
mod created;
mod drops;
mod engine;
mod extras;
mod http;
mod image;
mod magnet;
mod mcp;
mod memo;
mod memory;
mod mindmap;
mod site;
mod monitor;
mod paths;
mod peer;
mod reader;
mod status;
mod template;

use std::collections::HashMap;
use std::ffi::OsString;
use std::hash::{BuildHasher, Hasher};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
    /// Claude 세션에 Cypher 도구(MCP)를 연결할지. 세션마다 `--mcp-config` 파일을 따로 만든다.
    pub mcp: bool,
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

/// 전달 프로그램이 접속할 때 내는 확인값(128비트). 운영체제의 난수로 만든다.
fn random_token() -> String {
    #[link(name = "advapi32")]
    extern "system" {
        #[link_name = "SystemFunction036"]
        fn rtl_gen_random(buffer: *mut u8, len: u32) -> u8;
    }
    let mut bytes = [0u8; 16];
    // SAFETY: 길이가 맞는 유효한 버퍼를 넘긴다.
    if unsafe { rtl_gen_random(bytes.as_mut_ptr(), bytes.len() as u32) } != 0 {
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    // 난수를 얻지 못하면(거의 없다) 실행마다 달라지는 해시 씨앗으로 대신한다.
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
/// Claude Code가 어떤 셸로 실행하든 통하도록 슬래시 경로를 쓰고, 셸이 달리 읽는 글자(공백, 괄호, `&`, `;`, `'` 등)가
/// 있으면 짧은 경로로 바꾼다. 그래도 남으면 따옴표로 감싼다(bash·cmd에서 동작).
fn bridge_command(bridge: &Path) -> String {
    let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let mut command = slash(bridge);
    if !is_plain_for_shell(&command) {
        if let Some(short) = short_path(bridge) {
            command = slash(&short);
        }
    }
    if !is_plain_for_shell(&command) {
        command = format!("\"{command}\"");
    }
    command
}

/// 따옴표 없이 적어도 셸이 한 낱말로 읽는 경로인지: 글자·숫자(한글 포함)와 `/ : . _ - ~`뿐일 때.
fn is_plain_for_shell(command: &str) -> bool {
    command.chars().all(|c| c.is_alphanumeric() || "/:._-~".contains(c))
}

/// 상태줄·훅 명령으로 쓸 전달 프로그램 호출문. 전달 프로그램이 실행 파일 옆에 없으면 None.
fn bridge_for_claude() -> Option<String> {
    let bridge = std::env::current_exe().ok()?.with_file_name("cypher-bridge.exe");
    bridge.exists().then(|| bridge_command(&bridge))
}

/// Claude 세션에 Cypher 도구를 연결하는 MCP 설정 파일을 만든다(`--mcp-config`로 넘긴다).
/// 전달 프로그램이 MCP 서버 노릇을 하며 이 앱으로 요청을 나른다. 어느 세션이 부른 도구인지 알 수 있게
/// (같은 그룹의 브라우저·항목을 찾는다) 세션 번호를 넣어 세션마다 파일을 따로 만든다.
pub fn write_mcp_config(port: u16, token: &str, session: u32) -> Option<PathBuf> {
    let bridge = std::env::current_exe().ok()?.with_file_name("cypher-bridge.exe");
    if !bridge.exists() {
        return None;
    }
    let json = serde_json::json!({
        "mcpServers": {
            "cypher": {
                "command": bridge.to_string_lossy(),
                "args": ["mcp"],
                "env": { "CYPHER_PORT": port.to_string(), "CYPHER_TOKEN": token, "CYPHER_SESSION": session.to_string() }
            }
        }
    });
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("mcp-{}-{session}.json", std::process::id()));
    std::fs::write(&path, json.to_string()).ok()?;
    Some(path)
}

/// 본창 상단 메뉴. 사이드바에 두지 않는 기능을 모은다.
/// 단축키는 화면 쪽에서 처리하므로 여기서는 안내 글자로만 붙인다(등록하면 두 번 실행된다).
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    let item = |id: &str, text: &str| MenuItemBuilder::with_id(id, text).build(app);
    let tools = SubmenuBuilder::new(app, "도구")
        .item(&item("palette", "빠른 이동·명령 찾기\tCtrl+Shift+K")?)
        .item(&item("reopen", "닫은 항목 다시 열기…\tCtrl+Shift+R")?)
        .item(&item("notices", "알림 기록\tCtrl+Shift+A")?)
        .item(&item("templates", "그룹 틀: 저장·불러오기…")?)
        .separator()
        .item(&item("snippets", "자주 쓰는 지시\tCtrl+Shift+P")?)
        .item(&item("broadcast", "동시 입력 켜기/끄기\tCtrl+Shift+B")?)
        .item(&item("memory", "기억 내보내기·가져오기")?)
        .build()?;
    let settings = SubmenuBuilder::new(app, "설정")
        .item(&item("look", "모양: 테마·글꼴·글자 크기…")?)
        .item(&item("home", "브라우저 시작 페이지…")?)
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

/// 본창을 닫아 달라는 요청을 받아 화면에 저장할 틈을 주고 있는지.
static CLOSING: AtomicBool = AtomicBool::new(false);
/// 화면이 저장을 마쳤다고 알려 오기를 기다리는 시간. 넘으면 그냥 닫는다.
const CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// 화면이 쓰던 것을 모두 저장했다: 본창을 닫는다(닫히면 앱이 끝난다).
#[tauri::command]
fn app_close(app: tauri::AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.destroy();
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // 본창을 닫으면 앱을 끝낸다.
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                // 닫으려 할 때 바로 닫지 않고 화면에 알린다. 화면은 방금 친 글(자동 저장을 기다리던 메모·요청 파일·브레인스토밍)과
                // 배치를 저장한 뒤 `app_close`로 닫는다. 그러지 않으면 닫기 직전에 친 글이 사라진다.
                // 화면이 답하지 않으면 잠시 뒤에 그냥 닫고, 그 전에 다시 닫으려 하면 바로 닫는다.
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if !CLOSING.swap(true, Ordering::SeqCst) {
                        api.prevent_close();
                        let _ = window.emit_to("main", "app-closing", ());
                        let window = window.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(CLOSE_WAIT);
                            let _ = window.destroy();
                        });
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    chrome::shutdown(); // "chrome" 엔진으로 띄워 둔 브라우저도 함께 닫는다.
                    window.app_handle().exit(0);
                }
                _ => {}
            }
        })
        .setup(|app| {
            let token = random_token();
            let status_port = status::start(app.handle().clone(), token.clone())?;
            clipboard::clean_old_pastes();
            clean_old_settings();
            memo::clean_closed();
            mindmap::clean_closed();
            image::clean_closed();
            let config_path = config_path();
            let (config, config_error) = Config::load(&config_path);
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
                mcp: config.browser.enabled,
                config,
                config_error,
                config_path,
            });
            // 그룹별 브라우저 프로필이 한 폴더로 합쳐지지 않게 한다(본창의 웹뷰가 만들어진 뒤여야 한다).
            site::adopt_data_folder_override(app.handle());
            monitor::start(app.handle().clone());
            drops::watch(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_close,
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
            extras::git_revert,
            extras::git_commit,
            reader::reader_pick,
            reader::reader_read,
            reader::reader_stamp,
            template::template_list,
            template::template_save,
            template::template_read,
            template::template_remove,
            template::template_folder,
            template::dirs_exist,
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
            site::site_profile,
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
            mcp::mcp_reply,
            mindmap::mindmap_read,
            mindmap::mindmap_write,
            mindmap::mindmap_remove,
            mindmap::mindmap_export,
            mindmap::mindmap_pdf,
            commands::store_dirs,
            closed::closed_restore,
            commands::ui_zoom,
            engine::browser_engine_info,
            engine::browser_latest_version,
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

    #[test]
    fn 셸이_달리_읽는_글자가_있으면_따옴표로_감싼다() {
        // 공백이 없어도 괄호·&·;·작은따옴표는 셸이 낱말을 끊거나 달리 읽는다(없는 경로라 짧은 경로로는 못 바꾼다).
        for path in [r"D:\없는(new)\cypher-bridge.exe", r"D:\없는&곳\cypher-bridge.exe", r"D:\없는;곳\cypher-bridge.exe", r"D:\it's\cypher-bridge.exe"] {
            let command = bridge_command(Path::new(path));
            assert!(command.starts_with('"') && command.ends_with('"'), "{command}");
        }
        // 한글·숫자·점·밑줄·붙임표뿐인 경로는 그대로 쓴다.
        assert_eq!(bridge_command(Path::new(r"D:\도구_v1.2-b\cypher-bridge.exe")), "D:/도구_v1.2-b/cypher-bridge.exe");
    }
}
