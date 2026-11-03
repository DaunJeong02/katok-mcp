---
type: 매뉴얼
title: katok-mcp 전송
topic: katok-mcp
---

# 전송

전송은 되돌릴 수 없습니다. 기본은 방 확인(`dry_run`)만 됩니다.

## 켜는 조건

| mode | 켜는 조건 |
|---|---|
| `dry_run` | 항상 |
| `draft` | `KATOK_MCP_ACCEPT_USE_POLICY=1` |
| `send` | 위 조건 + `KATOK_MCP_ALLOW_SEND=1` + 호출 때 `confirm: true` |

보낼 방은 `KATOK_MCP_ALLOWED_CHATS=<chat_id,...>` 로 제한합니다.

## 환경변수 넣기

**Claude Code** — 저장소 루트에서 다시 등록합니다.

```bash
claude mcp remove katok-mcp -s user
claude mcp add --scope user katok-mcp \
  -e KATOK_MCP_ACCEPT_USE_POLICY=1 -e KATOK_MCP_ALLOWED_CHATS=<chat_id,...> \
  -- "$(command -v node)" "$PWD/mcp/dist/index.js"
```

**Codex** — `~/.codex/config.toml` 을 고칩니다.

```toml
[mcp_servers.katok-mcp]
env = { KATOK_MCP_ACCEPT_USE_POLICY = "1", KATOK_MCP_ALLOWED_CHATS = "<chat_id,...>" }
```

`scripts/install.sh` 를 다시 실행해도 두 도구의 환경변수는 그대로 남습니다.

## 금지

반복 · 대량 · 예약 전송, 스팸, 사칭에는 쓰지 마십시오.
