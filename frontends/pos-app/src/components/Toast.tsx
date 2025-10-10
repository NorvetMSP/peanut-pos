import { useEffect, useState } from 'react';

type ToastProps = {
  message: string;
  actionLabel?: string;
  onAction?: () => void;
  durationMs?: number;
  onClose?: () => void;
  variant?: 'default' | 'success' | 'error';
};

export default function Toast({ message, actionLabel, onAction, durationMs = 4000, onClose, variant = 'default' }: ToastProps) {
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    const t = window.setTimeout(() => {
      setVisible(false);
      onClose?.();
    }, Math.max(1000, durationMs));
    return () => window.clearTimeout(t);
  }, [durationMs, onClose]);

  if (!visible) return null;

  const bg = variant === 'success' ? '#065f46' : variant === 'error' ? '#7f1d1d' : '#1f2937';
  const border = variant === 'success' ? '#10b981' : variant === 'error' ? '#f87171' : '#374151';
  const actionColor = variant === 'success' ? '#a7f3d0' : variant === 'error' ? '#fecaca' : '#60a5fa';

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: 'fixed',
        bottom: '16px',
        right: '16px',
        background: bg,
        color: 'white',
        padding: '12px 16px',
        borderRadius: 6,
        border: `1px solid ${border}`,
        boxShadow: '0 4px 10px rgba(0,0,0,0.3)',
        display: 'flex',
        gap: 12,
        alignItems: 'center',
        zIndex: 9999,
      }}
    >
      <span>{message}</span>
      {actionLabel && (
        <button
          type="button"
          onClick={onAction}
          style={{
            background: 'transparent',
            color: actionColor,
            border: `1px solid ${actionColor}`,
            padding: '6px 10px',
            borderRadius: 4,
            cursor: 'pointer',
          }}
        >
          {actionLabel}
        </button>
      )}
    </div>
  );
}
