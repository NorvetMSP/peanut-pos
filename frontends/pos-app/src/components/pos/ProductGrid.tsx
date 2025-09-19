import React from 'react';
import type { Product } from '../../hooks/useProducts';

type ProductGridProps = {
  products: Product[];
  onAddProduct: (product: Product) => void;
};

const ProductGrid: React.FC<ProductGridProps> = ({ products, onAddProduct }) => (
  <div className="cashier-product-grid">
    {products.map(product => (
      <div key={product.id} className="cashier-product-card">
        {product.image_url ? (
          <img src={product.image_url} alt={product.name} className="cashier-product-card__image" />
        ) : (
          <div className="cashier-product-card__image cashier-product-card__image--placeholder">No image</div>
        )}
        <div className="cashier-product-card__body">
          <div className="cashier-product-card__name" title={product.name}>
            {product.name}
          </div>
          <div className="cashier-product-card__meta">
            {product.sku && <span className="cashier-product-card__sku">SKU: {product.sku}</span>}
            {product.category && <span className="cashier-product-card__category">{product.category}</span>}
          </div>
          <div className="cashier-product-card__bottom">
            <span className="cashier-product-card__price">${product.price.toFixed(2)}</span>
            <button
              type="button"
              className="cashier-product-card__add"
              onClick={() => onAddProduct(product)}
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
