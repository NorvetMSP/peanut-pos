import type { CustomerRecord } from './types';

// Runtime type guard for CustomerRecord for future dynamic parsing scenarios
export function isCustomerRecord(value: unknown): value is CustomerRecord {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Partial<CustomerRecord> & Record<string, unknown>;
  return (
    typeof v.id === 'string' &&
    typeof v.name === 'string' &&
    typeof v.created_at === 'string' &&
    (typeof v.email === 'string' || v.email === null) &&
    (typeof v.phone === 'string' || v.phone === null)
  );
}
