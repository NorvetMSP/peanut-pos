import type { DeviceStatus } from '../../devices/types';

type Props = { status: DeviceStatus };

export default function PrinterStatusBanner({ status }: Props) {
  if (status.state === 'ready' || status.state === 'connecting') return null;

  let message = '';
  let bg = '#1f2937';
  if (status.state === 'disconnected') {
    message = 'Receipt printer not connected';
    bg = '#374151';
  } else if (status.state === 'busy') {
    message = `Printer busy${status.task ? `: ${status.task}` : ''}`;
    bg = '#4b5563';
  } else if (status.state === 'error') {
    message = `Printer error: ${status.message ?? status.code}`;
    bg = '#7f1d1d';
  }

  return (
    <div
      role="status"
      style={{
        background: bg,
        color: 'white',
        padding: '8px 12px',
        borderLeft: '4px solid #f59e0b',
        fontSize: 14,
      }}
    >
      {message}
    </div>
  );
}
