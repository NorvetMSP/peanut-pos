import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { afterAll, afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { CustomersPageContent } from '../CustomersPage';

type FetchArgs = [RequestInfo | URL, RequestInit | undefined];

const fetchMock = vi.fn<FetchArgs, Promise<Response>>();

const getFetchCall = (index: number): FetchArgs | undefined => {
  const calls = fetchMock.mock.calls as FetchArgs[];
  return calls[index];
};

const getLastFetchCall = (): FetchArgs | undefined => {
  const calls = fetchMock.mock.calls as FetchArgs[];
  return calls[calls.length - 1];
};
const originalFetch = globalThis.fetch;

let authState: ReturnType<typeof createAuthState>;

const createAuthState = () => ({
  isLoggedIn: true,
  currentUser: {
    tenant_id: 'tenant-1',
    name: 'Dana Admin',
    email: 'dana@example.com',
  },
  token: 'token-123',
});

const hasAnyRoleMock = vi.fn<(roles: readonly string[]) => boolean>();

vi.mock('../../AuthContext', () => ({
  useAuth: () => authState,
}));

vi.mock('../../hooks/useRoleAccess', () => ({
  useHasAnyRole: (roles: readonly string[]) => hasAnyRoleMock(roles),
}));

const createJsonResponse = (payload: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
    ...init,
  });

const renderPage = () =>
  render(
    <MemoryRouter>
      <CustomersPageContent />
    </MemoryRouter>,
  );

beforeEach(() => {
  authState = createAuthState();
  hasAnyRoleMock.mockImplementation((roles) => !roles.includes('super_admin'));
  fetchMock.mockReset();
  globalThis.fetch = fetchMock as unknown as typeof fetch;
});

afterEach(() => {
  hasAnyRoleMock.mockReset();
});

afterAll(() => {
  globalThis.fetch = originalFetch;
});

describe('CustomersPageContent', () => {
  test('searches for customers and displays results', async () => {
    const searchResponse = createJsonResponse([
      {
        id: 'cust-1',
        name: 'Alice Smith',
        email: 'alice@example.com',
        phone: null,
        created_at: '2025-10-02T10:15:00.000Z',
      },
    ]);

    fetchMock.mockResolvedValueOnce(searchResponse);

    renderPage();

    const user = userEvent.setup();
    const searchField = screen.getByLabelText(/search customers/i);
    await user.type(searchField, 'Alice');
    await user.click(screen.getByRole('button', { name: /search/i }));

    await screen.findByText('Alice Smith');

    const firstCall = getFetchCall(0);
    expect(firstCall?.[0]).toBe('http://localhost:8089/customers?q=Alice');
    const firstInit = firstCall?.[1];
    const firstHeaders = new Headers((firstInit?.headers ?? {}));
    expect(firstHeaders.get('Authorization')).toBe('Bearer token-123');
    expect(firstHeaders.get('X-Tenant-ID')).toBe('tenant-1');
  });

  test('edits a customer and updates the table', async () => {
    const searchResponse = createJsonResponse([
      {
        id: 'cust-1',
        name: 'Alice Smith',
        email: 'alice@example.com',
        phone: null,
        created_at: '2025-10-02T10:15:00.000Z',
      },
    ]);
    const updateResponse = createJsonResponse({
      id: 'cust-1',
      name: 'Alice Johnson',
      email: 'alice@example.com',
      phone: '555-0001',
      created_at: '2025-10-02T10:15:00.000Z',
    });

    fetchMock
      .mockResolvedValueOnce(searchResponse)
      .mockResolvedValueOnce(updateResponse);

    renderPage();
    const user = userEvent.setup();

    await user.type(screen.getByLabelText(/search customers/i), 'Alice');
    await user.click(screen.getByRole('button', { name: /search/i }));
    await screen.findByText('Alice Smith');

    await user.click(screen.getByRole('button', { name: /edit/i }));

    const modalHeading = await screen.findByRole('heading', {
      name: /edit alice smith/i,
    });
    const modalElement = modalHeading.closest('[data-testid="modal-container"]');
    if (!(modalElement instanceof HTMLElement)) {
      throw new Error('Modal container not found');
    }
    const withinModal = within(modalElement);

    const nameField = withinModal.getByLabelText(/^Name$/i);
    await user.clear(nameField);
    await user.type(nameField, 'Alice Johnson');
    await user.type(withinModal.getByLabelText(/phone/i), '555-0001');

    await user.click(withinModal.getByRole('button', { name: /save changes/i }));

    await screen.findByText('Customer updated successfully.');
    expect(screen.getByText('Alice Johnson')).toBeInTheDocument();
    const updateCall = getFetchCall(1);
    expect(updateCall?.[0]).toBe('http://localhost:8089/customers/cust-1');
    const updateInit = updateCall?.[1];
    expect(updateInit?.method).toBe('PUT');
  });

  test('deletes a customer when confirmed', async () => {
    const searchResponse = createJsonResponse([
      {
        id: 'cust-1',
        name: 'Alice Smith',
        email: 'alice@example.com',
        phone: null,
        created_at: '2025-10-02T10:15:00.000Z',
      },
    ]);
    const deleteResponse = createJsonResponse({}, { status: 200 });

    fetchMock
      .mockResolvedValueOnce(searchResponse)
      .mockResolvedValueOnce(deleteResponse);

    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);

    renderPage();
    const user = userEvent.setup();

    await user.type(screen.getByLabelText(/search customers/i), 'Alice');
    await user.click(screen.getByRole('button', { name: /search/i }));
    await screen.findByText('Alice Smith');

    await user.click(screen.getByRole('button', { name: /edit/i }));
    const modalHeading = await screen.findByRole('heading', {
      name: /edit alice smith/i,
    });
    const modalElement = modalHeading.closest('[data-testid="modal-container"]');
    if (!(modalElement instanceof HTMLElement)) {
      confirmSpy.mockRestore();
      throw new Error('Modal container not found');
    }
    const withinModal = within(modalElement);

    await user.click(withinModal.getByRole('button', { name: /delete customer/i }));

    await screen.findByText('Customer deleted and anonymized.');
    confirmSpy.mockRestore();
    const deleteCall = getLastFetchCall();
    expect(deleteCall?.[0]).toBe('http://localhost:8089/customers/cust-1/gdpr/delete');
    const deleteInit = deleteCall?.[1];
    expect(deleteInit?.method).toBe('POST');
    expect(screen.queryByText('Alice Smith')).not.toBeInTheDocument();
  });

  test('renders customer audit history', async () => {
    const searchResponse = createJsonResponse([
      {
        id: 'cust-1',
        name: 'Alice Smith',
        email: 'alice@example.com',
        phone: null,
        created_at: '2025-10-02T10:15:00.000Z',
      },
    ]);

    const auditResponse = createJsonResponse([
      {
        timestamp: '2025-10-02T12:00:00.000Z',
        action: 'Profile Updated',
        actor: 'Dana Admin',
        details: 'Updated phone number to 555-0001',
      },
    ]);

    fetchMock
      .mockResolvedValueOnce(searchResponse)
      .mockResolvedValueOnce(auditResponse);

    renderPage();
    const user = userEvent.setup();

    await user.type(screen.getByLabelText(/search customers/i), 'Alice');
    await user.click(screen.getByRole('button', { name: /search/i }));
    await screen.findByText('Alice Smith');

    await user.click(screen.getByRole('button', { name: /view activity/i }));

    await screen.findByText(/Profile Updated/i);
    const auditCall = getLastFetchCall();
    expect(auditCall?.[0]).toBe('http://localhost:8089/customers/cust-1/audit');
    const auditInit = auditCall?.[1];
    expect(auditInit?.method ?? 'GET').toBe('GET');
  });
});






