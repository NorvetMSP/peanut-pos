import { Routes, Route } from 'react-router-dom';
import Login from './pages/Login';
import Sales from './pages/Sales';
import { RequireAuth } from './AuthContext';

function App() {
  return (
    <Routes>
      <Route path="/" element={<Login />} />
      <Route path="/sales" element={
        <RequireAuth>
          <Sales />
        </RequireAuth>
      } />
    </Routes>
  ); 
}
export default App
