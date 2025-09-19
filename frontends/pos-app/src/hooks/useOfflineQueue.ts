import { useOrders } from '../OrderContext';

type UseOfflineQueueResult = {
  queuedOrders: ReturnType<typeof useOrders>['queuedOrders'];
  isSyncing: boolean;
  retryQueue: () => Promise<void>;
};

export const useOfflineQueue = (): UseOfflineQueueResult => {
  const { queuedOrders, isSyncing, retryQueue } = useOrders();
  return {
    queuedOrders,
    isSyncing,
    retryQueue: async () => {
      await retryQueue();
    },
  };
};
