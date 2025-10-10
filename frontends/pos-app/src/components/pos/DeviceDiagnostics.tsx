import React, { useEffect, useState } from 'react';
import { getSnapshot } from '../../services/telemetry';
import { usePrinterStatus } from '../../hooks/usePrinterStatus';

export default function DeviceDiagnostics() {
  const status = usePrinterStatus(5000);
  const [queueDepth, setQueueDepth] = useState<number>(0);
  const [lastAttempt, setLastAttempt] = useState<number | null>(null);
  const [open, setOpen] = useState<boolean>(false);

  useEffect(() => {
    const t = window.setInterval(() => {
      const snap = getSnapshot();
      const depthEntry = Array.from(snap.gauges.entries()).find(([k]) => k.startsWith('pos.print.queue_depth'));
      setQueueDepth(depthEntry ? Number(depthEntry[1]) : 0);
      const last = Array.from(snap.gauges.entries()).find(([k]) => k.startsWith('pos.print.retry.last_attempt'))?.[1];
      setLastAttempt(typeof last === 'number' ? last : null);
    }, 1500);
    return () => window.clearInterval(t);
  }, []);

  return (
    <div style={{ padding: '6px 10px', border: '1px solid #e5e7eb', borderRadius: 6, background: '#f9fafb' }}>
      <button
        type="button"
        onClick={() => setOpen(v => !v)}
        style={{ fontWeight: 600, background: 'transparent', border: 'none', cursor: 'pointer' }}
        aria-expanded={open}
      >
        Device diagnostics {open ? '▾' : '▸'}
      </button>
      {open && (
        <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', fontSize: 12, marginTop: 6 }}>
          <span>Printer: {status.state}</span>
          <span>Queue depth: {queueDepth}</span>
          <span>Last retry: {lastAttempt ? new Date(lastAttempt).toLocaleTimeString() : '—'}</span>
        </div>
      )}
    </div>
  );
}
