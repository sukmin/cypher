//! 로컬 통로에 접속한 쪽이 어느 세션의 프로세스인지 알아낸다.
//!
//! 통로(`status.rs`)로 오는 신호에는 "몇 번 세션이 보냈다"가 적혀 있지만, 그것은 보낸 쪽이 스스로 적은 값이다.
//! 확인값은 Cypher가 띄운 모든 세션의 프로세스가 알고 있으므로, 그 값만으로는 한 세션이 다른 세션인 척
//! 접속하는 것을 막지 못한다(다른 그룹의 항목을 다루거나 거짓 알림을 낼 수 있게 된다).
//! 그래서 적힌 번호를 믿지 않고, 접속한 프로세스의 조상을 따라 올라가 어느 세션이 띄운 프로세스인지 직접 본다.
//! 어느 세션의 자손도 아닌 프로세스의 접속은 받지 않는다.

use std::net::SocketAddr;

/// 조상을 따라 올라가는 단계 수의 한도(세션 프로그램 → 셸 → 전달 프로그램 정도면 몇 단계다).
const MAX_DEPTH: usize = 32;

type Handle = *mut std::ffi::c_void;

#[link(name = "iphlpapi")]
extern "system" {
    fn GetExtendedTcpTable(table: *mut u8, size: *mut u32, order: i32, family: u32, class: u32, reserved: u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetProcessTimes(process: Handle, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(process: Handle, class: u32, info: *mut BasicInfo, len: u32, returned: *mut u32) -> i32;
}

/// `PROCESS_BASIC_INFORMATION`. 여기서는 부모 프로세스 번호만 쓴다.
#[repr(C)]
struct BasicInfo {
    exit_status: isize,
    peb: *mut std::ffi::c_void,
    affinity: usize,
    priority: isize,
    pid: usize,
    parent: usize,
}

const AF_INET: u32 = 2;
/// 연결마다 그 연결을 가진 프로세스 번호가 함께 나오는 표.
const TCP_TABLE_OWNER_PID_CONNECTIONS: u32 = 4;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// 표 한 줄(`MIB_TCPROW_OWNER_PID`)의 크기: 상태, 이쪽 주소·포트, 저쪽 주소·포트, 프로세스 번호(각 4바이트).
const ROW: usize = 24;

/// 이 컴퓨터 안에서 `client`(접속한 쪽) → `server`(우리)로 맺은 연결을 가진 프로세스의 번호.
pub fn pid_of(client: SocketAddr, server: SocketAddr) -> Option<u32> {
    let (SocketAddr::V4(client), SocketAddr::V4(server)) = (client, server) else { return None };
    let mut size: u32 = 0;
    let mut table: Vec<u8> = Vec::new();
    // 표는 그사이에도 자랄 수 있으므로 넉넉히 잡고, 모자라면 다시 묻는다.
    for _ in 0..4 {
        // SAFETY: `table`은 `size` 바이트 이상이고(처음에는 크기만 묻는다), `size`는 유효한 변수다.
        let buffer = if table.is_empty() { std::ptr::null_mut() } else { table.as_mut_ptr() };
        let result = unsafe { GetExtendedTcpTable(buffer, &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_CONNECTIONS, 0) };
        if result == 0 && !table.is_empty() {
            return find_row(&table, client.ip().octets(), client.port(), server.port());
        }
        if result != ERROR_INSUFFICIENT_BUFFER && result != 0 {
            return None;
        }
        size = size.saturating_add(ROW as u32 * 32).max(4096);
        table = vec![0u8; size as usize];
    }
    None
}

/// 표에서 "이쪽이 `ip:port`, 저쪽 포트가 `remote_port`"인 줄을 찾아 프로세스 번호를 돌려준다.
/// 표의 맨 앞 4바이트는 줄 수, 포트는 네트워크 순서(큰 자리 먼저)로 아래 두 바이트에 들어 있다.
fn find_row(table: &[u8], ip: [u8; 4], port: u16, remote_port: u16) -> Option<u32> {
    let word = |at: usize| table.get(at..at + 4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]));
    let port_at = |at: usize| table.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let rows = word(0)? as usize;
    (0..rows).find_map(|index| {
        let at = 4 + index * ROW;
        let local_ip = table.get(at + 4..at + 8)?;
        (local_ip == ip && port_at(at + 8)? == port && port_at(at + 16)? == remote_port).then(|| word(at + 20)).flatten()
    })
}

/// 프로세스의 (부모 번호, 만든 시각). 볼 수 없으면 None.
fn parent_of(pid: u32) -> Option<(u32, u64)> {
    // SAFETY: 받은 손잡이는 널이 아닐 때만 쓰고, 끝에 닫는다. 구조체와 변수는 모두 유효한 자리다.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut info = BasicInfo { exit_status: 0, peb: std::ptr::null_mut(), affinity: 0, priority: 0, pid: 0, parent: 0 };
        let mut returned = 0u32;
        let status = NtQueryInformationProcess(process, 0, &mut info, std::mem::size_of::<BasicInfo>() as u32, &mut returned);
        let (mut created, mut exited, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        let timed = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(process);
        (status >= 0 && timed != 0).then_some((info.parent as u32, created))
    }
}

/// `pid`와 그 조상들의 번호(자기 자신이 맨 앞). 부모가 이미 끝났거나 번호가 다른 프로세스에 다시 쓰였으면
/// 거기서 멈춘다(부모는 자식보다 먼저 만들어졌어야 한다).
pub fn lineage(pid: u32) -> Vec<u32> {
    let mut chain = vec![pid];
    let Some((mut parent, mut born)) = parent_of(pid) else { return chain };
    while chain.len() < MAX_DEPTH && parent != 0 && !chain.contains(&parent) {
        let Some((next, parent_born)) = parent_of(parent) else { break };
        if parent_born > born {
            break; // 그 번호는 나중에 생긴 다른 프로세스의 것이다.
        }
        chain.push(parent);
        (parent, born) = (next, parent_born);
    }
    chain
}

/// 조상 줄(`chain`)에 든 프로세스를 띄운 세션의 번호. `roots`는 (세션 번호, 그 세션 프로그램의 프로세스 번호).
/// 가장 가까운 조상의 세션을 고른다(세션 안에서 다른 세션이 띄워질 일은 없지만, 있다면 안쪽 것이 맞다).
pub fn session_of(chain: &[u32], roots: &[(u32, u32)]) -> Option<u32> {
    chain.iter().find_map(|pid| roots.iter().find(|(_, root)| root == pid).map(|(session, _)| *session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn 접속한_프로세스의_번호를_찾는다() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_side, peer) = listener.accept().unwrap();
        // 접속한 쪽도 이 시험 프로세스다.
        assert_eq!(pid_of(peer, server_side.local_addr().unwrap()), Some(std::process::id()));
        assert_eq!(peer, client.local_addr().unwrap());
        // 없는 연결은 찾지 못한다.
        let nowhere: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert_eq!(pid_of(nowhere, server_side.local_addr().unwrap()), None);
    }

    #[test]
    fn 표에서_줄을_찾는다() {
        let row = |ip: [u8; 4], port: u16, remote: u16, pid: u32| {
            let mut bytes = vec![0u8; ROW];
            bytes[4..8].copy_from_slice(&ip);
            bytes[8..10].copy_from_slice(&port.to_be_bytes());
            bytes[16..18].copy_from_slice(&remote.to_be_bytes());
            bytes[20..24].copy_from_slice(&pid.to_ne_bytes());
            bytes
        };
        let mut table = 3u32.to_ne_bytes().to_vec();
        table.extend(row([127, 0, 0, 1], 50000, 443, 11));
        table.extend(row([10, 0, 0, 5], 50001, 9000, 22));
        table.extend(row([127, 0, 0, 1], 50001, 9000, 33));
        assert_eq!(find_row(&table, [127, 0, 0, 1], 50001, 9000), Some(33));
        assert_eq!(find_row(&table, [127, 0, 0, 1], 50000, 9000), None);
        // 줄 수가 실제보다 크게 적혀 있어도 범위를 벗어나 읽지 않는다.
        let mut short = 9u32.to_ne_bytes().to_vec();
        short.extend(row([127, 0, 0, 1], 1, 2, 3));
        assert_eq!(find_row(&short, [127, 0, 0, 1], 9, 9), None);
        assert_eq!(find_row(&[], [127, 0, 0, 1], 1, 2), None);
    }

    #[test]
    fn 조상을_따라_올라가_세션을_찾는다() {
        // 이 시험 프로세스의 조상 줄: 자기 자신이 맨 앞이고, 부모(cargo 등)가 뒤따른다.
        let chain = lineage(std::process::id());
        assert_eq!(chain[0], std::process::id());
        assert!(chain.len() >= 2, "부모 프로세스를 찾지 못했다: {chain:?}");
        // 자식을 띄우면 그 자식의 조상 줄에 이 프로세스가 있다.
        let mut child = std::process::Command::new("cmd").args(["/C", "pause"]).stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null()).spawn().unwrap();
        let line = lineage(child.id());
        assert_eq!(line.get(1), Some(&std::process::id()), "{line:?}");
        let _ = child.kill();
        let _ = child.wait();

        let roots = [(1, 100), (2, 200)];
        assert_eq!(session_of(&[300, 250, 200, 4], &roots), Some(2)); // 세션 프로그램의 손자
        assert_eq!(session_of(&[100], &roots), Some(1));              // 세션 프로그램 자신
        assert_eq!(session_of(&[300, 250, 4], &roots), None);         // 어느 세션의 자손도 아님
        assert_eq!(session_of(&[], &roots), None);
    }
}
