# CLAUDE.md

이 저장소(katok-mcp 모노레포)의 작업 규칙. `cli/`(katok CLI)와 `mcp/`(MCP 서버) 둘 다에 적용한다.

## 프라이버시·테스트 규칙: 저장소 전체 적용

`cli/AGENTS.md`(= `cli/CLAUDE.md`)의 프라이버시·테스트 규칙은 `cli/` 안에서만이 아니라 **이 저장소 전체**에 그대로 적용한다. 핵심만 다시 적는다.

- 실제 아카이브에서 나온 것(방 이름, 사람 이름, 호칭, 메시지 본문, 첨부 파일명, chat_id 같은 식별자)은 코드·테스트·README·주석·CHANGELOG·커밋 메시지·PR 어디에도 넣지 않는다. 규칙만 적고 그 규칙을 낳은 증거는 대화에만 둔다.
- 테스트 픽스처는 합성 데이터만 쓴다. 실제 카카오톡 설치나 실제 로컬 DB에 의존하는 자동 테스트를 만들지 않는다.
- 실제 설치를 상대로 한 확인은 수동으로만 하고, 내용이 아니라 개수·id·해시·상태로 검증한다.
- 전송은 되돌릴 수 없고 다른 사람에게 닿는다. 테스트할 방을 스스로 고르지 않는다. 대상 확인이 목적이면 `--dry-run`을 쓴다.
- 아카이브, 인덱스, 임베딩 캐시, 인증 캐시, 로그, 다운로드한 첨부는 ignore 경로에 둔다. 텔레메트리를 넣지 않는다. 프라이버시 검사를 데모 편의로 약화하지 않는다.
- README, CLI 도움말, 프라이버시 동작은 같은 변경에서 함께 맞춘다.

## cli/ 규칙

- `cli/`는 `36_katok`의 `entercut` 브랜치를 git subtree로 가져온 것이다. 변경은 작게, 별도 커밋으로 하고, 나중에 `git subtree pull`이 되도록 이력을 건드리지 않는다.
- `cli/`를 수정하면 CI가 도는 게이트를 전부 돌린다: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, `cargo publish --dry-run`, `scripts/verify_release_config.py`. 고정 툴체인(`cli/Cargo.toml`의 `rust-version`)으로 확인하고, 일부만 돌려 통과했다고 보고하지 않는다.
- `/Users/jeongdaun/enternext/01_business/36_katok`, `35_talk-2-build`, `33_enter-cut`은 읽기 전용이다. 수정·커밋하지 않는다.

## mcp/ 규칙

- **실행 표면은 `katok` CLI 하나.** MCP 서버는 카카오 DB, 키 파일, 네트워크를 직접 만지지 않는다. 예외는 키체인 접근용 `/usr/bin/security`와 키 생성용 `/usr/bin/openssl` 호출뿐이다.
- 모든 katok 호출에 `--data-dir`를 명시한다. 기본은 `~/Library/Application Support/katok-mcp`(`KATOK_MCP_DATA_DIR`로 덮어씀). enter-cut, talk-2-build 경로·키체인 항목을 기본값으로 되살리지 않는다.
- **키 값은 결과, 로그, 에러, 테스트 출력, 커밋 어디에도 나오면 안 된다.** `execFile` 에러 메시지는 argv(키 포함)를 담으므로 `security add-generic-password` 실패는 stderr만으로 메시지를 만든다. `KatokError.detail.args`에도 키가 들어가지 않게 한다.
- 새 도구는 `registerTool`로 등록하고 annotations 네 가지(readOnlyHint, destructiveHint, idempotentHint, openWorldHint)와 outputSchema를 선언한다. 결과는 `{ ok, data, out_dir? }` 래퍼(`structuredContent` 포함), 에러는 다음 행동을 담은 문장으로 쓴다. 인자 구성은 `args.ts`의 순수 함수로 두고 단위 테스트한다.
- 전송 게이트(`confirm` + `KATOK_MCP_ALLOW_SEND` + `KATOK_MCP_ACCEPT_USE_POLICY` + `KATOK_MCP_ALLOWED_CHATS`, 기본 `--no-open`, draft+image 거부, 본문 stdin 전달)를 약화하지 않는다. 자동 호출로 `send`/`draft`/`--macos-probe`를 실행하지 않는다.
- `katok media backfill --dry-run`은 CDN 요청만 건너뛴다. 사진·영상의 로컬 캐시 복호화는 dry run에서도 파일을 쓴다. 규모만 확인할 때는 `kind: ["file"]`로 한다.
- 단위 테스트는 외부 효과(키체인, openssl, 파일 시스템, katok)를 주입해 모킹한다. 실제 키체인 항목을 만들거나 지우는 테스트를 넣지 않는다.
- 변경 후 `cd mcp && pnpm check`가 통과해야 한다.

## Enter Message 사이드카 소비자

`cli/`는 Enter Message(`01_business/08_enter-message`)가 Tauri 사이드카(`externalBin`)로 번들한다. 고정 커밋의 소스를 그쪽 `release.yml`이 cargo로 빌드한다. 작업 항목과 상태의 정본은 그 저장소의 `docs/kakaotalk-tab/ROADMAP.md`다.

- **`--json` 출력이 API다.** 필드 추가만 허용한다. 삭제·의미 변경은 하지 않는다. 바꿔야 하면 새 플래그로 분기한다.
- **기본 동작을 바꾸지 않는다.** 새 기능은 새 서브커맨드나 플래그로만 넣는다. 플래그 없이 부르면 지금과 같아야 `mcp/`와 `35_talk-2-build`가 그대로 돈다.
- **호출 계약.** `--data-dir` 필수, 키는 `KATOK_ARCHIVE_KEY` 환경변수, stdout은 JSON만, 실패는 0이 아닌 종료 코드와 stderr 한 줄 원인. 이 계약을 어기면 Enter Message 상태 줄이 원인을 못 보여 준다.
- **`messages --json`의 `mentions_me`는 내 user_id 평문 없이 멘션 여부를 알기 위한 필드다.** `mentioned_user_ids`를 `is_mine`과 같은 규칙(`sha256(id) == account_hash`, `--me`가 있으면 그 값)으로 비교하고, 방 전체 멘션(`mentions_all`)은 포함하지 않는다.
- `cli/` 변경은 작게, 별도 커밋으로, 기존 규칙대로 CI 게이트 5개를 전부 돌린다. `36_katok` 반영은 별도 결정이다.
- 원격은 아직 없다. GitHub `EnterNext/katok-mcp` 등록은 사용자 확인 후에 한다.
