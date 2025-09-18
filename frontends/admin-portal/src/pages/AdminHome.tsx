import React from 'react';
import { useNavigate, useLocation } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import './AdminHomeModern.css';

const AdminHome: React.FC = () => {
  const { isLoggedIn, logout } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();

  React.useEffect(() => {
    if (!isLoggedIn && location.pathname !== '/login') {
      void navigate('/login', { replace: true });
    }
  }, [isLoggedIn, navigate, location]);

  const handleNavigate = (path: string) => () => {
    void navigate(path);
  };

  const handleLogout = () => {
    logout();
    void navigate('/');
  };

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col">
      <div className="admin-home-modern">
        <div className="admin-home-header">
          <h1>NovaPOS Admin Portal</h1>
          <p>Welcome! Manage your store, products, users, and settings from one modern dashboard.</p>
        </div>
        <div className="admin-home-section">
          <div className="admin-home-card" onClick={handleNavigate('/dashboard')}>
            <div className="admin-home-card-title">Dashboard</div>
            <div className="admin-home-card-desc">View analytics, sales, and quick stats.</div>
            <button className="admin-home-card-btn" type="button">Go to Dashboard</button>
          </div>
          <div className="admin-home-card" onClick={handleNavigate('/products')}>
            <div className="admin-home-card-title">Products</div>
            <div className="admin-home-card-desc">Manage your product catalog and pricing.</div>
            <button className="admin-home-card-btn" type="button">Go to Products</button>
          </div>
          <div className="admin-home-card" onClick={handleNavigate('/users')}>
            <div className="admin-home-card-title">Users</div>
            <div className="admin-home-card-desc">View and manage user accounts and roles.</div>
            <button className="admin-home-card-btn" type="button">Go to Users</button>
          </div>
          <div className="admin-home-card" onClick={handleNavigate('/settings')}>
            <div className="admin-home-card-title">Settings</div>
            <div className="admin-home-card-desc">Configure store details and preferences.</div>
            <button className="admin-home-card-btn" type="button">Go to Settings</button>
          </div>
        </div>
        <div style={{ textAlign: 'right', padding: '1rem 2rem' }}>
          <button
            className="admin-home-card-btn"
            style={{ background: '#e53e3e' }}
            onClick={handleLogout}
            type="button"
          >
            Logout
          </button>
        </div>
      </div>
    </div>
  );
};

export default AdminHome;
