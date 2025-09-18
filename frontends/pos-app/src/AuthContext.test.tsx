import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AuthProvider, useAuth } from './AuthContext';

type NavigatorOnLineDescriptor = PropertyDescriptor | undefined;

type FetchMock = ReturnType<typeof vi.fn>;

const originalNavigatorOnLine: NavigatorOnLineDescriptor = Object.getOwnPropertyDescriptor(window.navigator, 'onLine');
const originalFetch = globalThis.fetch;

const DEFAULT_TIMEOUT_MS = Number(import.meta.env.VITE_SESSION_TIMEOUT_MS ?? 5 * 60 * 1000);

let fetchMock: FetchMock;

const setNavigatorStatus = (value: boolean) => {
  Object.defineProperty(window.navigator, 'onLine', {
    configurable: true,
    get: () => value,
  });
};

const restoreNavigatorStatus = () => {
  if (originalNavigatorOnLine) {
    Object.defineProperty(window.navigator, 'onLine', originalNavigatorOnLine);
  } else {
    delete (window.navigator as { onLine?: boolean }).onLine;
  }
};

describe('AuthContext', () => {
  beforeEach(() => {
    vi.useRealTimers();
    localStorage.clear();
    setNavigatorStatus(true);
    fetchMock = vi.fn();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
  });

  afterEach(() => {
    localStorage.clear();
    if (originalFetch) {
      globalThis.fetch = originalFetch;
    } else {
      delete (globalThis as { fetch?: typeof fetch }).fetch;
    }
    restoreNavigatorStatus();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  describe('offline session handling', () => {
    it('revives a stored session when offline without contacting the API', async () => {
      const storedSession = {
        token: 'offline-token',
        user: { tenant_id: 'tenant-123', role: 'cashier' },
        timestamp: Date.now(),
      };
      localStorage.setItem('session', JSON.stringify(storedSession));
      setNavigatorStatus(false);

      const { result } = renderHook(() => useAuth(), { wrapper: AuthProvider });

      expect(result.current.isLoggedIn).toBe(true);
      expect(result.current.token).toBe(storedSession.token);
      expect(result.current.currentUser).toEqual(storedSession.user);

      let loginResult: boolean | undefined;
      await act(async () => {
        loginResult = await result.current.login('cached-user', 'cached-pass');
      });

      expect(loginResult).toBe(true);
      expect(result.current.loginError).toBeNull();
      expect(result.current.isLoggedIn).toBe(true);
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it('prevents offline login when no valid session is stored', async () => {
      setNavigatorStatus(false);

      const { result } = renderHook(() => useAuth(), { wrapper: AuthProvider });

      expect(result.current.isLoggedIn).toBe(false);

      let loginResult: boolean | undefined;
      await act(async () => {
        loginResult = await result.current.login('missing', 'session');
      });

      expect(loginResult).toBe(false);
      expect(result.current.isLoggedIn).toBe(false);
      expect(result.current.loginError).toBe('Login requires connection');
    });
  });

  it('flags manager escalation when the account is locked', async () => {
    const lockedUntil = new Date(Date.now() + 10_000).toISOString();
    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 423,
      headers: {
        get: (key: string) => (key.toLowerCase() === 'content-type' ? 'application/json' : null),
      },
      json: async () => ({
        code: 'ACCOUNT_LOCKED',
        message: 'This account is locked. Please contact a manager for assistance.',
        locked_until: lockedUntil,
      }),
    });

    const { result } = renderHook(() => useAuth(), { wrapper: AuthProvider });

    let loginResult: boolean | undefined;
    await act(async () => {
      loginResult = await result.current.login('locked@tenant.dev', 'password');
    });

    expect(loginResult).toBe(false);
    expect(result.current.requiresManager).toBe(true);
    expect(result.current.lockedUntil).toBe(lockedUntil);
    expect(result.current.loginError).toBe('This account is locked. Please contact a manager for assistance.');
  });

  it('expires the session after inactivity', async () => {
    vi.useFakeTimers();
    const storedSession = {
      token: 'active-token',
      user: { tenant_id: 'tenant-123', role: 'cashier' },
      timestamp: Date.now(),
    };
    localStorage.setItem('session', JSON.stringify(storedSession));

    const { result } = renderHook(() => useAuth(), { wrapper: AuthProvider });

    expect(result.current.isLoggedIn).toBe(true);

    await act(async () => {
      vi.advanceTimersByTime(DEFAULT_TIMEOUT_MS + 1);
    });

    expect(result.current.isLoggedIn).toBe(false);
    expect(result.current.loginError).toBe('Session expired due to inactivity.');
    expect(result.current.requiresManager).toBe(false);
    vi.useRealTimers();
  });
});
