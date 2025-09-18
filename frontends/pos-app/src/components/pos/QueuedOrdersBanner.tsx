import React from 'react';

type QueuedOrdersBannerProps = {
  count: number;
  syncing: boolean;
  onSync: () => void;
};

const QueuedOrdersBanner: React.FC<QueuedOrdersBannerProps> = ({ count, syncing, onSync }) => (
  <div className="w-full bg-sky-200 text-sky-900 px-6 py-3 text-sm text-center flex items-center justify-center gap-3">
    {syncing ? (
      <>Synchronizing queued orders...</>
    ) : (
      <>
        {count} order{count === 1 ? '' : 's'} waiting to sync.
        <button
          type="button"
          onClick={onSync}
          className="ml-2 px-3 py-1 rounded bg-sky-700 text-white text-xs font-semibold hover:bg-sky-800"
        >
          Retry Sync
        </button>
      </>
    )}
  </div>
);

export default QueuedOrdersBanner;
