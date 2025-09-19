import React from 'react';

type OfflineBannerProps = {
  queuedCount: number;
};

const OfflineBanner: React.FC<OfflineBannerProps> = ({ queuedCount }) => (
  <div className="cashier-banner cashier-banner--offline">
    Offline mode — {queuedCount} order{queuedCount === 1 ? '' : 's'} queued. Sales will sync automatically once reconnected.
  </div>
);

export default OfflineBanner;
