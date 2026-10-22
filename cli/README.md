# katok

`katok`은 Apple Silicon Mac에서 카카오톡 대화를 로컬로 읽고, 키워드·벡터
검색과 사용자가 명시적으로 승인한 메시지 전송을 제공하는 CLI입니다.

읽기·검색·인덱싱을 위해 카카오톡 대화 내용을 별도의 `katok` 서버로 올리지
않습니다. macOS에 저장된 카카오톡 DB를 읽어 개인 Mac 안에 정규화된
아카이브와 검색 인덱스를 만들고, `katok search ...` 명령으로 필요한 대화를
찾습니다.

`katok send`는 카카오 원격 서버의 비공개 프로토콜이나 비공식 API를 직접
호출하지 않고, 사용자의 Mac에서 실행 중인 공식 KakaoTalk 앱을 macOS
Accessibility로 조작합니다. 이는 카카오의 승인이나 이용제한 면제를 의미하지
않습니다. 실제 사용 전에 [허용 사용 정책](ACCEPTABLE_USE_POLICY.md)과
[면책 고지](DISCLAIMER.md)를 읽으십시오.

## 무엇을 해주나

- 카카오톡 macOS 앱의 로컬 DB를 읽어 대화 아카이브를 만듭니다.
- 정확한 단어 매칭용 `keyword`, SQLite FTS5 기반 `bm25`, EmbeddingGemma 기반 `semantic` 검색을 제공합니다.
- 긴 대화는 카카오톡 흐름에 맞게 chunk로 나누고, 5분 안팎의 같은 채팅방 대화는 parent window로 묶어 벡터 검색 품질을 높입니다.
- 검색 결과는 짧은 snippet과 chunk id만 보여줍니다. 원문 전체는 사용자가 명시적으로 `katok chunk get <chunk-id>`를 실행할 때만 출력합니다.
- 에이전트는 Vercel Agent Skills/Codex Skills에서 `skills/katok/SKILL.md`를 통해 CLI만 호출하면 됩니다.
- 명시적인 정책 동의와 macOS Accessibility 권한 아래에서 텍스트·이미지를
  공식 KakaoTalk 앱 UI로 전달할 수 있습니다.

## 지원 환경

- Apple Silicon Mac
- macOS 카카오톡 앱
- 터미널 앱의 전체 디스크 접근 권한

Intel Mac은 지원하지 않습니다. 현재 로컬 임베딩 경로가 `fastembed`와 ONNX Runtime을 사용하며, 이 dependency set은 `x86_64-apple-darwin`용 prebuilt ONNX Runtime을 제공하지 않습니다.

## 설치

Homebrew:

```bash
brew tap NomaDamas/katok https://github.com/NomaDamas/katok.git
brew install katok
```

Cargo:

```bash
cargo install katok
```

기본 설치는 macOS에서 `katok send`(`private-send` feature)와 로컬 벡터 검색
(`semantic` feature)을 모두 포함합니다. 검색·아카이브 기능만 필요한 경우 전송
기능 없이 설치할 수 있습니다.

```bash
cargo install katok --no-default-features --features semantic
```

반대로 읽기·동기화·미디어·전송만 쓰고 벡터 검색이 필요 없으면 `semantic`을 빼서
fastembed와 ONNX Runtime 없이 설치할 수 있습니다. 이 빌드에서 `katok index`와
`katok search semantic`은 바로 에러(`semantic_feature_disabled`)를 내고,
keyword·bm25 검색은 그대로 동작합니다.

```bash
cargo install katok --no-default-features --features private-send
```

Cargo로 설치했는데 `katok: command not found`가 나오면 현재 셸이 Cargo binary 경로를 못 보고 있는 상태입니다.

```bash
export PATH="$HOME/.cargo/bin:$PATH"
katok --help
```

영구 적용은 사용하는 셸 설정에 추가합니다.

```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
exec zsh -l
```

처음 설치한 뒤에는 터미널에 전체 디스크 접근 권한을 주세요.

```bash
katok permissions macos
```

열린 System Settings에서 현재 사용하는 Terminal, iTerm, Codex 앱 또는 설치된 `katok` 실행 파일을 Full Disk Access에 추가하세요. macOS TCC 권한은 사용자가 시스템 설정에서 직접 허용해야 하므로 CLI가 자기 자신에게 권한을 영구 부여할 수는 없습니다.

```bash
katok doctor --json
```

`doctor`는 기본값으로 로컬 인덱스 freshness와 프롬프트가 필요 없는 상태만 확인하므로 macOS 권한 prompt를 띄우지 않습니다.
또한 `freshness` 섹션에서 마지막 `sync`와 `index` 완료 시각을 보여줍니다.
카카오톡 앱, 컨테이너, DB 파일 개수, 인증 캐시 여부까지 확인하려면 아래처럼 명시적으로 실행합니다.

```bash
katok doctor --macos-probe --json
```

이 probe는 `source_access`도 채웁니다. 컨테이너에 실제로 `opendir`하고 DB 파일 하나를 `open(O_RDONLY)`해 본 결과입니다. `status`는 `ok`, `denied`(EPERM·EACCES), `missing`(컨테이너나 DB 파일 없음), `error`(그 밖의 실패) 중 하나이고, `failed_at`(`opendir`·`open`)과 `errno`가 실패한 단계를 알려 줍니다. 권한 상태를 프롬프트 없이 물어볼 공개 API가 없어서 실제로 열어 보는 방식이고, 그래서 기본 `doctor`에서는 실행하지 않고 `"status": "not_checked"`로 둡니다. 카카오톡 앱이 떠 있는지(`kakao_app.status`: `running`·`not_running`, `pgrep -x KakaoTalk`)와 `forget`이 남긴 semantic 재빌드 표식(`semantic_rebuild_required`)은 프롬프트를 띄우지 않으므로 기본 `doctor`에도 나옵니다.

이 probe는 macOS가 "katok would like to access data from other apps" 권한 요청을 띄울 수 있습니다. 요청이 떠 있는 동안 첫 `opendir`이 응답을 기다리며 멈춥니다. 반복 요청을 줄이려면 `katok permissions macos`로 System Settings를 연 뒤 사용 중인 Terminal/iTerm/Codex 앱이나 설치된 `katok` 실행 파일을 Full Disk Access에 허용하세요.

`katok send`는 별도의 Accessibility 권한이 필요합니다.

```bash
katok permissions macos --accessibility
```

Accessibility 권한은 사용자의 로컬 Mac에서 KakaoTalk UI를 조작할 수 있게 할
뿐, 카카오가 자동 전송을 승인했다는 뜻은 아닙니다.

권한 설정을 처음부터 안내받으려면:

```bash
scripts/katok-macos-setup.sh
```

자세한 흐름은 `docs/macos-first-run.md`에 있습니다. 카카오톡 DB 스키마, 미디어 캐시 파일명 규칙, Pkv2 복호화, WAL 읽기 불변식 같은 내부 구조는 `docs/kakao-media-internals.md`에 정리돼 있습니다.

## 기본 사용 흐름

```bash
katok doctor --json
katok sync --source macos --json
katok index --json
katok search keyword "계약서" --json
katok search bm25 "지난주 미팅 자료" --json
katok search semantic "최근에 논의한 세금 신고 일정" --json
katok search bm25 "지난주 미팅 자료" --limit 30 --json
```

각 `search` 명령은 `--limit <N>`(기본 10)으로 반환할 결과 개수를 조절할 수 있습니다.

검색 최신성이 중요하면 검색 전에 항상 `katok doctor --json`의 `freshness`를 확인하세요. 이 기본 doctor는 macOS app data probe를 실행하지 않으므로 권한 prompt 없이 사용할 수 있습니다. `sync_before_search`가 `true`이면 `katok sync --source macos --json`을 먼저 실행하고, `index_before_semantic_search`가 `true`이면 `katok index --json`을 실행한 뒤 semantic search를 사용합니다. doctor와 semantic search는 archive revision을 현재 committed index generation과 비교하므로, sync 뒤 index가 오래됐거나 vector ID가 archive와 어긋나면 검색 전에 명시적으로 재인덱싱을 요구합니다.

검색 결과에서 더 넓은 맥락이 필요하면 chunk 명령을 사용합니다.

```bash
katok chunk get <chunk-id> --json
katok chunk context <chunk-id> --json
katok chunk parent <chunk-id> --json
```

- `chunk get`은 해당 chunk 원문을 가져옵니다.
- `chunk context`는 같은 채팅방의 바로 앞뒤 chunk를 보여줍니다.
- `chunk parent`는 semantic search가 사용한 더 큰 parent window를 보여줍니다.

카카오톡 첨부를 추출하려면 media 명령을 사용합니다. 사진(type 2), 앨범(type 27), 영상(type 3), 그리고 일반 파일(type 18)을 다룹니다. 일반 파일은 하나의 메시지 타입이 zip·pdf·xlsx·hwp·pptx 등 모든 확장자를 덮으므로 형식별 대응이 따로 필요하지 않습니다.

```bash
katok media get --chat <chat-id> --json
katok media get --chat <chat-id> --kind file --json
katok media get --chat <chat-id> --log <log-id> --out ./katok-media --no-cdn --json
```

각 프레임은 로컬 full 캐시(`.img`/`.vid`), CDN presigned GET, 로컬 thumbnail `.thm`, stub 순서로 해석됩니다. 추출 자체가 사용자가 명령을 실행해 opt in하는 기능이며, 네트워크를 사용하는 유일한 동작은 attachment metadata의 CDN presigned GET입니다. CDN 응답은 `cs` SHA-1과 일치한 bytes만 저장하고, `--no-cdn`을 주면 CDN tier를 끄고 로컬 캐시만 사용합니다. 기본 출력 위치는 katok data directory 아래 `media/<chat-id>/`입니다.

**일반 파일은 로컬 캐시가 없습니다.** 카카오톡은 사진·영상만 컨테이너에 캐시하고 파일 첨부는 디스크에 남기지 않으므로, 파일의 tier는 CDN 하나뿐이고 `--no-cdn`으로는 아무것도 받을 수 없습니다. 저장 파일명은 첨부의 원본 이름을 그대로 씁니다(`<logId>_<원본이름>`) — zip 본문은 확장자 sniffing으로 `.bin`이 되므로 이름이 확장자의 권위입니다.

**presigned 서명은 약 14일 뒤 만료되고, 만료되면 410으로 사라집니다.** 로컬 사본이 없는 파일 첨부에서는 이것이 곧 영구 유실을 뜻하므로, 정기적으로 `media backfill`을 돌려 창이 닫히기 전에 보존하는 것이 이 기능의 실제 사용법입니다.

```bash
katok media backfill --dry-run --json
katok media backfill --json
katok media backfill --kind file --kind video --json
```

`backfill`은 미디어가 있는 모든 방을 돌면서 아직 만료되지 않은 링크만 받습니다. 이미 저장된 프레임은 네트워크 호출 없이 건너뛰므로 재실행이 멱등하고, 중단된 실행을 그대로 이어받습니다. 기본 kind는 `file`입니다(사진·영상은 로컬 캐시가 있지만 파일은 없기 때문). `--dry-run`은 요청을 한 번도 보내지 않고 파일도 하나도 쓰지 않은 채 각 프레임이 어느 tier로 갈지만 보고하므로, 받을 대상과 만료된 대상을 미리 구분할 수 있습니다. 로컬 캐시의 `full`/`thumb` 사본은 tier 판정을 위해 메모리에서만 복호화하고 `--out`에는 쓰지 않으며, 그 레코드에는 `path`가 없습니다.

`--json` 출력 스키마의 주요 필드는 다음과 같습니다.

- `chat_id`, `log_id`, `limit`, `kinds`, `output_dir`, `cdn_enabled`
- `frame_count`: 읽은 frame 수
- `records[]`: `logId`, `idx`, `kind`, `name`, `w`, `h`, `cs`, `s`, `tier`, `tier_reason`, `path`, `sha1`, `sender`, `ts`
- `errors[]`: tier 실패 관측값, `logId`, `idx`, `stage`, `path`, `error`
- `tier_counts`: `full`, `cdn`, `thumb`, `stub`, `existing`, `planned` 별 개수

`tier_reason`은 왜 그 tier로 떨어졌는지 말합니다. `cdn-expired`는 서명 만료, `cdn-too-large`는 선언 크기가 `--max-bytes`를 넘어 요청 전에 거절, `cdn-unverifiable`은 `cs` 지문이 없어 검증할 수 없어 거절, `unavailable`은 로컬 캐시가 애초에 존재하지 않는 파일 첨부를 뜻합니다.

## 검색 방식

`katok search keyword`는 빠르고 결정적인 부분 문자열 검색입니다. 정확한 단어, 이름, 계좌번호, 고유명사처럼 그대로 기억나는 값을 찾을 때 씁니다. 검색어 안의 `%`, `_`, `\`는 와일드카드가 아니라 그 문자 자체로 찾습니다. 결과는 오래된 chunk부터 `--limit`개를 채웁니다. `--order newest`를 주면 최신 chunk부터 채우므로, 적은 `--limit`로 최근 언급만 볼 때 씁니다.

`katok search bm25`는 SQLite FTS5 BM25 랭킹을 사용합니다. 여러 단어가 섞인 일반 질의에 적합합니다.

BM25 입력은 FTS5 연산식이 아니라 일반 검색어로 처리됩니다. `+`, `-`, 따옴표, 괄호 같은 문자가 포함되어도 문자 그대로 tokenizer에 전달되며 FTS5 column filter나 boolean 문법으로 실행되지 않습니다.

`katok search semantic`은 EmbeddingGemma로 만든 로컬 벡터 인덱스를 사용합니다. 표현이 정확히 기억나지 않아도 의미가 비슷한 대화를 찾을 수 있습니다.

`katok index`는 새 generation을 완전히 만든 뒤 `CURRENT` 포인터를 원자적으로 교체합니다. 실패하면 이전 generation이 그대로 유지되고 명령은 non-zero로 끝납니다. `--full`은 기존 vector를 재사용하지 않는 완전 rebuild이고, 기본 index는 healthy generation의 동일 vector만 재사용하며 무결성 불일치가 있으면 archive에서 self-heal합니다. 인덱싱 도중 `katok sync`가 archive를 바꿔도 index는 시작 시점 스냅샷 기준으로 commit됩니다. 이후 검색은 stale로 거부하고, 다음 `katok index`가 바뀐 window만 증분으로 임베딩해 따라잡습니다.

## EmbeddingGemma 로컬 벡터 검색

`katok index`는 기본값으로 `embeddinggemma-300m-q4`를 앱 프로세스 안에서 실행합니다.

- Python 서버가 필요 없습니다.
- Jina, TEI, 별도 로컬 HTTP embedding endpoint가 필요 없습니다.
- 첫 실행 때 모델 artifact를 사용자 cache 폴더(macOS: `~/Library/Caches/katok/fastembed`)에 내려받고, 이후에는 로컬 cache를 재사용합니다. 작업 디렉터리에 따라 달라지지 않으며, `FASTEMBED_CACHE_DIR`로 위치를 바꿀 수 있습니다.
- 벡터 인덱스와 semantic documents는 사용자 Mac 안의 katok data directory에만 저장됩니다.

설정 예:

```toml
embedder_model = "embeddinggemma-300m-q4"
embedding_batch_size = 64
vector_dimension = 768
semantic_dir = "semantic"
```

테스트나 오프라인 QA에서는 모델 다운로드 없이 deterministic vector를 사용할 수 있습니다.

```bash
KATOK_EMBEDDER=local-test katok index --json
KATOK_EMBEDDER=mock katok index --json
```

실사용 경로에서는 원격 embedding endpoint 설정을 받지 않습니다. 오래된 `embedder_base_url` 또는 `allow_remote_embeddings` 설정이 있으면 거부합니다.

## Vercel Agent Skills / Codex Skills에서 쓰기

이 저장소에는 얇은 agent skill wrapper가 포함되어 있습니다.

```text
skills/katok/SKILL.md
```

에이전트는 카카오톡 DB나 SQLCipher 내부를 직접 만지지 않고, 아래 흐름만 사용해야 합니다.

```bash
katok doctor --json
katok sync --source macos --json
katok index --json
katok search semantic "찾고 싶은 내용" --json
katok chunk get <chunk-id> --json
```

권장 패턴:

1. 검색 전에 `katok doctor --json`의 `freshness`를 봅니다.
2. `sync_before_search`가 `true`이거나 최신 대화가 중요하면 `katok sync --source macos --json`을 실행합니다.
3. semantic search 전에 `index_before_semantic_search`가 `true`이면 `katok index --json`을 실행합니다.
4. 처음에는 `katok search keyword`, `katok search bm25`, `katok search semantic`으로 후보를 좁힙니다.
5. 사용자가 특정 결과를 열어 달라고 하거나 chunk id를 제공했을 때만 `katok chunk get`으로 원문을 봅니다.
6. semantic search 결과의 `child_chunk_ids`에서 정확한 원문으로 이동할 때는 `katok chunk context`와 `katok chunk parent`를 사용합니다.
7. skill은 결과를 요약만 하고, indexing logic이나 DB 해독 logic을 자체 구현하지 않습니다.

## macOS 소스 어댑터

`katok sync --source macos`는 Rust 코드로 카카오톡 macOS 설치를 직접 읽습니다. 런타임에 Python, `kakaocli`, 별도 helper 서버가 필요 없습니다.

sync는 자주 실행해도 되도록 증분으로 동작합니다. 메시지가 실제로 바뀐 채팅방의 tail만 다시 계산하므로, 일반적인 append sync 비용은 전체 아카이브 크기보다 변경 범위에 가깝게 움직입니다. 전량을 다시 계산하는 경우는 세 가지입니다. 빈 아카이브에 처음 실행하는 sync, `chunk_gap_group_seconds`/`chunk_gap_direct_seconds` 를 바꾼 뒤 처음 실행하는 sync, 그리고 chunk 경계 규칙이 바뀐 버전으로 올린 뒤 처음 실행하는 sync 입니다. 이 버전을 기록하기 전에 만들어진 기존 아카이브도 여기 해당하므로 업그레이드 직후 sync 한 번은 전량을 다시 계산합니다. 그 뒤로는 다시 증분으로 돌아옵니다. 출력에 `rebuilt_chats`와 단계별 소요 시간(`timings_ms`의 `read_source`, `upsert_messages`, `rebuild_chunks`)이 포함되므로 느린 실행의 원인을 단계 단위로 확인할 수 있습니다.

`katok sync --source macos --incremental --json`은 카카오톡 DB에서 지난 sync 뒤에 들어온 행만 읽습니다. 매 sync가 DB 파일마다 가장 최근 로컬 행 8개(`NTChatMessage`의 rowid와 그 행의 `chatId`·`logId`)를 아카이브에 남기고, 다음 증분 sync는 그 가운데 아직 같은 메시지를 가리키는 가장 최근 행 뒤부터 읽습니다. rowid는 보낸 시각이 아니라 이 맥에 저장된 순서라서, 맥이 잠들었다 깨어 늦게 받은 옛 메시지도 놓치지 않습니다. 남긴 행이 모두 지워졌거나 바뀌었거나(교체·VACUUM) 커서가 없는 DB는 전체를 읽습니다. 증분 sync는 새로 들어온 행과 교체된 행만 보므로, 카카오톡이 제자리에서 고친 메시지와 새 메시지가 없는 방의 이름 변경은 플래그 없는 다음 sync가 반영합니다. 출력에는 이 플래그를 줬을 때만 `incremental`(모든 DB를 커서 뒤부터 읽었는지)이 붙습니다. `--prune-preview`/`--prune-deleted`와 함께 쓸 수 없습니다.

`katok sync --source macos --chat <chat_id> --json`은 지정한 방만 읽어 아카이브에 넣습니다. `--chat`은 여러 번 줄 수 있습니다. 다른 방의 행은 카카오톡 DB 쿼리 단계에서 걸러지므로 본문을 메모리로 읽지도 않고, 아카이브의 `chats`에도 들어가지 않습니다. macOS 소스의 chat_id는 숫자여야 하고, 빈 값은 거부합니다(빈 값을 버리면 모든 방을 읽는 sync가 되기 때문입니다). `--prune-preview`/`--prune-deleted`와 함께 쓸 수 없습니다.

`katok sync --source macos --chat <chat_id> --since <unix_ms> --json`은 아카이브에 아직 없는 방을 그 시각 이후부터만 넣습니다. 시각은 보낸 시각(`sentAt`)과 밀리초 단위로 비교합니다. `--chat` 없이는 쓸 수 없습니다. "아직 없는 방"은 메시지 행도 바닥도 없는 방입니다. 그런 방에 `--since`가 그 방의 바닥(`chat_floors`)으로 저장되고, 이후 모든 sync가 증분·전체·무필터를 가리지 않고 바닥 이전 행을 버립니다. 그래서 커서는 평소처럼 테이블의 최신 행까지 전진하고, 커서 앞 재읽기 구간이나 교체된 행, 늦게 받은 옛 메시지로도 옛 행이 들어오지 않습니다. 이미 메시지나 바닥이 있는 방은 `--since`를 무시합니다. 범위를 바꾸려면(예: "지금부터" → "30일") 그 방을 `forget --yes`로 지운 뒤 새 `--since`로 다시 sync합니다. `forget`은 바닥도 지우므로 `--since` 없이 다시 넣으면 전체 기록이 들어옵니다.

커서 설계: 증분 커서는 방과 무관하게 DB 테이블 전체의 가장 최근 행 8개입니다. 필터 sync가 이 커서를 무필터 커서 자리에 저장하면, 건너뛴 방의 새 행이 커서 앞에 남아 다음 무필터 `--incremental`이 그 행을 영영 읽지 않습니다. 그래서 필터 sync는 무필터 커서를 읽지도 쓰지도 않고, 방 집합(정렬·중복 제거한 chat_id의 해시)마다 별도 커서를 씁니다. 같은 집합으로 다시 실행하면 순서·중복과 무관하게 증분으로 읽고, 집합이 바뀌면(방 추가·제거) 첫 실행은 그 방들을 전체로 읽습니다. 새로 추가한 방의 과거 기록도 그때 들어옵니다. 집합마다 남는 커서는 DB 파일당 8행이라 지우지 않습니다. 단 `forget --yes`는 무필터·필터 커서를 전부 지웁니다. 커서는 어느 방을 지나갔는지 기록하지 않아서, 지운 방이 든 커서만 골라낼 수 없고 남겨 두면 그 방을 다시 sync할 때 커서 앞의 과거 기록이 빠지기 때문입니다. 대가는 다음 `--incremental` sync 한 번이 전체를 읽는 것입니다.

요구사항:

- 터미널 앱이 `~/Library/Containers/com.kakao.KakaoTalkMac/` 아래 파일을 읽을 수 있도록 전체 디스크 접근 권한을 받아야 합니다.
- 카카오톡 앱에서 열렸거나 동기화된 채팅방의 로컬 DB 기록만 읽을 수 있습니다.
- 최초 sync 때 암호화된 SQLCipher DB에서 계정 식별자를 복구하고, `{user_id, uuid}`만 mode `0600` cache로 저장합니다. 키 material 자체는 저장하지 않습니다.

fixture로 개발/테스트할 때는 실제 카카오톡 설치가 필요 없습니다.

```bash
katok source chats --source fixture tests/fixtures/kakao/replies.jsonl --json
katok sync --source fixture tests/fixtures/kakao/replies.jsonl --json
```

합성 데이터로 실행할 때는 `--data-dir <임시경로>` 플래그로 반드시 격리하세요. `KATOK_DATA_DIR` 환경변수는 없습니다. 설정해도 조용히 무시되고 실제 아카이브에 기록됩니다.

## 메시지 전송

전송은 되돌릴 수 없고 다른 사람에게 도달할 수 있습니다. 먼저 대상 방만
확인하는 `--dry-run`을 사용하십시오.

```bash
katok send --chat <chat-id> --dry-run --json
katok send --room "정확한 방 이름" --dry-run --json
```

방 이름은 고유하지 않습니다. 채팅 목록에 같은 이름의 방이 둘 이상이면 `--room`은 고르지 않고 거부합니다. `--chat`은 아카이브의 마지막 메시지 시각으로 채팅 목록의 줄을 고릅니다. 이미 열린 창은 제목, 곧 이름만 보여 주므로 어느 방의 창인지 알 수 없습니다. 그래서 아카이브에 같은 이름의 방이 둘 이상이면 `--chat`은 열린 창이 하나뿐이어도 그 창을 바로 쓰지 않고 채팅 목록에서 방을 엽니다. 클릭 뒤에 새로 뜬 창이 그 방입니다. 원래 열려 있던 창은 클릭이 그 창을 앞으로 가져왔고 약 2초 동안 새 창이 뜨지 않을 때만 그 방으로 봅니다. 그래서 `--dry-run` 뒤에 같은 `--chat`으로 보내도 됩니다. 이때는 화면을 잠깐 쓰고 약 2초가 더 걸립니다. `--no-open`이면 열지 않고 거부합니다. 같은 이름의 창이 둘 열려 있으면 언제나 거부합니다. 아카이브는 sync한 방만 압니다. `sync --chat`으로 일부 방만 넣은 아카이브는 넣지 않은 같은 이름의 방을 세지 못합니다.

실제 텍스트·이미지 전송 또는 초안 입력에는
[`ACCEPTABLE_USE_POLICY.md`](ACCEPTABLE_USE_POLICY.md)와
[`DISCLAIMER.md`](DISCLAIMER.md)를 읽었다는 명시적 확인이 필요합니다.

```bash
katok send --chat <chat-id> --text "확인한 메시지" --accept-use-policy --json
katok send --chat <chat-id> --image ./photo.jpg --accept-use-policy --json
katok send --chat <chat-id> --text "검토할 초안" --draft --accept-use-policy --json
```

`--accept-use-policy`는 법률 준수나 카카오의 승인을 보증하지 않습니다. 불법
스팸, 사칭·계정 도용, 신고·차단·거부 이후의 연락, 스토킹·괴롭힘, 반복·대량·
무인 전송, 개인정보 침해 및 보호조치 우회에는 사용할 수 없습니다. 업무용
광고·알림은 카카오톡 채널, 비즈메시지, 알림톡 등 목적에 맞는 공식 제품을
사용하십시오.

이 구현의 네트워크 경계는 다음과 같습니다.

- `katok send` 자체는 HTTP, WebSocket, 소켓 또는 카카오 원격 비공개
  프로토콜·비공식 API를 직접 호출하지 않습니다.
- 로컬 Accessibility, `CGEvent`, pasteboard, AppKit, 로컬 파일과 katok
  아카이브만 사용합니다.
- 실제 네트워크 전송은 로그인된 공식 KakaoTalk 앱이 수행합니다.
- 이 설명은 `send` 경로에 한정됩니다. `media` 명령의 presigned CDN 다운로드
  및 최초 모델 artifact 준비 등 다른 기능은 네트워크를 사용할 수 있습니다.

## CLI 명령 요약

```bash
katok --version
katok doctor --json
katok source chats --source macos --json
katok sync --source macos --json
katok sync --source macos --incremental --json
katok sync --source macos --chat <chat-id> --incremental --json
katok sync --source macos --chat <chat-id> --since <unix-ms> --json
katok sync --json
katok index --json
katok search keyword "보고서" --json
katok search keyword "보고서" --order newest --limit 5 --json
katok search bm25 "보고서" --json
katok search semantic "회의 보고서" --json
katok chunk get <chunk-id> --json
katok chunk context <chunk-id> --json
katok chunk parent <chunk-id> --json
katok transcript --chat <chat-id> --json
katok transcript --chat <chat-id> --since 2026-07-19T15:00:00Z --json
katok media get --chat <chat-id> --no-cdn --json
katok wipe-index --yes --json
katok forget --chat <chat-id> --json
katok forget --chat <chat-id> --yes --json
katok send --chat <chat-id> --dry-run --json
katok send --chat <chat-id> --text "메시지" --accept-use-policy --json
```

`transcript`와 `messages`의 `--since`·`--until`은 RFC3339 시각을 받습니다. 오프셋은 `Z`, `+09:00` 어느 것이든 되고, 같은 순간이면 같은 행을 고릅니다. 아카이브는 시각을 UTC `+00:00` 꼴로 저장하므로 katok이 비교 전에 그 꼴로 바꿉니다. 날짜만 주면(`2026-07-20`) 그날 00:00:00 UTC입니다. `--until`에 날짜만 주면 그날의 시작까지만 들어갑니다. 그 밖의 값은 종료 코드 2의 인자 오류입니다.

`katok transcript`는 한 채팅방에서 실제로 오간 말을 시간 순서대로 Markdown 파일로 내보냅니다. 검색이 "어떤 chunk가 관련 있나"에 답한다면 이 명령은 "무슨 말이 오갔나"에 답하므로, 방 하나를 밀린 채로 따라 읽을 때 씁니다. 라이브 카카오톡 DB가 아니라 아카이브를 읽으므로 최근 대화가 필요하면 `sync`를 먼저 실행합니다. 범위에 메시지가 없으면 파일을 만들지 않고, 파일 이름에 message id 범위가 들어가므로 나중 실행이 이전 결과를 덮어쓰지 않습니다. 카카오톡 시스템 메시지(입장·퇴장·초대)는 아카이브에는 남고 transcript에서만 빠집니다.

`katok forget --chat <chat-id>`는 한 방을 아카이브에서 지웁니다. `--chat`은 여러 번 줄 수 있습니다. `--yes` 없이 실행하면 아무것도 지우지 않고 지울 행 수만 테이블별로 보여 줍니다. `--yes`를 주면 그 방의 `messages`·`chats`·`chunks`·`chunks_fts`·`chunk_messages`·`chunk_parent_refs`·`parent_chunks`·`parent_chunk_children`·`reply_edges`·`chat_floors` 행을 한 트랜잭션으로 지우고, 기본 위치(`<data-dir>/media/<chat-id>`, `<data-dir>/transcripts`)의 저장된 미디어와 transcript도 지웁니다. `--out`으로 다른 곳에 저장한 파일은 추적하지 않습니다. 다른 방에서 이 방 메시지에 단 답장은 행이 남고 부모만 `parent_not_in_archive`로 바뀝니다. semantic 벡터는 방 단위로 지울 수 없으므로, 인덱스가 있으면 semantic 디렉터리에 `REBUILD_REQUIRED` 표식을 남기고 `next_step`으로 `katok index --full --json`을 안내합니다. `doctor --json`의 `semantic_rebuild_required`가 이 표식을 보여 줍니다. 다음 `katok index`는 아카이브의 현재 창만으로 새 generation을 만들고 옛 generation을 지우므로, 시작 전에 있던 표식을 지웁니다(실행 도중에 생긴 표식은 남깁니다). 카카오톡 원본은 건드리지 않으므로, 그 방을 읽는 sync가 다시 돌면 과거 기록까지 전부 다시 들어옵니다(커서 초기화는 위 커서 설계 참고). 먼저 `sync --chat` 목록에서 빼세요.

권한 진단이 필요할 때만:

```bash
katok doctor --macos-probe --json
```

`doctor --json`의 freshness 예:

```json
{
  "freshness": {
    "last_sync": {
      "completed_at": "2026-06-15T05:00:00Z",
      "source": "macos",
      "total_messages": 12345,
      "chunks": 6789
    },
    "last_index": {
      "completed_at": "2026-06-15T05:03:00Z",
      "embedder": "embeddinggemma/local",
      "vectorstore": "local",
      "semantic_units": "parent_windows",
      "embedded_texts": 42
    },
    "recommendation": {
      "sync_before_search": false,
      "index_before_semantic_search": false,
      "reason": "archive and semantic index have completed at least once; re-run sync/index when freshness matters"
    }
  }
}
```

## 앱에 사이드카로 넣을 때

`katok`을 다른 앱이 번들해 부를 때 쓰는 출력입니다. `--json` 출력은 필드를 더하기만 하고 기존 필드를 지우거나 뜻을 바꾸지 않습니다. `tests/fixtures/json-keys/`의 골든 파일이 Enter Message가 의존하는 키 계약이고, `tests/json_contract.rs`가 키가 빠지면 실패합니다(새 키는 `UPDATE_GOLDEN=1 cargo test --test json_contract`로 골든을 갱신해 함께 커밋합니다). 방 단위 sync는 `sync --chat`, 방 삭제는 `forget`(위 설명)을 씁니다.

`katok messages --chat <chat-id> --since <시각> --json`에 앞서 받은 메시지의 `timestamp`를 그대로 넘기면 그 메시지부터 나옵니다. `--since`는 그 시각을 포함하고, `Z` 꼴로 바꿔 넘겨도 같습니다. 출력의 `since`·`until`은 넘긴 문자열을 그대로 돌려줍니다.

`katok messages --chat <chat-id> --json`의 각 메시지에는 멘션 세 필드가 붙습니다.

- `mentioned_user_ids`: 그 메시지가 멘션한 사용자 id 배열입니다. 카카오톡 `attachment` 메타데이터의 최상위 `mentions[].user_id`만 읽고, 답장이 인용한 메시지의 `src_mentions`는 읽지 않습니다. 본문의 "@"는 근거로 쓰지 않습니다. id는 JavaScript 숫자로 정확히 담기지 않을 만큼 크므로 `sender_id`처럼 10진 문자열입니다. 멘션이 없으면 빈 배열입니다.
- `mentions_all`: 방 전체 멘션(`all_mention`)이면 `true`입니다.
- `mentions_me`: `mentioned_user_ids`에 내 user_id가 있으면 `true`입니다. 내 user_id 평문 없이 멘션 여부를 알기 위한 필드입니다. `is_mine`과 같은 규칙으로 비교합니다. id의 sha256을 아카이브의 `account_hash`와 견주고, `--me <id>`를 주면 그 값과 견줍니다. 방 전체 멘션은 `mentions_all`에만 나타나고 이 값을 켜지 않습니다.

멘션은 macOS 소스만 채웁니다. 이 필드가 생기기 전에 아카이브된 메시지는 플래그 없는 sync가 원본을 다시 읽을 때 채워지고, 그 전에는 빈 값으로 보입니다. 멘션이 없는 기존 행은 업그레이드 뒤 sync에서 바뀐 행으로 세지 않습니다.

`katok rooms --json`의 각 방에는 카카오톡 `NTChatRoom`이 기록한 읽음 위치 세 필드가 붙습니다. 어느 값이 "내가 읽은 곳"을 정확히 따라가는지 아직 확정하지 않았으므로 셋 다 내보냅니다.

- `last_seen_log_id`: `lastSeenLogId`입니다.
- `new_message_count`: `countOfNewMessage`입니다.
- `my_watermark_log_id`: `watermarkKeys`/`watermarkValues` 쌍에서 내 user_id 자리의 값입니다.

log id는 `mentioned_user_ids`와 같은 이유로 10진 문자열이고, `new_message_count`는 숫자입니다. 원본에 열이 없거나 값을 읽지 못하면 `null`입니다. 값은 macOS sync가 아카이브의 `chats`에 저장한 것이고 `rooms`는 원본을 직접 읽지 않습니다. sync는 증분이든 아니든 매번 모든 방(`--chat`이면 그 방들)의 읽음 위치를 다시 읽으므로, 새 메시지 없이 카카오톡에서 방을 읽기만 해도 다음 sync에 반영됩니다. 아카이브에 없는 방은 읽음 위치만으로 추가되지 않습니다.

같은 `rooms --json`에는 방 구성 세 필드도 붙습니다. 읽음 위치와 같은 방식으로 매 macOS sync가 다시 읽어 저장하고, 원본에 열이 없거나 값을 읽지 못하면 `null`입니다.

- `member_count`: `NTChatRoom.activeMembersCount`, 나를 포함한 방 인원입니다. `displayMemberIds`는 카카오톡이 미리보기로 보여 주는 몇 명뿐이라 인원수로 쓰지 않습니다.
- `member_user_ids`: `displayMemberIds`의 user_id에서 나를 뺀 10진 문자열 배열입니다. 전체 멤버 목록이 아니라 미리보기 목록입니다. 미리보기가 없는 1:1 방은 `directChatMemberUserId`(상대)를 씁니다.
- `is_open_chat`: `linkId`가 0이 아니면 `true`입니다.

연결 상태는 `katok doctor --json`의 `kakao_app`과 `semantic_rebuild_required`, 권한 상태는 `katok doctor --macos-probe --json`의 `source_access`로 봅니다(위 설치 절). `source_access`를 기본 `doctor`에 넣지 않은 이유는 기본 `doctor`가 권한 프롬프트를 띄우지 않는다는 약속 때문입니다. 이 약속에 기대어 `doctor --json`을 자동으로 부르는 호출자(예: semantic search 전마다 doctor를 확인하는 MCP 서버)가 있습니다.

## 개인정보와 로컬 파일

이 프로젝트가 다루는 파일은 모두 민감 정보로 취급합니다.

- 카카오톡 DB 경로와 SQLCipher 관련 정보
- 정규화된 메시지 아카이브
- semantic documents
- embedding cache와 vector index
- 검색 근거와 로그

생성된 아카이브, 인덱스, cache, 로그는 git에 넣지 않습니다. 자동화 테스트는 합성 fixture만 사용합니다. 실제 카카오톡 smoke test는 수동으로만 수행하고, 사용자가 명시하지 않은 대화 원문은 출력하지 않습니다.

## 개발

```bash
cargo fmt --all -- --check
cargo build
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
python3 scripts/verify_release_config.py
```

## 참고 프로젝트

아래 프로젝트들은 조사 과정의 참고 자료입니다. 현재 `katok`의 주 경로는 macOS 로컬 DB를 개인 Mac 안의 아카이브, BM25 index, EmbeddingGemma vector index로 바꾸는 방식입니다.

- `silver-flight-group/kakaocli`: macOS local DB read/search/sync CLI.
- `JungHoonGhae/openkakao-cli`: local DB read/search plus LOCO-oriented flows.
- `xistoh162108/kakaotalk_analyzer`: export CSV analysis with embedding and SPLADE ideas.
- `teddylee777/kakaotalk-gpt`: export txt/csv RAG with FAISS/Chroma retrievers.
- `sanggubot/doppelganger-gpt`: KakaoTalk txt to Chroma example.
- `uoneway/kakaotalk_msg_preprocessor`: exported txt parser.
- `claudianus/kakaotalk-chat-analyzer`: CSV export to anonymized HTML report.
