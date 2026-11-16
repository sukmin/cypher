//! 창 자석의 Windows 쪽 연결: 창을 끄는 동안(WM_MOVING) 위치를 고쳐 화면 가장자리나
//! Cypher의 다른 창에 붙인다. 붙을 위치 계산은 `cypher_core::magnet`에 있다.
//!
//! 끌기 시작할 때 마우스와 창 모서리의 간격을 기억해 두고, 움직일 때마다 "마우스를 그대로
//! 따라간 위치"를 다시 구해 거기서 붙일지 정한다. Windows가 준 위치를 그대로 고치면 붙은 자리가
//! 다음 계산의 기준이 되어 잘 떨어지지 않기 때문이다. Ctrl을 누른 채 끌면 붙지 않는다.

use cypher_core::magnet::{snap, Rect};
use std::cell::RefCell;
use std::ffi::c_void;

type Hwnd = isize;
type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

const WM_NCDESTROY: u32 = 0x0082;
const WM_MOVING: u32 = 0x0216;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
const MONITOR_DEFAULTTONEAREST: u32 = 2;
const VK_CONTROL: i32 = 0x11;
const SUBCLASS_ID: usize = 0xC1F;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[link(name = "user32")]
extern "system" {
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn MonitorFromPoint(point: Point, flags: u32) -> isize;
    fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
    fn GetDpiForWindow(hwnd: Hwnd) -> u32;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn IsIconic(hwnd: Hwnd) -> i32;
    fn GetKeyState(key: i32) -> i16;
    fn EnumWindows(each: unsafe extern "system" fn(Hwnd, isize) -> i32, data: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn QueryFullProcessImageNameW(process: *mut c_void, flags: u32, name: *mut u16, size: *mut u32) -> i32;
}

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// 이보다 작은 창(도움말 풍선 같은 것)에는 붙지 않는다.
const MIN_PEER_SIZE: i32 = 120;

/// 그 프로세스의 실행 파일 이름(폴더는 뺀다). 볼 수 없으면 None.
unsafe fn program_name(pid: u32) -> Option<String> {
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if process.is_null() {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let read = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut len);
    CloseHandle(process);
    if read == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buffer[..(len as usize).min(buffer.len())]);
    path.rsplit(['\\', '/']).next().map(str::to_string)
}

unsafe extern "system" fn collect(hwnd: Hwnd, list: isize) -> i32 {
    // SAFETY: `list`는 `peer_windows`가 넘긴, 이 부름 동안 살아 있는 Vec의 주소다.
    (*(list as *mut Vec<Hwnd>)).push(hwnd);
    1
}

/// 다른 Cypher 창들: 이 프로그램과 이름이 같은 실행 파일의 다른 프로세스가 띄운, 화면에 보이는 최상위 창.
/// Cypher는 프로세스마다 창이 하나라서, 붙을 "다른 Cypher 창"은 늘 다른 프로세스의 창이다.
unsafe fn peer_windows() -> Vec<Hwnd> {
    let ours = std::env::current_exe().ok().and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()));
    let Some(ours) = ours else { return Vec::new() };
    let mut all: Vec<Hwnd> = Vec::new();
    EnumWindows(collect, &mut all as *mut Vec<Hwnd> as isize);
    let me = std::process::id();
    // 프로세스마다 한 번만 이름을 알아본다(창이 여럿인 프로그램이 많다).
    let mut known: Vec<(u32, bool)> = Vec::new();
    all.into_iter()
        .filter(|&hwnd| {
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                return false;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 || pid == me {
                return false;
            }
            match known.iter().find(|(seen, _)| *seen == pid) {
                Some(&(_, same)) => same,
                None => {
                    let same = program_name(pid).is_some_and(|name| name.eq_ignore_ascii_case(&ours));
                    known.push((pid, same));
                    same
                }
            }
        })
        .collect()
}

/// 그 프로세스가 화면에 보이는 창(도움말 풍선 같은 작은 것은 빼고)을 띄웠는지.
/// 업데이트 뒤 새 버전의 창이 떴는지 볼 때 쓴다.
pub fn has_visible_window(pid: u32) -> bool {
    unsafe {
        let mut all: Vec<Hwnd> = Vec::new();
        EnumWindows(collect, &mut all as *mut Vec<Hwnd> as isize);
        all.into_iter().any(|hwnd| {
            if IsWindowVisible(hwnd) == 0 {
                return false;
            }
            let mut owner = 0u32;
            GetWindowThreadProcessId(hwnd, &mut owner);
            let rect = window_rect(hwnd);
            owner == pid && rect.right - rect.left >= MIN_PEER_SIZE && rect.bottom - rect.top >= MIN_PEER_SIZE
        })
    }
}

#[link(name = "comctl32")]
extern "system" {
    fn SetWindowSubclass(hwnd: Hwnd, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: Hwnd, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(hwnd: Hwnd, attribute: u32, value: *mut c_void, size: u32) -> i32;
}

/// 끌고 있는 창의 상태.
struct Drag {
    hwnd: Hwnd,
    /// 마우스에서 창 왼쪽 위 모서리까지의 간격.
    offset: (i32, i32),
    size: (i32, i32),
    /// 창 사각형과 눈에 보이는 테두리의 차이(보이지 않는 크기 조절 여백).
    inset: Rect,
    /// 끌기 시작할 때 떠 있던 다른 Cypher 창들(다른 프로세스의 창).
    peers: Vec<Hwnd>,
}

#[derive(Default)]
struct State {
    /// 자석이 달린 Cypher 창들.
    windows: Vec<Hwnd>,
    drag: Option<Drag>,
    /// 붙는 거리(화면 배율 100% 기준 픽셀).
    distance: i32,
}

thread_local! {
    // 창 메시지는 모두 창 스레드에서 오므로 스레드 안에만 둔다.
    static STATE: RefCell<State> = RefCell::new(State::default());
}

unsafe fn window_rect(hwnd: Hwnd) -> Rect {
    let mut rect = Rect::default();
    GetWindowRect(hwnd, &mut rect);
    rect
}

/// 눈에 보이는 창 테두리. 구할 수 없으면 창 사각형 그대로.
unsafe fn frame_rect(hwnd: Hwnd) -> Rect {
    let mut rect = Rect::default();
    let size = std::mem::size_of::<Rect>() as u32;
    if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut rect as *mut Rect).cast(), size) != 0 {
        return window_rect(hwnd);
    }
    rect
}

unsafe fn cursor() -> Point {
    let mut point = Point::default();
    GetCursorPos(&mut point);
    point
}

unsafe fn begin(state: &mut State, hwnd: Hwnd) {
    let (window, frame, mouse) = (window_rect(hwnd), frame_rect(hwnd), cursor());
    state.drag = Some(Drag {
        hwnd,
        offset: (mouse.x - window.left, mouse.y - window.top),
        size: (window.right - window.left, window.bottom - window.top),
        inset: Rect {
            left: frame.left - window.left,
            top: frame.top - window.top,
            right: window.right - frame.right,
            bottom: window.bottom - frame.bottom,
        },
        peers: peer_windows(),
    });
}

/// Windows가 제안한 위치(`proposed`)를 자석 위치로 고친다.
unsafe fn adjust(state: &mut State, hwnd: Hwnd, proposed: &mut Rect) {
    let distance = state.distance;
    let peers = state.drag.as_ref().map(|drag| drag.peers.as_slice()).unwrap_or_default();
    let others: Vec<Rect> = state
        .windows
        .iter()
        .chain(peers)
        .filter(|&&other| other != hwnd && IsWindowVisible(other) != 0 && IsIconic(other) == 0)
        .map(|&other| frame_rect(other))
        .filter(|rect| rect.right - rect.left >= MIN_PEER_SIZE && rect.bottom - rect.top >= MIN_PEER_SIZE)
        .collect();
    let Some(drag) = state.drag.as_mut().filter(|d| d.hwnd == hwnd) else { return };
    let mouse = cursor();
    let size = (proposed.right - proposed.left, proposed.bottom - proposed.top);
    if size != drag.size {
        // 최대화된 창을 끌어 내리는 등 끄는 도중 크기가 바뀌면 간격을 다시 잡는다.
        drag.size = size;
        drag.offset = (mouse.x - proposed.left, mouse.y - proposed.top);
        return;
    }
    if GetKeyState(VK_CONTROL) < 0 {
        return; // Ctrl을 누른 채 끌면 붙지 않는다.
    }
    let (left, top) = (mouse.x - drag.offset.0, mouse.y - drag.offset.1);
    let visible = Rect {
        left: left + drag.inset.left,
        top: top + drag.inset.top,
        right: left + size.0 - drag.inset.right,
        bottom: top + size.1 - drag.inset.bottom,
    };
    let mut info = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: Rect::default(), work: Rect::default(), flags: 0 };
    if GetMonitorInfoW(MonitorFromPoint(mouse, MONITOR_DEFAULTTONEAREST), &mut info) == 0 {
        return;
    }
    let scaled = distance * GetDpiForWindow(hwnd).max(96) as i32 / 96;
    let (dx, dy) = snap(visible, info.work, &others, scaled);
    *proposed = Rect { left: left + dx, top: top + dy, right: left + dx + size.0, bottom: top + dy + size.1 };
}

unsafe extern "system" fn subclass(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize, _id: usize, _data: usize) -> isize {
    match message {
        WM_ENTERSIZEMOVE => STATE.with(|s| begin(&mut s.borrow_mut(), hwnd)),
        WM_MOVING if lparam != 0 => STATE.with(|s| adjust(&mut s.borrow_mut(), hwnd, &mut *(lparam as *mut Rect))),
        WM_EXITSIZEMOVE => STATE.with(|s| s.borrow_mut().drag = None),
        WM_NCDESTROY => {
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                state.windows.retain(|&w| w != hwnd);
                state.drag = None;
            });
            RemoveWindowSubclass(hwnd, subclass, SUBCLASS_ID);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

/// 창에 자석을 단다. 그 창을 만든 스레드(창 스레드)에서 불러야 한다.
/// `distance`는 화면 배율 100% 기준의 붙는 거리(픽셀).
pub fn attach(hwnd: Hwnd, distance: u32) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.distance = distance.min(200) as i32;
        if !state.windows.contains(&hwnd) {
            state.windows.push(hwnd);
        }
    });
    // SAFETY: 살아 있는 우리 창에, 창 스레드에서, 프로그램이 끝날 때까지 유효한 함수를 단다.
    unsafe {
        SetWindowSubclass(hwnd, subclass, SUBCLASS_ID, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 프로세스의_실행_파일_이름을_읽고_다른_cypher_창을_찾는다() {
        let ours = std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        // SAFETY: 읽기만 하는 Windows 함수들이다.
        unsafe {
            assert_eq!(program_name(std::process::id()), Some(ours));
            assert_eq!(program_name(0xffff_fff0), None);
            // 시험 프로그램과 이름이 같은 다른 프로세스의 창은 없다(창을 모으다 죽지 않는 것도 본다).
            assert!(peer_windows().is_empty());
        }
    }
}
