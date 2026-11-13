//! 사이트 항목(사이드바의 브라우저): 웹 페이지를 본창 안(그 항목의 창 자리)에 띄운다.
//!
//! 많은 사이트가 다른 페이지 안에 끼워 넣는 것(iframe)을 막아 두었기 때문에, 화면 요소로 넣지 않고
//! 본창에 웹뷰를 하나 더 붙여 그 항목의 창 자리에 겹쳐 놓는다. 화면 쪽이 자리(위치·크기)를 알려 주면
//! 그대로 옮기고, 그 창이 화면에 없거나 메뉴·대화상자가 열려 있는 동안에는 숨긴다.
//!
//! 같은 그룹의 Claude 세션은 이 웹뷰를 브라우저 도구로 조작할 수 있다(`browser.rs`). 화면에 보이지 않는
//! 사이트를 조작할 때는 웹뷰를 창 밖 자리에 잠깐 꺼내 놓는다(숨긴 웹뷰는 화면을 그리지 않아 찍을 수 없다).
//! 조작하는 중에 화면이 숨기라고 하면(메뉴·대화상자가 열림, 다른 항목으로 바뀜) 숨기는 대신 창 밖 자리로 옮긴다.
//!
//! 이 웹뷰가 여는 웹 페이지에는 어떤 앱 명령 권한도 주지 않는다.
//!
//! 로그인 상태(쿠키·저장소)는 **같은 그룹의 사이트 항목끼리만** 함께 쓴다. 그룹마다 브라우저 프로필(데이터 폴더
//! `%LOCALAPPDATA%\cypher\site-profiles\<프로필 번호>`)이 따로 있어서, 한 그룹에서 로그인해 둔 계정을 다른 그룹의
//! 사이트 항목이나 그 그룹의 Claude 세션이 쓸 수 없다. 그룹이 없는 사이트 항목은 본창과 같은 공용 프로필을 쓴다
//! (Claude 세션은 그룹 없는 사이트 항목을 조작할 수 없다). 항목이 다른 그룹으로 옮겨지면 웹뷰를 닫고
//! 새 그룹의 프로필로 다시 만든다.
//!
//! 어느 항목이 어느 프로필을 써야 하는지는 화면이 정해서 알려 준다(`site_profile`). 웹뷰를 만드는 쪽(화면의 `site_open`,
//! Claude의 도구 호출)은 그것을 바꾸지 못한다: 들고 온 프로필이 화면이 알려 준 것과 다르면(도는 사이에 항목이 다른 그룹으로
//! 옮겨졌다) 웹뷰를 만들지 않고 거절한다. 그러지 않으면 옮겨 간 항목이 옛 그룹의 로그인 상태로 되살아난다.
//! 화면이 알려 준 것이 없는 항목(닫힌 항목)의 웹뷰도 만들지 않는다.

use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// 화면 쪽이 마지막으로 알려 준 자리.
#[derive(Clone, Copy)]
struct Place {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
}

static PLACES: LazyLock<Mutex<HashMap<u32, Place>>> = LazyLock::new(Default::default);
/// 지금 떠 있는 웹뷰가 어느 프로필로 만들어졌는지: 사이트 번호 → 프로필 번호(공용이면 빈 글).
static PROFILES: LazyLock<Mutex<HashMap<u32, String>>> = LazyLock::new(Default::default);
/// 그 사이트 항목이 써야 하는 프로필(화면이 알려 준 것). 웹뷰는 이 프로필로만 만든다.
static ASSIGNED: LazyLock<Mutex<HashMap<u32, String>>> = LazyLock::new(Default::default);
/// 공용 프로필의 데이터 폴더를 따로 지정해야 할 때의 그 폴더(`adopt_data_folder_override` 참고). 평소에는 None.
static SHARED_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
/// 지금 조작하고 있는 사이트와, 그 사이트를 쓰고 있는 조작의 수(조작 중에는 숨기지 않는다).
/// 같은 사이트를 여러 조작이 함께 쓸 수 있다(Claude 세션마다 연결이 따로이고, 화면의 캡처도 있다).
/// 웹뷰를 꺼내 놓거나 숨기는 일은 이 잠금을 쥔 채로 한다(시작하는 조작과 끝나는 조작이 엇갈리지 않게).
static BUSY: LazyLock<Mutex<HashMap<u32, usize>>> = LazyLock::new(Default::default);
/// 사이트 웹뷰를 만들고 닫는 일은 한 번에 하나씩만 한다. 같은 사이트의 웹뷰를 화면과 도구가 동시에 만들지 않게 하고,
/// 한 웹뷰를 닫는 일이 같은 프로필의 다른 웹뷰를 만드는 도중에 끼어들지 않게 한다(그 프로필의 마지막 웹뷰가 닫히면
/// 엔진이 내려가는데, 그 순간에 만들고 있던 웹뷰는 끝내 만들어지지 않고 멈춘다).
static CREATE: Mutex<()> = Mutex::new(());

/// 웹뷰 엔진(WebView2)에 넘기는 실행 인자. 엔진이 사용자의 요청과 무관하게 뒤에서 하는 통신(구성 요소 갱신,
/// 안정성 보고 등)을 끈다. 앞의 `--disable-features=…`는 웹뷰 라이브러리의 기본값이다.
/// 같은 저장소를 쓰는 웹뷰는 인자가 같아야 하므로 본창(`tauri.conf.json`의 additionalBrowserArgs)과 똑같이 둔다.
pub const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,msImplicitSignin --disable-sync --disable-background-networking --disable-component-update --disable-domain-reliability";

/// 화면에 없는 사이트를 조작할 때 웹뷰를 꺼내 놓는 창 밖 자리와, 자리를 모를 때의 크기.
const PARKED_X: f64 = -30000.0;
const PARKED_SIZE: (f64, f64) = (1280.0, 860.0);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn label(id: u32) -> String {
    format!("site-{id}")
}

/// 프로필 번호는 화면이 만들어 건넨다. 폴더 이름으로 쓰기 전에 영문·숫자뿐인지 확인한다.
pub fn profile_key(key: &str) -> Result<&str, String> {
    if (1..=40).contains(&key.len()) && key.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(key)
    } else {
        Err("브라우저 프로필 번호가 올바르지 않습니다".into())
    }
}

/// 그 프로필의 데이터 폴더. 공용 프로필(빈 글)이면 본창과 같은 저장소(보통은 None = 엔진의 기본 폴더).
fn profile_dir(profile: &str) -> Result<Option<PathBuf>, String> {
    if profile.is_empty() {
        return Ok(SHARED_DIR.lock().unwrap().clone());
    }
    let dir = crate::data_dir().ok_or("데이터 폴더를 찾지 못했습니다")?.join("site-profiles").join(profile_key(profile)?);
    std::fs::create_dir_all(&dir).map_err(err)?;
    Ok(Some(dir))
}

/// 웹뷰 엔진은 환경 변수 `WEBVIEW2_USER_DATA_FOLDER`가 있으면 앱이 정한 데이터 폴더를 무시하고 그 폴더만 쓴다.
/// 그대로 두면 그룹별 프로필이 모두 그 한 폴더로 합쳐진다. 그래서 본창의 웹뷰가 만들어진 뒤에(본창은 그 폴더를
/// 그대로 쓴다) 이 프로세스에서 그 변수를 지우고, 공용 프로필(그룹 없는 사이트 항목)에는 그 폴더를 직접 지정한다.
/// 평소에는 이 변수가 없다. 시험할 때 본창의 저장소를 다른 곳으로 돌려 띄우는 경우에 해당한다.
/// (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`는 건드리지 않는다: 같은 폴더를 쓰는 엔진은 실행 인자가 같아야 한다.)
pub fn adopt_data_folder_override(app: &AppHandle) {
    const NAME: &str = "WEBVIEW2_USER_DATA_FOLDER";
    if app.get_webview_window("main").is_none() {
        return; // 본창이 아직 없으면 건드리지 않는다(본창이 엉뚱한 저장소를 쓰게 된다).
    }
    if let Some(folder) = std::env::var_os(NAME).filter(|folder| !folder.is_empty()) {
        *SHARED_DIR.lock().unwrap() = Some(PathBuf::from(folder));
        std::env::remove_var(NAME);
    }
}

fn place_of(id: u32) -> Option<Place> {
    PLACES.lock().unwrap().get(&id).copied()
}

fn is_visible(id: u32) -> bool {
    place_of(id).is_some_and(|place| place.visible)
}

/// 사이트 웹뷰의 주소·제목이 바뀌었을 때 화면에 알리는 내용.
#[derive(Clone, Serialize)]
struct Changed {
    id: u32,
    url: Option<String>,
    title: Option<String>,
}

fn apply(webview: &tauri::Webview, place: Place) {
    let busy = BUSY.lock().unwrap();
    if place.visible {
        let _ = webview.set_position(LogicalPosition::new(place.x, place.y));
        let _ = webview.set_size(LogicalSize::new(place.width.max(1.0), place.height.max(1.0)));
        let _ = webview.show();
    } else if busy.contains_key(&id_of(webview)) {
        // 조작 중이면 숨기지 않는다(숨긴 웹뷰는 화면을 그리지 않는다). 제자리에 두면 메뉴·대화상자나 그 자리에 온
        // 다른 항목을 덮으므로 창 밖 자리로 옮긴다. 조작이 끝나면 숨기고, 다시 보이게 되면 위에서 제자리에 놓는다.
        let _ = webview.set_position(LogicalPosition::new(PARKED_X, 0.0));
    } else {
        let _ = webview.hide();
    }
}

/// 조작 하나가 그 사이트를 쓰기 시작한다. 그 사이트를 쓰는 조작이 이것뿐이면 true.
fn enter(busy: &mut HashMap<u32, usize>, id: u32) -> bool {
    let count = busy.entry(id).or_default();
    *count += 1;
    *count == 1
}

/// 조작 하나가 그 사이트를 다 썼다. 그 사이트를 쓰는 조작이 더 없으면 true.
fn leave(busy: &mut HashMap<u32, usize>, id: u32) -> bool {
    match busy.get_mut(&id) {
        Some(count) if *count > 1 => {
            *count -= 1;
            false
        }
        _ => {
            busy.remove(&id);
            true
        }
    }
}

/// 조작하는 동안 들고 있는 것. 놓으면(조작이 패닉으로 끝나도) 수가 줄고, 그 사이트를 쓰는 조작이 더 없을 때만
/// 화면에 없는 웹뷰를 다시 숨긴다(먼저 끝난 조작이 아직 도는 조작의 웹뷰를 숨기지 않게).
struct Busy<'a>(&'a tauri::Webview);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        let id = id_of(self.0);
        let mut busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
        if leave(&mut busy, id) && !is_visible(id) {
            let _ = self.0.hide();
        }
    }
}

fn id_of(webview: &tauri::Webview) -> u32 {
    webview.label().strip_prefix("site-").and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// 도구가 도는 사이에 항목이 다른 그룹으로 옮겨졌을 때 Claude에게 돌려주는 말.
const MOVED: &str = "이 브라우저(사이트 항목)는 방금 다른 그룹으로 옮겨졌습니다. 다시 시도하면 이 세션이 다룰 수 있는 브라우저를 새로 고릅니다.";

/// 도구가 도는 사이에 항목이 닫혔을 때 Claude에게 돌려주는 말.
const GONE: &str = "이 브라우저(사이트 항목)는 방금 닫혔습니다. 다시 시도하면 이 세션이 다룰 수 있는 브라우저를 새로 고릅니다.";

/// 그 사이트의 웹뷰를 `profile`로 만들어도 되는지: 화면이 알려 준 프로필과 같아야 한다.
/// 화면은 항목을 열 때와 도구에 넘기기 직전에 늘 먼저 알려 주므로, 알려 준 것이 없다면 그 항목은 닫힌 것이다
/// (닫힌 항목의 웹뷰를 만들면 화면 어디에도 없는 채로 남는다).
fn may_use(assigned: &HashMap<u32, String>, id: u32, profile: &str) -> Result<(), &'static str> {
    match assigned.get(&id) {
        Some(current) if current == profile => Ok(()),
        Some(_) => Err(MOVED),
        None => Err(GONE),
    }
}

/// 그 사이트의 웹뷰를 닫고, 닫힌 것을 확인한다(같은 이름으로 새로 만들 수 있게).
fn close_and_wait(app: &AppHandle, id: u32) -> Result<(), String> {
    if let Some(profile) = PROFILES.lock().unwrap().remove(&id) {
        // 이 웹뷰에 떠 있던 확인 창과 정해 둔 답도 함께 사라진다.
        crate::browser::forget_dialogs(&crate::browser::site_dialogs(id, &profile));
    }
    if let Some(existing) = app.get_webview(&label(id)) {
        let _ = existing.close();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.get_webview(&label(id)).is_some() {
        if Instant::now() > deadline {
            return Err("옛 그룹의 브라우저 화면을 닫지 못했습니다. 잠시 뒤에 다시 해 보세요.".into());
        }
        thread::sleep(Duration::from_millis(30));
    }
    Ok(())
}

/// 웹뷰가 없으면 그 프로필로 만든다. `place`가 없으면(Claude가 먼저 연 경우) 숨긴 채로 만든다.
/// 이미 있는 웹뷰가 다른 프로필로 만들어진 것이면(항목이 다른 그룹으로 옮겨졌다) 닫고 새 프로필로 다시 만든다.
/// `profile`이 화면이 알려 준 그 항목의 프로필과 다르면 만들지 않는다. 창 스레드에서 부르면 안 된다(닫히기를 기다린다).
/// 오래 걸릴 수 있으니(앞선 만들기·닫기를 기다린다) 비동기 명령에서는 `blocking`으로 감싸 부른다.
fn create(app: &AppHandle, id: u32, url: &str, place: Option<Place>, profile: &str) -> Result<tauri::Webview, String> {
    let _guard = CREATE.lock().unwrap();
    may_use(&ASSIGNED.lock().unwrap(), id, profile)?;
    if let Some(existing) = app.get_webview(&label(id)) {
        if PROFILES.lock().unwrap().get(&id).is_some_and(|made_with| made_with == profile) {
            return Ok(existing);
        }
        close_and_wait(app, id)?;
    }
    let data_dir = profile_dir(profile)?;
    let target = crate::browser::normalize_url(url)?;
    let window = app.get_window("main").ok_or("본창이 없습니다")?;
    let (for_title, for_load, for_new) = (app.clone(), app.clone(), app.clone());
    let mut builder = WebviewBuilder::new(label(id), WebviewUrl::External(target)).additional_browser_args(BROWSER_ARGS);
    if let Some(dir) = data_dir {
        builder = builder.data_directory(dir);
    }
    let builder = builder
        // 파일을 끌어다 놓는 것은 페이지가 직접 받게 한다(사이트의 끌어서 올리기가 동작하도록).
        .disable_drag_drop_handler()
        // 웹 주소가 아닌 곳(file:, 앱 내부 주소 등)으로는 가지 못하게 한다.
        .on_navigation(|url| matches!(url.scheme(), "http" | "https" | "about"))
        // 새 창으로 열리는 링크는 같은 자리에서 연다(통제되지 않는 창이 생기지 않게).
        .on_new_window(move |url, _features| {
            if let Some(webview) = for_new.get_webview(&label(id)) {
                let _ = webview.navigate(url);
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |_webview, title| {
            let _ = for_title.emit_to("main", "site-changed", Changed { id, url: None, title: Some(title) });
        })
        .on_page_load(move |webview, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let url = webview.url().ok().map(|u| u.to_string());
                let _ = for_load.emit_to("main", "site-changed", Changed { id, url, title: None });
            }
        });
    let shown = place.filter(|p| p.visible);
    let (x, y) = shown.map_or((PARKED_X, 0.0), |p| (p.x, p.y));
    let (width, height) = shown.map_or(PARKED_SIZE, |p| (p.width.max(1.0), p.height.max(1.0)));
    let webview = window.add_child(builder, LogicalPosition::new(x, y), LogicalSize::new(width, height)).map_err(err)?;
    PROFILES.lock().unwrap().insert(id, profile.to_string());
    // 페이지가 띄우는 확인 창(alert 등)을 지켜본다: Claude의 도구가 쓰는 중에 뜬 창은 바로 닫아야 도구가 멈추지 않는다.
    crate::browser::watch_dialogs(&webview, crate::browser::site_dialogs(id, profile));
    if shown.is_none() {
        let _ = webview.hide();
    }
    Ok(webview)
}

/// Claude의 브라우저 도구가 쓴다: 그 사이트 항목의 웹뷰가 없으면 화면에 보이지 않는 채로(그 그룹의 프로필로) 만든다.
/// 그 사이에 항목이 다른 그룹으로 옮겨져 프로필이 달라졌으면 거절한다.
pub fn ensure(app: &AppHandle, id: u32, url: &str, profile: &str) -> Result<tauri::Webview, String> {
    create(app, id, url, None, profile)
}

/// 사이트를 조작하는 동안 웹뷰가 화면을 그리게 한다. 화면에 없는 사이트는 창 밖 자리에 잠깐 꺼내 놓았다가
/// 끝나면 다시 숨긴다. 작업 스레드 전용.
pub fn with_shown<T>(app: &AppHandle, id: u32, run: impl FnOnce(&tauri::Webview) -> T) -> Result<T, String> {
    let webview = app.get_webview(&label(id)).ok_or("사이트가 아직 열리지 않았습니다")?;
    Ok(with_shown_on(&webview, run))
}

/// `with_shown`과 같지만, 이미 손에 든 웹뷰에 대해 한다. 그 웹뷰가 그사이에 닫혔으면(항목이 다른 그룹으로 옮겨져
/// 새 프로필로 다시 만들어졌다) 조작은 오류로 끝난다: 이름으로 다시 찾지 않으므로 다른 프로필의 웹뷰를 건드리지 않는다.
pub fn with_shown_on<T>(webview: &tauri::Webview, run: impl FnOnce(&tauri::Webview) -> T) -> T {
    let id = id_of(webview);
    let parked = {
        let mut busy = BUSY.lock().unwrap();
        let first = enter(&mut busy, id);
        let parked = !is_visible(id);
        if parked {
            let _ = webview.set_position(LogicalPosition::new(PARKED_X, 0.0));
            // 크기는 처음 꺼낼 때만 정한다(먼저 시작한 조작이 도는 중에 페이지 크기가 바뀌지 않게).
            if first {
                let (width, height) = place_of(id)
                    .map(|p| (p.width, p.height))
                    .filter(|(w, h)| *w >= 320.0 && *h >= 240.0)
                    .unwrap_or(PARKED_SIZE);
                let _ = webview.set_size(LogicalSize::new(width, height));
            }
            let _ = webview.show();
        }
        parked
    };
    let _busy = Busy(webview);
    if parked {
        thread::sleep(Duration::from_millis(150));
    }
    run(webview)
}

/// 기다리는 일(잠금, 웹뷰가 닫히기, 웹뷰 만들기)을 비동기 명령의 작업 스레드가 아닌, 막혀도 되는 스레드에서 한다.
/// 그러지 않으면 여러 사이트가 한꺼번에 프로필을 바꿀 때(그룹 합치기 등) 다른 명령들이 함께 멈춘다.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(err)?
}

/// 사이트 웹뷰를 만들어 그 자리(본창 안의 위치·크기, 화면 배율 100% 기준)에 놓는다. 이미 있으면 자리만 옮긴다.
/// `profile`은 그 항목의 그룹이 쓰는 브라우저 프로필의 번호(그룹이 없으면 빈 글). 지금 열려 있는 주소를 돌려준다.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn site_open(app: AppHandle, id: u32, url: String, x: f64, y: f64, width: f64, height: f64, profile: Option<String>) -> Result<String, String> {
    let place = Place { x, y, width, height, visible: true };
    PLACES.lock().unwrap().insert(id, place);
    let profile = profile.unwrap_or_default();
    if !profile.is_empty() {
        profile_key(&profile)?;
    }
    // 어느 프로필을 쓸지는 화면이 `site_profile`로 먼저 알려 둔다. 여기서 고쳐 적지 않는다: 늦게 처리된 옛 열기 요청이
    // 그사이 바뀐 프로필을 되돌려 놓지 않게 한다(그런 요청은 아래에서 거절되고 화면이 새 프로필로 다시 연다).
    blocking(move || {
        let webview = create(&app, id, &url, Some(place), &profile)?;
        // 만드는 사이에 화면이 자리를 바꿨을 수 있다. 마지막으로 알려 준 자리에 놓는다.
        apply(&webview, place_of(id).unwrap_or(place));
        Ok(webview.url().map(|u| u.to_string()).unwrap_or(url))
    })
    .await
}

/// 자리를 옮기거나, 보이기·숨기기를 바꾼다. 아직 만들지 않은 웹뷰면 자리만 기억해 둔다.
#[tauri::command]
pub fn site_bounds(app: AppHandle, id: u32, x: f64, y: f64, width: f64, height: f64, visible: bool) {
    let place = Place { x, y, width, height, visible };
    PLACES.lock().unwrap().insert(id, place);
    if let Some(webview) = app.get_webview(&label(id)) {
        apply(&webview, place);
    }
}

#[tauri::command]
pub fn site_navigate(app: AppHandle, id: u32, url: String) -> Result<String, String> {
    let target = crate::browser::normalize_url(&url)?;
    app.get_webview(&label(id)).ok_or("사이트가 아직 열리지 않았습니다")?.navigate(target.clone()).map_err(err)?;
    Ok(target.to_string())
}

/// 뒤로·앞으로·새로 고침.
#[tauri::command]
pub fn site_action(app: AppHandle, id: u32, action: String) -> Result<(), String> {
    let webview = app.get_webview(&label(id)).ok_or("사이트가 아직 열리지 않았습니다")?;
    match action.as_str() {
        "back" => webview.eval("history.back()"),
        "forward" => webview.eval("history.forward()"),
        "reload" => webview.reload(),
        _ => return Err("모르는 동작입니다".into()),
    }
    .map_err(err)
}

/// 사이트에 보이는 화면(또는 `full`이면 페이지 전체)을 찍어 이미지 저장소에 넣는다.
/// 화면 쪽이 돌려받은 그림으로 이 사이트 항목과 같은 그룹에 이미지 항목을 만든다.
#[tauri::command]
pub async fn site_capture(app: AppHandle, id: u32, full: Option<bool>) -> Result<crate::image::Captured, String> {
    with_shown(&app, id, |webview| {
        crate::image::capture(&|method, params| crate::browser::webview_cdp(webview, method, params), full.unwrap_or(false))
    })?
    .inspect_err(|problem| crate::log(&format!("사이트 캡처 실패: {problem}")))
}

/// 화면이 그 사이트 항목이 써야 하는 프로필을 알려 준다(항목을 만들었을 때와 그룹이 바뀌었을 때).
/// 다른 프로필로 떠 있던 웹뷰는 닫는다. 다시 보이거나 도구가 쓸 때 새 프로필로 만들어진다.
#[tauri::command]
pub async fn site_profile(app: AppHandle, id: u32, profile: String) -> Result<(), String> {
    if !profile.is_empty() {
        profile_key(&profile)?;
    }
    blocking(move || {
        let _guard = CREATE.lock().unwrap();
        let stale = PROFILES.lock().unwrap().get(&id).is_some_and(|made_with| *made_with != profile);
        ASSIGNED.lock().unwrap().insert(id, profile);
        if stale {
            close_and_wait(&app, id)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn site_close(app: AppHandle, id: u32) {
    let _ = blocking(move || {
        let _guard = CREATE.lock().unwrap();
        PLACES.lock().unwrap().remove(&id);
        ASSIGNED.lock().unwrap().remove(&id);
        close_and_wait(&app, id)
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 화면이_알려_준_프로필과_다르면_웹뷰를_만들지_않는다() {
        let mut assigned = HashMap::new();
        // 화면이 알려 준 적이 없는 항목(닫힌 항목)은 어떤 프로필로도 만들지 않는다.
        assert_eq!(may_use(&assigned, 5, "pa"), Err(GONE));
        assert_eq!(may_use(&assigned, 5, ""), Err(GONE));
        assigned.insert(5, "pa".to_string());
        assert_eq!(may_use(&assigned, 5, "pa"), Ok(()));
        // 그 항목이 다른 그룹으로 옮겨져 화면이 새 프로필을 알려 왔다: 옛 프로필을 들고 온 도구 호출은 거절한다.
        assigned.insert(5, "pb".to_string());
        assert_eq!(may_use(&assigned, 5, "pa"), Err(MOVED));
        assert_eq!(may_use(&assigned, 5, "pb"), Ok(()));
        // 그룹이 없는 항목(공용 프로필, 빈 글)도 같은 규칙이다.
        assigned.insert(6, String::new());
        assert_eq!(may_use(&assigned, 6, ""), Ok(()));
        assert_eq!(may_use(&assigned, 6, "pa"), Err(MOVED));
        // 닫히면(화면이 알려 준 것을 지우면) 다시 거절한다.
        assigned.remove(&5);
        assert_eq!(may_use(&assigned, 5, "pb"), Err(GONE));
    }

    #[test]
    fn 같은_사이트를_쓰는_조작이_모두_끝나야_숨긴다() {
        let mut busy = HashMap::new();
        // 처음 시작한 조작만 "처음"이다(그때만 꺼내 놓는 크기를 정한다).
        assert!(enter(&mut busy, 5));
        assert!(!enter(&mut busy, 5));
        // 다른 사이트는 따로 센다.
        assert!(enter(&mut busy, 6));
        // 먼저 끝난 조작은 "마지막"이 아니다: 아직 도는 조작이 있으니 숨기지 않고, 조작 중이라는 표시도 남는다.
        assert!(!leave(&mut busy, 5));
        assert!(busy.contains_key(&5));
        // 마지막 조작이 끝나면 숨겨도 되고, 조작 중이라는 표시가 지워진다.
        assert!(leave(&mut busy, 5));
        assert!(!busy.contains_key(&5));
        assert!(busy.contains_key(&6));
        // 다시 시작하면 또 처음이다.
        assert!(enter(&mut busy, 5));
        assert!(leave(&mut busy, 5));
        assert!(leave(&mut busy, 6));
        assert!(busy.is_empty());
    }

    #[test]
    fn 프로필_번호는_영문과_숫자만() {
        assert_eq!(profile_key("pmwem0abc123"), Ok("pmwem0abc123"));
        for bad in ["", "..", "a/b", "a\\b", "C:x", "그룹", "a b", &"a".repeat(41)] {
            assert!(profile_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn 사이트_웹뷰와_본창의_엔진_실행_인자가_같다() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["app"]["windows"][0]["additionalBrowserArgs"], BROWSER_ARGS);
        // 엔진이 뒤에서 스스로 하는 통신을 끄는 인자가 들어 있다.
        for flag in ["--disable-background-networking", "--disable-component-update", "--disable-domain-reliability"] {
            assert!(BROWSER_ARGS.contains(flag), "{flag}");
        }
    }
}
