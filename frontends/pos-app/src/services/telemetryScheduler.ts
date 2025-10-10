import { flushTelemetry } from './telemetry';

let intervalId: number | null = null;

export function startTelemetryScheduler(getAuthToken?: () => string | undefined) {
  const url = import.meta.env.VITE_TELEMETRY_INGEST_URL as string | undefined;
  if (!url) return;
  const iv = Number(import.meta.env.VITE_TELEMETRY_MIN_INTERVAL_MS ?? 5000);
  if (intervalId) return;
  intervalId = window.setInterval(() => {
    const token = getAuthToken?.();
    void flushTelemetry({ endpoint: url, authToken: token, minIntervalMs: iv });
  }, Math.max(2000, iv));
}

export function stopTelemetryScheduler() {
  if (intervalId) {
    window.clearInterval(intervalId);
    intervalId = null;
  }
}
