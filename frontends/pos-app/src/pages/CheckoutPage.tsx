import React from "react";
import { useCart } from "../CartContext";
import { useAuth } from "../AuthContext";
import './CheckoutPageModern.css';

const PRODUCT_SERVICE_URL = (import.meta.env.VITE_PRODUCT_SERVICE_URL ?? "http://localhost:8081").replace(/\/$/, "");
const STORAGE_PREFIX = "productCache";

const mapProductActiveFlags = (raw: unknown): Record<string, boolean> => {
  if (!Array.isArray(raw)) return {};
  const result: Record<string, boolean> = {};
  for (const entry of raw) {
    if (!entry || typeof entry !== "object") continue;
    const candidate = entry as Record<string, unknown>;
    const rawId = candidate.id;
    if (typeof rawId !== "string" && typeof rawId !== "number") continue;
    const id = typeof rawId === "string" ? rawId : String(rawId);
    const activeValue = candidate.active;
    result[id] = typeof activeValue === "boolean" ? activeValue : true;
  }
  return result;
};


export default function CheckoutPage() {
  const { cart, totalAmount } = useCart();
  const { currentUser, token } = useAuth();
  const tenantId = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;
  const [paymentMethod, setPaymentMethod] = React.useState<'card' | 'cash' | 'crypto'>('card');
  const [cardDetails, setCardDetails] = React.useState({ number: '', name: '', expiry: '', cvc: '' });
  const [billing, setBilling] = React.useState({ address: '', city: '', zip: '' });
  const [shipping, setShipping] = React.useState({ address: '', city: '', zip: '' });
  const [customer, setCustomer] = React.useState({ name: '', email: '', phone: '' });
  const [cryptoWallet, setCryptoWallet] = React.useState('');
  const [cashNote, setCashNote] = React.useState('');
  const [errors, setErrors] = React.useState<string[]>([]);
  const [submitted, setSubmitted] = React.useState(false);
  const [orderNumber, setOrderNumber] = React.useState<string | null>(null);
  const [review, setReview] = React.useState(false);

  // Pull the freshest active flags, falling back to cached catalog when offline.
  const resolveCatalogSnapshot = React.useCallback(async (): Promise<Record<string, boolean>> => {
    if (!tenantId) return {};
    const headers: Record<string, string> = { "X-Tenant-ID": tenantId };
    if (token) headers["Authorization"] = `Bearer ${token}`;
    const storage = typeof window !== "undefined" ? window.localStorage : null;

    try {
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, { headers });
      if (!response.ok) {
        throw new Error(`Failed to fetch products (${response.status})`);
      }
      const payload = await response.json();
      return mapProductActiveFlags(payload);
    } catch (err) {
      console.warn('Using cached catalog for submission validation', err);
      if (!storage) return {};
      const cached = storage.getItem(`${STORAGE_PREFIX}:${tenantId}`);
      if (!cached) return {};
      try {
        const parsed = JSON.parse(cached);
        return mapProductActiveFlags(parsed);
      } catch (cacheError) {
        console.warn('Unable to parse cached catalog during checkout validation', cacheError);
        return {};
      }
    }
  }, [tenantId, token]);

  const detectInactiveItems = React.useCallback(async (): Promise<string[]> => {
    if (!cart.length) return [];
    const catalogSnapshot = await resolveCatalogSnapshot();
    if (!Object.keys(catalogSnapshot).length) return [];
    const flagged = cart.reduce<string[]>((acc, item) => {
      if (catalogSnapshot[item.id] === false) {
        acc.push(item.name);
      }
      return acc;
    }, []);
    return Array.from(new Set(flagged));
  }, [cart, resolveCatalogSnapshot]);

  const handleCardChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setCardDetails({ ...cardDetails, [e.target.name]: e.target.value });
  };
  const handleBillingChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setBilling({ ...billing, [e.target.name]: e.target.value });
  };
  const handleShippingChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setShipping({ ...shipping, [e.target.name]: e.target.value });
  };
  const handleCustomerChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setCustomer({ ...customer, [e.target.name]: e.target.value });
  };

  const validate = () => {
    const errs: string[] = [];
    if (!customer.name) errs.push('Name is required');
    if (!customer.email) errs.push('Email is required');
    if (!customer.phone) errs.push('Phone is required');
    if (!shipping.address) errs.push('Shipping address is required');
    if (!shipping.city) errs.push('Shipping city is required');
    if (!shipping.zip) errs.push('Shipping ZIP is required');
    if (!billing.address) errs.push('Billing address is required');
    if (!billing.city) errs.push('Billing city is required');
    if (!billing.zip) errs.push('Billing ZIP is required');
    if (paymentMethod === 'card') {
      if (!cardDetails.number) errs.push('Card number is required');
      if (!cardDetails.name) errs.push('Name on card is required');
      if (!cardDetails.expiry) errs.push('Expiry is required');
      if (!cardDetails.cvc) errs.push('CVC is required');
    }
    if (paymentMethod === 'crypto' && !cryptoWallet) errs.push('Wallet address is required');
    return errs;
  };

  const handleReview = (e: React.FormEvent) => {
    e.preventDefault();
    const errs = validate();
    setErrors(errs);
    if (errs.length === 0) {
      setReview(true);
    }
  };

  const handleSubmit = (event?: React.SyntheticEvent) => {
    event?.preventDefault?.();

    const finalizeOrder = () => {
      const generatedOrderNumber = 'NP-' + Math.floor(100000 + Math.random() * 900000).toString();
      setOrderNumber(generatedOrderNumber);
      setSubmitted(true);
      // TODO: process order
    };

    detectInactiveItems()
      .then(inactiveItems => {
        if (inactiveItems.length > 0) {
          const detail = inactiveItems.join(', ');
          const message = `Inactive items detected: ${detail}. Manager override assumed; proceeding with locked pricing.`;
          console.warn(message);
          if (typeof window !== 'undefined' && typeof window.alert === 'function') {
            window.alert(message);
          }
        }
      })
      .catch(err => {
        console.warn('Unable to verify catalog before checkout submission', err);
      })
      .finally(finalizeOrder);
  };

  return (
    <div style={{ background: '#f6f8fa', minHeight: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div className="checkout-modern">
        <div className="checkout-header">
          <h1>Checkout</h1>
          <p>Complete your purchase and review your order details below.</p>
        </div>
        <div className="checkout-section">
          <h2>Order Summary</h2>
          {cart.length === 0 ? (
            <div className="error">Your cart is empty.</div>
          ) : (
            <ul style={{ marginBottom: '1rem' }}>
              {cart.map(item => (
                <li key={item.id} style={{ display: 'flex', justifyContent: 'space-between', padding: '6px 0', fontSize: '1rem' }}>
                  <span>{item.name} x {item.quantity}</span>
                  <span>${(item.price * item.quantity).toFixed(2)}</span>
                </li>
              ))}
            </ul>
          )}
          <div style={{ display: 'flex', justifyContent: 'space-between', fontWeight: 'bold', borderTop: '2px solid #efefef', paddingTop: '8px', fontSize: '1.1rem' }}>
            <span>Total</span>
            <span>${totalAmount.toFixed(2)}</span>
          </div>
        </div>
        <div className="checkout-section">
          {!review ? (
            <form onSubmit={handleReview} noValidate>
              <label>Customer Info</label>
              <input name="name" value={customer.name} onChange={handleCustomerChange} placeholder="Full Name" />
              <input name="email" value={customer.email} onChange={handleCustomerChange} placeholder="Email" />
              <input name="phone" value={customer.phone} onChange={handleCustomerChange} placeholder="Phone" />
              <label>Shipping Address</label>
              <input name="address" value={shipping.address} onChange={handleShippingChange} placeholder="Street Address" />
              <input name="city" value={shipping.city} onChange={handleShippingChange} placeholder="City" />
              <input name="zip" value={shipping.zip} onChange={handleShippingChange} placeholder="ZIP Code" />
              <label>Billing Address</label>
              <input name="address" value={billing.address} onChange={handleBillingChange} placeholder="Street Address" />
              <input name="city" value={billing.city} onChange={handleBillingChange} placeholder="City" />
              <input name="zip" value={billing.zip} onChange={handleBillingChange} placeholder="ZIP Code" />
              <label>Payment Method</label>
              <div className="payment-methods">
                <button type="button" className={`payment-btn${paymentMethod==='card'?' selected':''}`} onClick={()=>setPaymentMethod('card')}>Card</button>
                <button type="button" className={`payment-btn${paymentMethod==='cash'?' selected':''}`} onClick={()=>setPaymentMethod('cash')}>Cash</button>
                <button type="button" className={`payment-btn${paymentMethod==='crypto'?' selected':''}`} onClick={()=>setPaymentMethod('crypto')}>Crypto</button>
              </div>
              {paymentMethod === 'card' && (
                <>
                  <label>Card Details</label>
                  <input name="number" value={cardDetails.number} onChange={handleCardChange} placeholder="Card Number" />
                  <input name="name" value={cardDetails.name} onChange={handleCardChange} placeholder="Name on Card" />
                  <div style={{ display: 'flex', gap: '8px' }}>
                    <input name="expiry" value={cardDetails.expiry} onChange={handleCardChange} placeholder="MM/YY" style={{ width: '50%' }} />
                    <input name="cvc" value={cardDetails.cvc} onChange={handleCardChange} placeholder="CVC" style={{ width: '50%' }} />
                  </div>
                </>
              )}
              {paymentMethod === 'crypto' && (
                <>
                  <label>Crypto Wallet Address</label>
                  <input value={cryptoWallet} onChange={e=>setCryptoWallet(e.target.value)} placeholder="Wallet Address" />
                </>
              )}
              {paymentMethod === 'cash' && (
                <>
                  <label>Cash Payment Note</label>
                  <input value={cashNote} onChange={e=>setCashNote(e.target.value)} placeholder="Note for cashier (optional)" />
                </>
              )}
              {errors.length > 0 && (
                <div className="error">
                  {errors.map((err, i) => <div key={i}>{err}</div>)}
                </div>
              )}
              <button type="submit" className="action-btn">Review & Confirm</button>
            </form>
          ) : (
            <>
              <h2>Review & Confirm</h2>
              <div style={{ textAlign: 'left', marginBottom: '1rem' }}>
                <div><strong>Customer Info:</strong> {customer.name}, {customer.email}, {customer.phone}</div>
                <div><strong>Shipping:</strong> {shipping.address}, {shipping.city}, {shipping.zip}</div>
                <div><strong>Billing:</strong> {billing.address}, {billing.city}, {billing.zip}</div>
                <div><strong>Payment:</strong> {paymentMethod === 'card' ? 'Card' : paymentMethod === 'cash' ? 'Cash' : 'Crypto'}</div>
                {paymentMethod === 'card' && (
                  <div>Card: ****{cardDetails.number.slice(-4)}, {cardDetails.name}, {cardDetails.expiry}</div>
                )}
                {paymentMethod === 'crypto' && (
                  <div>Wallet: {cryptoWallet}</div>
                )}
                {paymentMethod === 'cash' && (
                  <div>Note: {cashNote}</div>
                )}
              </div>
              <div style={{ fontWeight: 'bold', marginBottom: '1rem' }}>Order Total: ${totalAmount.toFixed(2)}</div>
              <button className="action-btn" onClick={handleSubmit}>Place Order</button>
            </>
          )}
        </div>
        {submitted && orderNumber && (
          <div className="bg-green-100 text-green-700 p-4 rounded mt-4 text-center">
            <div className="text-lg font-bold mb-2">Order placed successfully!</div>
            <div className="mb-1">Your order number is <span className="font-mono bg-white px-2 py-1 rounded text-primary">{orderNumber}</span></div>
            <div className="mb-1">Thank you for shopping with NovaPOS!</div>
            <div className="text-xs text-gray-500 mt-2">NovaPOS &copy; 2025</div>
          </div>
        )}
        <div className="checkout-footer">NovaPOS &copy; 2025</div>
      </div>
    </div>
  );
}
