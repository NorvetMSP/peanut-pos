import React from 'react';
import type { PaymentMethod } from '../../OrderContext';

type SubmitSalePanelProps = {
  total: number;
  paymentMethod: PaymentMethod;
  onPaymentMethodChange: (method: PaymentMethod) => void;
  onSubmit: () => void;
  submitting: boolean;
};

const SubmitSalePanel: React.FC<SubmitSalePanelProps> = ({ total, paymentMethod, onPaymentMethodChange, onSubmit, submitting }) => (
  <div className="mt-4 bg-white dark:bg-gray-800 rounded-lg shadow p-4">
    <div className="flex items-center justify-between mb-3">
      <span className="text-lg font-bold text-gray-800 dark:text-gray-100">Total: ${total.toFixed(2)}</span>
      <select
        value={paymentMethod}
        onChange={event => onPaymentMethodChange(event.target.value as PaymentMethod)}
        className="px-3 py-1 border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-cyan-500"
        aria-label="Select payment method"
      >
        <option value="cash">Cash</option>
        <option value="card">Card</option>
        <option value="crypto">Crypto</option>
      </select>
    </div>
    <button
      type="button"
      onClick={onSubmit}
      disabled={submitting}
      className="w-full py-2 bg-cyan-600 text-white font-semibold rounded hover:bg-cyan-700 disabled:opacity-50"
    >
      {submitting ? 'Processing...' : 'Submit Sale'}
    </button>
  </div>
);

export default SubmitSalePanel;
