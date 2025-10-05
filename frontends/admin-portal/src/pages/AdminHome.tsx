import React from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { useAuth } from "../AuthContext";
import { useHasAnyRole } from "../hooks/useRoleAccess";
import {
  ADMIN_ROLES,
  MANAGER_ROLES,
  SUPER_ADMIN_ROLES,
  roleLabel,
} from "../rbac";
import "./AdminHomeModern.css";

const AdminHome: React.FC = () => {
  const { isLoggedIn, logout, currentUser, primaryRole } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();

  const canManageCatalog = useHasAnyRole(MANAGER_ROLES);
  const canManageOrders = useHasAnyRole(MANAGER_ROLES);
  const canManageCustomers = useHasAnyRole(MANAGER_ROLES);
  const canManageUsers = useHasAnyRole(ADMIN_ROLES);
  const canManageSettings = useHasAnyRole(SUPER_ADMIN_ROLES);
  const canViewReports = useHasAnyRole(MANAGER_ROLES);

  React.useEffect(() => {
    if (!isLoggedIn && location.pathname !== "/login") {
      void navigate("/login", { replace: true });
    }
  }, [isLoggedIn, navigate, location]);

  const handleNavigate = (path: string) => () => {
    void navigate(path);
  };

  const handleLogout = () => {
    logout();
    void navigate("/");
  };

  const roleDisplay = primaryRole ? roleLabel(primaryRole) : null;
  const userName = currentUser?.name ?? currentUser?.email ?? "Administrator";

  return (
    <div className="min-h-screen bg-gray-100 dark:bg-gray-900 flex flex-col">
      <div className="admin-home-modern">
        <div className="admin-home-header">
          <h1>NovaPOS Admin Portal</h1>
          <p>
            Welcome, {userName}! Manage your store, products, users, and
            settings from one modern dashboard.
          </p>
          {roleDisplay && (
            <p
              style={{ marginTop: "0.5rem", color: "#19b4b9", fontWeight: 500 }}
            >
              Current role: {roleDisplay}
            </p>
          )}
        </div>
        <div className="admin-home-section">
          <div
            className="admin-home-card"
            onClick={handleNavigate("/dashboard")}
          >
            <div className="admin-home-card-title">Dashboard</div>
            <div className="admin-home-card-desc">
              View analytics, sales, and quick stats.
            </div>
            <button className="admin-home-card-btn" type="button">
              Go to Dashboard
            </button>
          </div>
          {canManageCatalog && (
            <div
              className="admin-home-card"
              onClick={handleNavigate("/products")}
            >
              <div className="admin-home-card-title">Products</div>
              <div className="admin-home-card-desc">
                Manage your product catalog and pricing.
              </div>
              <button className="admin-home-card-btn" type="button">
                Go to Products
              </button>
            </div>
          )}
          {canManageOrders && (
            <div
              className="admin-home-card"
              onClick={handleNavigate("/orders")}
            >
              <div className="admin-home-card-title">Orders</div>
              <div className="admin-home-card-desc">
                Search orders, receipts, and returns.
              </div>
              <button className="admin-home-card-btn" type="button">
                Go to Orders
              </button>
            </div>
          )}
          {canViewReports && (
            <div
              className="admin-home-card"
              onClick={handleNavigate("/reports/settlement")}
            >
              <div className="admin-home-card-title">Reports</div>
              <div className="admin-home-card-desc">
                View daily settlement totals by payment method.
              </div>
              <button className="admin-home-card-btn" type="button">
                Go to Reports
              </button>
            </div>
          )}
          {canManageCustomers && (
            <div
              className="admin-home-card"
              onClick={handleNavigate("/customers")}
            >
              <div className="admin-home-card-title">Customers</div>
              <div className="admin-home-card-desc">
                Look up profiles and keep customer info up to date.
              </div>
              <button className="admin-home-card-btn" type="button">
                Go to Customers
              </button>
            </div>
          )}
          {canManageUsers && (
            <div className="admin-home-card" onClick={handleNavigate("/users")}>
              <div className="admin-home-card-title">Users</div>
              <div className="admin-home-card-desc">
                View and manage user accounts and roles.
              </div>
              <button className="admin-home-card-btn" type="button">
                Go to Users
              </button>
            </div>
          )}
          {canManageSettings && (
            <div
              className="admin-home-card"
              onClick={handleNavigate("/settings")}
            >
              <div className="admin-home-card-title">Settings</div>
              <div className="admin-home-card-desc">
                Configure store details and preferences.
              </div>
              <button className="admin-home-card-btn" type="button">
                Go to Settings
              </button>
            </div>
          )}
        </div>
        <div style={{ textAlign: "right", padding: "1rem 2rem" }}>
          <button
            className="admin-home-card-btn"
            style={{ background: "#e53e3e" }}
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
