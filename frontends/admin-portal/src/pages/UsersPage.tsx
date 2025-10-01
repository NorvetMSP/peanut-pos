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

type AuditEvent = {
  timestamp: string;
  action: string;
  actor?: string;
  details?: string;
};

const generateTempPassword = (): string => {
  const letters = Math.random().toString(36).slice(2, 8);
  const digits = Math.floor(Math.random() * 900 + 100);
  return `${letters}${digits}!A`;
};

const normalizeUser = (entry: unknown): ServiceUser | null => {
  if (!entry || typeof entry !== "object") return null;
  const candidate = entry as Record<string, unknown>;
  const id = typeof candidate.id === "string" ? candidate.id : null;
  const name = typeof candidate.name === "string" ? candidate.name : null;
  const email = typeof candidate.email === "string" ? candidate.email : null;
  const role = typeof candidate.role === "string" ? candidate.role : null;
  if (!id || !name || !email || !role) return null;

  const isActive =
    typeof candidate.is_active === "boolean" ? candidate.is_active : true;
  const createdAt =
    typeof candidate.created_at === "string" ? candidate.created_at : "";
  const updatedAt =
    typeof candidate.updated_at === "string" ? candidate.updated_at : "";
  const lastResetRaw = candidate.last_password_reset;
  const lastReset =
    typeof lastResetRaw === "string"
      ? lastResetRaw
      : lastResetRaw === null
      ? null
      : null;
  const forceReset =
    typeof candidate.force_password_reset === "boolean"
      ? candidate.force_password_reset
      : false;

  return {
    id,
    name,
    email,
    role,
    is_active: isActive,
    created_at: createdAt,
    updated_at: updatedAt,
    last_password_reset: lastReset,
    force_password_reset: forceReset,
  };
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
  const [selectedUser, setSelectedUser] = useState<ServiceUser | null>(null);
  const [editModalOpen, setEditModalOpen] = useState(false);
  const [editForm, setEditForm] = useState<{ name: string; role: string }>({
    name: "",
    role: DEFAULT_ROLE,
  });
  const [resetModalOpen, setResetModalOpen] = useState(false);
  const [resetPasswordValue, setResetPasswordValue] = useState("");
  const [auditModalOpen, setAuditModalOpen] = useState(false);
  const [auditEvents, setAuditEvents] = useState<AuditEvent[]>([]);
  const [auditLoading, setAuditLoading] = useState(false);
  const [auditError, setAuditError] = useState<string | null>(null);
  const [actionInProgress, setActionInProgress] = useState(false);

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

  const closeModals = () => {
    setEditModalOpen(false);
    setResetModalOpen(false);
    setAuditModalOpen(false);
    setSelectedUser(null);
    setResetPasswordValue("");
    setAuditEvents([]);
    setAuditError(null);
  };

  const openEditModal = (user: ServiceUser) => {
    setSelectedUser(user);
    setEditForm({ name: user.name, role: user.role });
    setEditModalOpen(true);
    setResetModalOpen(false);
    setAuditModalOpen(false);
    setSuccessMessage(null);
  };

  const handleEditInputChange = (field: keyof typeof editForm, value: string) => {
    setEditForm((prev) => ({ ...prev, [field]: value }));
  };

  const handleEditSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!selectedUser) return;
    if (!ensureTenantContext()) return;

    const trimmedName = editForm.name.trim();
    if (!trimmedName) {
      setError("Name must not be empty.");
      return;
    }

    setActionInProgress(true);
    setError(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/users/${selectedUser.id}`, {
        method: "PUT",
        headers: buildHeaders(true),
        body: JSON.stringify({ name: trimmedName, role: editForm.role }),
      });
      if (!response.ok) {
        throw new Error(`Update user failed (${response.status})`);
      }
      await fetchUsers();
      setSuccessMessage(`User ${selectedUser.email} updated.`);
      closeModals();
    } catch (err) {
      console.error("Unable to update user", err);
      setError("Unable to update user. Please try again.");
    } finally {
      setActionInProgress(false);
    }
  };

  const handleToggleActive = async (user: ServiceUser) => {
    if (!ensureTenantContext()) return;
    const targetStatus = !user.is_active;
    const actionLabel = targetStatus ? "reactivate" : "deactivate";
    if (!window.confirm(`Are you sure you want to ${actionLabel} ${user.email}?`)) {
      return;
    }

    setActionInProgress(true);
    setError(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/users/${user.id}`, {
        method: "PUT",
        headers: buildHeaders(true),
        body: JSON.stringify({ is_active: targetStatus }),
      });
      if (!response.ok) {
        throw new Error(`Toggle user failed (${response.status})`);
      }
      await fetchUsers();
      setSuccessMessage(
        targetStatus
          ? `${user.email} has been reactivated.`
          : `${user.email} has been deactivated.`,
      );
    } catch (err) {
      console.error("Unable to toggle user status", err);
      setError("Unable to update user status. Please try again.");
    } finally {
      setActionInProgress(false);
    }
  };

  const openResetModal = (user: ServiceUser) => {
    setSelectedUser(user);
    setResetPasswordValue(generateTempPassword());
    setResetModalOpen(true);
    setEditModalOpen(false);
    setAuditModalOpen(false);
    setSuccessMessage(null);
  };

  const handleResetSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!selectedUser) return;
    if (!ensureTenantContext()) return;

    const trimmed = resetPasswordValue.trim();
    if (trimmed.length < 8) {
      setError("Temporary password must be at least 8 characters.");
      return;
    }

    setActionInProgress(true);
    setError(null);
    try {
      const response = await fetch(
        `${AUTH_SERVICE_URL}/users/${selectedUser.id}/reset-password`,
        {
          method: "POST",
          headers: buildHeaders(true),
          body: JSON.stringify({ password: trimmed }),
        },
      );
      if (!response.ok) {
        throw new Error(`Reset password failed (${response.status})`);
      }
      await fetchUsers();
      setSuccessMessage(`Temporary password issued for ${selectedUser.email}.`);
      closeModals();
    } catch (err) {
      console.error("Unable to reset password", err);
      setError("Unable to reset password. Please try again.");
    } finally {
      setActionInProgress(false);
    }
  };

  const handleGeneratePassword = () => {
    setResetPasswordValue(generateTempPassword());
  };

  const formatDisplayDate = (value?: string | null): string => {
    if (!value) return "--";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString();
  };

  const openAuditModal = async (user: ServiceUser) => {
    setSelectedUser(user);
    setAuditModalOpen(true);
    setEditModalOpen(false);
    setResetModalOpen(false);
    setAuditLoading(true);
    setAuditError(null);
    try {
      const response = await fetch(`${AUTH_SERVICE_URL}/users/${user.id}/audit`, {
        headers: buildHeaders(),
      });
      if (response.status === 204) {
        setAuditEvents([]);
        setAuditLoading(false);
        return;
      }
      if (response.status === 404) {
        setAuditEvents([]);
        setAuditError("No audit history available yet.");
        setAuditLoading(false);
        return;
      }
      if (!response.ok) {
        throw new Error(`Fetch audit failed (${response.status})`);
      }
      const payload = (await response.json()) as unknown;
      const events: AuditEvent[] = Array.isArray(payload)
        ? payload.reduce<AuditEvent[]>((acc, entry) => {
            if (!entry || typeof entry !== "object") return acc;
            const candidate = entry as Record<string, unknown>;
            const timestamp = typeof candidate.timestamp === "string" ? candidate.timestamp : undefined;
            const action = typeof candidate.action === "string" ? candidate.action : undefined;
            if (!timestamp || !action) return acc; // skip invalid
            const actor = typeof candidate.actor === "string" ? candidate.actor : undefined;
            const details = typeof candidate.details === "string" ? candidate.details : undefined;
            acc.push({ timestamp, action, actor, details });
            return acc;
          }, [])
        : [];
      events.sort(
        (a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime(),
      );
      setAuditEvents(events);
    } catch (err) {
      console.error("Unable to load audit history", err);
      setAuditEvents([]);
      setAuditError("Unable to load audit history.");
    } finally {
      setAuditLoading(false);
    }
  };

  const sortedUsers = useMemo(() => {
    return [...users].sort((a, b) => {
      if (a.is_active !== b.is_active) {
        return a.is_active ? -1 : 1;
      }
      return a.name.localeCompare(b.name);
    });
  }, [users]);

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
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">
                      Status
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">
                      Last Updated
                    </th>
                    <th className="px-4 py-2 text-right text-xs font-semibold uppercase tracking-wider text-gray-600 dark:text-gray-300">
                      Actions
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {sortedUsers.length === 0 ? (
                    <tr>
                      <td
                        colSpan={6}
                        className="px-4 py-6 text-center text-sm text-gray-500 dark:text-gray-300"
                      >
                        {isLoading
                          ? "Loading users..."
                          : "No users found for this tenant."}
                      </td>
                    </tr>
                  ) : (
                    sortedUsers.map((user) => (
                      <tr
                        key={user.id}
                        className={`bg-white dark:bg-gray-800 ${user.is_active ? '' : 'opacity-60'}`}
                      >
                        <td className="px-4 py-2 text-sm text-gray-900 dark:text-gray-100">
                          {user.name}
                        </td>
                        <td className="px-4 py-2 text-sm text-gray-700 dark:text-gray-200">
                          {user.email}
                        </td>
                        <td className="px-4 py-2 text-sm text-gray-700 dark:text-gray-200">
                          {roleLabel(user.role)}
                        </td>
                        <td className="px-4 py-2 text-sm">
                          <span
                            className={`inline-flex items-center rounded-full px-2 py-1 text-xs font-semibold ${
                              user.is_active
                                ? 'bg-green-100 text-green-800'
                                : 'bg-gray-200 text-gray-600'
                            }`}
                          >
                            {user.is_active ? 'Active' : 'Inactive'}
                          </span>
                        </td>
                        <td className="px-4 py-2 text-sm text-gray-600 dark:text-gray-300">
                          {formatDisplayDate(user.updated_at)}
                        </td>
                        <td className="px-4 py-2 text-sm text-right">
                          <div className="flex flex-wrap justify-end gap-2">
                            <button
                              type="button"
                              className="text-sm text-indigo-600 hover:underline disabled:opacity-50"
                              onClick={() => openEditModal(user)}
                              disabled={actionInProgress}
                            >
                              Edit
                            </button>
                            <button
                              type="button"
                              className="text-sm text-indigo-600 hover:underline disabled:opacity-50"
                              onClick={() => void handleToggleActive(user)}
                              disabled={actionInProgress}
                            >
                              {user.is_active ? 'Deactivate' : 'Activate'}
                            </button>
                            <button
                              type="button"
                              className="text-sm text-indigo-600 hover:underline disabled:opacity-50"
                              onClick={() => openResetModal(user)}
                              disabled={actionInProgress}
                            >
                              Reset Password
                            </button>
                            <button
                              type="button"
                              className="text-sm text-indigo-600 hover:underline disabled:opacity-50"
                              onClick={() => void openAuditModal(user)}
                              disabled={auditLoading && selectedUser?.id === user.id}
                            >
                              View Audit
                            </button>
                          </div>
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
      {editModalOpen && selectedUser && (
        <Modal title={`Edit ${selectedUser.name}`} onClose={closeModals}>
          <form className="space-y-4" onSubmit={(event) => { void handleEditSubmit(event); }}>
            <div className="flex flex-col">
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
                Full Name
              </label>
              <input
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
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
                Role
              </label>
              <select
                value={editForm.role}
                onChange={(event) =>
                  handleEditInputChange("role", event.target.value)
                }
                className="mt-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                disabled={roleOptions.length === 0}
              >
                {roleOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="px-4 py-2 rounded bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-200"
                onClick={closeModals}
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
          </form>
        </Modal>
      )}
      {resetModalOpen && selectedUser && (
        <Modal
          title={`Reset Password for ${selectedUser.email}`}
          onClose={closeModals}
        >
          <p className="text-sm text-gray-600 dark:text-gray-300">
            Generate a new temporary password. The user will be prompted to change
            it on next sign-in.
          </p>
          <form className="space-y-4" onSubmit={(event) => { void handleResetSubmit(event); }}>
            <div className="flex flex-col">
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
                Temporary Password
              </label>
              <div className="mt-1 flex gap-2">
                <input
                  type="text"
                  value={resetPasswordValue}
                  onChange={(event) => setResetPasswordValue(event.target.value)}
                  className="flex-1 rounded-md border border-gray-300 px-3 py-2 text-gray-900 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary"
                  required
                  minLength={8}
                />
                <button
                  type="button"
                  className="px-3 py-2 rounded bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-200"
                  onClick={handleGeneratePassword}
                >
                  Regenerate
                </button>
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="px-4 py-2 rounded bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-200"
                onClick={closeModals}
                disabled={actionInProgress}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="px-4 py-2 rounded bg-indigo-600 text-white disabled:opacity-50"
                disabled={actionInProgress}
              >
                {actionInProgress ? "Resetting..." : "Reset Password"}
              </button>
            </div>
          </form>
        </Modal>
      )}
      {auditModalOpen && selectedUser && (
        <Modal
          title={`Audit History for ${selectedUser.email}`}
          onClose={closeModals}
        >
          {auditLoading ? (
            <p className="text-sm text-gray-600 dark:text-gray-300">Loading...</p>
          ) : auditError ? (
            <p className="text-sm text-red-600 dark:text-red-400">{auditError}</p>
          ) : auditEvents.length === 0 ? (
            <p className="text-sm text-gray-600 dark:text-gray-300">
              No audit events recorded for this user yet.
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
    <div className="w-full max-w-lg rounded-lg bg-white dark:bg-gray-800 p-6 shadow-xl">
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



