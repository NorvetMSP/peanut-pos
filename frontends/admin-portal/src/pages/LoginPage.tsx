import React, { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';

const LoginPage: React.FC = () => {
  const { login, loginError, isAuthenticating, isLoggedIn, mfaRequired, mfaEnrollmentRequired } = useAuth();
  const navigate = useNavigate();
  const [credentials, setCredentials] = useState({ email: '', password: '' });
  const [mfaCode, setMfaCode] = useState('');
  const [localError, setLocalError] = useState<string | null>(null);

  useEffect(() => {
    if (isLoggedIn) {
      void navigate('/home', { replace: true });
    }
  }, [isLoggedIn, navigate]);

  useEffect(() => {
    if (!mfaRequired) {
      setMfaCode('');
    }
  }, [mfaRequired]);

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setLocalError(null);
    const success = await login(
      credentials.email,
      credentials.password,
      mfaRequired ? mfaCode : undefined,
    );
    if (success) {
      setMfaCode('');
      void navigate('/home', { replace: true });
    } else if (!loginError) {
      setLocalError('Login failed. Please try again.');
    }
  };

  const errorMessage = loginError ?? localError;
  const isSubmitDisabled =
    isAuthenticating ||
    credentials.email.trim().length === 0 ||
    credentials.password.trim().length === 0 ||
    (mfaRequired && mfaCode.trim().length === 0);

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-100 dark:bg-gray-900">
      <div className="w-full max-w-md bg-white dark:bg-gray-800 rounded-lg shadow-lg p-6">
        <h1 className="text-2xl font-bold mb-6 text-center text-gray-800 dark:text-gray-100">
          NovaPOS Admin Portal
        </h1>
        <form onSubmit={event => { void handleSubmit(event); }} className="space-y-4">
          <div>
            <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="email">Email</label>
            <input
              id="email"
              type="email"
              value={credentials.email}
              onChange={e => setCredentials({ ...credentials, email: e.target.value })}
              className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
              placeholder="admin@novapos.local"
              required
              disabled={isSubmitDisabled}
            />
          </div>
          <div>
            <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="password">Password</label>
            <input
              id="password"
              type="password"
              value={credentials.password}
              onChange={e => setCredentials({ ...credentials, password: e.target.value })}
              className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
              placeholder="Enter password"
              required
              disabled={isSubmitDisabled}
            />
          </div>
          {mfaRequired && (
            <div>
              <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="mfaCode">MFA Code</label>
              <input
                id="mfaCode"
                type="text"
                inputMode="numeric"
                pattern="\\d*"
                maxLength={6}
                value={mfaCode}
                onChange={e => setMfaCode(e.target.value)}
                className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
                placeholder="Enter 6-digit code"
                autoComplete="one-time-code"
                disabled={isAuthenticating}
              />
              <p className="text-xs text-gray-500 mt-1">Enter the 6-digit code from your authenticator app.</p>
            </div>
          )}
          {mfaEnrollmentRequired && (
            <p className="text-xs text-blue-600 text-center">MFA enrollment is required for this account. Follow the security runbook to enroll before continuing.</p>
          )}
          {errorMessage && <p className="text-red-500 text-center text-sm">{errorMessage}</p>}
          <button
            type="submit"
            disabled={isSubmitDisabled}
            className="w-full py-2 px-4 rounded-md text-white"
            style={{
              background: isSubmitDisabled ? '#9ca3af' : '#19b4b9',
              cursor: isSubmitDisabled ? 'not-allowed' : 'pointer',
            }}
            onMouseOver={e => {
              if (!isSubmitDisabled) e.currentTarget.style.background = '#153a5b';
            }}
            onMouseOut={e => {
              e.currentTarget.style.background = isSubmitDisabled ? '#9ca3af' : '#19b4b9';
            }}
          >
            {isSubmitDisabled ? 'Logging in...' : 'Log In'}
          </button>
        </form>
      </div>
    </div>
  );
};

export default LoginPage;
