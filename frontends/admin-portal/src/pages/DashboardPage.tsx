import React, { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../AuthContext';
import './AdminSectionModern.css';
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts';

const data = [
  { month: 'Jan', sales: 30 },
  { month: 'Feb', sales: 45 },
  { month: 'Mar', sales: 28 },
  { month: 'Apr', sales: 60 },
  { month: 'May', sales: 50 },
  { month: 'Jun', sales: 75 }
];

const DashboardPage: React.FC = () => {
  const { isLoggedIn, logout } = useAuth();
  const navigate = useNavigate();

  // Redirect to login if not authenticated
  useEffect(() => {
    if (!isLoggedIn) navigate('/');  // redirect to login (root leads to /login)
  }, [isLoggedIn, navigate]);

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col">
      <div className="admin-section-modern">
        <div className="admin-section-header">
          <h2>Dashboard</h2>
          <p>View analytics, sales, and quick stats.</p>
        </div>
        <div className="admin-section-content">
          <div className="w-full max-w-xl mx-auto">
            <ResponsiveContainer width="100%" height={300}>
              <LineChart data={data} margin={{ top: 20, right: 20, bottom: 5, left: 0 }}>
                <CartesianGrid strokeDasharray="3 3" />
                <XAxis dataKey="month" />
                <YAxis />
                <Tooltip />
                <Legend />
                <Line type="monotone" dataKey="sales" stroke="#8884d8" strokeWidth={2} activeDot={{ r: 8 }} />
              </LineChart>
            </ResponsiveContainer>
          </div>
          <div style={{ textAlign: 'right', marginTop: '2rem' }}>
            <button className="admin-section-btn" onClick={() => navigate('/home')}>Back to Admin Home</button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default DashboardPage;
