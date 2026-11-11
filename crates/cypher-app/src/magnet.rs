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
    });
}

/// Windows가 제안한 위치(`proposed`)를 자석 위치로 고친다.
unsafe fn adjust(state: &mut State, hwnd: Hwnd, proposed: &mut Rect) {
    let distance = state.distance;
    let others: Vec<Rect> = state
        .windows
        .iter()
        .filter(|&&other| other != hwnd && IsWindowVisible(other) != 0 && IsIconic(other) == 0)
        .map(|&other| frame_rect(other))
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
