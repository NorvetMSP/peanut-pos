import React from 'react';

type QueuedOrdersBannerProps = {
  count: number;
  syncing: boolean;
  onSync: () => void;
};

const QueuedOrdersBanner: React.FC<QueuedOrdersBannerProps> = ({ count, syncing, onSync }) => (
  <div className="cashier-banner cashier-banner--queue">
    {syncing ? (
      <>Synchronizing queued orders...</>
    ) : (
      <>
        {count} order{count === 1 ? '' : 's'} awaiting sync.
        <button type="button" className="cashier-queue-sync" onClick={onSync}>
          Retry Sync
        </button>
      </>
    )}
  </div>
);

export default QueuedOrdersBanner;
