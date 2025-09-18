import React from "react";
import { useCart } from "../CartContext";

// Map CartContext CartItem to template's item format
interface Item {
  id: string;
  category?: string;
  title: string;
  price: number;
  qty: number;
  image?: string;
}

const defaultShippingOptions = [
  { id: "standard", label: "Standard Delivery", cost: 5.0 },
  { id: "express", label: "Express (2-day)", cost: 12.0 },
  { id: "pickup", label: "In-store Pickup", cost: 0.0 },
];

export default function EcommerceTemplate() {
  const { cart, removeItem, incrementItemQuantity, decrementItemQuantity } = useCart();
  const [shippingId, setShippingId] = React.useState<string>("standard");
  const [code, setCode] = React.useState<string>("");
  const [shippingOptions] = React.useState(defaultShippingOptions);
  const [discount, setDiscount] = React.useState<number>(0);
  const [promoError, setPromoError] = React.useState<string>("");

  const items: Item[] = cart.map((i) => ({
    id: i.id,
    title: i.name,
    price: i.price,
    qty: i.quantity,
    image: undefined,
  }));

  const itemsCount = items.reduce((n, i) => n + i.qty, 0);
  const subtotal = items.reduce((sum, i) => sum + i.price * i.qty, 0);
  const shipping = shippingOptions.find((o) => o.id === shippingId)?.cost ?? 0;
  const total = Math.max(0, subtotal + shipping - discount);

  const handlePromo = (e: React.FormEvent) => {
    e.preventDefault();
    if (code.trim().toLowerCase() === "save10") {
      setDiscount(10);
      setPromoError("");
    } else if (code.trim() === "") {
      setDiscount(0);
      setPromoError("");
    } else {
      setDiscount(0);
      setPromoError("Invalid promo code");
    }
  };

  const dec = (id: string) => {
    const item = cart.find((i) => i.id === id);
    if (item && item.quantity > 1) {
      decrementItemQuantity(id);
    }
  };
  const inc = (id: string) => {
    const item = cart.find((i) => i.id === id);
    if (item) {
      incrementItemQuantity(id);
    }
  };
  const remove = (id: string) => removeItem(id);

  const eur = (n: number) =>
    new Intl.NumberFormat(undefined, { style: "currency", currency: "EUR", maximumFractionDigits: 2 }).format(n);

  return (
    <div className="min-h-screen flex bg-gray-200 font-sans text-sm font-bold">
      <div className="w-full p-8">
        <div className="mx-auto max-w-3xl w-11/12 shadow-lg rounded-xl bg-white">
          <div className="flex flex-wrap m-0">
            {/* Cart */}
            <div className="flex-1 bg-white p-8 rounded-l-xl">
              <div className="mb-12 flex items-center justify-between">
                <h4 className="text-xl font-bold">Shopping Cart</h4>
                <div className="text-right text-gray-500 font-normal">{itemsCount} item{itemsCount !== 1 ? "s" : ""}</div>
              </div>

              {items.map((it) => (
                <div key={it.id} className="flex items-center py-4 border-t border-b border-gray-200">
                  <div className="flex-shrink-0 w-14 h-14 bg-gray-100 flex items-center justify-center rounded">
                    {it.image ? (
                      <img className="max-w-full h-auto block" src={it.image} alt={it.title} />
                    ) : (
                      <div className="w-14 h-14 bg-gray-300" />
                    )}
                  </div>
                  <div className="flex-1 px-4 min-w-0">
                    <div className="text-gray-500 font-normal">{it.category || ''}</div>
                    <div>{it.title}</div>
                  </div>
                  <div className="flex items-center gap-2">
                    <button className="px-2 text-lg" onClick={() => dec(it.id)} aria-label={`Decrease ${it.title}`}>
                      &minus;
                    </button>
                    <span className="border border-gray-300 px-2 py-1 rounded text-center min-w-[2rem]">{it.qty}</span>
                    <button className="px-2 text-lg" onClick={() => inc(it.id)} aria-label={`Increase ${it.title}`}>
                      +
                    </button>
                  </div>
                  <div className="text-right px-4">
                    {eur(it.price * it.qty)}{' '}
                    <span className="ml-2 text-xs cursor-pointer" onClick={() => remove(it.id)} title="Remove">
                      &#10005;
                    </span>
                  </div>
                </div>
              ))}

              <div className="mt-12 flex items-center gap-2">
                <a
                  href="/sales"
                  className="text-black no-underline focus:outline-none focus:ring-2 focus:ring-blue-500"
                  aria-label="Back to shop"
                >
                  &larr;
                </a>
                <span className="text-gray-500 font-normal">Back to shop</span>
              </div>
            </div>

            {/* Summary */}
            <div className="flex-1 bg-gray-200 rounded-r-xl p-8 text-gray-700">
              <div>
                <h5 className="mt-8 text-lg font-bold">Summary</h5>
              </div>
              <hr className="mt-5 border-t border-gray-200" />
              <div className="flex justify-between py-4">
                <div>ITEMS {itemsCount}</div>
                <div>{eur(subtotal)}</div>
              </div>

              <form onSubmit={handlePromo} className="py-4">
                <p className="mb-2">SHIPPING</p>
                <select
                  value={shippingId}
                  onChange={(e) => setShippingId(e.target.value)}
                  aria-label="Select shipping method"
                  className="border border-gray-300 p-2 mb-4 outline-none w-full bg-gray-100 font-normal"
                >
                  {shippingOptions.map((o) => (
                    <option key={o.id} value={o.id} className="text-gray-500">
                      {o.label} - {eur(o.cost)}
                    </option>
                  ))}
                </select>
                <p className="mb-2">GIVE CODE</p>
                <input
                  id="code"
                  placeholder="Enter your code"
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  className="border border-gray-300 p-2 mb-4 outline-none w-full bg-gray-100 font-normal"
                />
                <button type="submit" className="bg-black border-black text-white w-full text-xs mt-0 p-2 rounded-none cursor-pointer">Apply</button>
                {promoError && <div className="text-red-600 text-xs mt-2">{promoError}</div>}
              </form>

              <div className="flex justify-between py-4 border-t border-gray-200">
                <div>TOTAL PRICE</div>
                <div>{eur(total)}</div>
              </div>
              {discount > 0 && (
                <div className="flex justify-between py-4 text-green-600">
                  <div>Promo Discount</div>
                  <div>- {eur(discount)}</div>
                </div>
              )}
              <button
                className="bg-black border-black text-white w-full text-xs mt-4 p-2 rounded-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-green-500"
                onClick={() => { window.location.href = '/checkout'; }}
                aria-label="Proceed to checkout"
              >
                CHECKOUT
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
