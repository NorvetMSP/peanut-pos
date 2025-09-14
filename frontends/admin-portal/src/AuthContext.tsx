import React, { createContext, useContext, useState } from 'react';
import { Navigate, useLocation } from 'react-router-dom';

interface AuthContextValue {
  isLoggedIn: boolean;
  login: (username: string, password: string) => boolean;
  logout: () => void;
}

// Create context
const AuthContext = createContext<AuthContextValue | undefined>(undefined);

// AuthProvider component to wrap the app
export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [isLoggedIn, setIsLoggedIn] = useState(false);

  // Dummy login: accept fixed credentials (e.g. admin/password)
  const login = (username: string, password: string) => {
    if (username === 'admin' && password === 'password') {
      setIsLoggedIn(true);
      return true;
    }
    return false;
  };

  const logout = () => {
    setIsLoggedIn(false);
  };

  return (
    <AuthContext.Provider value={{ isLoggedIn, login, logout }}>
      {children}
    </AuthContext.Provider>
  );
};

// Hook to use auth context
export const useAuth = () => {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error('useAuth must be used within an AuthProvider');
  return ctx;
};

// Protected route component (optional, not used in this simple setup)
export const RequireAuth: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { isLoggedIn } = useAuth();
  const location = useLocation();
  if (!isLoggedIn) {
    // If not authenticated, redirect to login
    return <Navigate to="/login" state={{ from: location }} replace />;
  }
  return <>{children}</>;
};
