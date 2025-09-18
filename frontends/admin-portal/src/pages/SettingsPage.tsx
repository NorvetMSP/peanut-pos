import React, {
  useCallback,
  useEffect,
  useMemo,
  useState,
} from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import { resolveServiceUrl } from '../utils/env';
import './AdminSectionModern.css';

const AUTH_SERVICE_URL = resolveServiceUrl('VITE_AUTH_SERVICE_URL', 'http://localhost:8085');

type TenantRecord = {
  id: string;
  name: string;
};

type IntegrationKeyRecord = {
  id: string;
  tenant_id: string;
  label: string;
  key_suffix: string;
  created_at: string;
  revoked_at: string | null;
};

type IntegrationKeyCreated = IntegrationKeyRecord & {
  api_key: string;
};

type TenantJson = Record<string, unknown>;
type IntegrationKeyJson = Record<string, unknown>;

const normalizeTenant = (entry: unknown): TenantRecord | null => {
  if (typeof entry !== 'object' || entry === null) return null;
  const candidate = entry as TenantJson;
  const id = candidate.id;
  const name = candidate.name;
  if (typeof id !== 'string' || typeof name !== 'string') return null;
  return { id, name };
};

const normalizeIntegrationKey = (entry: unknown): IntegrationKeyRecord | null => {
  if (typeof entry !== 'object' || entry === null) return null;
  const candidate = entry as IntegrationKeyJson;
  const id = candidate.id;
  const tenantId = candidate.tenant_id;
  const label = candidate.label;
  const suffix = candidate.key_suffix;
  const createdAt = candidate.created_at;
  const revokedAt = candidate.revoked_at;
  if (
    typeof id !== 'string' ||
    typeof tenantId !== 'string' ||
    typeof label !== 'string' ||
    typeof suffix !== 'string' ||
    typeof createdAt !== 'string'
  ) {
    return null;
  }
  return {
    id,
    tenant_id: tenantId,
    label,
    key_suffix: suffix,
    created_at: createdAt,
    revoked_at: typeof revokedAt === 'string' ? revokedAt : null,
  };
};

const isIntegrationKeyCreated = (value: unknown): value is IntegrationKeyCreated => {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as IntegrationKeyJson;
  return typeof candidate.api_key === 'string' && normalizeIntegrationKey(value) !== null;
};

const sortTenants = (items: TenantRecord[]): TenantRecord[] =>
  [...items].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));

const formatTimestamp = (value: string): string => {
  try {
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) return value;
    return parsed.toLocaleString();
  } catch (err) {
    console.warn('Unable to format timestamp', err);
    return value;
  }
};

const SettingsPage: React.FC = () => {
  const { isLoggedIn, currentUser, token } = useAuth();
  const navigate = useNavigate();

  const [tenants, setTenants] = useState<TenantRecord[]>([]);
  const [selectedTenantId, setSelectedTenantId] = useState<string | null>(null);
  const [integrationKeys, setIntegrationKeys] = useState<IntegrationKeyRecord[]>([]);

  const [loadingTenants, setLoadingTenants] = useState(false);
  const [loadingKeys, setLoadingKeys] = useState(false);
  const [creatingTenant, setCreatingTenant] = useState(false);
  const [creatingKey, setCreatingKey] = useState(false);

  const [newTenantName, setNewTenantName] = useState('');
  const [keyLabel, setKeyLabel] = useState('');
  const [revokeExisting, setRevokeExisting] = useState(false);
  const [latestKeySecret, setLatestKeySecret] = useState<string | null>(null);

  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  const tenantHeader = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;
  const isSuperAdmin = currentUser?.role === 'super_admin';

  useEffect(() => {
    if (!isLoggedIn) void navigate('/login', { replace: true });
  }, [isLoggedIn, navigate]);

  const buildHeaders = useCallback(
    (includeJson = false): Record<string, string> => {
      const headers: Record<string, string> = {};
      if (tenantHeader) headers['X-Tenant-ID'] = tenantHeader;
      if (token) headers['Authorization'] = `Bearer ${token}`;
      if (includeJson) headers['Content-Type'] = 'application/json';
      return headers;
    },
    [tenantHeader, token]
  );

  const fetchIntegrationKeys = useCallback(
    async (tenantId: string): Promise<void> => {
      setLoadingKeys(true);
      setError(null);
      try {
        const response = await fetch(`${AUTH_SERVICE_URL}/tenants/${tenantId}/integration-keys`, {
          headers: buildHeaders(),
        });
        if (!response.ok) {
          throw new Error(`Failed to fetch keys (${response.status})`);
        }
        const payload = (await response.json()) as unknown;
        const keys: IntegrationKeyRecord[] = Array.isArray(payload)
          ? payload
              .map(normalizeIntegrationKey)
              .filter((item): item is IntegrationKeyRecord => item !== null)
          : [];
        setIntegrationKeys(keys);
      } catch (err) {
        console.error('Unable to load integration keys', err);
        setError('Unable to retrieve integration keys.');
        setIntegrationKeys([]);
      } finally {
        setLoadingKeys(false);
      }
    },
    [buildHeaders]
  );

  const fetchTenants = useCallback(async (): Promise<void> => {
    if (!tenantHeader) return;
    if (!isSuperAdmin) {
      const record: TenantRecord = { id: tenantHeader, name: 'Current Tenant' };
      setTenants([record]);
      setSelectedTenantId(prev => prev ?? tenantHeader);
      return;
    }

    setLoadingTenants(true);
    setError(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/tenants`, {
        headers: buildHeaders(),
      });
      if (!response.ok) {
        throw new Error(`Failed to fetch tenants (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const records: TenantRecord[] = Array.isArray(payload)
        ? payload
            .map(normalizeTenant)
            .filter((item): item is TenantRecord => item !== null)
        : [];
      const ordered = sortTenants(records);
      setTenants(ordered);
      setSelectedTenantId(prev => {
        if (prev && ordered.some(record => record.id === prev)) {
          return prev;
        }
        return ordered[0]?.id ?? null;
      });
    } catch (err) {
      console.error('Unable to load tenants', err);
      setError('Unable to retrieve tenant list.');
      setTenants([]);
      setSelectedTenantId(null);
    } finally {
      setLoadingTenants(false);
    }
  }, [buildHeaders, isSuperAdmin, tenantHeader]);

  useEffect(() => {
    if (tenantHeader) {
      void fetchTenants();
    }
  }, [fetchTenants, tenantHeader]);

  useEffect(() => {
    if (selectedTenantId) {
      setLatestKeySecret(null);
      void fetchIntegrationKeys(selectedTenantId);
    } else {
      setIntegrationKeys([]);
    }
  }, [selectedTenantId, fetchIntegrationKeys]);

  const handleCreateTenant = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!isSuperAdmin) return;
    const trimmed = newTenantName.trim();
    if (!trimmed) {
      setError('Tenant name is required.');
      return;
    }
    setCreatingTenant(true);
    setError(null);
    setSuccessMessage(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/tenants`, {
        method: 'POST',
        headers: buildHeaders(true),
        body: JSON.stringify({ name: trimmed }),
      });
      if (!response.ok) {
        throw new Error(`Create tenant failed (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const created = normalizeTenant(payload);
      if (!created) {
        throw new Error('Invalid tenant payload');
      }
      const updated = sortTenants([...tenants, created]);
      setTenants(updated);
      setNewTenantName('');
      setSuccessMessage(`Tenant "${created.name}" created.`);
      setSelectedTenantId(created.id);
    } catch (err) {
      console.error('Unable to create tenant', err);
      setError('Unable to create tenant. Please try again.');
    } finally {
      setCreatingTenant(false);
    }
  };

  const handleCreateKey = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!selectedTenantId) {
      setError('Select a tenant before creating a key.');
      return;
    }
    const trimmedLabel = keyLabel.trim();
    setCreatingKey(true);
    setError(null);
    setSuccessMessage(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/tenants/${selectedTenantId}/integration-keys`, {
        method: 'POST',
        headers: buildHeaders(true),
        body: JSON.stringify({ label: trimmedLabel || undefined, revoke_existing: revokeExisting }),
      });
      if (!response.ok) {
        throw new Error(`Create integration key failed (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      if (!isIntegrationKeyCreated(payload)) {
        throw new Error('Invalid key response');
      }
      const created = normalizeIntegrationKey(payload);
      if (created) {
        void fetchIntegrationKeys(selectedTenantId);
      }
      setLatestKeySecret(payload.api_key);
      setSuccessMessage('A new integration key was generated. Copy it now; it will not be shown again.');
      setKeyLabel('');
      setRevokeExisting(false);
    } catch (err) {
      console.error('Unable to create integration key', err);
      setError('Unable to create integration key.');
    } finally {
      setCreatingKey(false);
    }
  };

  const handleRevokeKey = async (keyId: string): Promise<void> => {
    if (!keyId) return;
    setError(null);
    setSuccessMessage(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/integration-keys/${keyId}/revoke`, {
        method: 'POST',
        headers: buildHeaders(),
      });
      if (!response.ok) {
        throw new Error(`Revoke integration key failed (${response.status})`);
      }
      if (selectedTenantId) {
        void fetchIntegrationKeys(selectedTenantId);
      }
      setSuccessMessage('Integration key revoked.');
    } catch (err) {
      console.error('Unable to revoke integration key', err);
      setError('Unable to revoke integration key.');
    }
  };

  const handleCopySecret = async (): Promise<void> => {
    if (!latestKeySecret) return;
    try {
      await navigator.clipboard.writeText(latestKeySecret);
      setSuccessMessage('Integration key copied to clipboard.');
    } catch (err) {
      console.warn('Clipboard copy failed', err);
    }
  };

  const tenantOptions = useMemo(
    () => tenants.map(item => ({ value: item.id, label: item.name })),
    [tenants]
  );

  return (
    <div
      className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col"
      style={{
        fontFamily: 'Raleway, sans-serif',
        background: 'linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)',
      }}
    >
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>Tenant &amp; Integration Settings</h2>
          <p>Provision tenants and manage integration API keys on a per-tenant basis.</p>
        </div>

        <div className="admin-section-content">
          {error && (
            <div className="rounded bg-red-100 text-red-800 px-4 py-3 mb-4">{error}</div>
          )}
          {successMessage && (
            <div className="rounded bg-green-100 text-green-800 px-4 py-3 mb-4">{successMessage}</div>
          )}
          {latestKeySecret && (
            <div className="rounded bg-amber-50 border border-amber-300 text-amber-900 px-4 py-4 mb-6">
              <p className="font-semibold mb-2">New API Key</p>
              <p className="font-mono break-all text-sm mb-3">{latestKeySecret}</p>
              <button className="admin-section-btn" onClick={() => void handleCopySecret()} type="button">
                Copy Key
              </button>
            </div>
          )}

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow mb-6">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Tenants</h3>
            {isSuperAdmin ? (
              <>
                <form className="grid gap-4 md:grid-cols-2" onSubmit={event => { void handleCreateTenant(event); }}>
                  <div className="flex flex-col md:col-span-2">
                    <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Tenant Name</label>
                    <input
                      type="text"
                      value={newTenantName}
                      onChange={event => setNewTenantName(event.target.value)}
                      className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                      placeholder="Retailer HQ"
                      required
                    />
                  </div>
                  <div className="md:col-span-2 flex justify-end">
                    <button className="admin-section-btn" type="submit" disabled={creatingTenant}>
                      {creatingTenant ? 'Creating...' : 'Create Tenant'}
                    </button>
                  </div>
                </form>
                <div className="mt-6">
                  <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-200 mb-2">Existing Tenants</h4>
                  {loadingTenants ? (
                    <p className="text-sm text-gray-500">Loading tenants...</p>
                  ) : tenants.length === 0 ? (
                    <p className="text-sm text-gray-500">No tenants available.</p>
                  ) : (
                    <ul className="space-y-2 text-sm text-gray-700 dark:text-gray-200">
                      {tenantOptions.map(option => (
                        <li key={option.value} className="flex items-center justify-between">
                          <span>{option.label}</span>
                          <span className="font-mono text-xs text-gray-500">{option.value}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              </>
            ) : (
              <p className="text-sm text-gray-600 dark:text-gray-300">
                Tenant provisioning is only available to super administrators.
              </p>
            )}
          </section>

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
            <div className="flex flex-col md:flex-row md:items-end md:justify-between gap-4">
              <div>
                <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Integration Keys</h3>
                <p className="text-sm text-gray-600 dark:text-gray-300">
                  Generate and rotate API keys used by external systems per tenant.
                </p>
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Tenant</label>
                <select
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  value={selectedTenantId ?? ''}
                  onChange={event => setSelectedTenantId(event.target.value || null)}
                  disabled={!isSuperAdmin || tenantOptions.length <= 1}
                >
                  {tenantOptions.map(option => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            <form className="mt-6 grid gap-4 md:grid-cols-2" onSubmit={event => { void handleCreateKey(event); }}>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">
                  Key Label
                </label>
                <input
                  type="text"
                  value={keyLabel}
                  onChange={event => setKeyLabel(event.target.value)}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  placeholder="Valor Gateway"
                />
              </div>
              <div className="flex items-center mt-6 md:mt-0">
                <label className="inline-flex items-center text-sm text-gray-600 dark:text-gray-300">
                  <input
                    type="checkbox"
                    className="mr-2"
                    checked={revokeExisting}
                    onChange={event => setRevokeExisting(event.target.checked)}
                  />
                  Revoke existing active keys for this tenant
                </label>
              </div>
              <div className="md:col-span-2 flex justify-end">
                <button className="admin-section-btn" type="submit" disabled={creatingKey || !selectedTenantId}>
                  {creatingKey ? 'Generating...' : 'Generate Key'}
                </button>
              </div>
            </form>

            <div className="mt-6">
              <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-200 mb-2">Active &amp; Revoked Keys</h4>
              {loadingKeys ? (
                <p className="text-sm text-gray-500">Loading keys...</p>
              ) : integrationKeys.length === 0 ? (
                <p className="text-sm text-gray-500">No integration keys found for this tenant.</p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700 text-sm">
                    <thead className="bg-gray-50 dark:bg-gray-700">
                      <tr>
                        <th className="px-4 py-2 text-left font-semibold text-gray-600 dark:text-gray-300">Label</th>
                        <th className="px-4 py-2 text-left font-semibold text-gray-600 dark:text-gray-300">Suffix</th>
                        <th className="px-4 py-2 text-left font-semibold text-gray-600 dark:text-gray-300">Created</th>
                        <th className="px-4 py-2 text-left font-semibold text-gray-600 dark:text-gray-300">Status</th>
                        <th className="px-4 py-2"></th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                      {integrationKeys.map(key => {
                        const isRevoked = Boolean(key.revoked_at);
                        return (
                          <tr key={key.id} className="bg-white dark:bg-gray-800">
                            <td className="px-4 py-2 text-gray-900 dark:text-gray-100">{key.label || 'Unnamed Key'}</td>
                            <td className="px-4 py-2 font-mono text-xs text-gray-600 dark:text-gray-300">{key.key_suffix}</td>
                            <td className="px-4 py-2 text-gray-700 dark:text-gray-200">{formatTimestamp(key.created_at)}</td>
                            <td className="px-4 py-2 text-gray-700 dark:text-gray-200">
                              {isRevoked ? 'Revoked' : 'Active'}
                              {isRevoked && key.revoked_at ? ` • ${formatTimestamp(key.revoked_at)}` : ''}
                            </td>
                            <td className="px-4 py-2 text-right">
                              <button
                                type="button"
                                className="text-sm text-red-600 hover:text-red-700 disabled:text-gray-400"
                                onClick={() => void handleRevokeKey(key.id)}
                                disabled={isRevoked}
                              >
                                Revoke
                              </button>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          </section>
        </div>

        <div style={{ textAlign: 'right', marginTop: '2rem' }}>
          <button className="admin-section-btn" onClick={() => void navigate('/home')} type="button">
            Back to Admin Home
          </button>
        </div>
      </div>
    </div>
  );
};

export default SettingsPage;

