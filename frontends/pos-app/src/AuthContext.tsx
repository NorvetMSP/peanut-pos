import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { Navigate, useLocation } from 'react-router-dom';

type AuthUser = {
  tenant_id?: string;
  role?: string;
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
  token: string | null;
  login: (username: string, password: string, mfaCode?: string) => Promise<boolean>;
  logout: () => void;
  loginError: string | null;
  isAuthenticating: boolean;
  requiresManager: boolean;
  lockedUntil: string | null;
  managerContactUri: string;
  clearLoginError: () => void;
  mfaRequired: boolean;
  mfaEnrollmentRequired: boolean;
}

const parseDuration = (raw: unknown, fallback: number): number => {
  if (typeof raw === 'string' && raw.trim().length > 0) {
    const parsed = Number(raw);
    if (Number.isFinite(parsed) && parsed > 0) return parsed;
  }
  if (typeof raw === 'number' && Number.isFinite(raw) && raw > 0) {
    return raw;
  }
  return fallback;
};

const SESSION_INACTIVITY_LIMIT_MS = parseDuration(
  import.meta.env.VITE_SESSION_TIMEOUT_MS,
  5 * 60 * 1000,
);
const AUTH_SERVICE_URL = (import.meta.env.VITE_AUTH_SERVICE_URL ?? 'http://localhost:3000').replace(/\/$/, '');
const LOGIN_ENDPOINT = `${AUTH_SERVICE_URL}/login`;
const SESSION_ENDPOINT = `${AUTH_SERVICE_URL}/session`;
const LOGOUT_ENDPOINT = `${AUTH_SERVICE_URL}/logout`;
const MANAGER_CONTACT_URI =
  import.meta.env.VITE_MANAGER_CONTACT_URI ??
  'mailto:manager@novapos.local?subject=NovaPOS%20Login%20Assistance';

const ACTIVITY_EVENTS: Array<keyof DocumentEventMap> = [
  'click',
  'keydown',
  'mousemove',
  'touchstart',
];

const AuthContext = createContext<AuthContextValue | undefined>(undefined);

const isBrowser = typeof window !== 'undefined';
const isOnline = () => (typeof navigator !== 'undefined' ? navigator.onLine : true);

interface LoginErrorPayload {
  code?: string;
  message?: string;
  locked_until?: string;
}

interface LoginResponse {
  token: string;
  user: AuthUser;
}

const isLoginResponse = (value: unknown): value is LoginResponse => {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.token === 'string' && typeof candidate.user === 'object' && candidate.user !== null;
};

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [session, setSession] = useState<StoredSession | null>(null);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [requiresManager, setRequiresManager] = useState(false);
  const [lockedUntil, setLockedUntil] = useState<string | null>(null);
  const [mfaRequired, setMfaRequired] = useState(false);
  const [mfaEnrollmentRequired, setMfaEnrollmentRequired] = useState(false);
  const sessionRef = useRef<StoredSession | null>(session);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  const applySession = useCallback((value: StoredSession | null) => {
    setSession(value);
  }, []);

  const clearLoginError = useCallback(() => {
    setLoginError(null);
    setRequiresManager(false);
    setLockedUntil(null);
  }, []);

  const resetMfaState = useCallback(() => {
    setMfaRequired(false);
    setMfaEnrollmentRequired(false);
  }, []);

  useEffect(() => {
    let cancelled = false;

    const hydrateSession = async () => {
      try {
        const response = await fetch(SESSION_ENDPOINT, {
          method: 'GET',
          credentials: 'include',
        });

        if (!response.ok) {
          if (!cancelled) {
            applySession(null);
            setLoginError(null);
            setRequiresManager(false);
            setLockedUntil(null);
            resetMfaState();
          }
          return;
        }

        const contentType = response.headers.get('content-type') ?? '';
        if (!contentType.includes('application/json')) {
          if (!cancelled) {
            applySession(null);
            setLoginError(null);
            setRequiresManager(false);
            setLockedUntil(null);
            resetMfaState();
          }
          return;
        }

        const data = (await response.json()) as unknown;
        if (!isLoginResponse(data)) {
          if (!cancelled) {
            applySession(null);
            setLoginError(null);
            setRequiresManager(false);
            setLockedUntil(null);
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
          setRequiresManager(false);
          setLockedUntil(null);
          resetMfaState();
        }
      } catch (err) {
        if (!cancelled) {
          console.warn('Unable to refresh session', err);
        }
      }
    };

    hydrateSession();

    return () => {
      cancelled = true;
    };
  }, [applySession, resetMfaState, setLoginError, setRequiresManager, setLockedUntil]);

  useEffect(() => {
    if (!isBrowser || !session) return;

    let timeoutId: ReturnType<typeof window.setTimeout> | undefined;

    const heartbeat = () => {
      if (!sessionRef.current) return;
      const updated: StoredSession = { ...sessionRef.current, timestamp: Date.now() };
      sessionRef.current = updated;
    };

    const handleActivity = () => {
      if (!sessionRef.current) return;
      if (timeoutId) {
        window.clearTimeout(timeoutId);
      }
      heartbeat();
      timeoutId = window.setTimeout(() => {
        sessionRef.current = null;
        applySession(null);
        setLoginError('Session expired due to inactivity.');
        setRequiresManager(false);
        setLockedUntil(null);
      }, SESSION_INACTIVITY_LIMIT_MS);
    };

    handleActivity();
    ACTIVITY_EVENTS.forEach(event => document.addEventListener(event, handleActivity, { passive: true }));

    return () => {
      if (timeoutId) {
        window.clearTimeout(timeoutId);
      }
      ACTIVITY_EVENTS.forEach(event => document.removeEventListener(event, handleActivity));
    };
  }, [applySession, session]);

  const login = useCallback<AuthContextValue['login']>(async (username: string, password: string, mfaCode?: string) => {
    setIsAuthenticating(true);
    clearLoginError();

    if (!isOnline()) {
      setIsAuthenticating(false);
      setRequiresManager(false);
      setLockedUntil(null);
      resetMfaState();
      setLoginError('Login requires a network connection. Please reconnect and try again.');
      return false;
    }

    try {
      const payload: Record<string, unknown> = {
        email: username,
        password,
      };
      if (typeof mfaCode === 'string' && mfaCode.trim().length > 0) {
        payload.mfaCode = mfaCode.trim();
      }

      const response = await fetch(LOGIN_ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify(payload),
      });

      if (!response.ok) {
        let friendlyMessage = 'Unable to login. Please try again.';
        let managerRequired = false;
        let lockTarget: string | null = null;
        let errorCode: string | undefined;

        const contentType = response.headers.get('content-type') ?? '';
        if (contentType.includes('application/json')) {
          try {
            const data: LoginErrorPayload = await response.json();
            if (typeof data.message === 'string' && data.message.trim().length > 0) {
              friendlyMessage = data.message;
            } else if (response.status === 401) {
              friendlyMessage = 'Invalid credentials, please try again.';
            }
            if (data.code === 'ACCOUNT_LOCKED') {
              managerRequired = true;
              lockTarget = typeof data.locked_until === 'string' ? data.locked_until : null;
            }
            errorCode = data.code;
          } catch (err) {
            console.warn('Unable to parse authentication error payload', err);
            if (response.status === 401) {
              friendlyMessage = 'Invalid credentials, please try again.';
            }
          }
        } else if (response.status === 401) {
          friendlyMessage = 'Invalid credentials, please try again.';
        }

        switch (errorCode) {
          case 'MFA_REQUIRED':
            setMfaRequired(true);
            friendlyMessage = friendlyMessage === 'Unable to login. Please try again.'
              ? 'Enter the 6-digit code from your authenticator app.'
              : friendlyMessage;
            break;
          case 'MFA_CODE_INVALID':
            setMfaRequired(true);
            friendlyMessage = 'Invalid MFA code. Please try again.';
            break;
          case 'MFA_NOT_ENROLLED':
            setMfaEnrollmentRequired(true);
            friendlyMessage = 'MFA enrollment is required before this account can sign in.';
            break;
          default:
            resetMfaState();
            break;
        }

        setLoginError(friendlyMessage);
        setRequiresManager(managerRequired);
        setLockedUntil(lockTarget);
        if (!['MFA_REQUIRED', 'MFA_CODE_INVALID', 'MFA_NOT_ENROLLED'].includes(errorCode ?? '')) {
          resetMfaState();
        }
        return false;
      }

      const responseBody = (await response.json()) as unknown;
      if (!isLoginResponse(responseBody)) {
        setLoginError('Invalid response from authentication service.');
        setRequiresManager(false);
        setLockedUntil(null);
        resetMfaState();
        return false;
      }
      const data = responseBody;

      const newSession: StoredSession = {
        token: data.token,
        user: data.user,
        timestamp: Date.now(),
      };

      applySession(newSession);
      setLoginError(null);
      setRequiresManager(false);
      setLockedUntil(null);
      resetMfaState();
      return true;
    } catch (err) {
      console.error('Login failed', err);
      setLoginError('Unable to login. Please try again.');
      setRequiresManager(false);
      setLockedUntil(null);
      resetMfaState();
      return false;
    } finally {
      setIsAuthenticating(false);
    }
  }, [applySession, clearLoginError, resetMfaState]);

  const logout = useCallback(() => {
    applySession(null);
    setLoginError(null);
    setRequiresManager(false);
    setLockedUntil(null);
    resetMfaState();
    void fetch(LOGOUT_ENDPOINT, {
      method: 'POST',
      credentials: 'include',
    }).catch((err) => {
      console.warn('Failed to notify auth-service of logout', err);
    });
  }, [applySession, resetMfaState, setLoginError, setRequiresManager, setLockedUntil]);

  const contextValue = useMemo<AuthContextValue>(
    () => ({
      isLoggedIn: Boolean(session),
      currentUser: session?.user ?? null,
      token: session?.token ?? null,
      login,
      logout,
      loginError,
      isAuthenticating,
      requiresManager,
      lockedUntil,
      managerContactUri: MANAGER_CONTACT_URI,
      clearLoginError,
      mfaRequired,
      mfaEnrollmentRequired,
    }),
    [session, login, logout, loginError, isAuthenticating, requiresManager, lockedUntil, clearLoginError, mfaRequired, mfaEnrollmentRequired],
  );

  return <AuthContext.Provider value={contextValue}>{children}</AuthContext.Provider>;
};

// eslint-disable-next-line react-refresh/only-export-components
export const useAuth = () => {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error('useAuth must be used within AuthProvider');
  return ctx;
};

export const RequireAuth: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { isLoggedIn } = useAuth();
  const location = useLocation();
  if (!isLoggedIn) {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }
  return children;
};







