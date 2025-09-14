import { Routes, Route, Navigate } from 'react-router-dom';
import Login from './pages/LoginPage';
import Sales from './pages/SalesPage';

function App() {
  return (
    <>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/sales" element={<Sales />} />
        <Route path="*" element={<Navigate to='/login' replace />} />
      </Routes>
    </>
  ); 
}
export default App
