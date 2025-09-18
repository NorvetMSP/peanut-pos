import React from 'react';
import './CartCard.css';

type CartItem = {
  id: string;
  name: string;
  price: number;
  quantity: number;
  onRemove: () => void;
  onAdd?: () => void;
  onSubtract?: () => void;
};

type CartItemCardProps = {
  item: CartItem;
};

const CartItemCard: React.FC<CartItemCardProps> = ({ item }) => (
  <div className="cart-card-modern">
    <div className="cart-description">
      <h2>{item.name}</h2>
      <h4>In Cart</h4>
      <h1>${(item.price * item.quantity).toFixed(2)}</h1>
      <div className="cart-qty-gadget">
        <button className="cart-qty-btn" onClick={item.onSubtract} disabled={item.quantity <= 1}>-</button>
        <span className="cart-qty-modern">{item.quantity}</span>
        <button className="cart-qty-btn" onClick={item.onAdd}>+</button>
      </div>
      <button className="cart-remove-modern" onClick={item.onRemove}>Remove</button>
    </div>
  </div>
);

export default CartItemCard;
