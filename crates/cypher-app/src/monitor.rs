//! CPU·메모리 사용량: 세션마다 그 세션이 띄운 프로세스 전부의 합계.

use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager};

const INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Serialize)]
struct SessionUsage {
    id: u32,
    /// 컴퓨터 전체 CPU 대비 사용률(%).
    cpu: f32,
    /// 메모리 사용량(바이트).
    memory: u64,
}

#[derive(Clone, Serialize)]
struct UsagePayload {
    sessions: Vec<SessionUsage>,
}

/// 프로세스 하나: (부모 번호, 만든 시각(초, 모르면 0), CPU, 메모리)
type Proc = (Option<u32>, u64, f32, u64);

/// `root`와 그 자손 프로세스 전체의 (CPU 합, 메모리 합).
/// `procs`: 프로세스 번호 → 그 프로세스
fn tree_usage(root: u32, procs: &HashMap<u32, Proc>) -> (f32, u64) {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &(parent, born, _, _)) in procs {
        let Some(parent) = parent else { continue };
        // 부모보다 먼저 만들어진 프로세스는 그 부모의 자식이 아니다: 진짜 부모는 이미 끝났고, 그 번호를 나중에 생긴
        // 다른 프로세스가 다시 쓰고 있는 것이다. 가리지 않으면 무관한 프로세스들이 세션의 사용량에 합쳐진다.
        let parent_born = procs.get(&parent).map_or(0, |p| p.1);
        if born != 0 && parent_born != 0 && born < parent_born {
            continue;
        }
        children.entry(parent).or_default().push(pid);
    }
    let (mut cpu, mut memory) = (0.0, 0);
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue; // 번호 재사용으로 고리가 생겨도 멈추지 않게 한다.
        }
        if let Some(&(_, _, c, m)) = procs.get(&pid) {
            cpu += c;
            memory += m;
        }
        stack.extend(children.get(&pid).into_iter().flatten().copied());
    }
    (cpu, memory)
}

/// 주기적으로 사용량을 재서 화면에 알린다.
pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut sys = System::new();
        let cores = thread::available_parallelism().map_or(1.0, |n| n.get() as f32);
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        loop {
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

            let procs: HashMap<u32, Proc> = sys
                .processes()
                .iter()
                .map(|(pid, p)| (pid.as_u32(), (p.parent().map(Pid::as_u32), p.start_time(), p.cpu_usage(), p.memory())))
                .collect();
            let roots: Vec<(u32, u32)> = app
                .state::<AppState>()
                .sessions
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(&id, session)| Some((id, session.pid()?)))
                .collect();
            let sessions = roots
                .into_iter()
                .map(|(id, pid)| {
                    let (cpu, memory) = tree_usage(pid, &procs);
                    // 프로세스별 값은 코어 하나 기준이라 전체 기준으로 바꾼다.
                    SessionUsage { id, cpu: cpu / cores, memory }
                })
                .collect();

            let _ = app.emit(
                "system-usage",
                UsagePayload { sessions },
            );
            thread::sleep(INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 자손_프로세스까지_합친다() {
        let procs = HashMap::from([
            (1, (None, 100, 1.0, 10)),
            (2, (Some(1), 200, 2.0, 20)),    // 세션 프로그램
            (3, (Some(2), 200, 4.0, 40)),    // 그 자식(같은 초에 만들어짐)
            (4, (Some(3), 0, 8.0, 80)),      // 손자(만든 시각을 모름)
            (5, (Some(1), 300, 16.0, 160)),  // 무관한 프로세스
            (6, (Some(2), 150, 32.0, 320)),  // 번호 2를 쓰던 옛 프로세스의 자식: 세션 프로그램보다 먼저 만들어졌다
            (7, (Some(6), 160, 64.0, 640)),  // 그 자식
        ]);
        assert_eq!(tree_usage(2, &procs), (14.0, 140));
        assert_eq!(tree_usage(99, &procs), (0.0, 0));
    }
}
