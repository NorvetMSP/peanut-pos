import { useEffect, useState } from 'react';

type ToastProps = {
  message: string;
  actionLabel?: string;
  onAction?: () => void;
  durationMs?: number;
  onClose?: () => void;
};

export default function Toast({ message, actionLabel, onAction, durationMs = 4000, onClose }: ToastProps) {
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    const t = window.setTimeout(() => {
      setVisible(false);
      onClose?.();
    }, Math.max(1000, durationMs));
    return () => window.clearTimeout(t);
  }, [durationMs, onClose]);

  if (!visible) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: 'fixed',
        bottom: '16px',
        right: '16px',
        background: '#1f2937',
        color: 'white',
        padding: '12px 16px',
        borderRadius: 6,
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
            color: '#60a5fa',
            border: '1px solid #60a5fa',
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
