/**
 * Where tools may write files. An `out_dir` chosen by the caller must stay inside the data
 * directory or inside one of the roots the operator lists in KATOK_MCP_OUT_ROOTS (colon
 * separated absolute paths), so a prompt cannot point an export or a download at an arbitrary
 * folder. `~` expands to the home directory. Symlinks are resolved before comparing.
 */

import { realpathSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { dataDir, type Env } from "./paths.js";

export const OUT_ROOTS_ENV = "KATOK_MCP_OUT_ROOTS";

export class OutDirError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "OutDirError";
  }
}

/** `~` and `~/x` become home-relative; everything else is returned unchanged. */
export function expandHome(value: string, home: string = os.homedir()): string {
  if (value === "~") return home;
  if (value.startsWith("~/")) return path.join(home, value.slice(2));
  return value;
}

/** The data directory plus the absolute entries of KATOK_MCP_OUT_ROOTS, in that order. */
export function allowedOutRoots(env: Env, home: string = os.homedir()): string[] {
  const roots = [dataDir(env)];
  for (const entry of (env[OUT_ROOTS_ENV] ?? "").split(":")) {
    const expanded = expandHome(entry.trim(), home);
    // A relative entry would depend on the server's working directory: ignore it.
    if (expanded !== "" && path.isAbsolute(expanded)) roots.push(path.resolve(expanded));
  }
  return roots;
}

/** realpath of the deepest existing ancestor, with the not-yet-existing tail appended. */
function canonical(target: string): string {
  const tail: string[] = [];
  let current = target;
  for (;;) {
    try {
      return path.join(realpathSync(current), ...tail);
    } catch {
      const parent = path.dirname(current);
      if (parent === current) return target;
      tail.unshift(path.basename(current));
      current = parent;
    }
  }
}

function isInside(root: string, candidate: string): boolean {
  const rel = path.relative(root, candidate);
  return rel === "" || (rel !== ".." && !rel.startsWith(`..${path.sep}`) && !path.isAbsolute(rel));
}

/**
 * The absolute directory for a caller-supplied `out_dir`, or an OutDirError that names the
 * allowed locations. Nothing is created here.
 */
export function resolveOutDir(raw: string, env: Env, home: string = os.homedir()): string {
  const resolved = path.resolve(expandHome(raw, home));
  const roots = allowedOutRoots(env, home);
  const real = canonical(resolved);
  if (roots.some((root) => isInside(canonical(root), real))) return resolved;
  throw new OutDirError(
    `out_dir ${resolved} 은(는) 허용 범위 밖이라 거부했습니다. 허용되는 위치: ${roots.join(", ")} 및 그 하위 폴더. 다른 위치가 필요하면 서버 환경변수 ${OUT_ROOTS_ENV}에 콜론(:)으로 구분한 절대경로를 추가하세요 (예: ${OUT_ROOTS_ENV}=~/Documents/katok-export).`,
  );
}
