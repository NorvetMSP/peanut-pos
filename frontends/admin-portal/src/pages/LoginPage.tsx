import React, { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';

const LoginPage: React.FC = () => {
  const { login: authLogin } = useAuth();
  const navigate = useNavigate();
  const [credentials, setCredentials] = useState({ username: '', password: '' });
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const { username, password } = credentials;
    const success = authLogin(username, password);
    if (!success) {
      setError('Invalid credentials, please try again.');
    } else {
      setError(null);
      navigate('/home');  // go to home page on successful login
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-100 dark:bg-gray-900">
      <div className="w-full max-w-md bg-white dark:bg-gray-800 rounded-lg shadow-lg p-6">
        <h1 className="text-2xl font-bold mb-6 text-center text-gray-800 dark:text-gray-100">
          NovaPOS Admin Portal
        </h1>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="block text-gray-700 dark:text-gray-200 mb-1">Username</label>
            <input
              type="text"
              value={credentials.username}
              onChange={e => setCredentials({ ...credentials, username: e.target.value })}
              className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 
                         focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
              placeholder="Enter username"
              required
            />
          </div>
          <div>
            <label className="block text-gray-700 dark:text-gray-200 mb-1">Password</label>
            <input
              type="password"
              value={credentials.password}
              onChange={e => setCredentials({ ...credentials, password: e.target.value })}
              className="w-full px-3 py-2 border border-gray-300 rounded-md text-gray-900 
                         focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary"
              placeholder="Enter password"
              required
            />
          </div>
          {error && <p className="text-red-500 text-center">{error}</p>}
          <button
            type="submit"
            className="w-full py-2 px-4 rounded-md text-white"
            style={{ background: '#19b4b9' }}
            onMouseOver={e => (e.currentTarget.style.background = '#153a5b')}
            onMouseOut={e => (e.currentTarget.style.background = '#19b4b9')}
          >
            Log In
          </button>
        </form>
      </div>
    </div>
  );
};

export default LoginPage;
