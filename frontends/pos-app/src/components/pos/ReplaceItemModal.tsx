import React, { useMemo, useState } from 'react';
import type { CartItem } from '../../CartContext';
import type { Product } from '../../hooks/useProducts';

type ReplaceItemModalProps = {
  item: CartItem;
  products: Product[];
  onReplace: (replacementProductId: string) => void;
  onRemove: () => void;
  onCancel: () => void;
};

const ReplaceItemModal: React.FC<ReplaceItemModalProps> = ({ item, products, onReplace, onRemove, onCancel }) => {
  const [replacementId, setReplacementId] = useState<string>('');

  const replacementOptions = useMemo(() => products.filter(product => product.id !== item.id), [item.id, products]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl max-w-md w-full p-6">
        <h3 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-3">Item Unavailable</h3>
        <p className="text-sm text-gray-600 dark:text-gray-300 mb-4">
          {item.name} is no longer available. Replace it with another product or remove it from the cart.
        </p>
        <label className="block text-sm font-medium text-gray-600 dark:text-gray-300 mb-2" htmlFor="replace-item-select">
          Replace with
        </label>
        <select
          id="replace-item-select"
          value={replacementId}
          onChange={event => setReplacementId(event.target.value)}
          className="w-full px-3 py-2 border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-cyan-500 mb-4"
        >
          <option value="">Select a product</option>
          {replacementOptions.map(product => (
            <option key={product.id} value={product.id}>
              {product.name} — ${product.price.toFixed(2)}
            </option>
          ))}
        </select>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="px-3 py-2 text-sm text-gray-600 hover:text-gray-800"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onRemove}
            className="px-3 py-2 text-sm text-red-600 hover:text-red-700"
          >
            Remove Item
          </button>
          <button
            type="button"
            onClick={() => {
              if (replacementId) {
                onReplace(replacementId);
                setReplacementId('');
              }
            }}
            disabled={!replacementId}
            className="px-3 py-2 text-sm bg-cyan-600 text-white rounded hover:bg-cyan-700 disabled:opacity-50"
          >
            Replace
          </button>
        </div>
      </div>
    </div>
  );
};

export default ReplaceItemModal;
