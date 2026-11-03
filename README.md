# KakaoTalk MCP (katok-mcp)

Claude Code · Codex 에서 **맥 카카오톡 대화를 읽고 검색**합니다. 대화는 이 Mac 밖으로 나가지 않습니다.

> **비공식 도구입니다.** 카카오가 만들거나 승인하지 않았고, 계정 이용제한 가능성이 있습니다.
> 쓰기 전에 [이용 정책](cli/ACCEPTABLE_USE_POLICY.md)과 [면책 고지](cli/DISCLAIMER.md)를 읽으십시오.

## 빠른 시작

준비: Apple Silicon Mac · 로그인된 카카오톡 · Node 22+ · pnpm · Claude Code 또는 Codex

**1. 설치**

```bash
git clone https://github.com/DaunJeong02/katok-mcp.git && cd katok-mcp
scripts/install.sh
```

**2. 권한** — 아래 명령이 설정 화면을 엽니다. 도구를 띄우는 앱(Terminal · iTerm · Codex 앱)에 **전체 디스크 접근**을 켭니다.

```bash
cli/target/release/katok permissions macos
```

**3. 시작** — Claude Code · Codex 를 다시 시작하고 **"katok_init 실행해줘"** 라고 말합니다.
키체인 팝업이 뜨면 **항상 허용**을 누릅니다.

끝입니다. 이제 이렇게 묻습니다.

- "어제 온 카톡 정리해줘"
- "견적 얘기 나온 대화 찾아줘"
- "이 방 대화를 Markdown 으로 내보내줘"

<details>
<summary>설치 스크립트가 하는 일</summary>

- `katok` CLI 를 준비합니다. Rust 가 있으면 소스로 빌드하고, 없으면 [Releases](https://github.com/DaunJeong02/katok-mcp/releases) 에서 받아 SHA-256 을 검증합니다. 이 바이너리는 Apple 서명이 없는 ad-hoc 서명입니다. 소스 빌드를 피하려면 `KATOK_PREBUILT=1 scripts/install.sh`.
- MCP 서버를 빌드합니다.
- 설치된 도구에 모두 등록합니다.
  - Claude Code: `claude mcp add --scope user katok-mcp`
  - Codex: `codex mcp add katok-mcp` + `tool_timeout_sec = 1800`

</details>

## 업데이트 · 문제 해결

```bash
git pull && scripts/install.sh    # 업데이트. 끝나면 도구를 다시 시작합니다
```

문제가 생기면 **"katok_doctor 실행해줘"** 라고 말하고 결과의 `recommendation` · `warning` 을 따릅니다.

## 도구 (13개)

| 분류 | 도구 | 하는 일 |
|---|---|---|
| 시작 | `katok_init` | 키 생성 · 첫 동기화 |
| | `katok_sync` | 새 메시지 가져오기 |
| | `katok_doctor` | 상태 · 권한 점검 |
| 읽기 | `katok_rooms` | 방 목록 |
| | `katok_messages` | 한 방의 메시지 |
| | `katok_transcript` | 대화를 Markdown 으로 내보내기 |
| 검색 | `katok_search` | 키워드 · BM25 · 의미 검색 |
| | `katok_index` | 의미 검색 인덱스 만들기 (처음에 모델 약 200MB 를 받습니다) |
| | `katok_chunk` | 검색 결과 앞뒤 원문 |
| 첨부 | `katok_media_get` | 한 방의 첨부 저장 |
| | `katok_media_backfill` | 모든 방의 첨부 저장 (첨부 링크는 약 14일 뒤 만료) |
| 전송 | `katok_list_windows` | 열린 채팅 창 목록 |
| | `katok_send` | 메시지 보내기 (기본 꺼짐) |

## 전송 켜기

전송은 되돌릴 수 없어서 기본은 방 확인(`dry_run`)만 됩니다.

| mode | 켜는 조건 |
|---|---|
| `dry_run` | 항상 |
| `draft` | `KATOK_MCP_ACCEPT_USE_POLICY=1` |
| `send` | 위 조건 + `KATOK_MCP_ALLOW_SEND=1` + 호출 때 `confirm: true` |

보낼 방은 `KATOK_MCP_ALLOWED_CHATS=<chat_id,...>` 로 제한합니다. 환경변수는 이렇게 넣습니다.

- Claude Code: `claude mcp add --scope user katok-mcp -e KATOK_MCP_ACCEPT_USE_POLICY=1 -- "$(command -v node)" "$PWD/mcp/dist/index.js"`
- Codex: `~/.codex/config.toml` 의 `[mcp_servers.katok-mcp]` 아래 `env = { KATOK_MCP_ACCEPT_USE_POLICY = "1" }`

반복 · 대량 · 예약 전송, 스팸, 사칭에는 쓰지 마십시오.

## 데이터와 키

| 항목 | 위치 |
|---|---|
| 데이터 | `~/Library/Application Support/katok-mcp` (아카이브는 SQLCipher 암호화) |
| 키 | 키체인 `com.enternext.katok-mcp` / `archive-key` |

**키를 잃으면 아카이브를 복구할 수 없습니다.** 아래 명령으로 키를 확인해 안전한 곳에 백업합니다.

```bash
security find-generic-password -s com.enternext.katok-mcp -a archive-key -w
```

## 자동 동기화 (선택)

30분마다 동기화 · 첨부 저장 · 인덱스를 돌립니다. `katok_init` 을 먼저 한 번 실행합니다.

```bash
sed -e "s|__REPO__|$PWD|g" -e "s|__HOME__|$HOME|g" \
  -e "s|__DATA_DIR__|$HOME/Library/Application Support/katok-mcp|g" \
  scripts/launchd/com.enternext.katok-mcp.sync.plist.example \
  > ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
```

이 작업은 `/bin/bash` 와 `cli/target/release/katok` 에 전체 디스크 접근을 따로 줘야 합니다. 업데이트로 바이너리가 바뀌면 다시 승인합니다.

## 개발

```bash
cd mcp && pnpm check    # 타입 · 린트 · 빌드 · 단위 테스트
```

- 실제 대화 내용(방 이름 · 사람 · 본문 · `chat_id`)은 코드 · 테스트 · 문서에 넣지 않습니다.
- `cli/` 는 [katok](https://github.com/NomaDamas/katok) 의 fork 입니다. 고치면 [`cli/AGENTS.md`](cli/AGENTS.md) 의 CI 게이트를 모두 돌립니다.

## 라이선스

루트는 MIT(`LICENSE`), `cli/` 는 upstream 라이선스(`cli/LICENSE`)입니다.
