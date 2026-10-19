/**
 * A promise-chain mutex. Tasks run one at a time, in call order. A task that throws releases the
 * lock for the next one; its error goes only to its own caller.
 */
export type Lock = <T>(task: () => Promise<T>) => Promise<T>;

export function createLock(): Lock {
  let tail: Promise<unknown> = Promise.resolve();
  return <T>(task: () => Promise<T>): Promise<T> => {
    const run = tail.then(task);
    // `tail` must never reject, or one failed task would poison every later one.
    tail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  };
}
