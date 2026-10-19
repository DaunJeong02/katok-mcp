# katok-mcp

`katok-mcp`는 로컬 카카오톡 CLI [`katok`](../36_katok)을 감싸는 얇은 stdio MCP 서버입니다.
Claude Code 같은 MCP 클라이언트가 `katok`의 검색·조회·전송 기능을 도구로 호출할 수 있게 합니다.

> **비공식 도구입니다.** 이 프로젝트와 `katok`은 카카오(Kakao Corp.)가 만들거나 승인한 것이 아닙니다.
> `katok send`는 사용자의 Mac에서 실행 중인 공식 KakaoTalk 앱을 macOS Accessibility로 조작합니다.
> Accessibility 권한은 카카오의 승인이나 이용제한 면제를 뜻하지 않습니다. 사용 전에
> `36_katok/ACCEPTABLE_USE_POLICY.md`와 `36_katok/DISCLAIMER.md`를 읽으십시오.

## 동작 원칙

- 실행 표면은 `katok` CLI 하나입니다. 서버는 카카오 DB, 키 파일, 키체인을 직접 읽지 않습니다.
- 서버 자체는 네트워크를 쓰지 않습니다. `katok media get`은 `no_cdn: true`가 아니면 첨부의 presigned CDN URL을 받을 수 있습니다(katok의 유일한 네트워크 경로).
- 모든 호출에 `--json`을 붙이고, stdout JSON을 그대로 도구 결과로 돌려줍니다.
- 비JSON 출력, non-zero exit, 타임아웃, katok의 `{"ok": false, "error": …}` 응답은 모두 MCP 도구 에러(`isError: true`)가 됩니다. 에러 본문에는 exit code, stderr, stdout 앞부분, katok 에러 객체가 들어갑니다.
- 프로세스는 `execFile`로 띄웁니다(shell 없음). 타임아웃은 기본 120초, `sync`·`index`는 600초입니다.
- 전송 본문은 argv가 아니라 stdin으로 넘겨 프로세스 목록에 남지 않게 합니다.

## 설치

요구사항: Apple Silicon Mac, Node 22 이상, pnpm, 빌드된 `katok` 바이너리.

```bash
cd /Users/jeongdaun/enternext/01_business/39_katok-mcp
pnpm install
pnpm build
```

서버는 `katok` 바이너리를 다음 순서로 찾습니다.

1. 환경변수 `KATOK_BIN`
2. `/Users/jeongdaun/enternext/01_business/36_katok/target/release/katok`
3. `~/.cargo/bin/katok`

## 권한

- **Full Disk Access**: `katok_sync`가 `~/Library/Containers/com.kakao.KakaoTalkMac/`의 카카오톡 DB를 읽으려면 서버를 띄우는 프로세스(Terminal, iTerm, Claude Code 등)나 `katok` 실행 파일에 전체 디스크 접근 권한이 있어야 합니다. `katok permissions macos`로 설정 화면을 엽니다.
- **Accessibility**: `katok_send`, `katok_list_windows`는 손쉬운 사용 권한이 필요합니다. `katok permissions macos --accessibility`로 설정 화면을 엽니다.
- `katok_doctor`는 기본값으로 권한 팝업을 띄우지 않습니다. `probe: true`를 명시할 때만 `--macos-probe`를 붙이며, 이때 macOS가 "다른 앱의 데이터에 접근" 팝업을 띄울 수 있습니다.

## 아카이브 키 (`KATOK_ARCHIVE_KEY`)

katok 아카이브가 암호화돼 있으면 `rooms`·`messages`·`search`·`chunk`·`transcript`·`send --chat` 등이
`archive … is encrypted; set KATOK_ARCHIVE_KEY` 에러를 냅니다. 서버는 자기 환경변수를 `katok`에 그대로
넘기므로, 키는 서버 환경에 `KATOK_ARCHIVE_KEY`로 주면 됩니다. 서버가 키를 찾아 읽지는 않습니다.
`claude mcp add -e KATOK_ARCHIVE_KEY=…`로 넣으면 값이 `~/.claude.json`에 평문으로 저장된다는 점에 유의하십시오.

## Claude Code에 등록

읽기 전용으로 등록(전송 비활성):

```bash
claude mcp add --scope user katok-mcp -- node /Users/jeongdaun/enternext/01_business/39_katok-mcp/dist/index.js
```

환경변수를 함께 등록하는 예:

```bash
claude mcp add --scope user katok-mcp \
  -e KATOK_BIN=/Users/jeongdaun/enternext/01_business/36_katok/target/release/katok \
  -e KATOK_MCP_ACCEPT_USE_POLICY=1 \
  -e KATOK_MCP_ALLOWED_CHATS=1234567890,9876543210 \
  -- node /Users/jeongdaun/enternext/01_business/39_katok-mcp/dist/index.js
```

실제 전송까지 열려면 `-e KATOK_MCP_ALLOW_SEND=1`을 추가합니다. 확인은 `claude mcp list` 또는 `claude mcp get katok-mcp`로 합니다.

### 환경변수

| 변수 | 뜻 |
|---|---|
| `KATOK_BIN` | katok 실행 파일 경로(선택) |
| `KATOK_ARCHIVE_KEY` | katok 아카이브 키. 서버는 값을 katok에 넘기기만 함 |
| `KATOK_MCP_DATA_DIR` | 설정하면 모든 호출에 `--data-dir <값>`을 붙임(합성 데이터 QA용) |
| `KATOK_MCP_ACCEPT_USE_POLICY` | `1`일 때만 draft·send에 `--accept-use-policy`를 붙임. 없으면 draft·send 거부 |
| `KATOK_MCP_ALLOW_SEND` | `1`일 때만 `mode: "send"` 허용 |
| `KATOK_MCP_ALLOWED_CHATS` | 쉼표 구분 chat_id 목록. 설정하면 목록 밖 방으로의 draft·send 거부, `room` 이름 지정도 거부 |

## 도구

| 도구 | 실행하는 katok 명령 | 비고 |
|---|---|---|
| `katok_doctor` | `doctor [--macos-probe] --json` | `probe: true`일 때만 probe. 기본은 팝업 없음 |
| `katok_sync` | `sync --source macos [--incremental] --json` | Full Disk Access 필요, 600초 |
| `katok_rooms` | `rooms --json` | 아카이브의 방 목록 |
| `katok_messages` | `messages --chat <id> [--since] [--until] [--limit] --json` | RFC3339 범위 |
| `katok_search` | `search <keyword\|bm25\|semantic> [--limit] [--chat] --json -- <q>` | semantic은 doctor 먼저 확인, 인덱스가 필요하면 에러로 안내. `chat_id`는 keyword·bm25만 |
| `katok_index` | `index [--full] [--dry-run] --json` | 600초, 첫 실행은 임베딩 모델 다운로드 |
| `katok_chunk` | `chunk <get\|context\|parent> [--redact] [--include-message-ids] --json -- <id>` | 플래그는 `get` 전용 |
| `katok_transcript` | `transcript --chat <id> [--since] [--out <dir>] --json` | Markdown 파일 생성 |
| `katok_media_get` | `media get --chat <id> [--log] [--kind …] [--out] [--no-cdn] [--limit] --json` | `no_cdn: true`면 완전 로컬 |
| `katok_list_windows` | `send --list-windows --json` | 열린 채팅창 목록, 읽기 전용 |
| `katok_send` | `send …` (아래 게이트) | 기본 `dry_run` |

## 전송 게이트 (`katok_send`)

인자: `chat_id`(우선) 또는 `room`, `text` 또는 `image_path`, `mode`(`dry_run` 기본 · `draft` · `send`),
`confirm`, `allow_open`(기본 false), `take_focus_now`.

| mode | 실행 | 조건 |
|---|---|---|
| `dry_run` | `send --chat <id> --dry-run --no-open --json` | 항상 허용. 텍스트는 넘기지 않음 |
| `draft` | `send --chat <id> --draft --no-open --accept-use-policy --json` (본문은 stdin) | `KATOK_MCP_ACCEPT_USE_POLICY=1`, 허용 목록 통과. 텍스트만 |
| `send` | `send --chat <id> [--image <path>] --no-open --accept-use-policy --json` | `confirm: true` **그리고** `KATOK_MCP_ALLOW_SEND=1` **그리고** `KATOK_MCP_ACCEPT_USE_POLICY=1`, 허용 목록 통과 |

- 모든 거부는 katok 프로세스를 띄우기 전에 서버 안에서 일어납니다.
- `allow_open`이 false면 `--no-open`을 붙입니다. 닫힌 방은 화면에 열지 않고 실패합니다.
- `draft`에서 `image_path`는 거부합니다. katok은 `--image`가 있으면 `--draft`와 관계없이 이미지를 전송하기 때문입니다.
- 정책 동의 환경변수가 없으면 `ACCEPTABLE_USE_POLICY.md`, `DISCLAIMER.md` 경로를 알려 주며 거부합니다.
- 반복·대량·예약 전송, 스팸, 사칭, 거부 이후 연락 등에는 쓰지 마십시오(katok 정책 문서 참고).

## 개발

```bash
pnpm build    # tsc → dist/
pnpm dev      # tsc --watch
pnpm check    # typecheck + biome + build + 단위 테스트
pnpm smoke    # stdio 핸드셰이크: initialize → tools/list → doctor → rooms → semantic 사전검사 → send 거부 2건
```

`scripts/smoke.mjs`는 개수·키·상태만 출력하고 방 이름이나 메시지 본문은 출력하지 않습니다.
sync, index, draft, 실제 전송, `--macos-probe`, `katok_list_windows`는 호출하지 않습니다.
