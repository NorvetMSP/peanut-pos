import React from 'react';
import './CartCard.css';

type CartItem = {
  id: number;
  name: string;
  price: number;
  quantity: number;
  onRemove: () => void;
};

type CartCardProps = {
  item: CartItem;
};

const CartCard: React.FC<CartCardProps> = ({ item }) => (
  <div className="cart-card">
    <div className="cart-info">
      <span className="cart-name">{item.name}</span>
      <span className="cart-qty">x {item.quantity}</span>
    </div>
    <div className="cart-details">
      <span className="cart-price">${(item.price * item.quantity).toFixed(2)}</span>
      <button className="cart-remove" onClick={item.onRemove}>&times;</button>
    </div>
  </div>
);

export default CartCard;
