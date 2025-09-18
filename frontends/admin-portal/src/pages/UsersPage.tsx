import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import './AdminSectionModern.css';

const AUTH_SERVICE_URL = (import.meta.env.VITE_AUTH_SERVICE_URL ?? 'http://localhost:8085').replace(/\/$/, '');

type ServiceUser = {
  id: string;
  name: string;
  email: string;
  role: string;
};

type UserFormState = {
  name: string;
  email: string;
  password: string;
  role: string;
};

const ROLE_OPTIONS: Array<{ value: string; label: string }> = [
  { value: 'admin', label: 'Admin' },
  { value: 'manager', label: 'Manager' },
  { value: 'cashier', label: 'Cashier' },
];

const normalizeUser = (entry: unknown): ServiceUser | null => {
  if (!entry || typeof entry !== 'object') return null;
  const candidate = entry as Record<string, unknown>;
  const id = typeof candidate.id === 'string' ? candidate.id : null;
  const name = typeof candidate.name === 'string' ? candidate.name : null;
  const email = typeof candidate.email === 'string' ? candidate.email : null;
  const role = typeof candidate.role === 'string' ? candidate.role : null;
  if (!id || !name || !email || !role) return null;
  return { id, name, email, role };
};

const UsersPage: React.FC = () => {
  const { isLoggedIn, currentUser, token } = useAuth();
  const navigate = useNavigate();

  const [users, setUsers] = useState<ServiceUser[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [form, setForm] = useState<UserFormState>({
    name: '',
    email: '',
    password: '',
    role: ROLE_OPTIONS[2]?.value ?? 'cashier',
  });

  useEffect(() => {
    if (!isLoggedIn) navigate('/login', { replace: true });
  }, [isLoggedIn, navigate]);

  const tenantId = currentUser?.tenant_id ? String(currentUser.tenant_id) : null;

  const buildHeaders = useCallback(
    (includeJson = false): Record<string, string> => {
      const headers: Record<string, string> = {};
      if (tenantId) headers['X-Tenant-ID'] = tenantId;
      if (token) headers['Authorization'] = `Bearer ${token}`;
      if (includeJson) headers['Content-Type'] = 'application/json';
      return headers;
    },
    [tenantId, token]
  );

  const ensureTenantContext = useCallback((): boolean => {
    if (!tenantId) {
      setError('Tenant context is unavailable. Please log out and back in.');
      setUsers([]);
      return false;
    }
    return true;
  }, [tenantId]);

  const fetchUsers = useCallback(async () => {
    if (!ensureTenantContext()) return;
    setIsLoading(true);
    setError(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/users`, {
        headers: buildHeaders(),
      });
      if (!response.ok) {
        throw new Error(`Failed to fetch users (${response.status})`);
      }
      const payload = await response.json();
      const normalized = Array.isArray(payload)
        ? payload.map(normalizeUser).filter((item): item is ServiceUser => Boolean(item))
        : [];
      setUsers(normalized);
    } catch (err) {
      console.error('Unable to load users', err);
      setError('Unable to load users. Please try again.');
    } finally {
      setIsLoading(false);
    }
  }, [buildHeaders, ensureTenantContext]);

  useEffect(() => {
    fetchUsers();
  }, [fetchUsers]);

  const handleInputChange = (field: keyof UserFormState, value: string) => {
    setForm(prev => ({ ...prev, [field]: value }));
  };

  const validateEmail = (value: string): boolean => {
    if (!value) return false;
    const trimmed = value.trim();
    return /.+@.+\..+/.test(trimmed);
  };

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setSuccessMessage(null);

    if (!ensureTenantContext()) return;

    const trimmedName = form.name.trim();
    const trimmedEmail = form.email.trim();

    if (!trimmedName) {
      setError('Name is required.');
      return;
    }
    if (!validateEmail(trimmedEmail)) {
      setError('Enter a valid email address.');
      return;
    }
    if (!form.password) {
      setError('Password is required.');
      return;
    }

    setIsSubmitting(true);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/users`, {
        method: 'POST',
        headers: buildHeaders(true),
        body: JSON.stringify({
          name: trimmedName,
          email: trimmedEmail,
          password: form.password,
          role: form.role,
        }),
      });
      if (!response.ok) {
        throw new Error(`Failed to create user (${response.status})`);
      }
      const created = normalizeUser(await response.json());
      if (created) {
        setUsers(prev => [created, ...prev]);
      } else {
        fetchUsers();
      }
      setSuccessMessage(`User ${trimmedName} created.`);
      setForm({ name: '', email: '', password: '', role: form.role });
    } catch (err) {
      console.error('Create user failed', err);
      setError('Unable to create user. Please try again.');
    } finally {
      setIsSubmitting(false);
    }
  };

  const sortedUsers = useMemo(() => {
    return [...users].sort((a, b) => a.name.localeCompare(b.name));
  }, [users]);

  return (
    <div
      className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col"
      style={{ fontFamily: 'Raleway, sans-serif', background: 'linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)' }}
    >
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <div>
            <h2>User Management</h2>
            <p>Invite employees and review existing access.</p>
          </div>
          <button className="admin-section-btn" onClick={() => navigate('/home')}>
            Back to Admin Home
          </button>
        </div>

        <div className="admin-section-content grid gap-8">
          {error && (
            <div className="rounded-md bg-red-100 px-4 py-3 text-sm text-red-700 dark:bg-red-900/40 dark:text-red-200">
              {error}
            </div>
          )}
          {successMessage && (
            <div className="rounded-md bg-emerald-100 px-4 py-3 text-sm text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-100">
              {successMessage}
            </div>
          )}

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Add New User</h3>
            <form className="mt-4 grid gap-4 md:grid-cols-2" onSubmit={handleSubmit}>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Full Name</label>
                <input
                  type="text"
                  value={form.name}
                  onChange={event => handleInputChange('name', event.target.value)}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  placeholder="Jane Doe"
                  required
                />
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Email</label>
                <input
                  type="email"
                  value={form.email}
                  onChange={event => handleInputChange('email', event.target.value)}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  placeholder="jane@example.com"
                  required
                />
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Temporary Password</label>
                <input
                  type="password"
                  value={form.password}
                  onChange={event => handleInputChange('password', event.target.value)}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  placeholder="Set an initial password"
                  required
                />
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">Role</label>
                <select
                  value={form.role}
                  onChange={event => handleInputChange('role', event.target.value)}
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                >
                  {ROLE_OPTIONS.map(option => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </div>
              <div className="md:col-span-2 flex justify-end gap-2">
                <button
                  type="button"
                  className="px-4 py-2 rounded bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-200"
                  onClick={() => setForm({ name: '', email: '', password: '', role: ROLE_OPTIONS[2]?.value ?? 'cashier' })}
                  disabled={isSubmitting}
                >
                  Clear
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded text-white"
                  style={{ background: '#19b4b9' }}
                  onMouseOver={event => (event.currentTarget.style.background = '#153a5b')}
                  onMouseOut={event => (event.currentTarget.style.background = '#19b4b9')}
                  disabled={isSubmitting}
                >
                  {isSubmitting ? 'Creating...' : 'Create User'}
                </button>
              </div>
            </form>
          </section>

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
            <div className="flex items-center justify-between">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Current Users</h3>
              <button
                type="button"
                className="admin-section-btn"
                onClick={() => fetchUsers()}
                disabled={isLoading}
              >
                {isLoading ? 'Refreshing...' : 'Refresh List'}
              </button>
            </div>
            <div className="mt-4 overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                <thead className="bg-gray-50 dark:bg-gray-700">
                  <tr>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">Name</th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">Email</th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">Role</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {sortedUsers.length === 0 ? (
                    <tr>
                      <td colSpan={3} className="px-4 py-6 text-center text-sm text-gray-500 dark:text-gray-300">
                        {isLoading ? 'Loading users...' : 'No users found for this tenant.'}
                      </td>
                    </tr>
                  ) : (
                    sortedUsers.map(user => (
                      <tr key={user.id} className="bg-white dark:bg-gray-800">
                        <td className="px-4 py-2 text-sm text-gray-900 dark:text-gray-100">{user.name}</td>
                        <td className="px-4 py-2 text-sm text-gray-700 dark:text-gray-200">{user.email}</td>
                        <td className="px-4 py-2 text-sm text-gray-700 dark:text-gray-200 capitalize">{user.role}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
};

export default UsersPage;
