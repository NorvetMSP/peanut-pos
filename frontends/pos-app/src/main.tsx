import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App'
import { AuthProvider } from './AuthContext'
import { CartProvider } from './CartContext'
import { OrderProvider } from './OrderContext'
import { BrowserRouter } from 'react-router-dom'
import { enableConsoleTelemetry } from './services/telemetry'
import { startTelemetryScheduler } from './services/telemetryScheduler'
import { useAuth } from './AuthContext'
// Enable console telemetry if configured
if (import.meta.env.VITE_ENABLE_CONSOLE_TELEMETRY === 'true') {
  enableConsoleTelemetry(true)
}

function RootWithTelemetry() {
  // Don’t export this — just a small wrapper to access token
  const { token, currentUser } = useAuth();
  startTelemetryScheduler(
    () => (token ?? undefined),
    () => {
      const labels: Record<string, string> = {};
      const tid = (currentUser as any)?.tenant_id;
      if (typeof tid === 'string' && tid.trim().length > 0) labels.tenant_id = tid;
      const store = (currentUser as any)?.store_id || (currentUser as any)?.store || (currentUser as any)?.location;
      if (typeof store === 'string' && store.trim().length > 0) labels.store_id = store;
      return labels;
    }
  );
  return (
    <BrowserRouter>
      <App />
    </BrowserRouter>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AuthProvider>
      <OrderProvider>
        <CartProvider>
          <RootWithTelemetry />
        </CartProvider>
      </OrderProvider>
    </AuthProvider>
  </StrictMode>,
)
