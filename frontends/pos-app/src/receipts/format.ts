import type { PrintBlock, PrintJob } from "../devices/printer";
import type { CartItem } from "../CartContext";

export type SaleReceipt = {
  orderId?: string;
  storeLabel: string;
  cashierLabel: string;
  items: CartItem[];
  subtotal: number;
  tax?: number;
  total: number;
  paidMethod: string;
  createdAt: Date;
  footerNote?: string;
};

const fmtMoney = (n: number) => `$${n.toFixed(2)}`;

function line(text: string, width = 42, align: "left" | "center" | "right" = "left"): PrintBlock {
  const content = text.length > width ? text.slice(0, width) : text;
  return { type: "text", content, align };
}

function hr(width = 42): PrintBlock {
  return line("-".repeat(width));
}

export function buildSaleReceiptJob(data: SaleReceipt, width = 42): PrintJob {
  const blocks: PrintBlock[] = [];

  // Header
  blocks.push({ type: "text", content: "NovaPOS", align: "center", bold: true, size: "m" });
  blocks.push(line(data.storeLabel, width, "center"));
  blocks.push(line(`Cashier: ${data.cashierLabel}`, width, "center"));
  const ts = data.createdAt.toLocaleString();
  blocks.push(line(ts, width, "center"));
  if (data.orderId) blocks.push(line(`Order # ${data.orderId}`, width, "center"));
  blocks.push(hr(width));

  // Items
  data.items.forEach((it) => {
    const name = it.name.length > width ? it.name.slice(0, width - 1) : it.name;
    blocks.push(line(name, width, "left"));
    const qtyPrice = `x${it.quantity} @ ${fmtMoney(it.price)}`;
    const lineTotal = fmtMoney(it.price * it.quantity);
    const pad = Math.max(1, width - qtyPrice.length - lineTotal.length);
    blocks.push(line(`${qtyPrice}${" ".repeat(pad)}${lineTotal}`, width, "left"));
  });

  blocks.push(hr(width));

  // Totals
  const subtotal = data.subtotal;
  const tax = data.tax ?? Math.max(0, data.total - subtotal);
  const rows: Array<[string, number]> = [
    ["Subtotal", subtotal],
    ["Tax", tax],
    ["Total", data.total],
  ];
  for (const [label, val] of rows) {
    const left = label;
    const right = fmtMoney(val);
    const pad = Math.max(1, width - left.length - right.length);
    blocks.push(line(`${left}${" ".repeat(pad)}${right}`, width));
  }

  blocks.push(line(`Paid: ${data.paidMethod.toUpperCase()}`, width));
  blocks.push(hr(width));
  blocks.push(line("Thank you!", width, "center"));
  if (data.footerNote) blocks.push(line(data.footerNote, width, "center"));

  return { widthChars: width, blocks, cut: true };
}
