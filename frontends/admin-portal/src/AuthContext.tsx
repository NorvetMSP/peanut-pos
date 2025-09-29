import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import { Navigate, useLocation } from "react-router-dom";
import { ensureRoleOrder, ROLE_PRIORITY } from "./rbac";

type AuthUser = {
  id?: string;
  tenant_id?: string;
  name?: string;
  email?: string;
  role?: string;
  roles?: string[];
  [key: string]: unknown;
};

interface StoredSession {
  token: string;
  user: AuthUser;
  timestamp: number;
}

interface AuthContextValue {
  isLoggedIn: boolean;
  currentUser: AuthUser | null;
  roles: readonly string[];
  primaryRole: string | null;
  token: string | null;
  login: (
    username: string,
    password: string,
    mfaCode?: string,
  ) => Promise<boolean>;
  logout: () => void;
  hasAnyRole: (allowed: readonly string[]) => boolean;
  loginError: string | null;
  isAuthenticating: boolean;
  mfaRequired: boolean;
  mfaEnrollmentRequired: boolean;
  clearLoginError: () => void;
}

const resolveBaseUrl = (raw: string | undefined, fallback: string): string => {
  const candidate =
    typeof raw === "string" && raw.trim().length > 0 ? raw : fallback;
  return candidate.replace(/\/$/, "");
};

type EnvRecord = Record<string, string | undefined>;
const { VITE_AUTH_SERVICE_URL } = (import.meta.env ?? {}) as EnvRecord;
const AUTH_SERVICE_URL = resolveBaseUrl(
  VITE_AUTH_SERVICE_URL,
  "http://localhost:8085",
);
const LOGIN_ENDPOINT = `${AUTH_SERVICE_URL}/login`;
const SESSION_ENDPOINT = `${AUTH_SERVICE_URL}/session`;
const LOGOUT_ENDPOINT = `${AUTH_SERVICE_URL}/logout`;

const AuthContext = createContext<AuthContextValue | undefined>(undefined);

const isAuthUser = (value: unknown): value is AuthUser => {
  return typeof value === "object" && value !== null;
};

interface LoginResponse {
  token: string;
  user: AuthUser;
}

interface LoginErrorPayload {
  code?: string;
  message?: string;
}

const isLoginResponse = (value: unknown): value is LoginResponse => {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.token === "string" && isAuthUser(candidate.user);
};

const extractRoles = (user: AuthUser | null): string[] => {
  if (!user) return [];
  const collected = new Set<string>();
  if (typeof user.role === "string") {
    const trimmed = user.role.trim();
    if (trimmed.length > 0) {
      collected.add(trimmed);
    }
  }
  if (Array.isArray(user.roles)) {
    for (const value of user.roles) {
      if (typeof value === "string") {
        const trimmed = value.trim();
        if (trimmed.length > 0) {
          collected.add(trimmed);
        }
      }
    }
  }
  return ensureRoleOrder(collected);
};

const hasAnyRole = (roles: string[], allowed: readonly string[]): boolean => {
  if (allowed.length === 0) {
    return roles.length > 0;
  }
  return roles.some((role) => allowed.includes(role));
};
export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const [session, setSession] = useState<StoredSession | null>(null);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [mfaRequired, setMfaRequired] = useState(false);
  const [mfaEnrollmentRequired, setMfaEnrollmentRequired] = useState(false);

  const resetMfaState = useCallback(() => {
    setMfaRequired(false);
    setMfaEnrollmentRequired(false);
  }, []);

  const clearLoginError = useCallback(() => {
    setLoginError(null);
    resetMfaState();
  }, [resetMfaState]);

  const applySession = useCallback((value: StoredSession | null) => {
    setSession(value);
  }, []);

  useEffect(() => {
    let cancelled = false;

    const hydrateSession = async () => {
      try {
        const response = await fetch(SESSION_ENDPOINT, {
          method: "GET",
          credentials: "include",
        });

        if (!response.ok) {
          if (!cancelled) {
            applySession(null);
            resetMfaState();
          }
          return;
        }

        const contentType = response.headers.get("content-type") ?? "";
        if (!contentType.includes("application/json")) {
          if (!cancelled) {
            applySession(null);
            resetMfaState();
          }
          return;
        }

        const data = (await response.json()) as unknown;
        if (!isLoginResponse(data)) {
          if (!cancelled) {
            applySession(null);
            resetMfaState();
          }
          return;
        }

        if (!cancelled) {
          applySession({
            token: data.token,
            user: data.user,
            timestamp: Date.now(),
          });
          setLoginError(null);
          resetMfaState();
        }
      } catch (err) {
        if (!cancelled) {
          console.warn("Unable to refresh session", err);
        }
      }
    };

    void hydrateSession();
    return () => {
      cancelled = true;
    };
  }, [applySession, resetMfaState, setLoginError]);

  const login = useCallback<AuthContextValue["login"]>(
    async (username: string, password: string, mfaCode?: string) => {
      setIsAuthenticating(true);
      setLoginError(null);

      try {
        const payload: Record<string, unknown> = {
          email: username,
          password,
        };

        if (typeof mfaCode === "string" && mfaCode.trim().length > 0) {
          payload.mfaCode = mfaCode.trim();
        }

        const response = await fetch(LOGIN_ENDPOINT, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          credentials: "include",
          body: JSON.stringify(payload),
        });

        const contentType = response.headers.get("content-type") ?? "";

        if (!response.ok) {
          let message = "Invalid credentials, please try again.";
          let errorPayload: LoginErrorPayload | null = null;

          if (contentType.includes("application/json")) {
            try {
              errorPayload = (await response.json()) as LoginErrorPayload;
            } catch (err) {
              console.warn("Unable to parse authentication error payload", err);
            }
          }

          const errorCode = errorPayload?.code;

          switch (errorCode) {
            case "MFA_REQUIRED":
              setMfaRequired(true);
              message =
                errorPayload?.message ??
                "Enter the 6-digit code from your authenticator app.";
              break;
            case "MFA_CODE_INVALID":
              setMfaRequired(true);
              message =
                errorPayload?.message ?? "Invalid MFA code. Please try again.";
              break;
            case "MFA_NOT_ENROLLED":
              setMfaEnrollmentRequired(true);
              message =
                errorPayload?.message ??
                "MFA enrollment is required before this account can sign in.";
              break;
            default:
              resetMfaState();
              break;
          }

          setLoginError(message);
          return false;
        }

        if (!contentType.includes("application/json")) {
          setLoginError("Invalid response from authentication service.");
          resetMfaState();
          return false;
        }

        const data = (await response.json()) as unknown;
        if (!isLoginResponse(data)) {
          setLoginError("Invalid response from authentication service.");
          resetMfaState();
          return false;
        }

        const newSession: StoredSession = {
          token: data.token,
          user: data.user,
          timestamp: Date.now(),
        };

        applySession(newSession);
        setLoginError(null);
        resetMfaState();
        return true;
      } catch (err) {
        console.error("Login failed", err);
        setLoginError("Unable to login. Please try again.");
        resetMfaState();
        return false;
      } finally {
        setIsAuthenticating(false);
      }
    },
    [applySession, resetMfaState],
  );

  const logout = useCallback(() => {
    applySession(null);
    setLoginError(null);
    resetMfaState();
    void fetch(LOGOUT_ENDPOINT, {
      method: "POST",
      credentials: "include",
    }).catch((err) => {
      console.warn("Failed to notify auth-service of logout", err);
    });
  }, [applySession, resetMfaState, setLoginError]);

  const roleList = useMemo(
    () => extractRoles(session?.user ?? null),
    [session],
  );
  const primaryRole = useMemo(() => {
    for (const rank of ROLE_PRIORITY) {
      if (roleList.includes(rank)) {
        return rank;
      }
    }
    return roleList.length > 0 ? roleList[0] : null;
  }, [roleList]);

  const hasAnyRoleFn = useCallback(
    (allowed: readonly string[]) => hasAnyRole(roleList, allowed),
    [roleList],
  );

  const contextValue: AuthContextValue = {
    isLoggedIn: Boolean(session),
    currentUser: session?.user ?? null,
    roles: roleList,
    primaryRole,
    token: session?.token ?? null,
    login,
    logout,
    hasAnyRole: hasAnyRoleFn,
    loginError,
    isAuthenticating,
    mfaRequired,
    mfaEnrollmentRequired,
    clearLoginError,
  };

  return (
    <AuthContext.Provider value={contextValue}>{children}</AuthContext.Provider>
  );
};

// eslint-disable-next-line react-refresh/only-export-components
export const useAuth = () => {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within an AuthProvider");
  return ctx;
};

interface RequireRolesProps {
  roles: readonly string[];
  children: React.ReactNode;
  fallbackPath?: string;
}

export const RequireAuth: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const { isLoggedIn } = useAuth();
  const location = useLocation();
  if (!isLoggedIn) {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }
  return <>{children}</>;
};

export const RequireRoles: React.FC<RequireRolesProps> = ({
  roles,
  fallbackPath = "/home",
  children,
}) => {
  const { isLoggedIn, hasAnyRole: hasRoleAccess } = useAuth();
  const location = useLocation();

  if (!isLoggedIn) {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }

  if (!hasRoleAccess(roles)) {
    return <Navigate to={fallbackPath} state={{ from: location }} replace />;
  }

  return <>{children}</>;
};
