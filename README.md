# Cypher (사이퍼)

Claude Code 세션 여러 개를 한 창에서 띄우고 전환·감시하는 Windows용 터미널 앱입니다.
화면에 나온 웹 주소와 파일 경로를 클릭하면 브라우저나 탐색기가 열립니다.

## 빌드와 실행

Rust(MSVC)만 있으면 됩니다. Node는 필요 없습니다.

```
cargo build --workspace --release
target\release\cypher.exe
```

창 제목에 버전이 표시됩니다(`Cargo.toml`의 `[workspace.package] version`).
`cypher.exe`와 `cypher-bridge.exe`는 같은 폴더에 있어야 합니다(모델 표시·완료 알림이 전달 프로그램을 거칩니다).
`CYPHER_AUTOSTART=claude` 환경 변수를 주고 실행하면 실행 위치 폴더에서 세션이 바로 열립니다.

## 기능

| 기능 | 설명 |
|---|---|
| 세션 모아보기 | 사이드바에 폴더별로 묶어 표시. "+ 그룹"으로 이름 붙인 그룹을 만들어 폴더와 무관하게 묶을 수 있음(세션을 그룹 머리로 끌어다 놓기, 오른쪽 클릭 "그룹 지정", 그룹 머리의 +로 새 세션). 작업 중·대기·종료 상태, 이름과 색 지정 |
| 화면 분할 | 사이드바의 세션을 화면으로 끌어다 놓으면 놓은 위치대로 나뉨(창의 왼쪽·오른쪽 가장자리는 좌우 분할, 위·아래 가장자리는 상하 분할, 가운데는 그 자리에 열기). 창 사이 경계선을 끌어 크기 조절(두 번 누르면 반반). 최대 8개, 분할 모양과 비율은 다시 시작해도 유지 |
| 링크 클릭 | 웹 주소는 브라우저, 폴더는 탐색기, 파일은 탐색기에서 선택. Ctrl+클릭은 파일 열기 |
| 이미지 붙여넣기 | Ctrl+V 하나로 글자·파일·이미지를 구분. 붙여넣은 이미지는 터미널 아래에 미리보기 |
| 모델·effort 표시 | 세션마다 모델, effort, 컨텍스트 사용률, 비용, 돌고 있는 서브에이전트 수. 컨텍스트 80% 이상이면 색 경고 |
| 사용 한도·사용량 | 창 아래에 5시간·주간 한도와 CPU·메모리. 세션별 CPU·메모리는 사이드바에 표시 |
| 완료 알림 | 응답이 끝나거나 승인을 기다리면 앱 안 알림, 창이 뒤에 있으면 Windows 알림 |
| 대기 모아보기 | 대기 중인 세션만 보기, Ctrl+Shift+N으로 다음 대기 세션 이동 |
| 변경 내용 | 세션 폴더의 바뀐 파일 목록과 차이를 옆 창에 표시(git) |
| 워크트리 | 새 git 워크트리에서 Claude 시작(`claude --worktree`) |
| 자주 쓰는 지시 | 문장을 저장해 두고 클릭 한 번으로 입력 |
| 기억 내보내기·가져오기 | Claude Code의 폴더별 기억 파일을 다른 폴더로 복사하거나 들여옴(가져올 때 기존 기억은 백업, 색인은 병합) |
| 내장 브라우저 | 주소창과 즐겨찾기(★)가 달린 별도 창. 여기서 로그인하고 작업하던 페이지를 Claude 세션이 이어서 읽고 조작(클릭·입력·이동, 화면을 그림으로 보고 좌표 클릭) |
| HTTP 요청 | IntelliJ `.http` 형식의 요청을 옆 창 편집기에서 바로 쓰고 보냄(파일이 없으면 예시로 시작, Ctrl+S 저장, Ctrl+Enter 보내기). 실행마다 `history.http`에 요청·응답 기록. Claude 세션도 같은 파일을 도구로 실행 |
| 세션 복원 | 다시 시작하면 이전 세션과 배치를 되살리고 Claude 대화를 이어감 |
| 창 자석 | 창을 끌 때 화면 가장자리와 다른 Cypher 창에 달라붙음. 조금만 더 끌면 바로 떨어지고, Ctrl을 누른 채 끌면 붙지 않음 |
| 그 밖에 | 화면에서 찾기, 출력 저장, 동시 입력, 파일 끌어다 놓기 |

## 단축키

| 키 | 동작 |
|---|---|
| Ctrl+Shift+T / D | 새 세션 / 나란히 새 세션 |
| Ctrl+Shift+W / X | 세션 닫기 / 나란히 보기에서 빼기 |
| Ctrl+Tab | 다음 세션 (Shift: 이전) |
| Ctrl+Shift+←/→ | 나란히 보이는 창 사이 이동 |
| Ctrl+Shift+N | 다음 대기 세션 |
| Ctrl+Shift+F / G / P / B | 찾기 / 변경 내용 / 자주 쓰는 지시 / 동시 입력 |
| Ctrl+Shift+O / H | 내장 브라우저 / HTTP 요청 옆 창 |
| Ctrl+V | 붙여넣기(이미지 포함). Ctrl+Shift+V는 이미지를 파일 경로로 |
| Ctrl+C | 선택 영역이 있으면 복사 |
| Shift+Enter | 줄바꿈 |
| F1 | 단축키 보기 |

## 메뉴

사이드바 아래에는 대기·변경 내용·브라우저·HTTP 버튼만 있습니다.
자주 쓰는 지시, 동시 입력, 기억 내보내기·가져오기는 창 상단의 도구 메뉴에, 단축키 보기는 도움말 메뉴에 있습니다.
설정 메뉴에서는 Cypher 설정 파일과 Claude Code의 사용자 공통 설정(`~/.claude/settings.json`)·공통 지침(`~/.claude/CLAUDE.md`)을 엽니다.

## 설정

`%APPDATA%\cypher\config.toml`(상단 메뉴의 설정 → Cypher 설정 파일 열기). 고친 뒤 다시 시작하면 적용됩니다.
글꼴, 링크 클릭 방식, 편집기 명령(`code -g {file}:{line}:{col}`), 알림, 세션 복원, 창 자석, 브라우저, 프로필 추가를 정할 수 있습니다.

이 파일은 버전이 바뀌어도 그대로 쓰입니다. Cypher는 파일이 없을 때만 기본 파일을 만들고, 이미 있는 파일은 고치거나 덮어쓰지 않습니다.
모르는 항목은 무시하고 빠진 항목은 기본값을 씁니다. 새 버전에서 늘어난 항목은 같은 폴더의 `config.example.toml`에서 볼 수 있습니다(실행할 때마다 그 버전의 예시로 갱신).

## 구조

| 위치 | 역할 |
|---|---|
| `crates/cypher-core` | 링크 인식·해석·보안 규칙, 설정, 프로필 (화면·OS 의존 없음) |
| `crates/cypher-pty` | ConPTY 세션 생성·입출력·흐름 제어·종료 |
| `crates/cypher-bridge` | Claude Code 상태줄·훅 내용을 본체로 넘기는 전달 프로그램 |
| `crates/cypher-app` | Tauri 앱: 명령, 클립보드, 알림, 사용량, git, HTTP, 내장 브라우저, 창 자석 |
| `ui/` | 화면(빌드 단계 없음). `vendor/xterm`은 xterm.js 5.5 |

모델·effort·한도·완료 알림은 세션을 띄울 때 `claude --settings <파일>`로 상태줄과 훅을 주입해서 받습니다.
사용자의 `settings.json`은 건드리지 않습니다.

## 보안 규칙

터미널 출력은 신뢰할 수 없는 입력으로 다룹니다.

- 링크는 `http`, `https`, `file`만 엽니다.
- 경로는 실제로 존재할 때만 링크가 됩니다.
- 실행 파일과 스크립트는 열지 않고 탐색기에서 보여 주기만 합니다.
- 네트워크 경로(`\\서버\…`)는 존재 확인도 하지 않습니다.

## 시험

```
cargo test --workspace
```

화면까지 자동으로 확인하려면 웹뷰 원격 디버깅을 켜고 실행한 뒤, 그 포트(CDP)로 JS를 실행합니다.
화면 코드가 `window.cypher`(세션 열기·입력·화면 글자 읽기)과 `window.cypherErrors`(화면 오류 목록)를 내놓습니다.

```
set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333
set WEBVIEW2_USER_DATA_FOLDER=<시험용 빈 폴더>
target\debug\cypher.exe
```

## 내장 브라우저와 Claude 연동

사이드바의 "브라우저" 버튼(Ctrl+Shift+O)으로 엽니다. 로그인 상태는 다음 실행에도 유지됩니다.
Cypher가 띄운 Claude 세션에는 `cypher` 도구 묶음(MCP)이 자동으로 연결됩니다.

| 도구 | 하는 일 |
|---|---|
| `browser_read` | 지금 페이지의 주소·제목·글자와, 누르거나 입력할 수 있는 요소 목록(번호) |
| `browser_click` / `browser_type` / `browser_key` | 번호로 가리킨 요소 누르기, 글자 입력, 키 누르기 |
| `browser_screenshot` / `browser_click_at` | 화면을 그림으로 받아 보고, 그 그림의 좌표를 누르기 |
| `browser_navigate` / `browser_back` / `browser_scroll` / `browser_eval` | 이동, 뒤로, 스크롤, 페이지에서 스크립트 실행 |
| `http_list` / `http_run` | 폴더의 `.http` 요청 나열, 요청 실행(기록 포함) |

브라우저 창의 "Claude에게 넘기기"를 누르면 본창으로 돌아와 지금 Claude 세션 입력창에 머리말이 들어갑니다.
"새 세션으로 넘기기"는 같은 폴더·그룹에 새 Claude 세션을 나란히 열어 거기에 넘깁니다.
이어서 할 일을 적고 Enter를 누르면 Claude가 그 페이지에서 작업을 이어갑니다.

- 브라우저가 여는 웹 페이지는 Cypher의 내부 명령(터미널 입력, 파일 열기 등)을 부를 수 없습니다.
  명령은 권한 목록(`crates/cypher-app/build.rs`)에 올라 있고, 권한은 본창과 주소 줄에만 줍니다.
- 조작은 디버깅 포트를 열지 않고 WebView2의 개발자 도구 프로토콜을 앱 안에서 직접 부릅니다.
- 도구를 쓸 때의 승인 절차는 Claude Code의 권한 설정을 그대로 따릅니다.
- 설정의 `[browser] enabled = false`로 도구 연결을 끌 수 있습니다.

## HTTP 요청 파일

세션 폴더의 `.cypher\http\`, `http\`, 또는 폴더 바로 아래의 `*.http`를 읽습니다.

```
@host = https://api.example.com

### 로그인
POST {{host}}/login
Content-Type: application/json

{"id": "kim"}
```

실행하면 요청 파일 옆 `history.http`에 요청과 응답이 덧붙습니다(인증 헤더 값은 `***`로 가림).
기록 파일은 `.gitignore`에 자동으로 올라갑니다.
