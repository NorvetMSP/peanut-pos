// Minimal in-memory telemetry for POS; can be wired to backend later.
export type Labels = Record<string, string | number>;

type CounterKey = string;
type GaugeKey = string;

const counters = new Map<CounterKey, number>();
const gauges = new Map<GaugeKey, number>();

let consoleEnabled = false;
let lastSentAt = 0;

export function enableConsoleTelemetry(enable: boolean) {
  consoleEnabled = !!enable;
}

function keyWithLabels(name: string, labels?: Labels): string {
  if (!labels || Object.keys(labels).length === 0) return name;
  const parts = Object.keys(labels)
    .sort()
    .map(k => `${k}=${String(labels[k])}`);
  return `${name}{${parts.join(',')}}`;
}

export function incCounter(name: string, by = 1, labels?: Labels) {
  const k = keyWithLabels(name, labels);
  const v = (counters.get(k) ?? 0) + by;
  counters.set(k, v);
  if (consoleEnabled) console.debug(`[telemetry] counter ${k} = ${v}`);
}

export function setGauge(name: string, value: number, labels?: Labels) {
  const k = keyWithLabels(name, labels);
  gauges.set(k, value);
  if (consoleEnabled) console.debug(`[telemetry] gauge ${k} = ${value}`);
}

export function setTimestamp(name: string) {
  setGauge(name, Date.now());
}

export function getSnapshot() {
  return {
    counters: new Map(counters),
    gauges: new Map(gauges),
  };
}

export function resetTelemetry() {
  counters.clear();
  gauges.clear();
}

// Ingestion client (optional)
type IngestOptions = { endpoint: string; authToken?: string; minIntervalMs?: number; labels?: Labels };

export async function flushTelemetry(opts: IngestOptions) {
  const now = Date.now();
  const minIv = Math.max(1000, opts.minIntervalMs ?? 5000);
  if (now - lastSentAt < minIv) return; // rate-limit
  lastSentAt = now;
  const snapshot = getSnapshot();
  const payload = {
    ts: now,
    labels: opts.labels ?? {},
    counters: Array.from(snapshot.counters.entries()).map(([k, v]) => ({ name: k, value: v })),
    gauges: Array.from(snapshot.gauges.entries()).map(([k, v]) => ({ name: k, value: v })),
  };
  try {
    await fetch(opts.endpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(opts.authToken ? { Authorization: `Bearer ${opts.authToken}` } : {}),
      },
      body: JSON.stringify(payload),
    });
  } catch (e) {
    if (consoleEnabled) console.warn('[telemetry] ingest failed', e);
  }
}
