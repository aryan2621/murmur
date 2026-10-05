import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";

/**
 * One action at a time. While `run(key, fn)` is working, `busy` holds its key: buttons disable
 * themselves (and the one that started it shows a spinner), so a click made while something is
 * still happening in the background can't start it twice or land on a half-updated page.
 */
export function useBusy() {
  const [busy, setBusy] = useState<string | null>(null);
  // A ref as well, so two clicks within one render can't both get through.
  const current = useRef<string | null>(null);
  const run = useCallback(async (key: string, fn: () => unknown, errorPrefix?: string) => {
    if (current.current) return;
    current.current = key;
    setBusy(key);
    try {
      await fn();
    } catch (e) {
      if (errorPrefix !== undefined) toast.error(`${errorPrefix}${e}`);
      else throw e;
    } finally {
      current.current = null;
      setBusy(null);
    }
  }, []);
  return { busy, run };
}
