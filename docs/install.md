---
type: 매뉴얼
title: katok-mcp 설치 · 운영
topic: katok-mcp
---

# 설치 · 운영

## 설치 스크립트가 하는 일

`scripts/install.sh` 는 다시 실행해도 안전합니다.

1. `katok` CLI 를 준비합니다.
   - Rust 1.91 이 있으면 소스로 빌드합니다.
   - 없으면 [Releases](https://github.com/DaunJeong02/katok-mcp/releases) 에서 받아 SHA-256 을 검증합니다.
   - Rust 가 있어도 Release 를 쓰려면 `KATOK_PREBUILT=1 scripts/install.sh`.
2. MCP 서버(`mcp/`)를 빌드합니다.
3. 설치된 도구에 모두 등록합니다.
   - Claude Code: `claude mcp add --scope user katok-mcp`
   - Codex: `codex mcp add katok-mcp` + `~/.codex/config.toml` 에 `tool_timeout_sec = 1800`

Release 바이너리는 Apple 서명이 없는 ad-hoc 서명입니다. `curl` 로 받으므로 Gatekeeper 격리가 붙지 않습니다.
Apple Silicon 전용입니다. Intel Mac 은 Rust 를 설치해 소스로 빌드합니다.

## 업데이트

```bash
git pull && scripts/install.sh
```

끝나면 Claude Code · Codex 를 다시 시작합니다.

## 자동 동기화 (선택)

30분마다 동기화 · 첨부 저장 · 인덱스를 돌립니다. `katok_init` 을 먼저 한 번 실행합니다.

```bash
sed -e "s|__REPO__|$PWD|g" -e "s|__HOME__|$HOME|g" \
  -e "s|__DATA_DIR__|$HOME/Library/Application Support/katok-mcp|g" \
  scripts/launchd/com.enternext.katok-mcp.sync.plist.example \
  > ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.enternext.katok-mcp.sync.plist
```

이 작업은 `/bin/bash` 와 `cli/target/release/katok` 에 전체 디스크 접근을 따로 줘야 합니다.
업데이트로 바이너리가 바뀌면 다시 승인합니다.

## 문제 해결

**"katok_doctor 실행해줘"** 라고 말하고 결과의 `recommendation` · `warning` 을 따릅니다.

| 증상 | 조치 |
|---|---|
| `warning` 에 바이너리가 낡았다고 나온다 | `scripts/install.sh` 를 다시 실행합니다 |
| 권한 오류 | 도구를 띄우는 앱에 전체 디스크 접근을 켜고 그 앱을 다시 시작합니다 |
| Codex 에서 init · sync 가 시간 초과 | `config.toml` 의 `[mcp_servers.katok-mcp]` 에 `tool_timeout_sec = 1800` 이 있는지 봅니다 |
