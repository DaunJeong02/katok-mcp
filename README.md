# KakaoTalk MCP (katok-mcp)

Claude Code · Codex 에서 **맥 카카오톡 대화를 읽고 검색**합니다. 대화는 이 Mac 밖으로 나가지 않습니다.

> **비공식 도구입니다.** 카카오가 만들거나 승인하지 않았고, 계정 이용제한 가능성이 있습니다.
> 쓰기 전에 [이용 정책](cli/ACCEPTABLE_USE_POLICY.md)과 [면책 고지](cli/DISCLAIMER.md)를 읽으십시오.

## 할 수 있는 일

| 분류 | 할 수 있는 일 | 도구 |
|---|---|---|
| 시작 | 첫 동기화 · 새 메시지 가져오기 · 상태 점검 | `katok_init` · `katok_sync` · `katok_doctor` |
| 읽기 | 방 목록 · 메시지 읽기 · 대화를 Markdown 으로 내보내기 | `katok_rooms` · `katok_messages` · `katok_transcript` |
| 검색 | 키워드 · BM25 · 의미 검색, 검색 결과 앞뒤 원문 | `katok_search` · `katok_index` · `katok_chunk` |
| 첨부 | 사진 · 동영상 · 파일 저장 | `katok_media_get` · `katok_media_backfill` |
| 전송 | 메시지 보내기 (기본 꺼짐, [전송](docs/send.md)) | `katok_list_windows` · `katok_send` |

도구별 자세한 설명은 [도구](docs/tools.md)에 있습니다.

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

## 문서

| 문서 | 내용 |
|---|---|
| [설치 · 운영](docs/install.md) | 설치 스크립트가 하는 일 · 업데이트 · 자동 동기화 · 문제 해결 |
| [도구](docs/tools.md) | MCP 도구 13개 |
| [전송](docs/send.md) | 메시지 보내기를 켜는 조건과 방법 |
| [데이터와 키](docs/data.md) | 저장 위치 · 암호화 키 백업 |
| [개발](docs/development.md) | 테스트 · 프라이버시 규칙 · CLI fork |

## 라이선스

루트는 MIT(`LICENSE`), `cli/` 는 upstream 라이선스(`cli/LICENSE`)입니다.
