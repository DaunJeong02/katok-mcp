# katok-mcp

macOS 카카오톡 대화를 로컬에서 읽고 검색하고 첨부를 보존하는 `katok` CLI와, 그 CLI를 MCP 도구로 노출하는 서버입니다.

> **비공식 도구입니다.** 이 프로젝트와 `katok`은 카카오(Kakao Corp.)가 만들거나 승인하지 않았습니다.
> `katok send`는 사용자의 Mac에서 실행 중인 공식 KakaoTalk 앱을 macOS Accessibility로 조작합니다.
> Accessibility 권한은 카카오의 승인이나 이용제한 면제를 뜻하지 않습니다.
> 사용 전에 [`cli/ACCEPTABLE_USE_POLICY.md`](cli/ACCEPTABLE_USE_POLICY.md)와 [`cli/DISCLAIMER.md`](cli/DISCLAIMER.md)를 읽으십시오.

## 무엇을 하는가

- **읽기**: 카카오톡 로컬 DB를 암호화한 전용 아카이브로 복사합니다. 방 목록, 메시지, 트랜스크립트를 조회합니다.
- **검색**: `keyword`, `bm25`, `semantic` 세 가지 모드로 검색합니다. 임베딩은 로컬에서 만듭니다.
- **첨부**: 사진, 동영상, 파일을 디스크에 저장합니다. 파일 링크는 약 14일 뒤 만료되므로 주기적으로 받아 둡니다.
- **전송**: 공식 앱 UI를 통해 메시지를 전송합니다. 기본은 꺼져 있고, 여러 안전장치를 모두 열어야 동작합니다.

MCP 서버는 카카오 DB와 네트워크를 직접 만지지 않습니다. 모든 작업은 `katok` CLI 한 개를 통해 실행합니다.

## 구성

```
katok-mcp/
  cli/        katok CLI (Rust). upstream katok 포크의 entercut 브랜치를 git subtree로 가져온 것
  mcp/        TypeScript stdio MCP 서버
  scripts/
    install.sh                                          cli 빌드, mcp 빌드, claude mcp add
    launchd/com.enternext.katok-mcp.sync.plist.example  30분 주기 작업 템플릿
```

CLI 자체의 사용법은 [`cli/README.md`](cli/README.md)와 [`cli/CHANGELOG.md`](cli/CHANGELOG.md)를 보십시오.

## 요구사항

- Apple Silicon Mac (macOS). Intel Mac은 `cli/`가 지원하지 않습니다.
- 로그인된 KakaoTalk macOS 앱
- Rust 1.91 (`cli/Cargo.toml`의 `rust-version`)
- Node 22 이상, pnpm
- Claude Code (`claude` 명령)
- 권한: Claude Code를 띄우는 앱(Terminal, iTerm 등)에 **전체 디스크 접근**. 전송 기능을 쓸 때만 **손쉬운 사용(Accessibility)**.

## 설치

```bash
scripts/install.sh
```

스크립트는 다음 순서로 실행합니다.

1. `cli/`를 `cargo build --release`로 빌드합니다. 첫 빌드는 수 분 걸립니다.
2. `mcp/`를 `pnpm install --frozen-lockfile && pnpm build`로 빌드합니다.
3. `claude mcp add --scope user katok-mcp`로 서버를 등록합니다.

첫 설치는 환경변수를 하나도 등록하지 않습니다. 아카이브 키가 `~/.claude.json`에 평문으로 남지 않습니다.

스크립트는 다시 실행해도 안전합니다.

- 기존 등록의 `Environment:` 블록을 읽어 환경변수를 그대로 다시 등록합니다. 변수 이름만 출력하고 값은 출력하지 않습니다.
- 서버 명령에는 `command -v node`의 절대경로를 씁니다. GUI에서 띄운 Claude Code의 PATH에 의존하지 않기 위해서입니다.
- `claude mcp add`가 실패하면 원인을 stderr에 출력하고 비-0으로 종료합니다. 이때 기존 등록은 이미 지워진 상태입니다. 출력된 안내대로 다시 등록하십시오.

<details>
<summary>수동 설치</summary>

```bash
(cd cli && cargo build --release)
(cd mcp && pnpm install --frozen-lockfile && pnpm build)
claude mcp add --scope user katok-mcp -- "$(command -v node)" "$PWD/mcp/dist/index.js"
claude mcp get katok-mcp
```

</details>

## 첫 실행

1. `cli/target/release/katok permissions macos`를 실행해 전체 디스크 접근 설정 화면을 엽니다. Claude Code를 띄우는 앱에 권한을 줍니다.
2. Claude Code에서 `katok_init` 도구를 한 번 호출합니다.
3. 새 메시지를 가져오려면 `katok_sync`를 호출합니다.
4. `katok_rooms`, `katok_search` 등으로 조회합니다.

`katok_init`은 다음을 실행합니다.

- 키체인에 `com.enternext.katok-mcp` / `archive-key` 항목이 없으면 `openssl rand -hex 32`로 키를 만들어 저장합니다. 키 값은 결과와 로그에 나오지 않습니다.
- 데이터 폴더를 만들고 `katok sync --source macos --json`을 한 번 실행한 뒤 `doctor` 결과를 돌려줍니다.
- 아카이브가 이미 있으면 동기화하지 않고 `doctor` 결과만 돌려줍니다.

첫 실행에 macOS가 TCC("다른 앱의 데이터에 접근") 팝업이나 키체인 팝업을 띄울 수 있습니다. 키체인 팝업에서는 **항상 허용**을 누르십시오.

## 도구

모든 도구는 `{ ok, data, out_dir? }`를 텍스트와 `structuredContent`로 돌려줍니다. `data`는 katok의 JSON 그대로입니다. 실패는 `isError: true`와 다음 행동을 담은 문장입니다.

| 도구 | 하는 일 | 읽기/쓰기 |
|---|---|---|
| `katok_init` | 키체인 키 생성, 첫 동기화, 상태 점검 | 쓰기 |
| `katok_doctor` | 아카이브·인덱스 최신성과 권한 점검. `probe: true`일 때만 TCC 팝업이 뜰 수 있음 | 읽기 |
| `katok_sync` | 카카오톡 DB의 새 메시지를 아카이브로 복사. `with_media`로 첨부 저장까지 실행 | 쓰기 |
| `katok_rooms` | 방 목록 | 읽기 |
| `katok_messages` | 한 방의 메시지를 기간·개수로 조회 | 읽기 |
| `katok_search` | `keyword`, `bm25`, `semantic` 검색 | 읽기 |
| `katok_index` | semantic 검색용 벡터 인덱스 생성·갱신 | 쓰기 |
| `katok_chunk` | 검색 결과 chunk의 원문, 주변 chunk, parent window 조회 | 읽기 |
| `katok_transcript` | 한 방의 대화를 Markdown 파일로 내보냄 | 쓰기 |
| `katok_media_get` | 한 방의 사진·동영상·파일 저장 | 쓰기 |
| `katok_media_backfill` | 모든 방의 첨부 저장. `dry_run` 지원 | 쓰기 |
| `katok_list_windows` | 열려 있는 카카오톡 채팅 창 목록 | 읽기 |
| `katok_send` | 방 확인(`dry_run`), 입력창 채우기(`draft`), 전송(`send`) | 쓰기 |

총 13개입니다.

타임아웃은 기본 120초입니다. `init`, `sync`, `index`는 600초, `media backfill`은 30분입니다.

동시 호출은 한 서버 프로세스 안에서 두 줄로 직렬화합니다.

- 카카오톡 화면을 조작하는 `katok_send`, `katok_list_windows`는 한 줄로 실행합니다.
- 아카이브나 파일을 쓰는 `katok_init`, `katok_sync`, `katok_index`, `katok_media_get`, `katok_media_backfill`, `katok_transcript`는 다른 한 줄로 실행합니다.
- 읽기 도구는 기다리지 않습니다.
- 여러 Claude Code 세션이 각자 서버를 띄우면 이 락이 프로세스 사이를 막지 못합니다.

## 전송 안전장치

`katok_send`는 되돌릴 수 없고 다른 사람에게 닿습니다. 모든 거부는 katok을 띄우기 전에 서버 안에서 일어납니다.

| mode | 조건 |
|---|---|
| `dry_run` (기본) | 항상 허용. 방만 확인하고 본문은 넘기지 않음 |
| `draft` | `KATOK_MCP_ACCEPT_USE_POLICY=1`, 허용 방 목록 통과. 텍스트만 가능 |
| `send` | `confirm: true`, `KATOK_MCP_ALLOW_SEND=1`, `KATOK_MCP_ACCEPT_USE_POLICY=1`, 허용 방 목록 통과 |

- `KATOK_MCP_ALLOWED_CHATS`를 설정하면 목록 밖 방으로의 `draft`·`send`를 거부합니다. 이때 `room` 이름 지정도 거부하고 `chat_id`만 받습니다.
- `allow_open`의 기본값은 false입니다. 서버가 `--no-open`을 붙이므로 닫힌 방은 화면에 열지 않고 실패합니다.
- `draft`에서 `image_path`는 거부합니다. katok은 `--image`가 있으면 `--draft`와 관계없이 이미지를 전송하기 때문입니다.
- 본문은 argv가 아니라 stdin으로 넘깁니다.
- 전송이 실패하거나 타임아웃이어도 메시지는 이미 갔을 수 있습니다. 실패 안내는 재시도를 권하지 않습니다. 카카오톡에서 전송 여부를 먼저 확인하십시오.
- 정책 동의 환경변수가 없으면 `cli/ACCEPTABLE_USE_POLICY.md`와 `cli/DISCLAIMER.md` 경로를 알려 주며 거부합니다.

전송을 열려면 예를 들어 다음과 같이 등록합니다. 실제 전송까지 열 때만 `KATOK_MCP_ALLOW_SEND=1`을 추가합니다.

```bash
claude mcp add --scope user katok-mcp \
  -e KATOK_MCP_ACCEPT_USE_POLICY=1 -e KATOK_MCP_ALLOWED_CHATS=<chat_id,...> \
  -- "$(command -v node)" "$PWD/mcp/dist/index.js"
```

반복·대량·예약 전송, 스팸, 사칭에는 쓰지 마십시오.

## 첨부와 주기 작업

### 첨부 보존

- 파일 첨부는 로컬 캐시가 없습니다. 서명된 CDN 링크로만 받을 수 있고, 링크는 약 14일 뒤 만료됩니다(HTTP 410). 만료되면 파일을 되찾을 수 없습니다.
- 사진·동영상은 카카오톡이 화면에 띄울 때 늦게 채우는 로컬 캐시만 있고, 같은 만료 링크를 씁니다.
- `katok_media_backfill`은 모든 방을 돌며 유효한 첨부를 `<데이터 폴더>/media/<chat_id>/`에 저장합니다. 기본 종류는 `file`, `photo`, `video` 전부입니다.
- 이미 저장된 첨부는 네트워크 호출 없이 건너뜁니다. 다시 실행하면 중단된 작업을 이어받습니다.
- 먼저 `dry_run: true`로 규모를 확인하십시오. `tier_totals`의 `planned`는 CDN에서 받을 것, `stub`은 만료·없음, `existing`은 이미 저장됨, `full`/`thumb`은 로컬 캐시에서 복호화한 것입니다. `bytes`는 CDN 다운로드 총량입니다.
- `dry_run`은 CDN 요청만 건너뜁니다. 사진·동영상 중 로컬 캐시에 있는 것은 dry run에서도 `out_dir`에 복호화해 씁니다. 파일만(`kind: ["file"]`) dry run 하면 아무것도 쓰지 않습니다.
- `katok_sync`의 `with_media: true`는 동기화가 성공한 뒤 같은 backfill을 이어서 실행합니다. 결과는 `data.sync`와 `data.backfill`에 담기고, 저장 위치는 `out_dir`에 담깁니다.

### launchd로 30분마다 실행

템플릿은 `scripts/launchd/com.enternext.katok-mcp.sync.plist.example`입니다. `install.sh`는 이 작업을 설치하지 않습니다.

작업은 `sync --incremental`, `media backfill`(file·photo·video), `index` 순으로 실행합니다.

1. `katok_init`을 먼저 한 번 실행해 키체인 키를 만듭니다.
2. 템플릿을 채워 설치합니다.

   ```bash
   sed -e "s|__REPO__|$PWD|g" -e "s|__HOME__|$HOME|g" \
     -e "s|__DATA_DIR__|$HOME/Library/Application Support/katok-mcp|g" \
     scripts/launchd/com.enternext.katok-mcp.sync.plist.example \
     > ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
   ```

3. plist의 `EnvironmentVariables`를 MCP 등록과 같은 값으로 맞춥니다. 대상은 `KATOK_MCP_DATA_DIR`, `KATOK_MCP_KEYCHAIN_SERVICE`, `KATOK_MCP_KEYCHAIN_ACCOUNT`입니다. 값이 다르면 작업이 서버와 다른 아카이브나 다른 키를 씁니다.
4. `/bin/bash`와 `cli/target/release/katok`에 전체 디스크 접근 권한을 줍니다. launchd 작업은 터미널의 권한을 물려받지 않습니다.
5. 작업을 등록합니다.

   ```bash
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
   ```

해제는 다음 명령입니다.

```bash
launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
```

주의 사항:

- **재빌드하면 권한을 다시 승인하십시오.** 바이너리는 ad-hoc 서명이라 재빌드마다 코드 해시(cdhash)가 바뀝니다. macOS의 전체 디스크 접근 승인이 무효화될 수 있습니다. 시스템 설정의 개인정보 보호 및 보안에서 `cli/target/release/katok`을 다시 승인하십시오.
- **작업이 실행 중일 때는 `cargo build --release`를 하지 마십시오.** 실행 중인 바이너리를 교체하면 권한 검증이 실패합니다. 상태는 `launchctl print gui/$(id -u)/com.enternext.katok-mcp.sync | grep state`로 확인합니다.
- 각 단계의 stdout(katok JSON 결과, 실패 시 에러 봉투 포함)은 `<데이터 폴더>/last-sync.json`, `last-backfill.json`, `last-index.json`에 매번 덮어씁니다. 데이터 폴더는 0700, 파일은 0600입니다. 실행을 시작할 때 세 파일을 지우므로 파일이 없으면 그 단계가 실행되지 않았거나 끝나지 않은 것입니다.
- stderr는 `~/Library/Logs/katok-mcp-sync.log`에 남습니다. 작업이 중단되면 종료 코드와 시각이 한 줄 더 남습니다. 이 로그는 회전하지 않으니 가끔 비우십시오.

## 데이터 위치와 키

| 항목 | 위치 |
|---|---|
| 데이터 폴더 | `~/Library/Application Support/katok-mcp` (`KATOK_MCP_DATA_DIR`로 변경). 모든 katok 호출에 `--data-dir`로 명시 |
| 아카이브 | `<데이터 폴더>/archive.sqlite3` (SQLCipher 암호화) |
| 첨부 | `<데이터 폴더>/media/<chat_id>/` |
| 트랜스크립트 | `<데이터 폴더>/transcripts/` |
| 아카이브 키 | macOS 키체인 service `com.enternext.katok-mcp`, account `archive-key` |

### 키 처리

키 우선순위는 서버 환경의 `KATOK_ARCHIVE_KEY`, 키체인, 없음 순입니다.

- 키체인 조회는 성공한 결과만 프로세스 수명 동안 캐시합니다. 항목이 없음(NotFound)은 30초만 기억합니다. 팝업 거부, 타임아웃, 키체인 잠김 같은 그 밖의 실패는 캐시하지 않고 다음 호출이 다시 조회합니다.
- 키도 아카이브도 없으면 읽기 도구도 거부합니다. katok은 `archive.sqlite3`가 없으면 평문 아카이브를 새로 만들기 때문입니다. 서버는 `doctor`, `permissions`, `send --list-windows`, `send --list-rooms`를 뺀 모든 명령을 거부하고 `katok_init`을 먼저 실행하라고 안내합니다. 명시적 `KATOK_ARCHIVE_KEY`가 있으면 통과합니다.
- 평문 아카이브를 보호합니다. 키체인 키를 넣으면 katok이 평문 아카이브를 제자리에서 암호화합니다. 서버는 키를 넣기 전에 `archive.sqlite3`의 앞 16바이트를 확인하고, 평문이면 호출을 거부합니다.
- `katok_init`은 키를 덮어쓰지 않습니다. 키체인 조회가 "없음"이 아닌 이유로 실패하면 새 키를 만들지 않습니다. 아카이브가 이미 있는데 키가 없으면 새 키 생성을 거부합니다. 키 저장은 `-U` 없이 하므로 항목이 이미 있으면 실패하고, 그때는 저장된 키를 다시 읽어 씁니다.
- 키는 `/usr/bin/security -i`에 stdin으로 넘깁니다. 키가 명령행 인자에 나타나지 않아 `ps`에 보이지 않습니다.
- 키체인 항목을 잃으면 아카이브를 복구할 수 없습니다. 백업하려면 `security find-generic-password -s com.enternext.katok-mcp -a archive-key -w`로 키를 직접 확인해 안전한 곳에 보관하십시오.

### 파일을 쓰는 위치 (`out_dir`)

`katok_transcript`, `katok_media_get`, `katok_media_backfill`의 `out_dir`는 두 곳의 하위일 때만 허용합니다. 하나는 데이터 폴더입니다. 다른 하나는 `KATOK_MCP_OUT_ROOTS`에 적은 폴더입니다.

- `KATOK_MCP_OUT_ROOTS`는 콜론(`:`)으로 구분한 절대경로 목록입니다. `~`는 홈 폴더로 확장합니다.
- 심볼릭 링크는 실제 경로로 따라가 비교합니다. 상대경로 항목은 무시합니다.
- 범위를 벗어나면 katok을 띄우지 않고 허용 위치를 알려 주며 거부합니다.
- `out_dir`를 생략하면 기본 위치(`<데이터 폴더>/transcripts`, `<데이터 폴더>/media`)를 씁니다.

데이터 폴더 밖으로 내보내려면 예를 들어 다음과 같이 등록합니다.

```bash
claude mcp add --scope user katok-mcp -e KATOK_MCP_OUT_ROOTS=~/Documents/katok-export \
  -- "$(command -v node)" "$PWD/mcp/dist/index.js"
```

## 환경변수 (MCP 서버)

| 변수 | 뜻 |
|---|---|
| `KATOK_BIN` | katok 실행 파일 경로. 없으면 `<repo>/cli/target/release/katok`, 그다음 `~/.cargo/bin/katok` |
| `KATOK_MCP_DATA_DIR` | 데이터 폴더 덮어쓰기 |
| `KATOK_ARCHIVE_KEY` | 아카이브 키. 있으면 키체인보다 우선 |
| `KATOK_MCP_KEYCHAIN` | `0`이면 키체인 조회를 끔 |
| `KATOK_MCP_KEYCHAIN_SERVICE` | 키체인 service 변경 (기본 `com.enternext.katok-mcp`) |
| `KATOK_MCP_KEYCHAIN_ACCOUNT` | 키체인 account 변경 (기본 `archive-key`) |
| `KATOK_MCP_OUT_ROOTS` | 데이터 폴더 밖에서 `out_dir`로 허용할 폴더 목록 |
| `KATOK_MCP_ACCEPT_USE_POLICY` | `1`일 때만 `draft`·`send` 허용. 정책 문서에 동의했다는 뜻 |
| `KATOK_MCP_ALLOW_SEND` | `1`일 때만 `mode: "send"` 허용 |
| `KATOK_MCP_ALLOWED_CHATS` | 쉼표 구분 `chat_id` 목록. 설정하면 목록 밖 방으로의 `draft`·`send`와 `room` 이름 지정을 거부 |

## 검색 모드

| 모드 | 방식 | 적합한 질의 | 인덱스 |
|---|---|---|---|
| `keyword` | 정확한 부분 문자열 | 이름, 숫자, 고유명사처럼 그대로 기억나는 값 | 불필요 |
| `bm25` | SQLite FTS5 BM25 순위 | 여러 단어가 섞인 일반 질의 | 불필요 |
| `semantic` | 로컬 EmbeddingGemma 벡터 | 표현이 정확히 기억나지 않는 질의 | `katok_index` 필요 |

- `keyword`와 `bm25`는 `chat_id`로 방을 좁힐 수 있습니다. `semantic`은 좁힐 수 없습니다.
- 첫 `katok_index`는 임베딩 모델을 내려받고 모든 window를 임베딩합니다. 아카이브가 크면 수십 분 이상 걸립니다. 이후는 바뀐 window만 임베딩하는 증분이라 짧습니다.
- `katok_index`의 `full: true`는 모든 벡터를 새로 만듭니다.
- 인덱스가 없거나 낡으면 `katok_search`의 `semantic` 모드가 거부하고 `katok_index`를 먼저 실행하라고 안내합니다.

## 문제 해결

1. `katok_doctor`를 호출합니다. 아카이브와 인덱스 최신성, `freshness.recommendation`을 보여 줍니다.
2. 권한 문제가 의심되면 사용자가 명시적으로 원할 때만 `probe: true`를 씁니다. TCC 팝업이 뜰 수 있습니다.
3. 전체 디스크 접근 오류가 나면 Claude Code를 띄우는 앱을 시스템 설정에서 승인합니다. launchd 작업은 `/bin/bash`와 `katok` 바이너리를 별도로 승인합니다.
4. 전송이나 창 목록이 실패하면 손쉬운 사용 권한을 확인합니다. `cli/target/release/katok permissions macos --accessibility`가 설정 화면을 엽니다.

katok이 돌려주는 에러 코드:

- `semantic_index_stale`: 인덱스가 아카이브보다 낡았거나 손상됐습니다. `katok_index`를 실행합니다.
- `semantic_index_busy`: 다른 index가 이미 실행 중입니다. 끝난 뒤 다시 실행합니다. launchd 작업에서는 실패가 기록되고 다음 실행이 다시 시도합니다.

## 개발

```bash
# MCP 서버
cd mcp
pnpm check     # typecheck + biome + build + 단위 테스트
pnpm smoke     # stdio 핸드셰이크 + 실제 katok_init + 읽기 도구 + backfill dry run + send 거부 2건

# CLI (cli/를 수정했을 때)
cd cli && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets
```

- `cli/`의 CI 게이트는 위 세 명령에 더해 `cargo publish --dry-run`, `scripts/verify_release_config.py`, 이력 프라이버시 검사입니다. 자세한 절차는 `cli/AGENTS.md`와 `cli/.github/workflows/ci.yml`을 보십시오.
- `mcp/scripts/smoke.mjs`는 개수·키·상태만 출력합니다. 방 이름, 발신자, 메시지 본문, 파일명은 출력하지 않습니다.
- smoke는 `katok_init`을 실제로 실행합니다(키체인 키 생성과 첫 동기화). `draft`, 실제 전송, `--macos-probe`, 실제 backfill은 호출하지 않습니다.
- 합성 데이터로만 돌리려면 `KATOK_MCP_DATA_DIR=<임시폴더> KATOK_MCP_KEYCHAIN=0 SMOKE_SKIP_INIT=1 pnpm smoke`를 실행합니다.
- 테스트와 문서에 실제 아카이브의 내용(방 이름, 사람, 메시지, `chat_id`)을 넣지 마십시오. 픽스처는 합성 데이터만 씁니다.

### cli/ subtree 갱신

`cli/`는 upstream katok 포크의 `entercut` 브랜치입니다. 갱신하려면 다음 명령을 실행합니다. `<upstream>`은 그 포크의 경로나 URL입니다.

```bash
git subtree pull --prefix=cli <upstream> entercut
```

## 라이선스

저장소 루트는 MIT 라이선스입니다(`LICENSE`). `cli/`는 upstream의 라이선스(`cli/LICENSE`)를 그대로 유지합니다.
