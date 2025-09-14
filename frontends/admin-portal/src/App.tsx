import { Routes, Route, Navigate } from 'react-router-dom';
import LoginPage from './pages/LoginPage';
import DashboardPage from './pages/DashboardPage';
import ProductListPage from './pages/ProductListPage';
import UsersPage from './pages/UsersPage';
import SettingsPage from './pages/SettingsPage';
import AdminHome from './pages/AdminHome';

function App() {
  return (
    <Routes>
      {/* Public route: Login */}
      <Route path="/login" element={<LoginPage />} />

      {/* Home page */}
      <Route path="/home" element={<AdminHome />} />

      {/* Protected routes: only accessible after login */}
      <Route path="/dashboard" element={<DashboardPage />} />
      <Route path="/products" element={<ProductListPage />} />
      <Route path="/users" element={<UsersPage />} />
      <Route path="/settings" element={<SettingsPage />} />

      {/* Catch-all: redirect unknown paths to home */}
      <Route path="*" element={<Navigate to="/home" replace />} />
    </Routes>
  );
}

export default App;
