/**
 * Turns a failure into "what to do next". Matching runs on the error text only, so a hint can
 * never contain more than the error already shows (and the error never contains the key).
 */

const RULES: readonly { test: RegExp; hint: string }[] = [
  {
    test: /operation not permitted|permission denied|full disk access|EPERM|os error 1\b/i,
    hint: "다음 행동: macOS 시스템 설정 > 개인정보 보호 및 보안 > 전체 디스크 접근 권한에서 이 서버를 띄우는 앱(Terminal, Claude Code 등)을 켠 뒤 앱을 다시 시작하고 재시도하세요. `katok permissions macos`가 설정 화면을 엽니다.",
  },
  {
    test: /binary not found/i,
    hint: "다음 행동: 저장소 루트에서 scripts/install.sh를 실행하거나 cli/에서 `cargo build --release`로 katok을 빌드하세요. 다른 위치의 바이너리는 KATOK_BIN으로 지정합니다.",
  },
  {
    test: /timed out or was killed/i,
    hint: "다음 행동: 범위를 줄여 다시 시도하세요 (chat_id, limit, incremental 사용). 같은 인자로 반복하면 같은 지점에서 다시 멈춥니다.",
  },
  {
    test: /no chat .* in the archive|run sync first|archive .* not found|no such table/i,
    hint: "다음 행동: katok_sync(처음이면 katok_init)로 아카이브를 채운 뒤 katok_rooms로 chat_id를 확인하고 재시도하세요.",
  },
  {
    test: /cdn-expired|\b410\b/i,
    hint: "다음 행동: 첨부의 다운로드 링크가 만료됐습니다. 이미 받은 파일만 남아 있으니 새 첨부는 수신 직후 katok_media_backfill로 받으세요.",
  },
];

export function nextStepHint(text: string): string | null {
  for (const rule of RULES) {
    if (rule.test.test(text)) return rule.hint;
  }
  return null;
}
