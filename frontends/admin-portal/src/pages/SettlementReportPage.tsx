import React, { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { withRoleGuard } from '../components/RoleGuard';
import { MANAGER_ROLES } from '../rbac';
import { useAuth } from '../AuthContext';
import { parseReport, formatCurrency, type SettlementRow } from './settlementHelpers';

const ORDER_SERVICE_URL = (import.meta.env.VITE_ORDER_SERVICE_URL ?? 'http://localhost:8084').replace(/\/$/, '');

type Row = SettlementRow;

const SettlementReportPageContent: React.FC = () => {
  const { token, currentUser } = useAuth();
  const navigate = useNavigate();
  const [date, setDate] = useState<string>(() => new Date().toISOString().slice(0,10));
  const [rows, setRows] = useState<Row[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const tenantId = useMemo(() => currentUser?.tenant_id ? String(currentUser.tenant_id) : null, [currentUser]);

  useEffect(() => {
    const load = async () => {
      setLoading(true); setError(null);
      try {
        const params = new URLSearchParams({ date });
        const resp = await fetch(`${ORDER_SERVICE_URL}/reports/settlement?${params.toString()}`, {
          headers: {
            'Accept': 'application/json',
            ...(tenantId ? { 'X-Tenant-ID': tenantId } : {}),
            ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
          },
        });
        if (!resp.ok) throw new Error(`Failed to load report (${resp.status})`);
  const body = await resp.json();
  const parsed = parseReport(body);
  setRows(parsed.totals);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to load report');
      } finally {
        setLoading(false);
      }
    };
    void load();
  }, [date, tenantId, token]);

  return (
    <div className="admin-section-modern" style={{ marginTop: '2rem' }}>
      <div className="admin-section-header">
        <h2>Settlement Report</h2>
        <p>Totals by payment method</p>
      </div>
      <div className="admin-section-content">
        <label htmlFor="report-date">Date: </label>
        <input id="report-date" type="date" value={date} onChange={(e) => setDate(e.target.value)} />
        {loading && <p>Loading…</p>}
        {error && <p role="alert">{error}</p>}
        {!loading && !error && (
          <table>
            <thead>
              <tr><th>Method</th><th>Count</th><th>Amount</th></tr>
            </thead>
            <tbody>
              {rows.map(r => (
                <tr key={r.method}><td>{r.method}</td><td>{r.count}</td><td>{formatCurrency(r.amount)}</td></tr>
              ))}
              {rows.length === 0 && (
                <tr><td colSpan={3}>No data</td></tr>
              )}
            </tbody>
          </table>
        )}
      </div>
      {/* Navigation: provide a consistent way back to Admin Home, matching other pages */}
      <div style={{ textAlign: 'right', marginTop: '2rem' }}>
        <button
          className="admin-section-btn"
          onClick={() => void navigate('/home')}
          type="button"
        >
          Back to Admin Home
        </button>
      </div>
    </div>
  );
};

const SettlementReportPage = withRoleGuard(
  SettlementReportPageContent,
  MANAGER_ROLES,
  { title: 'Access Denied', message: 'You do not have permission to view settlement reports.' }
);
export default SettlementReportPage;
