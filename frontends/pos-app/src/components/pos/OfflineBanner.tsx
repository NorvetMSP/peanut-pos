import React from 'react';

type OfflineBannerProps = {
  queuedCount: number;
};

const OfflineBanner: React.FC<OfflineBannerProps> = ({ queuedCount }) => (
  <div className="w-full bg-amber-200 text-amber-900 px-6 py-3 text-sm text-center">
    Offline mode — {queuedCount} order{queuedCount === 1 ? '' : 's'} queued. Sales will sync automatically once reconnected.
  </div>
);

export default OfflineBanner;
