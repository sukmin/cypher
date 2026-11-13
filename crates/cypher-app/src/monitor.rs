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

/// `root`와 그 자손 프로세스 전체의 (CPU 합, 메모리 합).
/// `procs`: 프로세스 번호 → (부모 번호, CPU, 메모리)
fn tree_usage(root: u32, procs: &HashMap<u32, (Option<u32>, f32, u64)>) -> (f32, u64) {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &(parent, _, _)) in procs {
        if let Some(parent) = parent {
            children.entry(parent).or_default().push(pid);
        }
    }
    let (mut cpu, mut memory) = (0.0, 0);
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue; // 번호 재사용으로 고리가 생겨도 멈추지 않게 한다.
        }
        if let Some(&(_, c, m)) = procs.get(&pid) {
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

            let procs: HashMap<u32, (Option<u32>, f32, u64)> = sys
                .processes()
                .iter()
                .map(|(pid, p)| (pid.as_u32(), (p.parent().map(Pid::as_u32), p.cpu_usage(), p.memory())))
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
            (1, (None, 1.0, 10)),
            (2, (Some(1), 2.0, 20)),    // 세션 프로그램
            (3, (Some(2), 4.0, 40)),    // 그 자식
            (4, (Some(3), 8.0, 80)),    // 손자
            (5, (Some(1), 16.0, 160)),  // 무관한 프로세스
        ]);
        assert_eq!(tree_usage(2, &procs), (14.0, 140));
        assert_eq!(tree_usage(99, &procs), (0.0, 0));
    }
}
