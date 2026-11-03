---
type: 매뉴얼
title: katok-mcp 개발
topic: katok-mcp
---

# 개발

## 구성

| 폴더 | 내용 |
|---|---|
| `cli/` | `katok` CLI (Rust). [katok](https://github.com/NomaDamas/katok) 의 fork 입니다 |
| `mcp/` | MCP 서버 (TypeScript). 모든 작업을 `katok` 로 실행합니다 |
| `scripts/` | 설치 스크립트 · launchd 템플릿 |
| `.github/workflows/release.yml` | `v*` 태그를 push 하면 arm64 `katok` 과 SHA-256 을 Release 에 올립니다 |

## 테스트

```bash
cd mcp && pnpm check    # 타입 · 린트 · 빌드 · 단위 테스트
```

`cli/` 를 고치면 [`cli/AGENTS.md`](../cli/AGENTS.md) 의 CI 게이트를 모두 돌립니다.

## 규칙

- 실제 대화 내용(방 이름 · 사람 · 본문 · `chat_id`)은 코드 · 테스트 · 문서 · 커밋에 넣지 않습니다.
- `--json` 출력은 API 입니다. 필드는 추가만 합니다.
