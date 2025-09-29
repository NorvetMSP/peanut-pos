import { Routes, Route, Navigate } from "react-router-dom";
import LoginPage from "./pages/LoginPage";
import DashboardPage from "./pages/DashboardPage";
import ProductListPage from "./pages/ProductListPage";
import UsersPage from "./pages/UsersPage";
import SettingsPage from "./pages/SettingsPage";
import OrdersPage from "./pages/OrdersPage";
import ReturnsPage from "./pages/ReturnsPage";
import AdminHome from "./pages/AdminHome";
import { RequireAuth, RequireRoles } from "./AuthContext";
import { MANAGER_ROLES, ADMIN_ROLES, SUPER_ADMIN_ROLES } from "./rbac";

const PROTECTED_ROUTES = [
  { path: "/home", element: <AdminHome /> },
  { path: "/dashboard", element: <DashboardPage />, roles: MANAGER_ROLES },
  { path: "/products", element: <ProductListPage />, roles: MANAGER_ROLES },
  { path: "/orders", element: <OrdersPage />, roles: MANAGER_ROLES },
  { path: "/returns", element: <ReturnsPage />, roles: MANAGER_ROLES },
  { path: "/users", element: <UsersPage />, roles: ADMIN_ROLES },
  { path: "/settings", element: <SettingsPage />, roles: SUPER_ADMIN_ROLES },
] as const;

function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      {PROTECTED_ROUTES.map(({ path, element, roles }) => (
        <Route
          key={path}
          path={path}
          element={
            roles ? (
              <RequireRoles roles={roles}>{element}</RequireRoles>
            ) : (
              <RequireAuth>{element}</RequireAuth>
            )
          }
        />
      ))}
      <Route path="*" element={<Navigate to="/home" replace />} />
    </Routes>
  );
}

export default App;
