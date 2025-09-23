import React, { createContext, useCallback, useContext, useState } from 'react';
import { Navigate, useLocation } from 'react-router-dom';

type AuthUser = {
  id?: string;
  tenant_id?: string;
  name?: string;
  email?: string;
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
  mfaRequired: boolean;
  mfaEnrollmentRequired: boolean;
  clearLoginError: () => void;
}

const SESSION_STORAGE_KEY = 'admin-portal-session';
const SESSION_MAX_AGE_MS = 8 * 60 * 60 * 1000;

const resolveBaseUrl = (raw: string | undefined, fallback: string): string => {
  const candidate = typeof raw === 'string' && raw.trim().length > 0 ? raw : fallback;
  return candidate.replace(/\/$/, '');
};

type EnvRecord = Record<string, string | undefined>;
const { VITE_AUTH_SERVICE_URL } = (import.meta.env ?? {}) as EnvRecord;
const AUTH_SERVICE_URL = resolveBaseUrl(VITE_AUTH_SERVICE_URL, 'http://localhost:8085');
const LOGIN_ENDPOINT = `${AUTH_SERVICE_URL}/login`;

const AuthContext = createContext<AuthContextValue | undefined>(undefined);

const isBrowser = typeof window !== 'undefined';
const getStorage = (): Storage | null => (isBrowser ? window.localStorage : null);

const isAuthUser = (value: unknown): value is AuthUser => {
  return typeof value === 'object' && value !== null;
};

const isStoredSessionRecord = (value: unknown): value is StoredSession => {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.token === 'string' &&
    typeof candidate.timestamp === 'number' &&
    isAuthUser(candidate.user)
  );
};

const isSessionFresh = (session: StoredSession | null): session is StoredSession => {
  if (!session) return false;
  if (!session.token || typeof session.timestamp !== 'number') return false;
  const age = Date.now() - session.timestamp;
  return age < SESSION_MAX_AGE_MS;
};

const parseStoredSession = (raw: string): StoredSession | null => {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (isStoredSessionRecord(parsed) && isSessionFresh(parsed)) {
      return parsed;
    }
  } catch (err) {
    console.warn('Unable to parse stored session', err);
  }
  return null;
};

const readStoredSession = (): StoredSession | null => {
  const storage = getStorage();
  if (!storage) return null;
  const raw = storage.getItem(SESSION_STORAGE_KEY);
  if (!raw) return null;
  const parsed = parseStoredSession(raw);
  if (!parsed) {
    storage.removeItem(SESSION_STORAGE_KEY);
  }
  return parsed;
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

interface LoginResponse {
  token: string;
  user: AuthUser;
}

interface LoginErrorPayload {
  code?: string;
  message?: string;
}

const isLoginResponse = (value: unknown): value is LoginResponse => {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.token === 'string' && isAuthUser(candidate.user);
};

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [session, setSession] = useState<StoredSession | null>(() => readStoredSession());
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
    persistSession(value);
  }, []);

  const login = useCallback<AuthContextValue['login']>(
    async (username: string, password: string, mfaCode?: string) => {
      setIsAuthenticating(true);
      setLoginError(null);

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
          body: JSON.stringify(payload),
        });

        const contentType = response.headers.get('content-type') ?? '';

        if (!response.ok) {
          let message = 'Invalid credentials, please try again.';
          let errorPayload: LoginErrorPayload | null = null;

          if (contentType.includes('application/json')) {
            try {
              errorPayload = (await response.json()) as LoginErrorPayload;
            } catch (err) {
              console.warn('Unable to parse authentication error payload', err);
            }
          }

          const errorCode = errorPayload?.code;

          switch (errorCode) {
            case 'MFA_REQUIRED':
              setMfaRequired(true);
              message = errorPayload?.message ?? 'Enter the 6-digit code from your authenticator app.';
              break;
            case 'MFA_CODE_INVALID':
              setMfaRequired(true);
              message = errorPayload?.message ?? 'Invalid MFA code. Please try again.';
              break;
            case 'MFA_NOT_ENROLLED':
              setMfaEnrollmentRequired(true);
              message = errorPayload?.message ?? 'MFA enrollment is required before this account can sign in.';
              break;
            default:
              resetMfaState();
              break;
          }

          setLoginError(message);
          return false;
        }

        if (!contentType.includes('application/json')) {
          setLoginError('Invalid response from authentication service.');
          resetMfaState();
          return false;
        }

        const data = (await response.json()) as unknown;
        if (!isLoginResponse(data)) {
          setLoginError('Invalid response from authentication service.');
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
        console.error('Login failed', err);
        setLoginError('Unable to login. Please try again.');
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
  }, [applySession, resetMfaState]);

  const contextValue: AuthContextValue = {
    isLoggedIn: Boolean(session),
    currentUser: session?.user ?? null,
    token: session?.token ?? null,
    login,
    logout,
    loginError,
    isAuthenticating,
    mfaRequired,
    mfaEnrollmentRequired,
    clearLoginError,
  };

  return <AuthContext.Provider value={contextValue}>{children}</AuthContext.Provider>;
};

// eslint-disable-next-line react-refresh/only-export-components
export const useAuth = () => {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error('useAuth must be used within an AuthProvider');
  return ctx;
};

export const RequireAuth: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { isLoggedIn } = useAuth();
  const location = useLocation();
  if (!isLoggedIn) {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }
  return <>{children}</>;
};

