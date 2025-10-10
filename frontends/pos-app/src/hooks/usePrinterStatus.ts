import { useEffect, useState } from 'react';
import type { DeviceStatus } from '../devices/types';
import { getPrinter } from '../receipts/printService';

export function usePrinterStatus(pollMs = 5000) {
  const [status, setStatus] = useState<DeviceStatus>({ state: 'disconnected' });

  useEffect(() => {
    let mounted = true;
    let timer: number | undefined;

    const poll = async () => {
      try {
        const printer = await getPrinter();
        const st = await printer.status();
        if (mounted) setStatus(st);
      } catch {
        if (mounted) setStatus({ state: 'error', code: 'device_unavailable', message: 'Unavailable' });
      }
      timer = window.setTimeout(poll, Math.max(2000, pollMs));
    };

    void poll();
    return () => {
      mounted = false;
      if (timer) window.clearTimeout(timer);
    };
  }, [pollMs]);

  return status;
}
