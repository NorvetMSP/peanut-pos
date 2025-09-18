import React, { createContext, useCallback, useContext, useState } from 'react';
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
}

const SESSION_STORAGE_KEY = 'admin-portal-session';
const SESSION_MAX_AGE_MS = 8 * 60 * 60 * 1000;
const AUTH_SERVICE_URL = (import.meta.env.VITE_AUTH_SERVICE_URL ?? 'http://localhost:8085').replace(/\/$/, '');
const LOGIN_ENDPOINT = `${AUTH_SERVICE_URL}/login`;

const AuthContext = createContext<AuthContextValue | undefined>(undefined);

const isBrowser = typeof window !== 'undefined';
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

interface LoginResponse {
  token: string;
  user: AuthUser;
}

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [session, setSession] = useState<StoredSession | null>(() => readStoredSession());
  const [loginError, setLoginError] = useState<string | null>(null);
  const [isAuthenticating, setIsAuthenticating] = useState(false);

  const applySession = useCallback((value: StoredSession | null) => {
    setSession(value);
    persistSession(value);
  }, []);

  const login = useCallback<AuthContextValue['login']>(async (username, password) => {
    setIsAuthenticating(true);
    setLoginError(null);

    try {
      const response = await fetch(LOGIN_ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: username, password }),
      });

      if (!response.ok) {
        setLoginError('Invalid credentials, please try again.');
        return false;
      }

      const data: LoginResponse = await response.json();
      if (!data?.token || !data?.user) {
        setLoginError('Invalid response from authentication service.');
        return false;
      }

      const newSession: StoredSession = {
        token: data.token,
        user: data.user,
        timestamp: Date.now(),
      };

      applySession(newSession);
      setLoginError(null);
      return true;
    } catch (err) {
      console.error('Login failed', err);
      setLoginError('Unable to login. Please try again.');
      return false;
    } finally {
      setIsAuthenticating(false);
    }
  }, [applySession]);

  const logout = useCallback(() => {
    applySession(null);
    setLoginError(null);
  }, [applySession]);

  const contextValue: AuthContextValue = {
    isLoggedIn: Boolean(session),
    currentUser: session?.user ?? null,
    token: session?.token ?? null,
    login,
    logout,
    loginError,
    isAuthenticating,
  };

  return <AuthContext.Provider value={contextValue}>{children}</AuthContext.Provider>;
};

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
