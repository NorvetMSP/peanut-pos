import { useCallback, useRef, useState } from 'react';
import { useOrders } from '../OrderContext';
import type { DraftOrderPayload, SubmitOrderResult } from '../OrderContext';

type UseSubmitOrderResult = {
  submit: (order: DraftOrderPayload) => Promise<SubmitOrderResult | null>;
  submitting: boolean;
};

const MIN_SUBMIT_INTERVAL_MS = 1000;

export const useSubmitOrder = (): UseSubmitOrderResult => {
  const { submitOrder } = useOrders();
  const [submitting, setSubmitting] = useState(false);
  const lastSubmitRef = useRef<number>(0);
  const submittingRef = useRef<boolean>(false);

  const wrappedSubmit = useCallback(async (order: DraftOrderPayload) => {
    const now = Date.now();
    if (submittingRef.current) return null;
    if (now - lastSubmitRef.current < MIN_SUBMIT_INTERVAL_MS) return null;

    submittingRef.current = true;
    setSubmitting(true);
    lastSubmitRef.current = now;

    try {
      const result = await submitOrder(order);
      return result;
    } catch (err) {
      throw err;
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }, [submitOrder]);

  return { submit: wrappedSubmit, submitting };
};
