# katok-mcp

macOS 카카오톡 대화를 로컬에서 검색·조회하고 첨부(파일·사진·동영상)를 보존하는 **독립 모노레포**입니다.
`katok` CLI와, 그 CLI를 MCP 도구로 노출하는 stdio 서버가 한 저장소에 들어 있습니다.
enter-cut, talk-2-build 어느 쪽에도 의존하지 않고, 전용 아카이브와 전용 키체인 키를 씁니다.

> **비공식 도구입니다.** 이 프로젝트와 `katok`은 카카오(Kakao Corp.)가 만들거나 승인한 것이 아닙니다.
> `katok send`는 사용자의 Mac에서 실행 중인 공식 KakaoTalk 앱을 macOS Accessibility로 조작합니다.
> Accessibility 권한은 카카오의 승인이나 이용제한 면제를 뜻하지 않습니다. 사용 전에
> [`cli/ACCEPTABLE_USE_POLICY.md`](cli/ACCEPTABLE_USE_POLICY.md)와 [`cli/DISCLAIMER.md`](cli/DISCLAIMER.md)를 읽으십시오.

## 구조

```
katok-mcp/
  cli/        katok CLI (Rust). 36_katok의 entercut 브랜치를 git subtree로 가져옴(이력 보존)
  mcp/        TypeScript stdio MCP 서버. 실행 표면은 katok CLI 하나
  scripts/
    install.sh                                   cli 빌드 → mcp 빌드 → claude mcp add (재실행 시 기존 env 보존)
    launchd/com.enternext.katok-mcp.sync.plist.example   30분마다 sync + 첨부 보존 + 증분 index 템플릿
  README.md   이 문서
  CLAUDE.md   저장소 전체 규칙(프라이버시·테스트·mcp 규칙)
```

`cli/`는 `36_katok`의 `entercut` 브랜치입니다. `main`에는 아카이브 암호화(`KATOK_ARCHIVE_KEY`)와
`sync --incremental`이 없어서, 이 저장소가 쓰는 키체인 흐름이 동작하지 않습니다.

## 설치

요구사항: Apple Silicon Mac, Rust(cargo), Node 22 이상, pnpm, Claude Code.

```bash
scripts/install.sh
```

1. `cd cli && cargo build --release` (첫 빌드는 수 분)
2. `cd mcp && pnpm install --frozen-lockfile && pnpm build`
3. `claude mcp add --scope user katok-mcp [-e 기존 환경변수…] -- <node 절대경로> <repo>/mcp/dist/index.js`

처음 설치에는 환경변수가 하나도 들어가지 않습니다. 키 값이 `~/.claude.json`에 평문으로 남지 않습니다.
확인은 `claude mcp get katok-mcp`로 합니다.

**다시 실행해도 안전합니다.** 다시 빌드하고 기존 등록을 교체하되, 교체 전에 `claude mcp get katok-mcp`의 `Environment:` 블록을 읽어
`KATOK_MCP_DATA_DIR`, `KATOK_MCP_ALLOWED_CHATS`, `KATOK_MCP_OUT_ROOTS` 같은 기존 환경변수를 `-e KEY=VALUE`로 그대로 다시 넘깁니다(보존한 변수 이름은 출력하고 값은 출력하지 않습니다).
서버 명령은 설치 시점 PATH의 `node`를 `command -v node` 절대경로로 등록합니다(GUI에서 띄운 Claude Code의 PATH에 의존하지 않기 위해).
`claude mcp add`가 실패하면 그 사실과 복구 방법을 stderr에 출력하고 **비-0으로 종료**합니다. 이때 기존 등록은 이미 지워진 상태이므로 메시지의 안내대로 다시 등록하십시오.

## 첫 실행

1. **Full Disk Access**: 카카오톡 DB를 읽으려면 Claude Code를 띄우는 앱(Terminal, iTerm 등)에 전체 디스크 접근 권한이 필요합니다.
   `cli/target/release/katok permissions macos`가 설정 화면을 엽니다.
2. Claude Code에서 **`katok_init`** 도구를 한 번 호출합니다.
   - 키체인에 `com.enternext.katok-mcp` / `archive-key` 항목이 없으면 `openssl rand -hex 32`로 키를 만들어 저장합니다(키 값은 결과·로그에 나오지 않음).
   - 데이터 폴더 `~/Library/Application Support/katok-mcp`를 만들고 `katok sync --source macos --json`을 한 번 실행한 뒤 `doctor` 결과를 돌려줍니다.
   - 이미 초기화돼 있으면(아카이브가 있으면) 상태(`doctor`)만 돌려줍니다.
   - 첫 실행에 macOS가 TCC("다른 앱의 데이터에 접근") 또는 키체인 접근 팝업을 띄울 수 있습니다. 키체인은 **"항상 허용"**을 누르십시오.
3. 이후 `katok_rooms`, `katok_search` 등을 씁니다. 새 메시지는 `katok_sync`로 가져옵니다.

## 데이터와 키

| 항목 | 위치 |
|---|---|
| 데이터 폴더 | `~/Library/Application Support/katok-mcp` (`KATOK_MCP_DATA_DIR`로 변경). 모든 katok 호출에 `--data-dir`로 명시 |
| 아카이브 | `<데이터 폴더>/archive.sqlite3` (SQLCipher 암호화) |
| 첨부 | `<데이터 폴더>/media/<chat_id>/` |
| 트랜스크립트 | `<데이터 폴더>/transcripts/` |
| 아카이브 키 | macOS 키체인 service `com.enternext.katok-mcp`, account `archive-key` |

### 파일을 쓰는 위치 (`out_dir`)

`katok_transcript`, `katok_media_get`, `katok_media_backfill`의 `out_dir`는 **데이터 폴더 하위**이거나, 서버 환경변수 `KATOK_MCP_OUT_ROOTS`(콜론 `:` 구분 절대경로 목록)에 적힌 폴더의 하위일 때만 허용합니다.
`~`는 홈 폴더로 확장하고, 심볼릭 링크는 따라간 실제 경로로 비교하며, 상대경로 항목은 무시합니다. 벗어나면 katok을 띄우지 않고 허용되는 위치를 알려 주며 거부합니다. `out_dir`를 생략하면 기본 위치(`<데이터 폴더>/transcripts`, `<데이터 폴더>/media`)를 씁니다.
데이터 폴더 밖으로 내보내려면 예를 들어: `claude mcp add --scope user katok-mcp -e KATOK_MCP_OUT_ROOTS=~/Documents/katok-export -- node <repo>/mcp/dist/index.js`

### 동시 호출

한 서버 프로세스 안에서 도구 호출을 두 줄로 직렬화합니다. `katok_send`·`katok_list_windows`는 카카오톡 화면을 조작하므로 한 줄(`uiLock`), `katok_init`·`katok_sync`·`katok_index`·`katok_media_get`·`katok_media_backfill`·`katok_transcript`는 아카이브·파일을 쓰므로 다른 한 줄(`writeLock`)로 호출 순서대로 하나씩 실행합니다. 읽기 도구(`rooms`, `messages`, `search`, `chunk`, `doctor`)는 기다리지 않습니다. 여러 Claude Code 세션이 각자 서버를 띄우는 경우는 프로세스 간이라 이 락이 막지 못합니다.

키 우선순위: 서버 환경의 `KATOK_ARCHIVE_KEY` → 키체인 → 없음.

키체인 조회 캐시:

- **성공한 조회만** 프로세스 수명 동안 캐시합니다.
- 항목이 **없음**(NotFound)은 30초만 기억해, 항목이 없는 동안 키체인 팝업이 호출마다 반복되지 않게 합니다.
- 팝업 거부·타임아웃·키체인 잠김 같은 **그 밖의 실패는 캐시하지 않습니다.** 다음 호출이 다시 조회하므로, 한 번 실패했다고 서버가 재시작 전까지 키 없이 도는 일이 없습니다.
- `katok_init`은 키체인 경로를 탄 모든 경우(새 키 생성, 기존 키 사용, 동시 init이 먼저 만든 키 사용)에 캐시를 비운 뒤 동기화합니다.

안전장치:

- **키도 아카이브도 없으면 읽기 도구도 거부합니다.** katok은 `archive.sqlite3`가 없으면 평문 아카이브를 새로 만듭니다. 서버는 키 source가 `none`(환경변수·키체인 모두 키 없음)이고 `<데이터 폴더>/archive.sqlite3`도 없으면 `doctor`, `permissions`, `send --list-windows`, `send --list-rooms`를 뺀 모든 명령을 katok을 띄우기 전에 거부하고 `katok_init`을 먼저 실행하라고 안내합니다. 명시적 `KATOK_ARCHIVE_KEY`가 있으면 통과합니다.
- **평문 아카이브 보호**: 키체인 키를 넣으면 katok이 평문 아카이브를 제자리에서 암호화합니다. 서버는 키를 넣기 전에 `archive.sqlite3`의 앞 16바이트를 확인해, 평문이면 호출을 거부합니다.
- **`katok_init`은 키를 덮어쓰지 않습니다.** 키체인 조회가 "없음"이 아닌 이유로 실패하면 새 키를 만들지 않고, 아카이브가 이미 있는데 키가 없으면 새 키 생성을 거부합니다(기존 아카이브가 열리지 않게 되므로). 키 저장은 `-U` 없이 하므로 항목이 이미 있으면(동시에 init이 두 번 돈 경우) 실패하고, 그때는 저장된 키를 다시 읽어 그 키를 씁니다.
- 키체인 항목을 잃으면 아카이브를 복구할 수 없습니다. 키를 백업하려면 `security find-generic-password -s com.enternext.katok-mcp -a archive-key -w`로 직접 확인해 안전한 곳에 보관하십시오.
- 키 저장은 `/usr/bin/security -i`(대화형 모드)에 명령을 **stdin으로** 넘깁니다. 키가 명령행 인자에 나타나지 않아 `ps`에 보이지 않습니다. 키체인 service/account에 따옴표·역슬래시·제어문자가 있으면 저장을 거부합니다.

## 첨부 전부 받아오기

- **파일 첨부는 로컬 캐시가 없습니다.** 카카오톡은 사진·영상만 디스크에 캐시하고(그것도 화면에 띄울 때 늦게 채움), 파일은 서명된 CDN 링크로만 받을 수 있습니다.
- **그 서명 링크는 약 14일 뒤 만료됩니다**(만료되면 410). 만료 후엔 영구 유실이므로 **수신 후 빨리** 받아야 합니다.
- `katok_media_backfill`이 모든 방을 돌며 아직 유효한 첨부를 `<데이터 폴더>/media`에 저장합니다. 기본 종류는 `file`, `photo`, `video` 전부입니다(katok CLI 자체의 기본값은 `file`만). 이미 저장된 것은 네트워크 호출 없이 건너뛰므로 다시 실행해도 무료이고, 중단된 실행을 이어받습니다.
- 먼저 `dry_run: true`로 규모를 봅니다. 결과의 `tier_totals`: `planned`=CDN에서 받을 것, `stub`=만료·없음, `existing`=이미 저장됨, `full`/`thumb`=로컬 캐시에서 복호화. `bytes`는 CDN 다운로드 총량입니다.
- **주의(katok 동작)**: `dry_run`은 CDN 요청만 건너뜁니다. 사진·영상 중 카카오톡 로컬 캐시에 이미 있는 것(`full`/`thumb`)은 dry run에서도 `out_dir`로 복호화해 씁니다. 파일(`file`)만 dry run 하면 아무것도 쓰지 않습니다. 이는 `cli/`의 알려진 불일치이며 이 저장소에서는 고치지 않았습니다.
- `katok_sync`의 `with_media: true`는 sync가 성공한 뒤 같은 backfill을 이어서 실행하고 두 결과(`data.sync`, `data.backfill`)와 `out_dir`을 함께 돌려줍니다.
- 결과 JSON의 `out_dir`가 파일이 저장된 절대경로입니다.

## 30분마다 자동 동기화 (launchd)

템플릿: `scripts/launchd/com.enternext.katok-mcp.sync.plist.example`. 이 저장소는 설치하지 않습니다.

```bash
sed -e "s|__REPO__|$PWD|g" -e "s|__HOME__|$HOME|g" \
  -e "s|__DATA_DIR__|$HOME/Library/Application Support/katok-mcp|g" \
  scripts/launchd/com.enternext.katok-mcp.sync.plist.example \
  > ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
# 설치 전에 plist의 EnvironmentVariables를 MCP 등록과 같은 값으로 맞추십시오 (아래)
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist

# 해제
launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
```

- 먼저 `katok_init`을 한 번 실행해 키체인 키를 만들어 두십시오.
- **MCP 등록 env와 같은 값을 넣으십시오.** plist 상단 `EnvironmentVariables`의 `KATOK_MCP_DATA_DIR`, `KATOK_MCP_KEYCHAIN_SERVICE`, `KATOK_MCP_KEYCHAIN_ACCOUNT`는 MCP 서버의 같은 이름 환경변수(없으면 기본값 `~/Library/Application Support/katok-mcp`, `com.enternext.katok-mcp`, `archive-key`)와 같아야 합니다. 다르면 이 작업이 서버와 다른 아카이브나 다른 키를 씁니다. 경로를 plist 본문에 하드코딩하지 않으니, 데이터 폴더를 옮기면 이 변수만 바꾸면 됩니다.
- launchd 작업은 터미널의 Full Disk Access를 물려받지 않습니다. `/bin/bash`와 `cli/target/release/katok`에 전체 디스크 접근 권한을 주십시오.
- **재빌드하면 권한을 다시 승인하십시오.** 바이너리는 ad-hoc 서명이라 재빌드마다 cdhash(코드 해시)가 바뀌고, macOS의 전체 디스크 접근(또는 앱 데이터 접근) 승인이 무효화될 수 있습니다. 재빌드 후 시스템 설정 > 개인정보 보호 및 보안 > 전체 디스크 접근 권한에서 `cli/target/release/katok`을 다시 승인하십시오.
- **작업이 실행 중일 때는 `cargo build --release`를 하지 마십시오.** 실행 중 바이너리를 교체하면 권한 검증이 실패합니다. `launchctl print gui/$(id -u)/com.enternext.katok-mcp.sync | grep state`로 상태를 확인하십시오.
- 작업은 `sync --incremental`, `media backfill`(file·photo·video), `index` 순으로 실행합니다. **결과를 버리지 않습니다.**
  - 첫 `katok index`는 모든 window를 임베딩해 수십 분(약 90분/14k 문서) 걸릴 수 있습니다. 이후는 바뀐 window만 임베딩하는 증분이라 짧습니다. `katok index`가 `semantic_index_busy`로 실패하면 다른 index가 이미 돌고 있다는 뜻이며, 실패는 기록되고 다음 실행이 다시 시도합니다.
  - 각 단계의 stdout(katok JSON 결과: 개수·상태. 실패 시 에러 봉투도 stdout으로 나옴)는 `<데이터 폴더>/last-sync.json`(sync), `last-backfill.json`(backfill), `last-index.json`(index)에 매번 덮어써 저장합니다. 데이터 폴더는 0700, 파일은 0600입니다. 실행 시작 때 세 파일을 지우므로 파일이 없으면 그 단계가 실행되지 않았거나 끝나지 않은 것입니다.
  - stderr는 `~/Library/Logs/katok-mcp-sync.log`에 남고, 작업이 중단되면 종료 코드와 시각이 한 줄 더 남습니다. 이 로그는 회전하지 않으니 가끔 비우십시오.

## 환경변수 (MCP 서버)

| 변수 | 뜻 |
|---|---|
| `KATOK_BIN` | katok 실행 파일 경로. 없으면 `<repo>/cli/target/release/katok` → `~/.cargo/bin/katok` 순 |
| `KATOK_MCP_DATA_DIR` | 데이터 폴더 덮어쓰기(기본 `~/Library/Application Support/katok-mcp`) |
| `KATOK_ARCHIVE_KEY` | 아카이브 키. 있으면 키체인보다 우선 |
| `KATOK_MCP_KEYCHAIN` | `0`이면 키체인 조회 끔 |
| `KATOK_MCP_KEYCHAIN_SERVICE` / `_ACCOUNT` | 키체인 항목 변경(기본 `com.enternext.katok-mcp` / `archive-key`) |
| `KATOK_MCP_OUT_ROOTS` | 데이터 폴더 밖에서 `out_dir`로 허용할 폴더들(콜론 구분 절대경로, `~` 가능) |
| `KATOK_MCP_ACCEPT_USE_POLICY` | `1`일 때만 draft·send 허용(정책 문서에 동의했다는 뜻) |
| `KATOK_MCP_ALLOW_SEND` | `1`일 때만 `mode: "send"` 허용 |
| `KATOK_MCP_ALLOWED_CHATS` | 쉼표 구분 chat_id 목록. 설정하면 목록 밖 방으로의 draft·send 거부, `room` 이름 지정도 거부 |

## 도구

모든 도구는 `{ ok, data, out_dir? }`를 텍스트와 `structuredContent`로 돌려줍니다(`data`는 katok JSON 그대로). 실패는 `isError: true`와 다음 행동을 담은 문장입니다.
annotations 열은 읽기전용 / 파괴적 / 멱등 / 외부(open world) 순서입니다.

| 도구 | 실행하는 katok 명령 | R / D / I / O | 핵심 인자 |
|---|---|---|---|
| `katok_init` | 키체인 키 생성 → `sync --source macos` → `doctor` | 0 / 0 / 1 / 0 | 없음 |
| `katok_doctor` | `doctor [--macos-probe]` | 1 / 0 / 1 / 0 | `probe`(TCC 팝업 가능, 명시할 때만) |
| `katok_sync` | `sync --source macos [--incremental]` (+ `media backfill`) | 0 / 0 / 1 / 1 | `incremental`, `with_media` |
| `katok_rooms` | `rooms` | 1 / 0 / 1 / 0 | 없음 |
| `katok_messages` | `messages --chat` | 1 / 0 / 1 / 0 | `chat_id`, `since`, `until`, `limit` |
| `katok_search` | `search keyword\|bm25\|semantic` | 1 / 0 / 1 / 0 | `mode`, `query`, `limit`, `chat_id` |
| `katok_index` | `index [--full] [--dry-run]` | 0 / 0 / 1 / 1 | `full`, `dry_run` |
| `katok_chunk` | `chunk get\|context\|parent` | 1 / 0 / 1 / 0 | `action`, `chunk_id`, `redact` |
| `katok_transcript` | `transcript --chat` | 0 / 0 / 1 / 0 | `chat_id`, `since`, `out_dir` |
| `katok_media_get` | `media get --chat` | 0 / 0 / 1 / 1 | `chat_id`, `kind`, `log_id`, `limit`, `no_cdn`, `out_dir` |
| `katok_media_backfill` | `media backfill` | 0 / 0 / 1 / 1 | `chat_id`, `kind`, `dry_run`, `max_bytes`, `limit`, `out_dir` |
| `katok_list_windows` | `send --list-windows` | 1 / 0 / 1 / 1 | 없음 |
| `katok_send` | `send …` (아래 게이트) | 0 / 1 / 0 / 1 | `chat_id`, `text`, `image_path`, `mode`, `confirm` |

타임아웃: 기본 120초, `sync`·`index`·`init` 600초, `media backfill` 30분.

## 전송 게이트 (`katok_send`)

인자: `chat_id`(우선) 또는 `room`, `text` 또는 `image_path`, `mode`(`dry_run` 기본 · `draft` · `send`),
`confirm`, `allow_open`(기본 false), `take_focus_now`.

| mode | 실행 | 조건 |
|---|---|---|
| `dry_run` | `send --chat <id> --dry-run --no-open --json` | 항상 허용. 텍스트는 넘기지 않음 |
| `draft` | `send --chat <id> --draft --no-open --accept-use-policy --json` (본문은 stdin) | `KATOK_MCP_ACCEPT_USE_POLICY=1`, 허용 목록 통과. 텍스트만 |
| `send` | `send --chat <id> [--image <path>] --no-open --accept-use-policy --json` | `confirm: true` **그리고** `KATOK_MCP_ALLOW_SEND=1` **그리고** `KATOK_MCP_ACCEPT_USE_POLICY=1`, 허용 목록 통과 |

- 모든 거부는 katok 프로세스를 띄우기 전에 서버 안에서 일어납니다.
- **전송이 실패·타임아웃이어도 메시지는 이미 갔을 수 있습니다.** 그래서 `katok_send` 실패에는 "범위를 줄여 재시도" 같은 안내를 붙이지 않고, 카카오톡에서 전송 여부를 먼저 확인한 뒤에만 재시도하라는 안내를 붙입니다.
- `allow_open`이 false면 `--no-open`을 붙입니다. 닫힌 방은 화면에 열지 않고 실패합니다.
- `draft`에서 `image_path`는 거부합니다. katok은 `--image`가 있으면 `--draft`와 관계없이 이미지를 전송하기 때문입니다.
- 정책 동의 환경변수가 없으면 `cli/ACCEPTABLE_USE_POLICY.md`, `cli/DISCLAIMER.md` 경로를 알려 주며 거부합니다.
- 전송을 열려면 예를 들어:
  `claude mcp add --scope user katok-mcp -e KATOK_MCP_ACCEPT_USE_POLICY=1 -e KATOK_MCP_ALLOWED_CHATS=<chat_id,…> -- node <repo>/mcp/dist/index.js`
  (실제 전송까지는 `-e KATOK_MCP_ALLOW_SEND=1` 추가). 반복·대량·예약 전송, 스팸, 사칭에는 쓰지 마십시오.

## 개발

```bash
# CLI (cli/를 수정했을 때. 게이트는 cli/AGENTS.md 참고)
cd cli && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets

# MCP 서버
cd mcp
pnpm check     # typecheck + biome + build + 단위 테스트
pnpm smoke     # stdio 핸드셰이크 + 실제 katok_init + 읽기 도구 + backfill dry run + send 거부 2건
```

`mcp/scripts/smoke.mjs`는 개수·키·상태만 출력합니다. 방 이름, 발신자, 메시지 본문, 파일명은 출력하지 않습니다.
`katok_init`은 실제로 실행합니다(키체인 키 생성 + 첫 sync). draft, 실제 전송, `--macos-probe`, 실제 backfill은 호출하지 않습니다.
합성 데이터로만 돌리려면 `KATOK_MCP_DATA_DIR=<임시폴더> KATOK_MCP_KEYCHAIN=0 SMOKE_SKIP_INIT=1 pnpm smoke`.

### cli/ subtree 갱신

`cli/`는 `36_katok`의 `entercut` 브랜치에서 가져왔습니다. 갱신하려면:

```bash
git subtree pull --prefix=cli /Users/jeongdaun/enternext/01_business/36_katok entercut
```

`36_katok`은 이 저장소에서 읽기 전용으로만 다룹니다.
