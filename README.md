# katok-mcp

`katok-mcp`는 로컬 카카오톡 CLI [`katok`](../36_katok)을 감싸는 얇은 stdio MCP 서버입니다.
Claude Code 같은 MCP 클라이언트가 `katok`의 검색·조회·전송 기능을 도구로 호출할 수 있게 합니다.

> **비공식 도구입니다.** 이 프로젝트와 `katok`은 카카오(Kakao Corp.)가 만들거나 승인한 것이 아닙니다.
> `katok send`는 사용자의 Mac에서 실행 중인 공식 KakaoTalk 앱을 macOS Accessibility로 조작합니다.
> Accessibility 권한은 카카오의 승인이나 이용제한 면제를 뜻하지 않습니다. 사용 전에
> `36_katok/ACCEPTABLE_USE_POLICY.md`와 `36_katok/DISCLAIMER.md`를 읽으십시오.

## 동작 원칙

- 실행 표면은 `katok` CLI 하나입니다. 서버는 카카오 DB와 키 파일을 직접 읽지 않습니다. 아카이브 키만 `/usr/bin/security`로 키체인에서 읽을 수 있습니다(아래 참고).
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

katok 아카이브가 암호화돼 있으면 katok은 `KATOK_ARCHIVE_KEY`로 아카이브를 엽니다.
서버는 이 키를 다음 순서로 공급합니다.

1. **환경변수**: 서버 환경에 `KATOK_ARCHIVE_KEY`가 있으면 그 값을 그대로 씁니다. 키체인은 조회하지 않습니다.
2. **macOS 키체인**: 환경변수가 없으면 `/usr/bin/security find-generic-password -s <service> -a <account> -w`로 한 번 읽습니다.
   기본값은 service `com.enternext.enter-cut`, account `katok-archive-key`입니다.
   `KATOK_MCP_KEYCHAIN_SERVICE`, `KATOK_MCP_KEYCHAIN_ACCOUNT`로 바꿉니다.
   서버 프로세스가 살아 있는 동안 결과(성공·실패)를 메모리에 캐시합니다. 실패 뒤에 다시 읽게 하려면 서버를 재시작합니다.
3. **없음**: 키체인 조회가 실패하면 키 없이 실행합니다. katok이 암호화 에러를 내면 서버가
   "KATOK_ARCHIVE_KEY 환경변수 또는 키체인 항목이 필요" 안내와 실패 이유를 덧붙입니다.

`KATOK_MCP_KEYCHAIN=0`이면 키체인 조회를 끕니다. 키 값은 로그, 에러, 도구 결과 어디에도 나오지 않습니다.

주의할 점:

- 첫 조회 때 macOS가 `security`(또는 `node`)의 키체인 항목 접근을 묻는 팝업을 띄울 수 있습니다. **"항상 허용"**을 누르십시오. "허용"만 누르면 다음 서버 실행 때 다시 묻습니다. 팝업은 최대 60초 기다립니다.
- 기본 키체인 항목은 enter-cut 앱의 아카이브(`~/Library/Application Support/enter-cut/katok`) 키입니다. katok 기본 아카이브(`~/Library/Application Support/katok`)는 talk-2-build의 키로 암호화돼 있어 이 키로 열리지 않습니다. 이때 서버는 "file is not a database" 에러에 `KATOK_MCP_DATA_DIR` 안내를 덧붙입니다. enter-cut 아카이브를 쓰려면 `KATOK_MCP_DATA_DIR`를 그 폴더로 지정하십시오.
- katok은 키를 받았는데 아카이브가 평문이면 그 키로 아카이브를 **제자리 암호화**합니다(`archive.rs`의 `encrypt_in_place`). 서버는 키체인 키를 넣기 직전에 katok이 실제로 쓸 `<data_dir>/archive.sqlite3`의 앞 16바이트를 확인합니다. data_dir은 katok에 넘기는 `--data-dir`(`KATOK_MCP_DATA_DIR`)이고, 없으면 katok 기본 경로입니다.
  - 헤더가 `SQLite format 3\0`(평문)이면 키를 넣지 않고 도구 호출을 거부합니다: "이 아카이브는 평문입니다. 키체인 키를 주입하면 katok이 제자리에서 암호화하므로 중단했습니다. 의도한 것이면 KATOK_ARCHIVE_KEY를 직접 설정하세요."
  - 파일이 없으면(첫 sync 전) 키를 넣습니다. 새 아카이브가 그 키로 만들어집니다.
  - 헤더를 읽지 못하면 거부합니다. `KATOK_ARCHIVE_KEY`를 직접 설정한 경우는 확인하지 않습니다.
- `claude mcp add -e KATOK_ARCHIVE_KEY=…`로 넣으면 값이 `~/.claude.json`에 평문으로 저장됩니다. 키체인 폴백을 쓰면 이 설정이 필요 없습니다.

## Claude Code에 등록

기본 등록(전송 비활성). enter-cut 아카이브를 enter-cut 앱의 katok sidecar로 읽고, 키는 키체인에서 가져옵니다.
키 값은 등록 설정에 들어가지 않습니다.

```bash
claude mcp add --scope user katok-mcp \
  -e KATOK_BIN=/Users/jeongdaun/enternext/01_business/33_enter-cut/src-tauri/binaries/katok-aarch64-apple-darwin \
  -e "KATOK_MCP_DATA_DIR=/Users/jeongdaun/Library/Application Support/enter-cut/katok" \
  -- node /Users/jeongdaun/enternext/01_business/39_katok-mcp/dist/index.js
```

draft와 전송 게이트를 함께 설정하는 예:

```bash
claude mcp add --scope user katok-mcp \
  -e KATOK_BIN=/Users/jeongdaun/enternext/01_business/33_enter-cut/src-tauri/binaries/katok-aarch64-apple-darwin \
  -e "KATOK_MCP_DATA_DIR=/Users/jeongdaun/Library/Application Support/enter-cut/katok" \
  -e KATOK_MCP_ACCEPT_USE_POLICY=1 \
  -e KATOK_MCP_ALLOWED_CHATS=1234567890,9876543210 \
  -- node /Users/jeongdaun/enternext/01_business/39_katok-mcp/dist/index.js
```

실제 전송까지 열려면 `-e KATOK_MCP_ALLOW_SEND=1`을 추가합니다. 확인은 `claude mcp list` 또는 `claude mcp get katok-mcp`로 합니다.

### 환경변수

| 변수 | 뜻 |
|---|---|
| `KATOK_BIN` | katok 실행 파일 경로(선택) |
| `KATOK_ARCHIVE_KEY` | katok 아카이브 키. 있으면 키체인보다 우선 |
| `KATOK_MCP_KEYCHAIN` | `0`이면 키체인 조회 끔 |
| `KATOK_MCP_KEYCHAIN_SERVICE` | 키체인 service (기본 `com.enternext.enter-cut`) |
| `KATOK_MCP_KEYCHAIN_ACCOUNT` | 키체인 account (기본 `katok-archive-key`) |
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
