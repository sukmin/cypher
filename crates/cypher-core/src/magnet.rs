//! 창 자석: 끌고 있는 창이 화면 가장자리나 다른 창 가까이 오면 딱 붙을 위치를 계산한다.
//!
//! 여기에는 계산만 있다. 입력은 "마우스 위치 그대로 따라갔을 때의 창 위치"이고,
//! 결과는 거기서 얼마나 옮기면 붙는지다. 붙은 뒤의 위치가 아니라 마우스 기준 위치로
//! 매번 다시 계산하므로, 마우스를 붙는 거리 밖으로 끌면 곧바로 떨어진다.

/// 화면 좌표의 사각형(픽셀). `right`·`bottom`은 포함하지 않는 끝.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// 가장 가까운 붙을 곳까지의 이동량 (가로, 세로). 붙을 곳이 없으면 0.
///
/// - `window`: 자석이 없을 때의 창 위치(눈에 보이는 테두리 기준)
/// - `work`: 창이 있는 화면의 작업 영역(작업 표시줄 제외)
/// - `others`: 붙을 수 있는 다른 창들
/// - `distance`: 이 거리 안으로 들어오면 붙는다
pub fn snap(window: Rect, work: Rect, others: &[Rect], distance: i32) -> (i32, i32) {
    let mut xs = vec![work.left - window.left, work.right - window.right];
    let mut ys = vec![work.top - window.top, work.bottom - window.bottom];
    for other in others {
        // 세로로 겹치거나 가까운 창끼리만 좌우로 붙고, 가로로 가까운 창끼리만 위아래로 붙는다.
        let beside = window.top < other.bottom + distance && window.bottom > other.top - distance;
        let stacked = window.left < other.right + distance && window.right > other.left - distance;
        if beside {
            xs.extend([other.right - window.left, other.left - window.right]);
        }
        if stacked {
            ys.extend([other.bottom - window.top, other.top - window.bottom]);
        }
        // 가까이 있는 창과는 같은 쪽 가장자리도 나란히 맞춘다.
        if beside && stacked {
            xs.extend([other.left - window.left, other.right - window.right]);
            ys.extend([other.top - window.top, other.bottom - window.bottom]);
        }
    }
    let nearest = |candidates: Vec<i32>| {
        candidates.into_iter().filter(|d| d.abs() <= distance).min_by_key(|d| d.abs()).unwrap_or(0)
    };
    (nearest(xs), nearest(ys))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };

    fn at(left: i32, top: i32) -> Rect {
        Rect { left, top, right: left + 800, bottom: top + 600 }
    }

    #[test]
    fn 화면_가장자리에_붙는다() {
        assert_eq!(snap(at(9, 300), WORK, &[], 14), (-9, 0)); // 왼쪽
        assert_eq!(snap(at(1920 - 800 - 5, 300), WORK, &[], 14), (5, 0)); // 오른쪽
        assert_eq!(snap(at(400, -6), WORK, &[], 14), (0, 6)); // 위
        assert_eq!(snap(at(400, 1040 - 600 + 12), WORK, &[], 14), (0, -12)); // 아래(작업 표시줄 위)
        assert_eq!(snap(at(10, 8), WORK, &[], 14), (-10, -8)); // 모서리
    }

    #[test]
    fn 붙는_거리_밖이면_그대로_둔다() {
        assert_eq!(snap(at(15, 300), WORK, &[], 14), (0, 0));
        assert_eq!(snap(at(400, 200), WORK, &[], 14), (0, 0));
        // 붙어 있다가 마우스를 15픽셀 끌면 바로 떨어진다(입력이 마우스 기준 위치이므로).
        assert_eq!(snap(at(0, 300), WORK, &[], 14), (0, 0));
        assert_eq!(snap(at(15, 300), WORK, &[], 14), (0, 0));
    }

    #[test]
    fn 다른_창_옆에_붙고_가장자리를_맞춘다() {
        let other = Rect { left: 1000, top: 200, right: 1700, bottom: 900 };
        // 내 오른쪽을 그 창 왼쪽에: 800 폭이므로 left 190이면 right 990 → 10만큼 오른쪽으로.
        // 위쪽 가장자리도 195 → 200으로 맞춘다.
        assert_eq!(snap(at(190, 195), WORK, &[other], 14), (10, 5));
        // 그 창 아래에 붙기
        let below = Rect { left: 1010, top: 908, right: 1810, bottom: 1500 };
        assert_eq!(snap(below, Rect { left: 0, top: 0, right: 4000, bottom: 4000 }, &[other], 14), (-10, -8));
        // 세로로 멀리 떨어진 창과는 좌우로 붙지 않는다.
        let far = Rect { left: 1000, top: 3000, right: 1700, bottom: 3500 };
        assert_eq!(snap(at(190, 300), WORK, &[far], 14), (0, 0));
    }

    #[test]
    fn 가까운_쪽을_고른다() {
        // 화면 왼쪽까지 12, 다른 창 오른쪽 가장자리까지 3 → 3을 고른다.
        let other = Rect { left: -500, top: 0, right: 9, bottom: 600 };
        assert_eq!(snap(at(12, 100), WORK, &[other], 14).0, -3);
    }
}
