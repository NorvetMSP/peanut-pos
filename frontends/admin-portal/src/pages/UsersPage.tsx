import React, { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import './AdminSectionModern.css';

const UsersPage: React.FC = () => {
  const { isLoggedIn, logout } = useAuth();
  const navigate = useNavigate();

  useEffect(() => {
    if (!isLoggedIn) navigate('/');
  }, [isLoggedIn, navigate]);

  // Dummy users data
  const users = [
    { username: 'manager1', role: 'Store Manager' },
    { username: 'cashier1', role: 'Cashier' },
    { username: 'admin', role: 'Admin' }
  ];

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col" style={{ fontFamily: 'Raleway, sans-serif', background: 'linear-gradient(135deg, #f8fafc 0%, #e6f7fa 100%)' }}>
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>User Management</h2>
          <p>View and manage user accounts and roles.</p>
        </div>
        <div className="admin-section-content">
          <div className="overflow-x-auto max-w-md mx-auto">
            <table className="min-w-full bg-white dark:bg-gray-800 rounded shadow">
              <thead className="bg-gray-200 dark:bg-gray-700">
                <tr>
                  <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Username</th>
                  <th className="text-left px-4 py-2 text-gray-800 dark:text-gray-100">Role</th>
                </tr>
              </thead>
              <tbody>
                {users.map((user, idx) => (
                  <tr key={idx} className="border-b border-gray-200 dark:border-gray-700">
                    <td className="px-4 py-2 text-gray-900 dark:text-gray-100">{user.username}</td>
                  <td className="px-4 py-2 text-gray-900 dark:text-gray-100">{user.role}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        </div>
        <div style={{ textAlign: 'right', marginTop: '2rem' }}>
          <button className="admin-section-btn" onClick={() => navigate('/home')}>Back to Admin Home</button>
        </div>
      </div>
    </div>
  );
};

export default UsersPage;
