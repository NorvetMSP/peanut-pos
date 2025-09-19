import { useEffect } from 'react';
import { useOrders } from '../OrderContext';

export const useSyncOnReconnect = (): void => {
  const { retryQueue } = useOrders();

  useEffect(() => {
    const handleOnline = () => {
      retryQueue().catch(err => console.warn('Sync on reconnect failed', err));
    };
    window.addEventListener('online', handleOnline);
    return () => {
      window.removeEventListener('online', handleOnline);
    };
  }, [retryQueue]);
};
