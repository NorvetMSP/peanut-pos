import React from 'react';
import type { Product } from '../../hooks/useProducts';

type ProductGridProps = {
  products: Product[];
  onAddProduct: (product: Product) => void;
};

const ProductGrid: React.FC<ProductGridProps> = ({ products, onAddProduct }) => (
  <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-4">
    {products.map(product => (
      <div
        key={product.id}
        className="bg-white dark:bg-gray-800 shadow rounded-lg overflow-hidden flex flex-col"
      >
        {product.image_url ? (
          <img src={product.image_url} alt={product.name} className="h-28 object-cover" />
        ) : (
          <div className="h-28 bg-gray-100 dark:bg-gray-700 flex items-center justify-center text-gray-400 text-sm">
            No image
          </div>
        )}
        <div className="p-3 flex-1 flex flex-col">
          <div className="text-sm font-semibold text-gray-800 dark:text-gray-100 truncate" title={product.name}>
            {product.name}
          </div>
          {product.sku && (
            <div className="text-xs text-gray-500 dark:text-gray-400 mt-1">SKU: {product.sku}</div>
          )}
          {product.category && (
            <div className="text-xs text-gray-500 dark:text-gray-400">{product.category}</div>
          )}
          {product.description && (
            <div className="text-xs text-gray-500 dark:text-gray-400 mt-2 h-10 overflow-hidden">
              {product.description}
            </div>
          )}
          <div className="mt-auto pt-2 flex items-center justify-between">
            <span className="text-base font-bold text-gray-900 dark:text-gray-50">
              ${product.price.toFixed(2)}
            </span>
            <button
              type="button"
              onClick={() => onAddProduct(product)}
              className="px-3 py-1 text-sm rounded bg-cyan-600 text-white hover:bg-cyan-700"
            >
              Add
            </button>
          </div>
        </div>
      </div>
    ))}
  </div>
);

export default ProductGrid;
