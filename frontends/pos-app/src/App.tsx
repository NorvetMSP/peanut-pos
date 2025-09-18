import { Routes, Route, Navigate } from 'react-router-dom';
import Login from './pages/LoginPage';
import Sales from './pages/SalesPage';
import EcommerceTemplate from './pages/EcommerceTemplate';
import CheckoutPage from './pages/CheckoutPage';
import OrderHistoryPage from './pages/OrderHistoryPage';

function App() {
  return (
    <>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/sales" element={<Sales />} />
        <Route path="/cart" element={<EcommerceTemplate />} />
        <Route path="/checkout" element={<CheckoutPage />} />
        <Route path="/history" element={<OrderHistoryPage />} />
        <Route path="*" element={<Navigate to='/login' replace />} />
      </Routes>
    </>
  ); 
}
export default App
