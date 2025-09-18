import React from "react";
import { useNavigate } from "react-router-dom";
import { useCart } from "../CartContext";
import { useAuth } from "../AuthContext";
import { useOrders } from "../OrderContext";
import "./CheckoutPageModern.css";

const PRODUCT_SERVICE_URL = (import.meta.env.VITE_PRODUCT_SERVICE_URL ?? "http://localhost:8081").replace(/\/$/, "");
const STORAGE_PREFIX = "productCache";

type CatalogSnapshot = Record<string, { active: boolean; price: number }>;

const mapToCatalogSnapshot = (raw: unknown): CatalogSnapshot => {
  const snapshot: CatalogSnapshot = {};
  if (!Array.isArray(raw)) return snapshot;
  for (const entry of raw) {
    if (!entry || typeof entry !== "object") continue;
    const candidate = entry as Record<string, unknown>;
    const rawId = candidate.id;
    if (typeof rawId !== "string" && typeof rawId !== "number") continue;
    const id = typeof rawId === "string" ? rawId : String(rawId);
    const rawPrice = candidate.price;
    const price =
      typeof rawPrice === "number"
        ? rawPrice
        : typeof rawPrice === "string"
        ? Number(rawPrice)
        : NaN;
    if (!Number.isFinite(price)) continue;
    const activeValue = candidate.active;
    snapshot[id] = {
      active: typeof activeValue === "boolean" ? activeValue : true,
      price,
    };
  }
  return snapshot;
};


type SubmissionOutcome = {
  mode: "queued" | "submitted";
  reference: string;
  status: string;
  paymentStatus?: string;
  paymentUrl?: string | null;
  note?: string;
};

export default function CheckoutPage() {
  const navigate = useNavigate();
  const { cart, totalAmount, clearCart } = useCart();
  const { currentUser, token } = useAuth();
  const { submitOrder, isOnline } = useOrders();
  const tenantId = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;

  const [paymentMethod, setPaymentMethod] = React.useState<'card' | 'cash' | 'crypto'>('card');
  const [cardDetails, setCardDetails] = React.useState({ number: '', name: '', expiry: '', cvc: '' });
  const [billing, setBilling] = React.useState({ address: '', city: '', zip: '' });
  const [shipping, setShipping] = React.useState({ address: '', city: '', zip: '' });
  const [customer, setCustomer] = React.useState({ name: '', email: '', phone: '' });
  const [cryptoWallet, setCryptoWallet] = React.useState('');
  const [cashNote, setCashNote] = React.useState('');
  const [errors, setErrors] = React.useState<string[]>([]);
  const [review, setReview] = React.useState(false);
  const [submissionOutcome, setSubmissionOutcome] = React.useState<SubmissionOutcome | null>(null);
  const [submissionError, setSubmissionError] = React.useState<string | null>(null);
  const [isProcessing, setIsProcessing] = React.useState(false);

  const resolveCatalogSnapshot = React.useCallback(async (): Promise<CatalogSnapshot> => {
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
      return mapToCatalogSnapshot(payload);
    } catch (err) {
      console.warn('Using cached catalog for submission validation', err);
      if (!storage) return {};
      const cached = storage.getItem(`${STORAGE_PREFIX}:${tenantId}`);
      if (!cached) return {};
      try {
        const parsed = JSON.parse(cached) as { products?: unknown };
        if (parsed && typeof parsed === "object" && Array.isArray(parsed.products)) {
          return mapToCatalogSnapshot(parsed.products);
        }
        return mapToCatalogSnapshot(parsed);
      } catch (cacheError) {
        console.warn('Unable to parse cached catalog during checkout validation', cacheError);
        return {};
      }
    }
  }, [tenantId, token]);

  type CatalogValidationResult = {
    inactive: string[];
    priceChanges: Array<{ id: string; name: string; previous: number; next: number }>;
  };

  const detectCatalogIssues = React.useCallback(async (): Promise<CatalogValidationResult> => {
    if (!cart.length) {
      return { inactive: [], priceChanges: [] };
    }
    const snapshot = await resolveCatalogSnapshot();
    if (!Object.keys(snapshot).length) {
      return { inactive: [], priceChanges: [] };
    }

    const inactive: string[] = [];
    const priceChanges: Array<{ id: string; name: string; previous: number; next: number }> = [];

    for (const item of cart) {
      const entry = snapshot[item.id];
      if (!entry) {
        inactive.push(item.name);
        continue;
      }
      if (!entry.active) {
        inactive.push(item.name);
      }
      if (Math.abs(entry.price - item.price) > 0.0001) {
        priceChanges.push({ id: item.id, name: item.name, previous: item.price, next: entry.price });
      }
    }

    return {
      inactive: Array.from(new Set(inactive)),
      priceChanges,
    };
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
      setSubmissionOutcome(null);
      setSubmissionError(null);
    }
  };

  const handleSubmit = async () => {
    if (isProcessing) return;
    const errs = validate();
    setErrors(errs);
    if (errs.length > 0) {
      setSubmissionError('Please resolve the highlighted issues before submitting.');
      return;
    }
    if (!cart.length) {
      setSubmissionError('Cart is empty. Add items before submitting.');
      return;
    }

    setSubmissionError(null);
    setIsProcessing(true);

    try {
      const { inactive, priceChanges } = await detectCatalogIssues();
      if (inactive.length > 0) {
        const detail = inactive.join(', ');
        setSubmissionError(`Inactive items detected: ${detail}. Remove or replace these items before completing checkout.`);
        setIsProcessing(false);
        setReview(false);
        return;
      }

      if (priceChanges.length > 0) {
        const detail = priceChanges
          .map(change => `${change.name} (locked ${change.previous.toFixed(2)} vs catalog ${change.next.toFixed(2)})`)
          .join(', ');
        console.warn(`Catalog pricing updated for: ${detail}`);
        setSubmissionError(`Catalog pricing updated for: ${detail}. Locked cart prices remain in effect - review and submit again when ready.`);
        setIsProcessing(false);
        setReview(false);
        return;
      }

      const orderItems = cart.map(item => ({
        product_id: item.id,
        quantity: item.quantity,
        unit_price: item.price,
        line_total: Number((item.price * item.quantity).toFixed(2)),
      }));

      const metadata: Record<string, unknown> = {
        customer,
        shipping,
        billing,
        payment_method: paymentMethod,
        submitted_at: Date.now(),
      };
      if (paymentMethod === 'card') metadata['card_last4'] = cardDetails.number.slice(-4);
      if (paymentMethod === 'cash' && cashNote) metadata['cash_note'] = cashNote;
      if (paymentMethod === 'crypto' && cryptoWallet) metadata['crypto_wallet'] = cryptoWallet;

      const payload = {
        items: orderItems,
        payment_method: paymentMethod,
        total: Number(totalAmount.toFixed(2)),
        customer_id: customer.email.trim() || undefined,
        metadata,
      };

      const result = await submitOrder(payload);

      clearCart();
      setReview(false);
      setErrors([]);

      if (result.status === 'queued') {
        setSubmissionOutcome({
          mode: 'queued',
          reference: result.tempId,
          status: 'Queued (offline)',
          paymentStatus: 'Awaiting sync',
          paymentUrl: null,
          note: isOnline ? 'Submission failed, queued for retry.' : 'Device offline; order will sync automatically when back online.',
        });
      } else {
        const paymentStatus = result.paymentError
          ? `Payment error: ${result.paymentError}`
          : result.payment?.status ?? (paymentMethod === 'cash' ? 'paid' : 'pending');
        setSubmissionOutcome({
          mode: 'submitted',
          reference: result.order.id,
          status: result.order.status ?? 'Submitted',
          paymentStatus,
          paymentUrl: result.payment?.payment_url ?? null,
          note: result.paymentError ?? undefined,
        });
      }
    } catch (err) {
      console.error('Order submission failed', err);
      setSubmissionError(err instanceof Error ? err.message : 'Order submission failed.');
    } finally {
      setIsProcessing(false);
    }
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
              {!isOnline && (
                <div className="info">
                  Offline mode - order will be queued and synced automatically once reconnected.
                </div>
              )}
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
              <button type="submit" className="action-btn" disabled={isProcessing}>{isProcessing ? 'Reviewing...' : 'Review & Confirm'}</button>
            </form>
          ) : (
            <>
              {!isOnline && (
                <div className="info">
                  Offline mode - completing the sale will queue the order for automatic sync.
                </div>
              )}
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
              {submissionError && (
                <div className="error" style={{ marginBottom: '1rem' }}>{submissionError}</div>
              )}
              <button className="action-btn" onClick={handleSubmit} disabled={isProcessing}>
                {isProcessing ? 'Processing...' : 'Complete Sale'}
              </button>
              <button className="action-btn" style={{ marginTop: '0.75rem', background: '#e2e8f0', color: '#153a5b' }} onClick={() => setReview(false)} disabled={isProcessing}>
                Edit Details
              </button>
            </>
          )}
        </div>
        {submissionOutcome && (
          <div className={`p-4 rounded mt-4 text-center ${submissionOutcome.mode === 'queued' ? 'bg-amber-100 text-amber-800' : 'bg-emerald-100 text-emerald-700'}`}>
            <div className="text-lg font-bold mb-2">
              {submissionOutcome.mode === 'queued' ? 'Order queued for sync' : 'Order accepted by store'}
            </div>
            <div className="mb-1">Reference <span className="font-mono bg-white px-2 py-1 rounded text-primary">{submissionOutcome.reference}</span></div>
            <div className="mb-1">Status: {submissionOutcome.status}</div>
            {submissionOutcome.paymentStatus && <div className="mb-1">Payment: {submissionOutcome.paymentStatus}</div>}
            {submissionOutcome.paymentUrl && (
              <div className="mb-1 text-sm">
                Payment link: <a className="text-primary" href={submissionOutcome.paymentUrl} target="_blank" rel="noreferrer">Open payment page</a>
              </div>
            )}
            {submissionOutcome.note && <div className="text-xs text-amber-700 mt-2">{submissionOutcome.note}</div>}
            <div className="mt-3 flex flex-wrap justify-center gap-2">
              <button className="action-btn" onClick={() => navigate('/pos')} style={{ background: '#0f7b7f' }}>
                Open POS Terminal
              </button>
              <button className="action-btn" style={{ background: '#153a5b' }} onClick={() => navigate('/history')}>
                View Orders
              </button>
              <button className="action-btn" onClick={() => navigate('/sales')}>
                New Sale
              </button>
            </div>
          </div>
        )}
        {submissionError && !review && (
          <div className="error" style={{ marginTop: '1rem' }}>{submissionError}</div>
        )}
        <div className="checkout-footer">NovaPOS &copy; 2025</div>
      </div>
    </div>
  );
}
