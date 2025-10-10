import { useEffect, useState } from 'react';
import type { DeviceStatus } from '../devices/types';
import { getPrinter } from '../receipts/printService';

export function usePrinterStatus(pollMs = 5000) {
  const [status, setStatus] = useState<DeviceStatus>({ state: 'disconnected' });

  useEffect(() => {
    let mounted = true;
    let timer: number | undefined;
    let unsubscribe: (() => void) | undefined;

    const wire = async () => {
      const printer = await getPrinter();
      // Prefer event subscription if available
      if (typeof printer.on === 'function') {
        unsubscribe = printer.on('status', st => {
          if (mounted) setStatus(st);
        });
      } else {
        const poll = async () => {
          try {
            const st = await printer.status();
            if (mounted) setStatus(st);
          } catch {
            if (mounted) setStatus({ state: 'error', code: 'device_unavailable', message: 'Unavailable' });
          }
          timer = window.setTimeout(poll, Math.max(2000, pollMs));
        };
        void poll();
      }
    };

    void wire();

    return () => {
      mounted = false;
      if (timer) window.clearTimeout(timer);
      unsubscribe?.();
    };
  }, [pollMs]);

  return status;
}
