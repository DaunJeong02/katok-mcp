/**
 * Is the katok binary older than the source it was built from? scripts/install.sh writes
 * `cli/target/release/katok.ref` (the commit it built) next to the binary. katok_doctor reports
 * that stamp and warns when the in-repo binary has no stamp or is older than the newest file
 * under cli/src, which is how a branch switch without a rebuild shows up. Nothing is blocked.
 */

import { type Dirent, readdirSync, readFileSync, realpathSync, statSync } from "node:fs";
import path from "node:path";

export interface BinaryBuild {
  path: string;
  /** The commit in `<binary>.ref`, or null when there is no stamp. */
  ref: string | null;
  /** The binary's modification time (ISO 8601), or null when it could not be read. */
  mtime: string | null;
}

export interface BuildFacts {
  bin: string;
  /**
   * The binary is `<repo>/cli/target/release/katok` (compared by real path, so a symlink to it
   * counts): the only binary install.sh builds and stamps.
   */
  inRepo: boolean;
  binMtimeMs: number | null;
  ref: string | null;
  /**
   * When install.sh wrote the stamp. It writes it only after a successful `cargo build`, and
   * cargo leaves the binary untouched when nothing it compiles changed, so the newer of the two
   * times is when the build was last known current.
   */
  stampMtimeMs: number | null;
  newestSourceMtimeMs: number | null;
}

export const STALE_WARNING =
  "katok 바이너리가 소스보다 오래됐습니다. scripts/install.sh 를 다시 실행하세요.";
export const UNSTAMPED_WARNING =
  "katok 바이너리에 빌드 기록(katok.ref)이 없어 소스와 맞는지 알 수 없습니다. scripts/install.sh 를 다시 실행하세요.";

/**
 * Decide what katok_doctor reports. Pure. A binary outside the repo (KATOK_BIN, ~/.cargo/bin)
 * is reported but never warned about: install.sh does not build it, so it cannot fix it.
 */
export function binaryBuildStatus(facts: BuildFacts): {
  binary_build: BinaryBuild;
  warning?: string;
} {
  const binary_build: BinaryBuild = {
    path: facts.bin,
    ref: facts.ref,
    mtime: facts.binMtimeMs === null ? null : new Date(facts.binMtimeMs).toISOString(),
  };
  if (!facts.inRepo) return { binary_build };
  if (facts.ref === null) return { binary_build, warning: UNSTAMPED_WARNING };
  const builtAt = Math.max(facts.binMtimeMs ?? -1, facts.stampMtimeMs ?? -1);
  if (builtAt >= 0 && facts.newestSourceMtimeMs !== null && builtAt < facts.newestSourceMtimeMs) {
    return { binary_build, warning: STALE_WARNING };
  }
  return { binary_build };
}

export interface BuildFs {
  /** Modification time in ms, or null when the path cannot be read. */
  mtimeMs: (file: string) => number | null;
  /** File text, or null when it cannot be read. */
  readText: (file: string) => string | null;
  /** Newest modification time of any file under `dir`, or null. */
  newestMtimeMs: (dir: string) => number | null;
  /** The path with symlinks resolved, or the path itself when it cannot be resolved. */
  realpath: (file: string) => string;
}

function newestUnder(dir: string): number | null {
  let newest: number | null = null;
  const pending = [dir];
  while (pending.length > 0) {
    const current = pending.pop() as string;
    let entries: Dirent[];
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        pending.push(full);
      } else if (entry.isFile()) {
        try {
          const mtime = statSync(full).mtimeMs;
          if (newest === null || mtime > newest) newest = mtime;
        } catch {
          // A file removed during the walk does not change the answer.
        }
      }
    }
  }
  return newest;
}

export const nodeBuildFs: BuildFs = {
  mtimeMs: (file) => {
    try {
      return statSync(file).mtimeMs;
    } catch {
      return null;
    }
  },
  readText: (file) => {
    try {
      return readFileSync(file, "utf8");
    } catch {
      return null;
    }
  },
  newestMtimeMs: newestUnder,
  realpath: (file) => {
    try {
      return realpathSync(file);
    } catch {
      return path.resolve(file);
    }
  },
};

/** Gather the facts for `bin` (source tree walked only for the in-repo binary) and decide. */
export function inspectBinaryBuild(
  bin: string,
  repo: string,
  fs: BuildFs = nodeBuildFs,
): ReturnType<typeof binaryBuildStatus> {
  const real = fs.realpath(bin);
  const inRepo = real === fs.realpath(path.join(repo, "cli", "target", "release", "katok"));
  // The stamp sits next to the real binary, not next to a symlink pointing at it.
  const stamp = `${real}.ref`;
  const ref = fs.readText(stamp)?.trim() || null;
  return binaryBuildStatus({
    bin,
    inRepo,
    binMtimeMs: fs.mtimeMs(real),
    ref,
    stampMtimeMs: ref === null ? null : fs.mtimeMs(stamp),
    newestSourceMtimeMs: inRepo ? fs.newestMtimeMs(path.join(repo, "cli", "src")) : null,
  });
}
