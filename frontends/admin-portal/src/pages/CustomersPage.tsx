import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAuth } from "../AuthContext";
import { useHasAnyRole } from "../hooks/useRoleAccess";
import { resolveServiceUrl } from "../utils/env";
import { MANAGER_ROLES, SUPER_ADMIN_ROLES } from "../rbac";
import { withRoleGuard } from "../components/RoleGuard";
import "./AdminSectionModern.css";

const CUSTOMER_SERVICE_URL = resolveServiceUrl(
  "VITE_CUSTOMER_SERVICE_URL",
  "http://localhost:8089",
);

const AUTH_SERVICE_URL = resolveServiceUrl(
  "VITE_AUTH_SERVICE_URL",
  "http://localhost:8085",
);

type ServiceCustomer = {
  id: string;
  name: string;
  email: string | null;
  phone: string | null;
  created_at: string;
};

type TenantOption = {
  value: string;
  label: string;
};

type CustomerFormState = {
  name: string;
  email: string;
  phone: string;
};

type CustomerAuditEvent = {
  timestamp: string;
  action: string;
  actor?: string;
  details?: string;
};

const normalizeCustomer = (entry: unknown): ServiceCustomer | null => {
  if (!entry || typeof entry !== "object") return null;
  const candidate = entry as Record<string, unknown>;
  const id = typeof candidate.id === "string" ? candidate.id : null;
  const name = typeof candidate.name === "string" ? candidate.name : null;
  const createdAt =
    typeof candidate.created_at === "string" ? candidate.created_at : null;

  if (!id || !name || !createdAt) return null;

  const email =
    typeof candidate.email === "string"
      ? candidate.email
      : candidate.email === null
      ? null
      : null;
  const phone =
    typeof candidate.phone === "string"
      ? candidate.phone
      : candidate.phone === null
      ? null
      : null;

  return {
    id,
    name,
    email,
    phone,
    created_at: createdAt,
  };
};

const normalizeAuditEvent = (entry: unknown): CustomerAuditEvent | null => {
  if (!entry || typeof entry !== "object") return null;
  const candidate = entry as Record<string, unknown>;
  const timestamp =
    typeof candidate.timestamp === "string" ? candidate.timestamp : null;
  const action = typeof candidate.action === "string" ? candidate.action : null;

  if (!timestamp || !action) return null;

  const actor =
    typeof candidate.actor === "string" ? candidate.actor : undefined;
  const details =
    typeof candidate.details === "string" ? candidate.details : undefined;

  return { timestamp, action, actor, details };
};

const formatDisplayDate = (value?: string | null): string => {
  if (!value) return "--";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
};

const CustomersPageContent: React.FC = () => {
  const { isLoggedIn, currentUser, token } = useAuth();
  const navigate = useNavigate();
  const isSuperAdmin = useHasAnyRole(SUPER_ADMIN_ROLES);

  const tenantId = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;

  const [selectedTenantId, setSelectedTenantId] = useState<string | null>(null);
  const [tenantOptions, setTenantOptions] = useState<TenantOption[]>([]);
  const [tenantsLoading, setTenantsLoading] = useState(false);

  const [query, setQuery] = useState("");
  const [customers, setCustomers] = useState<ServiceCustomer[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  const [searchPerformed, setSearchPerformed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  const [editModalOpen, setEditModalOpen] = useState(false);
  const [selectedCustomer, setSelectedCustomer] =
    useState<ServiceCustomer | null>(null);
  const [editForm, setEditForm] = useState<CustomerFormState>({
    name: "",
    email: "",
    phone: "",
  });
  const [actionInProgress, setActionInProgress] = useState(false);

  const [auditModalOpen, setAuditModalOpen] = useState(false);
  const [auditEvents, setAuditEvents] = useState<CustomerAuditEvent[]>([]);
  const [auditLoading, setAuditLoading] = useState(false);
  const [auditError, setAuditError] = useState<string | null>(null);

  useEffect(() => {
    if (!isLoggedIn) {
      void navigate("/login", { replace: true });
    }
  }, [isLoggedIn, navigate]);

  useEffect(() => {
    if (!isSuperAdmin) {
      setSelectedTenantId(tenantId ?? null);
    }
  }, [isSuperAdmin, tenantId]);

  const effectiveTenantId = useMemo(
    () => (isSuperAdmin ? selectedTenantId ?? tenantId : tenantId),
    [isSuperAdmin, selectedTenantId, tenantId],
  );

  useEffect(() => {
    setCustomers([]);
    setSearchPerformed(false);
    setError(null);
    setSuccessMessage(null);
  }, [effectiveTenantId]);

  const buildHeaders = useCallback(
    (includeJson = false, tenantOverride?: string): Record<string, string> => {
      const headers: Record<string, string> = {};
      const tenantHeader = tenantOverride ?? effectiveTenantId;
      if (tenantHeader) headers["X-Tenant-ID"] = tenantHeader;
      if (token) headers.Authorization = `Bearer ${token}`;
      if (includeJson) headers["Content-Type"] = "application/json";
      return headers;
    },
    [effectiveTenantId, token],
  );

  const fetchTenants = useCallback(async (): Promise<void> => {
    if (!isSuperAdmin) return;
    setTenantsLoading(true);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/tenants`, {
        headers: buildHeaders(false, tenantId ?? undefined),
      });
      if (!response.ok) {
        throw new Error(`Failed to fetch tenants (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const options = Array.isArray(payload)
        ? payload
            .filter((entry): entry is { id: string; name?: string } => {
              return (
                !!entry &&
                typeof entry === "object" &&
                typeof (entry as { id?: unknown }).id === "string"
              );
            })
            .map((entry) => {
              const record = entry as { id: string; name?: string };
              return {
                value: record.id,
                label:
                  record.name && record.name.trim().length > 0
                    ? record.name
                    : record.id,
              } satisfies TenantOption;
            })
        : [];

      setTenantOptions(options);
      setSelectedTenantId((prev) => {
        if (prev && options.some((option) => option.value === prev)) {
          return prev;
        }
        return options[0]?.value ?? null;
      });
    } catch (err) {
      console.error("Unable to load tenant catalog", err);
      setError((prev) => prev ?? "Unable to load tenants. Please try again.");
    } finally {
      setTenantsLoading(false);
    }
  }, [buildHeaders, isSuperAdmin, tenantId]);

  useEffect(() => {
    if (isSuperAdmin) {
      void fetchTenants();
    }
  }, [fetchTenants, isSuperAdmin]);

  const handleSearch = useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      if (event) event.preventDefault();
      const term = query.trim();
      setError(null);
      setSuccessMessage(null);

      if (!term) {
        setCustomers([]);
        setSearchPerformed(false);
        setError("Enter a name, email, or phone number to search.");
        return;
      }

      if (!effectiveTenantId) {
        setError("Tenant context is unavailable. Please select a tenant.");
        return;
      }

      setIsSearching(true);
      try {
        const response = await fetch(
          `${CUSTOMER_SERVICE_URL}/customers?q=${encodeURIComponent(term)}`,
          {
            headers: buildHeaders(),
          },
        );
        if (!response.ok) {
          throw new Error(`Failed to search customers (${response.status})`);
        }
        const payload = (await response.json()) as unknown;
        const normalized = Array.isArray(payload)
          ? payload
              .map((entry) => normalizeCustomer(entry))
              .filter((entry): entry is ServiceCustomer => entry !== null)
          : [];
        setCustomers(normalized);
        setSearchPerformed(true);
      } catch (err) {
        console.error("Unable to search customers", err);
        setCustomers([]);
        setSearchPerformed(true);
        setError("Unable to search customers. Please try again.");
      } finally {
        setIsSearching(false);
      }
    },
    [buildHeaders, effectiveTenantId, query],
  );

  const sortedCustomers = useMemo(() => {
    return [...customers].sort((a, b) => a.name.localeCompare(b.name));
  }, [customers]);

  const openEditModal = useCallback(
    (customer: ServiceCustomer) => {
      setSelectedCustomer(customer);
      setEditForm({
        name: customer.name,
        email: customer.email ?? "",
        phone: customer.phone ?? "",
      });
      setSuccessMessage(null);
      setError(null);
      setEditModalOpen(true);
    },
    [],
  );

  const closeEditModal = useCallback(() => {
    setEditModalOpen(false);
    setSelectedCustomer(null);
    setActionInProgress(false);
  }, []);

  const handleEditInputChange = useCallback(
    (field: keyof CustomerFormState, value: string) => {
      setEditForm((prev) => ({ ...prev, [field]: value }));
    },
    [],
  );

  const handleEditSubmit = useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!selectedCustomer) return;
      if (!effectiveTenantId) {
        setError("Tenant context is unavailable. Please select a tenant.");
        return;
      }

      const payload = {
        name: editForm.name.trim(),
        email: editForm.email.trim() ? editForm.email.trim() : null,
        phone: editForm.phone.trim() ? editForm.phone.trim() : null,
      };

      setActionInProgress(true);
      setError(null);
      setSuccessMessage(null);

      try {
        const response = await fetch(
          `${CUSTOMER_SERVICE_URL}/customers/${selectedCustomer.id}`,
          {
            method: "PUT",
            headers: buildHeaders(true),
            body: JSON.stringify(payload),
          },
        );
        if (!response.ok) {
          throw new Error(`Failed to update customer (${response.status})`);
        }

        let updated: ServiceCustomer | null = null;
        const contentType = response.headers.get("content-type") ?? "";
        if (contentType.includes("application/json")) {
          try {
            const data = (await response.json()) as unknown;
            updated = normalizeCustomer(data);
          } catch (parseErr) {
            console.warn("Unable to parse customer update payload", parseErr);
          }
        }

        setCustomers((prev) => {
          return prev.map((entry) => {
            if (entry.id !== selectedCustomer.id) return entry;
            if (updated) return updated;
            return {
              ...entry,
              name: payload.name,
              email: payload.email,
              phone: payload.phone,
            };
          });
        });
        setSuccessMessage("Customer updated successfully.");
        closeEditModal();
      } catch (err) {
        console.error("Unable to update customer", err);
        setError("Unable to update the customer. Please try again.");
      } finally {
        setActionInProgress(false);
      }
    },
    [buildHeaders, closeEditModal, editForm.email, editForm.name, editForm.phone, effectiveTenantId, selectedCustomer],
  );

  const handleDeleteCustomer = useCallback(async () => {
    if (!selectedCustomer) return;
    if (!effectiveTenantId) {
      setError("Tenant context is unavailable. Please select a tenant.");
      return;
    }

    const confirmed = window.confirm(
      "This will anonymize the customer record and cannot be undone. Continue?",
    );
    if (!confirmed) return;

    setActionInProgress(true);
    setError(null);
    setSuccessMessage(null);

    try {
      const response = await fetch(
        `${CUSTOMER_SERVICE_URL}/customers/${selectedCustomer.id}/gdpr/delete`,
        {
          method: "POST",
          headers: buildHeaders(true),
        },
      );
      if (!response.ok) {
        throw new Error(`Failed to delete customer (${response.status})`);
      }
      setCustomers((prev) =>
        prev.filter((entry) => entry.id !== selectedCustomer.id),
      );
      setSuccessMessage("Customer deleted and anonymized.");
      closeEditModal();
    } catch (err) {
      console.error("Unable to delete customer", err);
      setError("Unable to delete the customer. Please try again.");
    } finally {
      setActionInProgress(false);
    }
  }, [buildHeaders, closeEditModal, effectiveTenantId, selectedCustomer]);

  const fetchCustomerAudit = useCallback(
    async (customerId: string) => {
      setAuditLoading(true);
      setAuditError(null);
      try {
        const response = await fetch(
          `${CUSTOMER_SERVICE_URL}/customers/${customerId}/audit`,
          {
            headers: buildHeaders(),
          },
        );
        if (!response.ok) {
          throw new Error(`Failed to load audit history (${response.status})`);
        }
        const payload = (await response.json()) as unknown;
        const normalized = Array.isArray(payload)
          ? payload
              .map((entry) => normalizeAuditEvent(entry))
              .filter((entry): entry is CustomerAuditEvent => entry !== null)
              .sort(
                (a, b) =>
                  new Date(b.timestamp).getTime() -
                  new Date(a.timestamp).getTime(),
              )
          : [];
        setAuditEvents(normalized);
      } catch (err) {
        console.error("Unable to load audit history", err);
        setAuditEvents([]);
        setAuditError("Unable to load audit history. Please try again.");
      } finally {
        setAuditLoading(false);
      }
    },
    [buildHeaders],
  );

  const openAuditModal = useCallback(
    (customer: ServiceCustomer) => {
      setSelectedCustomer(customer);
      setAuditEvents([]);
      setAuditError(null);
      setAuditModalOpen(true);
      void fetchCustomerAudit(customer.id);
    },
    [fetchCustomerAudit],
  );

  const closeAuditModal = useCallback(() => {
    setAuditModalOpen(false);
    setSelectedCustomer(null);
    setAuditEvents([]);
    setAuditError(null);
  }, []);

  return (
    <>
      <div
        className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col"
        style={{
          fontFamily: "Raleway, sans-serif",
          background: "linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)",
        }}
      >
        <div className="admin-section-modern">
          <div className="admin-section-header">
            <h2>Customers</h2>
            <p>Search and manage customer profiles across tenants.</p>
          </div>

          <div className="admin-section-content">
            {error && (
              <div className="rounded bg-red-100 text-red-800 px-4 py-3 mb-4">
                {error}
              </div>
            )}
            {successMessage && (
              <div className="rounded bg-green-100 text-green-800 px-4 py-3 mb-4">
                {successMessage}
              </div>
            )}

            {isSuperAdmin && (
              <div className="mb-6">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300" htmlFor="customer-tenant-select">
                  Tenant
                </label>
                <select
                  id="customer-tenant-select"
                  value={selectedTenantId ?? ""}
                  onChange={(event) => setSelectedTenantId(event.target.value || null)}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  disabled={tenantsLoading}
                >
                  {tenantsLoading && tenantOptions.length === 0 ? (
                    <option value="" disabled>
                      Loading tenants...
                    </option>
                  ) : tenantOptions.length === 0 ? (
                    <option value="" disabled>
                      No tenants available
                    </option>
                  ) : (
                    tenantOptions.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))
                  )}
                </select>
              </div>
            )}

            <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow mb-6">
              <form
                className="flex flex-col gap-4 md:flex-row"
                onSubmit={(event) => {
                  void handleSearch(event);
                }}
              >
                <div className="flex-1 flex flex-col">
                  <label className="text-sm font-medium text-gray-600 dark:text-gray-300" htmlFor="customer-search-input">
                    Search Customers
                  </label>
                  <input
                    id="customer-search-input"
                    type="text"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                    placeholder="Name, email, or phone"
                    disabled={isSearching}
                  />
                </div>
                <div className="flex items-end">
                  <button
                    type="submit"
                    className="admin-section-btn"
                    disabled={isSearching}
                  >
                    {isSearching ? "Searching..." : "Search"}
                  </button>
                </div>
              </form>
            </section>

            <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">
                Results
              </h3>
              {isSearching ? (
                <p className="text-sm text-gray-600 dark:text-gray-300">Loading customers...</p>
              ) : sortedCustomers.length === 0 && searchPerformed ? (
                <p className="text-sm text-gray-600 dark:text-gray-300">
                  No customers found for that search.
                </p>
              ) : sortedCustomers.length === 0 ? (
                <p className="text-sm text-gray-600 dark:text-gray-300">
                  Search for a customer by name, email, or phone.
                </p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                    <thead className="bg-gray-50 dark:bg-gray-900/20">
                      <tr>
                        <th className="px-4 py-3 text-left text-xs font-semibold uppercase tracking-wider text-gray-500 dark:text-gray-300">
                          Name
                        </th>
                        <th className="px-4 py-3 text-left text-xs font-semibold uppercase tracking-wider text-gray-500 dark:text-gray-300">
                          Email
                        </th>
                        <th className="px-4 py-3 text-left text-xs font-semibold uppercase tracking-wider text-gray-500 dark:text-gray-300">
                          Phone
                        </th>
                        <th className="px-4 py-3 text-left text-xs font-semibold uppercase tracking-wider text-gray-500 dark:text-gray-300">
                          Created
                        </th>
                        <th className="px-4 py-3 text-right text-xs font-semibold uppercase tracking-wider text-gray-500 dark:text-gray-300">
                          Actions
                        </th>
                      </tr>
                    </thead>
                    <tbody className="bg-white dark:bg-gray-800 divide-y divide-gray-200 dark:divide-gray-700">
                      {sortedCustomers.map((customer) => (
                        <tr key={customer.id}>
                          <td className="px-4 py-2 text-sm font-medium text-gray-900 dark:text-gray-100">
                            {customer.name}
                          </td>
                          <td className="px-4 py-2 text-sm text-gray-600 dark:text-gray-300">
                            {customer.email ?? "--"}
                          </td>
                          <td className="px-4 py-2 text-sm text-gray-600 dark:text-gray-300">
                            {customer.phone ?? "--"}
                          </td>
                          <td className="px-4 py-2 text-sm text-gray-600 dark:text-gray-300">
                            {formatDisplayDate(customer.created_at)}
                          </td>
                          <td className="px-4 py-2 text-sm">
                            <div className="flex flex-wrap items-center justify-end gap-3">
                              <button
                                type="button"
                                className="text-sm text-indigo-600 hover:underline"
                                onClick={() => openEditModal(customer)}
                              >
                                Edit
                              </button>
                              <button
                                type="button"
                                className="text-sm text-indigo-600 hover:underline"
                                onClick={() => openAuditModal(customer)}
                              >
                                View Activity
                              </button>
                            </div>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>

            <div style={{ textAlign: "right", marginTop: "2rem" }}>
              <button
                className="admin-section-btn"
                onClick={() => void navigate("/home")}
                type="button"
              >
                Back to Admin Home
              </button>
            </div>
          </div>
        </div>
      </div>

      {editModalOpen && selectedCustomer && (
        <Modal title={`Edit ${selectedCustomer.name}`} onClose={closeEditModal}>
          <form className="space-y-4" onSubmit={(event) => void handleEditSubmit(event)}>
            <div className="flex flex-col">
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300" htmlFor="customer-name-input">
                Name
              </label>
              <input
                id="customer-name-input"
                type="text"
                value={editForm.name}
                onChange={(event) =>
                  handleEditInputChange("name", event.target.value)
                }
                className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                required
              />
            </div>
            <div className="flex flex-col">
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300" htmlFor="customer-email-input">
                Email
              </label>
              <input
                id="customer-email-input"
                type="email"
                value={editForm.email}
                onChange={(event) =>
                  handleEditInputChange("email", event.target.value)
                }
                className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                placeholder="Optional"
              />
            </div>
            <div className="flex flex-col">
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300" htmlFor="customer-phone-input">
                Phone
              </label>
              <input
                id="customer-phone-input"
                type="tel"
                value={editForm.phone}
                onChange={(event) =>
                  handleEditInputChange("phone", event.target.value)
                }
                className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                placeholder="Optional"
              />
            </div>
            <div className="flex justify-between pt-2">
              <button
                type="button"
                className="text-sm text-red-600 hover:underline disabled:opacity-50"
                onClick={() => void handleDeleteCustomer()}
                disabled={actionInProgress}
              >
                Delete Customer
              </button>
              <div className="flex gap-2">
                <button
                  type="button"
                  className="px-4 py-2 rounded bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-200"
                  onClick={closeEditModal}
                  disabled={actionInProgress}
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded bg-indigo-600 text-white disabled:opacity-50"
                  disabled={actionInProgress}
                >
                  {actionInProgress ? "Saving..." : "Save Changes"}
                </button>
              </div>
            </div>
          </form>
        </Modal>
      )}

      {auditModalOpen && selectedCustomer && (
        <Modal
          title={`Activity History for ${selectedCustomer.name}`}
          onClose={closeAuditModal}
        >
          {auditLoading ? (
            <p className="text-sm text-gray-600 dark:text-gray-300">Loading...</p>
          ) : auditError ? (
            <p className="text-sm text-red-600 dark:text-red-400">{auditError}</p>
          ) : auditEvents.length === 0 ? (
            <p className="text-sm text-gray-600 dark:text-gray-300">
              No activity has been recorded for this customer yet.
            </p>
          ) : (
            <ul className="space-y-3 max-h-72 overflow-y-auto pr-1">
              {auditEvents.map((event, index) => (
                <li
                  key={`${event.timestamp}-${event.action}-${index}`}
                  className="rounded border border-gray-200 dark:border-gray-700 p-3"
                >
                  <p className="text-sm font-semibold text-gray-800 dark:text-gray-100">
                    {event.action}
                  </p>
                  <p className="text-xs text-gray-600 dark:text-gray-300">
                    {formatDisplayDate(event.timestamp)}
                  </p>
                  {event.actor && (
                    <p className="text-xs text-gray-600 dark:text-gray-300">
                      Actor: {event.actor}
                    </p>
                  )}
                  {event.details && (
                    <p className="mt-1 text-sm text-gray-700 dark:text-gray-200">
                      {event.details}
                    </p>
                  )}
                </li>
              ))}
            </ul>
          )}
        </Modal>
      )}
    </>
  );
};

type ModalProps = {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
};

const Modal: React.FC<ModalProps> = ({ title, onClose, children }) => (
  <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 px-4">
    <div data-testid="modal-container" className="w-full max-w-lg rounded-lg bg-white dark:bg-gray-800 p-6 shadow-xl">
      <div className="mb-4 flex items-center justify-between">
        <h4 className="text-lg font-semibold text-gray-900 dark:text-gray-100">{title}</h4>
        <button
          type="button"
          aria-label="Close dialog"
          className="text-xl text-gray-500 hover:text-gray-700"
          onClick={onClose}
        >
          {"\u00d7"}
        </button>
      </div>
      <div className="space-y-4">{children}</div>
    </div>
  </div>
);

const CustomersPage = withRoleGuard(CustomersPageContent, MANAGER_ROLES, {
  message: "You need manager or higher access to manage customers.",
});

export { CustomersPageContent };
export default CustomersPage;






