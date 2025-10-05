export type SettlementRow = { method: string; count: number; amount: number };
export type SettlementReport = { date: string; totals: SettlementRow[] };

export const parseReport = (value: unknown): SettlementReport => {
  const fallback: SettlementReport = { date: new Date().toISOString().slice(0,10), totals: [] };
  if (typeof value !== 'object' || value === null) return fallback;
  const record = value as Record<string, unknown>;
  const date = typeof record.date === 'string' && record.date.trim().length > 0
    ? record.date
    : fallback.date;
  const rows: SettlementRow[] = [];
  const rawTotals = record.totals;
  if (Array.isArray(rawTotals)) {
    for (const entry of rawTotals) {
      if (!entry || typeof entry !== 'object') continue;
      const row = entry as Record<string, unknown>;
      const method = typeof row.method === 'string' && row.method.trim().length > 0 ? row.method.trim() : 'unknown';
      const count = Number(row.count);
      const amount = typeof row.amount === 'number' ? row.amount : Number(row.amount);
      rows.push({
        method,
        count: Number.isFinite(count) ? count : 0,
        amount: Number.isFinite(amount) ? amount : 0,
      });
    }
  }
  return { date, totals: rows };
};

export const formatCurrency = (value: number, locale = 'en-US', currency = 'USD'): string => {
  try {
    return new Intl.NumberFormat(locale, { style: 'currency', currency, currencyDisplay: 'symbol', maximumFractionDigits: 2 }).format(value);
  } catch {
    // Fallback minimal formatting
    return `$${(Math.round(value * 100) / 100).toFixed(2)}`;
  }
};
