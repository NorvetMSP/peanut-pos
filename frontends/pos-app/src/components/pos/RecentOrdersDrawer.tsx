import React, { useEffect } from 'react';
import { useOrders } from '../../OrderContext';

type RecentOrdersDrawerProps = {
  open: boolean;
  onClose: () => void;
};

const RecentOrdersDrawer: React.FC<RecentOrdersDrawerProps> = ({ open, onClose }) => {
  const { recentOrders } = useOrders();

  useEffect(() => {
    if (!open) return;
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKey);
    return () => {
      window.removeEventListener('keydown', handleKey);
    };
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-40 bg-black/30 flex justify-end">
      <div className="relative ml-auto h-full w-full max-w-sm bg-white dark:bg-gray-800 shadow-xl p-4 overflow-y-auto">
        <h2 className="text-xl font-bold mb-4 text-gray-800 dark:text-gray-100">Recent Orders</h2>
        {recentOrders.length === 0 ? (
          <p className="text-gray-600 dark:text-gray-400">No recent orders.</p>
        ) : (
          <ul className="space-y-3">
            {recentOrders.map(order => (
              <li key={order.reference || order.id} className="p-3 bg-gray-50 dark:bg-gray-700 rounded">
                <div className="flex justify-between text-sm">
                  <span className="font-semibold">Ref: {order.reference}</span>
                  <span>{new Date(order.createdAt).toLocaleString()}</span>
                </div>
                <div className="text-xs mt-1">
                  Status: <span className="font-medium">{order.status}</span>
                  {order.paymentStatus && (
                    <>
                      <br />
                      Payment: <span className="font-medium">{order.paymentStatus}</span>
                    </>
                  )}
                  <br />
                  Total: ${order.total.toFixed(2)}
                  {order.offline && (
                    <>
                      <br />
                      <span className="text-amber-600">Queued (offline)</span>
                    </>
                  )}
                  {order.note && (
                    <>
                      <br />
                      <span className="text-red-600">Error: {order.note}</span>
                    </>
                  )}
                </div>
                {order.paymentUrl && (
                  <a
                    href={order.paymentUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-cyan-700 text-xs underline mt-1 inline-block"
                  >
                    Complete Payment
                  </a>
                )}
              </li>
            ))}
          </ul>
        )}
        <button
          type="button"
          onClick={onClose}
          className="absolute top-2 right-2 text-2xl text-gray-500 hover:text-gray-700"
          aria-label="Close recent orders"
        >
          &times;
        </button>
      </div>
    </div>
  );
};

export default RecentOrdersDrawer;
