import type { ComponentType } from "react";
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

type ProtectedRoute = {
  path: string;
  component: ComponentType;
  roles?: readonly string[];
};

const PROTECTED_ROUTES: readonly ProtectedRoute[] = [
  { path: "/home", component: AdminHome },
  { path: "/dashboard", component: DashboardPage, roles: MANAGER_ROLES },
  { path: "/products", component: ProductListPage, roles: MANAGER_ROLES },
  { path: "/orders", component: OrdersPage, roles: MANAGER_ROLES },
  { path: "/returns", component: ReturnsPage, roles: MANAGER_ROLES },
  { path: "/users", component: UsersPage, roles: ADMIN_ROLES },
  { path: "/settings", component: SettingsPage, roles: SUPER_ADMIN_ROLES },
] as const;

function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      {PROTECTED_ROUTES.map(({ path, component, roles }) => {
        const Element = component;
        return (
          <Route
            key={path}
            path={path}
            element={
              roles ? (
                <RequireRoles roles={roles}>
                  <Element />
                </RequireRoles>
              ) : (
                <RequireAuth>
                  <Element />
                </RequireAuth>
              )
            }
          />
        );
      })}
      <Route path="*" element={<Navigate to="/home" replace />} />
    </Routes>
  );
}

export default App;
