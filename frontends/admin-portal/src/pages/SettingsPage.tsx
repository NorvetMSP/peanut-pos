import React, { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import './AdminSectionModern.css';

const SettingsPage: React.FC = () => {
  const { isLoggedIn, logout } = useAuth();
  const navigate = useNavigate();

  useEffect(() => {
    if (!isLoggedIn) navigate('/');
  }, [isLoggedIn, navigate]);

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col" style={{ fontFamily: 'Raleway, sans-serif', background: 'linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)' }}>
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>Settings</h2>
          <p>Configure store details and preferences.</p>
        </div>
        <div className="admin-section-content">
          <div className="max-w-sm bg-white dark:bg-gray-800 rounded shadow p-4 mx-auto">
            <p className="mb-2 text-gray-800 dark:text-gray-100"><strong>Store Name:</strong> Demo Store</p>
            <p className="mb-2 text-gray-800 dark:text-gray-100"><strong>Location:</strong> 123 Main St, Hometown</p>
            <p className="mb-2 text-gray-800 dark:text-gray-100"><strong>Contact:</strong> (123) 456-7890</p>
            <p className="text-gray-800 dark:text-gray-100"><strong>Currency:</strong> USD</p>
          </div>
        </div>
        <div style={{ textAlign: 'right', marginTop: '2rem' }}>
          <button className="admin-section-btn" onClick={() => navigate('/home')}>Back to Admin Home</button>
        </div>
      </div>
    </div>
  );
};

export default SettingsPage;
