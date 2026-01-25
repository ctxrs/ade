import { useEffect, useRef, useState } from "react";

type UseAlignedNowMsOptions = {
  active?: boolean;
  intervalMs?: number;
};

export function useAlignedNowMs(options: UseAlignedNowMsOptions = {}): number {
  const { active = true, intervalMs = 1000 } = options;
  const interval = Math.max(1, intervalMs);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const lastSlotRef = useRef(Math.floor(nowMs / interval));

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    let timeoutId: number | null = null;
    let rafId: number | null = null;

    const tick = (force = false) => {
      const now = Date.now();
      const slot = Math.floor(now / interval);
      if (force || slot !== lastSlotRef.current) {
        lastSlotRef.current = slot;
        setNowMs(now);
      }
    };

    tick(true);

    const schedule = () => {
      if (cancelled) return;
      const now = Date.now();
      const delay = interval - (now % interval);
      timeoutId = window.setTimeout(() => {
        rafId = window.requestAnimationFrame(() => {
          tick();
          schedule();
        });
      }, delay);
    };

    schedule();

    const handleVisibility = () => {
      if (!document.hidden) tick(true);
    };
    document.addEventListener("visibilitychange", handleVisibility);
    window.addEventListener("focus", handleVisibility);

    return () => {
      cancelled = true;
      if (timeoutId !== null) window.clearTimeout(timeoutId);
      if (rafId !== null) window.cancelAnimationFrame(rafId);
      document.removeEventListener("visibilitychange", handleVisibility);
      window.removeEventListener("focus", handleVisibility);
    };
  }, [active, interval]);

  return nowMs;
}
