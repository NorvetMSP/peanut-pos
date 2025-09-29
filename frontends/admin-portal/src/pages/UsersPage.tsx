import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAuth } from "../AuthContext";
import { useHasAnyRole } from "../hooks/useRoleAccess";
import { resolveServiceUrl } from "../utils/env";
import AccessDenied from "../components/AccessDenied";
import {
  ADMIN_ROLES,
  ROLE_PRIORITY,
  ROLE_CASHIER,
  SUPER_ADMIN_ROLES,
  ensureRoleOrder,
  roleLabel,
} from "../rbac";
import "./AdminSectionModern.css";

const AUTH_SERVICE_URL = resolveServiceUrl(
  "VITE_AUTH_SERVICE_URL",
  "http://localhost:8085",
);

type ServiceUser = {
  id: string;
  name: string;
  email: string;
  role: string;
  is_active: boolean;
  created_at: string;
  updated_at: string;
  last_password_reset?: string | null;
  force_password_reset: boolean;
};

type UserFormState = {
  name: string;
  email: string;
  password: string;
  role: string;
};

const DEFAULT_ROLE = ROLE_CASHIER;

type RoleOption = { value: string; label: string };

type TenantOption = { value: string; label: string };

const normalizeUser = (entry: unknown): ServiceUser | null => {
  if (!entry || typeof entry !== "object") return null;
  const candidate = entry as Record<string, unknown>;
  const id = typeof candidate.id === "string" ? candidate.id : null;
  const name = typeof candidate.name === "string" ? candidate.name : null;
  const email = typeof candidate.email === "string" ? candidate.email : null;
  const role = typeof candidate.role === "string" ? candidate.role : null;
  if (!id || !name || !email || !role) return null;
  return { id, name, email, role };
};

const UsersPageContent: React.FC = () => {
  const { isLoggedIn, currentUser, token } = useAuth();
  const navigate = useNavigate();
  const canManageUsers = useHasAnyRole(ADMIN_ROLES);
  const isSuperAdmin = useHasAnyRole(SUPER_ADMIN_ROLES);

  const [users, setUsers] = useState<ServiceUser[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [rolesLoading, setRolesLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [roleOptions, setRoleOptions] = useState<RoleOption[]>([]);
  const [form, setForm] = useState<UserFormState>({
    name: "",
    email: "",
    password: "",
    role: DEFAULT_ROLE,
  });
  const [tenantOptions, setTenantOptions] = useState<TenantOption[]>([]);
  const [tenantsLoading, setTenantsLoading] = useState(false);
  const tenantId = currentUser?.tenant_id
    ? String(currentUser.tenant_id)
    : null;
  const [selectedTenantId, setSelectedTenantId] = useState<string | null>(null);

  const effectiveTenantId = useMemo(
    () => (isSuperAdmin ? (selectedTenantId ?? tenantId) : tenantId),
    [isSuperAdmin, selectedTenantId, tenantId],
  );

  useEffect(() => {
    if (!isLoggedIn) void navigate("/login", { replace: true });
  }, [isLoggedIn, navigate]);

  useEffect(() => {
    if (!isSuperAdmin) {
      setSelectedTenantId(tenantId ?? null);
    }
  }, [isSuperAdmin, tenantId]);

  const buildHeaders = useCallback(
    (includeJson = false, tenantOverride?: string): Record<string, string> => {
      const headers: Record<string, string> = {};
      const tenantHeader = tenantOverride ?? effectiveTenantId;
      if (tenantHeader) headers["X-Tenant-ID"] = tenantHeader;
      if (token) headers["Authorization"] = `Bearer ${token}`;
      if (includeJson) headers["Content-Type"] = "application/json";
      return headers;
    },
    [effectiveTenantId, token],
  );

  const fetchRoles = useCallback(async (): Promise<void> => {
    if (!canManageUsers) {
      setRolesLoading(false);
      setRoleOptions([]);
      return;
    }

    setRolesLoading(true);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/roles`, {
        headers: buildHeaders(),
      });
      if (!response.ok) {
        throw new Error(`Failed to fetch roles (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const values = Array.isArray(payload)
        ? payload.filter(
            (entry): entry is string =>
              typeof entry === "string" && entry.trim().length > 0,
          )
        : [];
      const ordered = ensureRoleOrder(
        values.length > 0 ? values : ROLE_PRIORITY,
      );
      const options = ordered.map((value) => ({
        value,
        label: roleLabel(value),
      }));
      setRoleOptions(options);
      setForm((prev) => {
        if (options.some((option) => option.value === prev.role)) {
          return prev;
        }
        return {
          ...prev,
          role: options[0]?.value ?? DEFAULT_ROLE,
        };
      });
    } catch (err) {
      console.error("Unable to load role catalog", err);
      const fallbackOptions = ensureRoleOrder(ROLE_PRIORITY).map((value) => ({
        value,
        label: roleLabel(value),
      }));
      setRoleOptions(fallbackOptions);
      setForm((prev) => ({
        ...prev,
        role: fallbackOptions[0]?.value ?? DEFAULT_ROLE,
      }));
    } finally {
      setRolesLoading(false);
    }
  }, [buildHeaders, canManageUsers]);

  const fetchTenants = useCallback(async (): Promise<void> => {
    if (!canManageUsers || !isSuperAdmin) return;
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
            .map((entry) => {
              if (!entry || typeof entry !== "object") return null;
              const candidate = entry as Record<string, unknown>;
              const id = typeof candidate.id === "string" ? candidate.id : null;
              const name =
                typeof candidate.name === "string" ? candidate.name : null;
              if (!id || !name) return null;
              return { value: id, label: name } as TenantOption;
            })
            .filter((option): option is TenantOption => option !== null)
        : [];
      options.sort((a, b) => a.label.localeCompare(b.label));
      setTenantOptions(options);
      setSelectedTenantId((prev) => {
        if (prev) return prev;
        if (options.length > 0) return options[0].value;
        return tenantId ?? null;
      });
    } catch (err) {
      console.error("Unable to load tenant catalog", err);
      setError((prev) => prev ?? "Unable to load tenants. Please try again.");
    } finally {
      setTenantsLoading(false);
    }
  }, [buildHeaders, canManageUsers, isSuperAdmin, tenantId]);

  useEffect(() => {
    if (!canManageUsers) {
      setRoleOptions([]);
      return;
    }
    void fetchRoles();
  }, [canManageUsers, fetchRoles]);

  useEffect(() => {
    if (!canManageUsers) {
      setTenantOptions([]);
      setSelectedTenantId(tenantId ?? null);
      return;
    }

    if (isSuperAdmin) {
      void fetchTenants();
    } else {
      setTenantOptions([]);
      setSelectedTenantId(tenantId ?? null);
    }
  }, [canManageUsers, fetchTenants, isSuperAdmin, tenantId]);

  const defaultRoleValue = useMemo(() => {
    if (roleOptions.length === 0) return DEFAULT_ROLE;
    const preferred = roleOptions.find(
      (option) => option.value === DEFAULT_ROLE,
    );
    return (preferred ?? roleOptions[0]).value;
  }, [roleOptions]);

  const ensureTenantContext = useCallback((): boolean => {
    if (!canManageUsers) {
      setError("You need an administrator role to manage users.");
      setUsers([]);
      return false;
    }

    if (!effectiveTenantId) {
      setError(
        "Tenant context is unavailable. Please select a tenant or log in again.",
      );
      setUsers([]);
      return false;
    }
    return true;
  }, [canManageUsers, effectiveTenantId]);

  const fetchUsers = useCallback(async (): Promise<void> => {
    if (!canManageUsers) {
      setUsers([]);
      return;
    }

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
      const payload = (await response.json()) as unknown;
      const normalized = Array.isArray(payload)
        ? payload
            .map(normalizeUser)
            .filter((item): item is ServiceUser => item !== null)
        : [];
      setUsers(normalized);
    } catch (err) {
      console.error("Unable to load users", err);
      setError("Unable to load users. Please try again.");
    } finally {
      setIsLoading(false);
    }
  }, [buildHeaders, canManageUsers, ensureTenantContext]);

  useEffect(() => {
    if (!canManageUsers) {
      setUsers([]);
      return;
    }
    void fetchUsers();
  }, [canManageUsers, fetchUsers]);

  const handleInputChange = (field: keyof UserFormState, value: string) => {
    setForm((prev) => ({ ...prev, [field]: value }));
  };

  const validateEmail = (value: string): boolean => {
    if (!value) return false;
    const trimmed = value.trim();
    return /.+@.+\..+/.test(trimmed);
  };

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    if (!canManageUsers) {
      setError("You need an administrator role to manage users.");
      return;
    }

    if (!ensureTenantContext()) return;

    if (!validateEmail(form.email)) {
      setError("Provide a valid email address.");
      return;
    }

    setError(null);
    setSuccessMessage(null);
    setIsSubmitting(true);

    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/users`, {
        method: "POST",
        headers: buildHeaders(true),
        body: JSON.stringify(form),
      });
      if (!response.ok) {
        throw new Error(`Create user failed (${response.status})`);
      }
      await fetchUsers();
      setSuccessMessage("User created successfully.");
      setForm((prev) => ({
        name: "",
        email: "",
        password: "",
        role: prev.role,
      }));
    } catch (err) {
      console.error("Create user failed", err);
      setError("Unable to create user. Please try again.");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleClear = () => {
    setForm({ name: "", email: "", password: "", role: defaultRoleValue });
  };

  const sortedUsers = useMemo(() => {
    return [...users].sort((a, b) => a.name.localeCompare(b.name));
  }, [users]);

  return (
    <div
      className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col"
      style={{
        fontFamily: "Raleway, sans-serif",
        background: "linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)",
      }}
    >
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>Users</h2>
          <p>
            Invite team members and manage their access levels for this tenant.
          </p>
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

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow mb-6">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
              Add New User
            </h3>
            <form
              className="mt-4 grid gap-4 md:grid-cols-2"
              onSubmit={(event) => {
                void handleSubmit(event);
              }}
            >
              {isSuperAdmin && (
                <div className="md:col-span-2 flex flex-col">
                  <label className="text-sm font-medium text-gray-600 dark:text-gray-300">
                    Tenant
                  </label>
                  <select
                    value={selectedTenantId ?? ""}
                    onChange={(event) =>
                      setSelectedTenantId(event.target.value || null)
                    }
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

              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">
                  Full Name
                </label>
                <input
                  type="text"
                  value={form.name}
                  onChange={(event) =>
                    handleInputChange("name", event.target.value)
                  }
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  placeholder="Jane Doe"
                  required
                />
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">
                  Email
                </label>
                <input
                  type="email"
                  value={form.email}
                  onChange={(event) =>
                    handleInputChange("email", event.target.value)
                  }
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  placeholder="jane@example.com"
                  required
                />
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">
                  Temporary Password
                </label>
                <input
                  type="password"
                  value={form.password}
                  onChange={(event) =>
                    handleInputChange("password", event.target.value)
                  }
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  placeholder="Set an initial password"
                  required
                />
              </div>
              <div className="flex flex-col">
                <label className="text-sm font-medium text-gray-600 dark:text-gray-300">
                  Role
                </label>
                <select
                  value={roleOptions.length === 0 ? "" : form.role}
                  onChange={(event) =>
                    handleInputChange("role", event.target.value)
                  }
                  className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  disabled={rolesLoading || roleOptions.length === 0}
                >
                  {rolesLoading && roleOptions.length === 0 ? (
                    <option value="" disabled>
                      Loading roles...
                    </option>
                  ) : roleOptions.length === 0 ? (
                    <option value="" disabled>
                      No roles available
                    </option>
                  ) : (
                    roleOptions.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))
                  )}
                </select>
              </div>
              <div className="md:col-span-2 flex justify-end gap-2">
                <button
                  type="button"
                  className="px-4 py-2 rounded bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-200"
                  onClick={handleClear}
                  disabled={isSubmitting}
                >
                  Clear
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 rounded text-white"
                  style={{ background: "#19b4b9" }}
                  onMouseOver={(event) =>
                    (event.currentTarget.style.background = "#153a5b")
                  }
                  onMouseOut={(event) =>
                    (event.currentTarget.style.background = "#19b4b9")
                  }
                  disabled={
                    isSubmitting ||
                    rolesLoading ||
                    roleOptions.length === 0 ||
                    tenantsLoading ||
                    !effectiveTenantId
                  }
                >
                  {isSubmitting ? "Creating..." : "Create User"}
                </button>
              </div>
            </form>
          </section>

          <section className="rounded-lg bg-white dark:bg-gray-800 p-6 shadow">
            <div className="flex items-center justify-between">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
                Current Users
              </h3>
              <button
                type="button"
                className="admin-section-btn"
                onClick={() => void fetchUsers()}
                disabled={isLoading}
              >
                {isLoading ? "Refreshing..." : "Refresh List"}
              </button>
            </div>
            <div className="mt-4 overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                <thead className="bg-gray-50 dark:bg-gray-700">
                  <tr>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">
                      Name
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">
                      Email
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">
                      Role
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {sortedUsers.length === 0 ? (
                    <tr>
                      <td
                        colSpan={3}
                        className="px-4 py-6 text-center text-sm text-gray-500 dark:text-gray-300"
                      >
                        {isLoading
                          ? "Loading users..."
                          : "No users found for this tenant."}
                      </td>
                    </tr>
                  ) : (
                    sortedUsers.map((user) => (
                      <tr key={user.id} className="bg-white dark:bg-gray-800">
                        <td className="px-4 py-2 text-sm text-gray-900 dark:text-gray-100">
                          {user.name}
                        </td>
                        <td className="px-4 py-2 text-sm text-gray-700 dark:text-gray-200">
                          {user.email}
                        </td>
                        <td className="px-4 py-2 text-sm text-gray-700 dark:text-gray-200">
                          {roleLabel(user.role)}
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
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
  );
};

const UsersPage: React.FC = () => {
  const canManageUsers = useHasAnyRole(ADMIN_ROLES);

  if (!canManageUsers) {
    return (
      <AccessDenied message="You need an administrator role to manage portal users." />
    );
  }

  return <UsersPageContent />;
};

export default UsersPage;
