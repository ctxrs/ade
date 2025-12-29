import { useEffect, useState } from "react";

export function useRelativeNowMs(intervalMs = 60_000): number {
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (intervalMs <= 0) return;
    const tick = () => setNowMs(Date.now());
    const timer = window.setInterval(tick, intervalMs);
    const handleVisibility = () => {
      if (!document.hidden) tick();
    };
    document.addEventListener("visibilitychange", handleVisibility);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, [intervalMs]);

  return nowMs;
}
