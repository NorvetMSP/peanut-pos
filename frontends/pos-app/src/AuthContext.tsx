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
  login: (username: string, password: string) => Promise<boolean>;
  logout: () => void;
  loginError: string | null;
  isAuthenticating: boolean;
  requiresManager: boolean;
  lockedUntil: string | null;
  managerContactUri: string;
  clearLoginError: () => void;
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
const SESSION_MAX_AGE_MS = parseDuration(
  import.meta.env.VITE_SESSION_MAX_AGE_MS,
  SESSION_INACTIVITY_LIMIT_MS,
);
const AUTH_SERVICE_URL = (import.meta.env.VITE_AUTH_SERVICE_URL ?? 'http://localhost:3000').replace(/\/$/, '');
const LOGIN_ENDPOINT = `${AUTH_SERVICE_URL}/login`;
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
const SESSION_STORAGE_KEY = 'session';
const getStorage = () => (isBrowser ? window.localStorage : null);

const isSessionFresh = (session: StoredSession | null): session is StoredSession => {
  if (!session) return false;
  if (!session.token || typeof session.timestamp !== 'number') return false;
  const age = Date.now() - session.timestamp;
  return age < SESSION_MAX_AGE_MS;
};

const readStoredSession = (): StoredSession | null => {
  const storage = getStorage();
  if (!storage) return null;
  const raw = storage.getItem(SESSION_STORAGE_KEY);
  if (!raw) return null;
  try {
    const parsed: StoredSession = JSON.parse(raw);
    if (isSessionFresh(parsed)) {
      return parsed;
    }
  } catch (err) {
    console.warn('Unable to parse stored session', err);
  }
  storage.removeItem(SESSION_STORAGE_KEY);
  return null;
};

const persistSession = (session: StoredSession | null) => {
  const storage = getStorage();
  if (!storage) return;
  if (!session) {
    storage.removeItem(SESSION_STORAGE_KEY);
    return;
  }
  storage.setItem(SESSION_STORAGE_KEY, JSON.stringify(session));
};

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

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [session, setSession] = useState<StoredSession | null>(() => readStoredSession());
  const [loginError, setLoginError] = useState<string | null>(null);
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [requiresManager, setRequiresManager] = useState(false);
  const [lockedUntil, setLockedUntil] = useState<string | null>(null);
  const sessionRef = useRef<StoredSession | null>(session);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  const applySession = useCallback((value: StoredSession | null) => {
    setSession(value);
    persistSession(value);
  }, []);

  const clearLoginError = useCallback(() => {
    setLoginError(null);
    setRequiresManager(false);
    setLockedUntil(null);
  }, []);

  useEffect(() => {
    if (!isBrowser || !session) return;

    let timeoutId: ReturnType<typeof window.setTimeout> | undefined;

    const heartbeat = () => {
      if (!sessionRef.current) return;
      const updated: StoredSession = { ...sessionRef.current, timestamp: Date.now() };
      sessionRef.current = updated;
      persistSession(updated);
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

  const login = useCallback<AuthContextValue['login']>(async (username, password) => {
    setIsAuthenticating(true);
    clearLoginError();

    if (!isOnline()) {
      const storedSession = readStoredSession();
      if (storedSession) {
        applySession(storedSession);
        setIsAuthenticating(false);
        return true;
      }
      setLoginError('Login requires connection');
      setIsAuthenticating(false);
      return false;
    }

    try {
      const response = await fetch(LOGIN_ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: username, password }),
      });

      if (!response.ok) {
        let friendlyMessage = 'Unable to login. Please try again.';
        let managerRequired = false;
        let lockTarget: string | null = null;

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
          } catch (err) {
            console.warn('Unable to parse authentication error payload', err);
          }
        } else if (response.status === 401) {
          friendlyMessage = 'Invalid credentials, please try again.';
        }

        setLoginError(friendlyMessage);
        setRequiresManager(managerRequired);
        setLockedUntil(lockTarget);
        return false;
      }

      const data: LoginResponse = await response.json();
      if (!data?.token || !data?.user) {
        setLoginError('Invalid response from authentication service.');
        setRequiresManager(false);
        setLockedUntil(null);
        return false;
      }

      const newSession: StoredSession = {
        token: data.token,
        user: data.user,
        timestamp: Date.now(),
      };

      applySession(newSession);
      setLoginError(null);
      setRequiresManager(false);
      setLockedUntil(null);
      return true;
    } catch (err) {
      console.error('Login failed', err);
      setLoginError('Unable to login. Please try again.');
      setRequiresManager(false);
      setLockedUntil(null);
      return false;
    } finally {
      setIsAuthenticating(false);
    }
  }, [applySession, clearLoginError]);

  const logout = useCallback(() => {
    applySession(null);
    setLoginError(null);
    setRequiresManager(false);
    setLockedUntil(null);
  }, [applySession]);

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
    }),
    [session, login, logout, loginError, isAuthenticating, requiresManager, lockedUntil, clearLoginError],
  );

  return <AuthContext.Provider value={contextValue}>{children}</AuthContext.Provider>;
};

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
