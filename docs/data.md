---
type: 매뉴얼
title: katok-mcp 데이터와 키
topic: katok-mcp
---

# 데이터와 키

| 항목 | 위치 |
|---|---|
| 데이터 폴더 | `~/Library/Application Support/katok-mcp` |
| 아카이브 | `<데이터 폴더>/archive.sqlite3` (SQLCipher 암호화) |
| 첨부 · 내보내기 | `<데이터 폴더>/media/` · `<데이터 폴더>/transcripts/` |
| 의미 검색 모델 | `~/Library/Caches/katok/fastembed` |
| 키 | 키체인 `com.enternext.katok-mcp` / `archive-key` |

## 키 백업

**키를 잃으면 아카이브를 복구할 수 없습니다.** 아래 명령으로 키를 확인해 비밀번호 관리자 같은 안전한 곳에 둡니다.

```bash
security find-generic-password -s com.enternext.katok-mcp -a archive-key -w
```

키를 대화 · 커밋 · 로그에 붙여 넣지 마십시오.
