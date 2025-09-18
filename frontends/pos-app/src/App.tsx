import { Routes, Route, Navigate } from 'react-router-dom';
import Login from './pages/LoginPage';
import Sales from './pages/SalesPage';
import EcommerceTemplate from './pages/EcommerceTemplate';
import CheckoutPage from './pages/CheckoutPage';
import OrderHistoryPage from './pages/OrderHistoryPage';
import { RequireAuth } from './AuthContext';

function App() {
  return (
    <>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route
          path="/sales"
          element={(
            <RequireAuth>
              <Sales />
            </RequireAuth>
          )}
        />
        <Route
          path="/cart"
          element={(
            <RequireAuth>
              <EcommerceTemplate />
            </RequireAuth>
          )}
        />
        <Route
          path="/checkout"
          element={(
            <RequireAuth>
              <CheckoutPage />
            </RequireAuth>
          )}
        />
        <Route
          path="/history"
          element={(
            <RequireAuth>
              <OrderHistoryPage />
            </RequireAuth>
          )}
        />
        <Route path="*" element={<Navigate to="/login" replace />} />
      </Routes>
    </>
  );
}

export default App;
