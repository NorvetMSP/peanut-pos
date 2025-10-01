// Shared e2e test types for customer management flows
export type CustomerRecord = {
  id: string;
  name: string;
  email: string | null;
  phone: string | null;
  created_at: string;
};

export type AuditEvent = {
  timestamp: string;
  action: string;
  actor: string;
  details: string;
};
