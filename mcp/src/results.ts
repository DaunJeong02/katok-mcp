/**
 * Result shaping for tools whose raw katok JSON is too large to hand back as is. Pure.
 */

/**
 * `katok index --json` lists every indexed window in `documents[]` (thousands of entries,
 * megabytes). Unless `verbose`, replace that array with its length as `document_count` and keep
 * every other field unchanged.
 */
export function shrinkIndexResult(data: unknown, verbose: boolean): unknown {
  if (verbose || typeof data !== "object" || data === null || Array.isArray(data)) return data;
  const { documents, ...rest } = data as Record<string, unknown>;
  if (!Array.isArray(documents)) return data;
  return { ...rest, document_count: documents.length };
}
