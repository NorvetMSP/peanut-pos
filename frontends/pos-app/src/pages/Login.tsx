// src/pages/Login.tsx
import React, { useState, useRef } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';

const Login: React.FC = () => {
  const auth = useAuth();
  const navigate = useNavigate();
  const usernameRef = useRef<HTMLInputElement>(null);
  const passwordRef = useRef<HTMLInputElement>(null);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const username = usernameRef.current?.value || '';
    const password = passwordRef.current?.value || '';
    const success = auth.login(username, password);
    if (!success) {
      setError('Invalid credentials, please try again.');
    } else {
      setError(null);
      navigate('/sales'); // go to Sales screen after successful login
    }
  };

  return (
    <div className="login-screen">
      <h2>Login to NovaPOS</h2>
      <form onSubmit={handleSubmit}>
        <div>
          <label>Username:</label>
          <input type="text" ref={usernameRef} required />
        </div>
        <div>
          <label>Password:</label>
          <input type="password" ref={passwordRef} required />
        </div>
        {error && <p className="error" style={{color:'red'}}>{error}</p>}
        <button type="submit">Login</button>
      </form>
    </div>
  );
};

export default Login;
