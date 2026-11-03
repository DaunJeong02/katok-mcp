---
type: 매뉴얼
title: katok-mcp 도구
topic: katok-mcp
---

# 도구 (13개)

| 분류 | 도구 | 하는 일 |
|---|---|---|
| 시작 | `katok_init` | 키 생성 · 첫 동기화 |
| | `katok_sync` | 새 메시지 가져오기 |
| | `katok_doctor` | 상태 · 권한 점검 |
| 읽기 | `katok_rooms` | 방 목록 |
| | `katok_messages` | 한 방의 메시지 |
| | `katok_transcript` | 대화를 Markdown 으로 내보내기 |
| 검색 | `katok_search` | 키워드 · BM25 · 의미 검색 |
| | `katok_index` | 의미 검색 인덱스 만들기 |
| | `katok_chunk` | 검색 결과 앞뒤 원문 |
| 첨부 | `katok_media_get` | 한 방의 첨부 저장 |
| | `katok_media_backfill` | 모든 방의 첨부 저장 |
| 전송 | `katok_list_windows` | 열린 채팅 창 목록 |
| | `katok_send` | 메시지 보내기 (기본 꺼짐, [전송](send.md)) |

## 참고

- **의미 검색**: `katok_index` 를 먼저 한 번 실행합니다. 처음에 로컬 임베딩 모델(약 200MB)을 받아 `~/Library/Caches/katok/fastembed` 에 둡니다. 색인은 이 Mac 의 CPU 로 돌고, 이후에는 새 메시지만 색인합니다.
- **첨부**: 카카오톡 첨부 링크는 약 14일 뒤 만료됩니다. 남기려면 `katok_media_backfill` 을 주기적으로 실행합니다.
