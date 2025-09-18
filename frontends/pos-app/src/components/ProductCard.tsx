import React from 'react';
import './ProductCard.css';

type Product = {
  id: string;
  name: string;
  description: string;
  price: number;
  image: string;
  sku?: string | null;
  onAdd: () => void;
};

const ProductCard: React.FC<{ product: Product }> = ({ product }) => (
  <div className="product-card">
    <img src={product.image} alt={product.name} />
    <div className="name">{product.name}</div>
    {product.sku && <div className="sku">SKU: {product.sku}</div>}
    <div className="desc">{product.description}</div>
    <div className="price">${product.price.toFixed(2)}</div>
    <button onClick={product.onAdd}>Add to Cart</button>
  </div>
);

export default ProductCard;
