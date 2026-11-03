# KakaoTalk MCP (katok-mcp)

Claude Code · Codex 에서 **맥 카카오톡 대화를 로컬로** 읽고 검색하는 MCP 서버입니다.
대화는 이 Mac 밖으로 나가지 않습니다.

> **비공식 도구입니다.** 카카오가 만들거나 승인하지 않았습니다. 카카오 계정 이용제한 가능성이 있습니다.
> 쓰기 전에 [`cli/ACCEPTABLE_USE_POLICY.md`](cli/ACCEPTABLE_USE_POLICY.md) 와 [`cli/DISCLAIMER.md`](cli/DISCLAIMER.md) 를 읽으십시오.

## 할 수 있는 일

| 분류 | 기능 |
|---|---|
| 대화 | 방 목록 · 메시지 읽기 · 대화 내보내기(Markdown) |
| 검색 | 키워드 · BM25 · 의미 검색(로컬 임베딩) |
| 첨부 | 사진 · 동영상 · 파일 저장 |
| 전송 | 메시지 보내기 (기본 꺼짐) |

## 구성

| 폴더 | 내용 |
|---|---|
| `cli/` | `katok` CLI (Rust). 카카오톡 로컬 DB 를 암호화 사본으로 복사합니다 |
| `mcp/` | MCP 서버 (TypeScript). 모든 작업을 `katok` 로 실행합니다 |
| `scripts/` | 설치 스크립트 · launchd 템플릿 |

## 설치

준비: Apple Silicon Mac, 로그인된 카카오톡, Rust 1.91, Node 22+, pnpm, Claude Code 또는 Codex

**1. 빌드·등록**

```bash
scripts/install.sh
```

CLI·서버를 빌드하고, 설치된 도구에 등록합니다. 다시 실행해도 안전합니다.

- Claude Code: `claude mcp add --scope user katok-mcp`
- Codex: `codex mcp add katok-mcp` + `tool_timeout_sec = 1800` (`~/.codex/config.toml`)

**2. 권한**

```bash
cli/target/release/katok permissions macos
```

도구를 띄우는 앱(Terminal · iTerm · Codex 앱 등)에 **전체 디스크 접근**을 줍니다.
키체인 팝업이 뜨면 **항상 허용**을 누릅니다.

**3. 첫 동기화**

도구를 다시 시작하고 "katok_init 실행해줘"라고 묻습니다.
키체인 키를 만들고 첫 동기화를 합니다. 이후 새 메시지는 `katok_sync` 로 가져옵니다.

## 도구 (13개)

| 도구 | 하는 일 |
|---|---|
| `katok_init` | 키 생성 · 첫 동기화 |
| `katok_sync` | 새 메시지 가져오기 |
| `katok_doctor` | 상태 · 권한 점검 |
| `katok_rooms` | 방 목록 |
| `katok_messages` | 한 방의 메시지 |
| `katok_search` | 검색 (`keyword` · `bm25` · `semantic`) |
| `katok_index` | 의미 검색 인덱스 만들기 |
| `katok_chunk` | 검색 결과 앞뒤 원문 |
| `katok_transcript` | 대화를 Markdown 으로 내보내기 |
| `katok_media_get` | 한 방의 첨부 저장 |
| `katok_media_backfill` | 모든 방의 첨부 저장 |
| `katok_list_windows` | 열린 채팅 창 목록 |
| `katok_send` | 메시지 보내기 |

파일 첨부 링크는 약 14일 뒤 만료됩니다. 남기려면 `katok_media_backfill` 을 주기적으로 실행합니다.

## 전송

전송은 되돌릴 수 없습니다. 기본은 방 확인(`dry_run`)만 됩니다.

| mode | 열리는 조건 |
|---|---|
| `dry_run` | 항상 |
| `draft` | `KATOK_MCP_ACCEPT_USE_POLICY=1` |
| `send` | 위 조건 + `KATOK_MCP_ALLOW_SEND=1` + `confirm: true` |

`KATOK_MCP_ALLOWED_CHATS` 로 보낼 수 있는 방을 제한합니다.

```bash
claude mcp add --scope user katok-mcp \
  -e KATOK_MCP_ACCEPT_USE_POLICY=1 -e KATOK_MCP_ALLOWED_CHATS=<chat_id,...> \
  -- "$(command -v node)" "$PWD/mcp/dist/index.js"
```

Codex 는 `~/.codex/config.toml` 의 `[mcp_servers.katok-mcp]` 아래에 `env = { KATOK_MCP_ACCEPT_USE_POLICY = "1" }` 처럼 적습니다.

반복 · 대량 · 예약 전송, 스팸, 사칭에는 쓰지 마십시오.

## 데이터와 키

| 항목 | 위치 |
|---|---|
| 데이터 폴더 | `~/Library/Application Support/katok-mcp` |
| 아카이브 | `<데이터 폴더>/archive.sqlite3` (SQLCipher 암호화) |
| 첨부 · 내보내기 | `<데이터 폴더>/media/` · `<데이터 폴더>/transcripts/` |
| 키 | 키체인 `com.enternext.katok-mcp` / `archive-key` |

**키를 잃으면 아카이브를 복구할 수 없습니다.** 백업은 아래 명령으로 키를 확인해 안전한 곳에 둡니다.

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

`/bin/bash` 와 `cli/target/release/katok` 에 전체 디스크 접근을 따로 줍니다.
재빌드하면 이 권한을 다시 승인합니다.

## 업데이트

```bash
git pull && scripts/install.sh
```

실행 후 도구를 다시 시작합니다.

## 문제 해결

먼저 `katok_doctor` 를 실행합니다. 결과의 `recommendation` · `warning` 을 따릅니다.
`warning` 이 있으면 바이너리가 낡은 것이니 `scripts/install.sh` 를 다시 실행합니다.

## 개발

```bash
cd mcp && pnpm check    # 타입 · 린트 · 빌드 · 단위 테스트
```

실제 대화 내용(방 이름 · 사람 · 본문 · `chat_id`)은 코드 · 테스트 · 문서에 넣지 않습니다.
`cli/` 를 고치면 [`cli/AGENTS.md`](cli/AGENTS.md) 의 CI 게이트를 모두 돌립니다.

## 라이선스

루트는 MIT(`LICENSE`), `cli/` 는 upstream 라이선스(`cli/LICENSE`)입니다.
