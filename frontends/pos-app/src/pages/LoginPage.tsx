import React, { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';

const LoginPage: React.FC = () => {
  const {
    login,
    loginError,
    isAuthenticating,
    isLoggedIn,
    requiresManager,
    managerContactUri,
    clearLoginError,
    lockedUntil,
  } = useAuth();
  const navigate = useNavigate();
  const [credentials, setCredentials] = useState({ username: '', password: '' });

  useEffect(() => {
    if (isLoggedIn) {
      navigate('/sales', { replace: true });
    }
  }, [isLoggedIn, navigate]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const { username, password } = credentials;
    await login(username, password);
  };

  const handleContactManager = useCallback(() => {
    if (!managerContactUri) return;
    if (managerContactUri.startsWith('mailto:') || managerContactUri.startsWith('tel:')) {
      window.location.href = managerContactUri;
      return;
    }
    window.open(managerContactUri, '_blank', 'noopener');
  }, [managerContactUri]);

  const handleFieldChange = (field: 'username' | 'password') => (event: React.ChangeEvent<HTMLInputElement>) => {
    setCredentials(prev => ({ ...prev, [field]: event.target.value }));
    if (loginError) {
      clearLoginError();
    }
  };

  const isSubmitDisabled = isAuthenticating || !credentials.username || !credentials.password;

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-100 dark:bg-gray-900">
      <div className="w-full max-w-md bg-white dark:bg-gray-800 rounded-lg shadow-lg p-6">
        <h1 className="text-2xl font-bold mb-6 text-center text-gray-800 dark:text-gray-100">Welcome to NovaPOS</h1>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="username">
              Username
            </label>
            <input
              id="username"
              type="text"
              value={credentials.username}
              onChange={handleFieldChange('username')}
              className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
              placeholder="Enter username"
              required
              autoComplete="username"
            />
          </div>
          <div>
            <label className="block text-gray-700 dark:text-gray-200 mb-1" htmlFor="password">
              Password
            </label>
            <input
              id="password"
              type="password"
              value={credentials.password}
              onChange={handleFieldChange('password')}
              className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
              placeholder="Enter password"
              required
              autoComplete="current-password"
            />
          </div>
          {loginError && (
            <div className="text-center space-y-2">
              <p className="text-sm text-red-500">{loginError}</p>
              {lockedUntil && (
                <p className="text-xs text-gray-500">
                  Locked until <span className="font-semibold">{new Date(lockedUntil).toLocaleString()}</span>
                </p>
              )}
              {requiresManager && (
                <button
                  type="button"
                  onClick={handleContactManager}
                  className="w-full py-2 px-4 rounded-md bg-indigo-600 hover:bg-indigo-700 text-white transition-colors"
                >
                  Contact Manager
                </button>
              )}
            </div>
          )}
          <button
            type="submit"
            disabled={isSubmitDisabled}
            className="w-full py-2 px-4 rounded-md text-white"
            style={{
              background: isSubmitDisabled ? '#9ca3af' : '#19b4b9',
              cursor: isSubmitDisabled ? 'not-allowed' : 'pointer',
            }}
            onMouseOver={e => {
              if (!isSubmitDisabled) {
                e.currentTarget.style.background = '#153a5b';
              }
            }}
            onMouseOut={e => {
              e.currentTarget.style.background = isSubmitDisabled ? '#9ca3af' : '#19b4b9';
            }}
          >
            {isAuthenticating ? 'Logging in...' : 'Log In'}
          </button>
        </form>
      </div>
    </div>
  );
};

export default LoginPage;
