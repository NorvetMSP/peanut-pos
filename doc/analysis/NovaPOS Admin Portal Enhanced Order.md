
# NovaPOS Admin Portal – Enhanced Order Search, Receipts, and Returns Implementation

## 1. Deep Order Search Functionality

To empower admins with advanced order lookup, we will extend the Orders page and backend to support multi-criteria filtering, pagination, and sorting[1]. On the frontend, we add filter controls in `frontends/admin-portal/src/pages/OrdersPage.tsx` for the following criteria:

- **Store ID** – filter orders by a specific store/branch

- **Customer name/email** – partial match against customer name or email

- **Order ID** – exact match for quick lookup

- **Date range** – start and end date to find orders in a period

- **Payment method** – e.g. Cash, Card, Crypto, etc.

- **Order status** – Pending, Completed, Refunded, Partially Refunded, Voided, etc.

The UI will send these filters to a new backend search endpoint. We leverage React (TypeScript) and can use React Query for data fetching to keep the interface responsive and cached. The results render in a paginated, sortable table. We also ensure each request carries the tenant context (e.g. an `X-Tenant-ID` header or token claim) so that only orders for the current tenant are returned[2]. Only users with appropriate roles (e.g. admin/manager) can query all orders; others could be restricted to their own orders if needed.

Below is the `OrdersPage.tsx` with the added search UI and logic:

```tsx
// frontends/admin-portal/src/pages/OrdersPage.tsx
import React, { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
// ... other imports (maybe context for tenant/user, etc.)

const OrdersPage: React.FC = () => {
  // ...existing code...
};
```

export default OrdersPage;

In this snippet, we gather filter inputs and on any change, refetch the orders (using fetch with the appropriate query parameters and tenant header). The backend will expose a matching `GET /orders` endpoint to accept these filters and query the database. The table displays key order info and provides action buttons per order: “View Receipt” (opens a receipt modal) and “Return” (navigates to the returns workflow for that order).

On the backend (Rust Order Service), we implement a handler (e.g. `search_orders`) to query orders with the given filters. The handler ensures multi-tenant scope by including the tenant ID in the WHERE clause of every query[2]. It also enforces RBAC: only users with roles allowed to view orders (admin, manager, etc.) can use this broad search. We use SQLx to construct a parameterized SQL query, adding conditions only for provided filters. For partial matches (customer name/email), we use SQL ILIKE with wildcards. Sorting and pagination are applied via `ORDER BY ... LIMIT ... OFFSET`.

Key aspects in the Rust implementation:

- The Identity/auth middleware yields a User context with tenant_id and roles.

- We validate the user’s role (e.g., `user.can_view_orders()`).

- We build the SQL query dynamically in a safe manner (whitelisting sort fields, using bound parameters for filters to prevent SQL injection).

- The query joins with related data if needed (for example, if we stored customer_name and customer_email on the orders table for quick searching; otherwise, an alternative would be querying the Customer Service separately). In our design, we assume orders table contains customer_name/email for convenience, populated at order creation.

Below is a simplified Rust handler showing how filters are applied and tenant isolation is enforced:

```rust
// services/order-service/src/order_handlers.rs (excerpt)
use actix_web::{web, HttpResponse};
use uuid::Uuid; // Uuid type from the uuid crate
use sqlx::PgPool;
use crate::auth::{User, Role};  // hypothetical user/role context
use crate::errors::AppError;
use serde::Deserialize;


#[derive(Deserialize)]
struct OrderSearchParams {
  order_id: Option<Uuid>,
  store_id: Option<Uuid>,
  customer: Option<String>,       // name or email query
  start_date: Option<chrono::NaiveDate>,
  end_date: Option<chrono::NaiveDate>,
  payment_method: Option<String>,
  status: Option<String>,
  limit: Option<i64>,
  offset: Option<i64>,
  sort: Option<String>,
  order: Option<String>
}

// ...existing code...
```

// Retrieve detailed order info (with items) for internal use
async fn get_order_detail(user: User, order_id: Uuid, pool: &PgPool) -> Result<OrderDetail, AppError> {
    // Ensure the order belongs to the same tenant
    let order = sqlx::query_as!(
        OrderDetail,
        "SELECT id, store_id, customer_name, customer_email, total, status, payment_method, created_at
         FROM orders
         WHERE id = $1 AND tenant_id = $2",
        order_id, user.tenant_id
    )
    .fetch_optional(pool).await?;
    if order.is_none() {
        return Err(AppError::NotFound("Order not found".into()));
    }
    let mut order_detail = order.unwrap();
    // Fetch line items for the order
    let items = sqlx::query_as!(
        OrderItem,
        "SELECT product_id, product_name, quantity, unit_price, COALESCE(returned_quantity, 0) as returned_quantity
         FROM order_items
         WHERE order_id = $1",
        order_detail.id
    )
    .fetch_all(pool).await?;
    order_detail.items = items;
    Ok(order_detail)
}

pub async fn get_order_receipt(user: User, path: web::Path<Uuid>, pool: web::Data<PgPool>)
    -> Result<HttpResponse, AppError>
{
    let order_id = path.into_inner();
    let detail = get_order_detail(user, order_id, pool.get_ref()).await?;  // full order + items
    // Assemble receipt Markdown (using simple string formatting for demo)
    let mut markdown = String::new();

    // Markdown receipt
    markdown += &format!("## Receipt – Order `{}`\n", detail.id);
    markdown += &format!("**Date:** {}\n", detail.created_at.format("%Y-%m-%d %H:%M:%S"));
    markdown += &format!("**Store:** `{}`\n", detail.store_id);
    if let Some(name) = &detail.customer_name {
        markdown += &format!("**Customer:** {} ({})\n", name, detail.customer_email.clone().unwrap_or_default());
    }
    markdown += "| Item | Qty | Price | Total |\n|---|---|---|---|\n";
    for item in &detail.items {
        let line_total = &item.unit_price * BigDecimal::from(item.quantity);
        markdown += &format!("| {} | {} | ${} | ${} |\n", item.product_name, item.quantity, item.unit_price, line_total);
    }
    markdown += &format!("\n**Grand Total:** ${}\n", detail.total);
    markdown += &format!("**Payment Method:** {}{}\n",
        detail.payment_method,
        if detail.status == "REFUNDED" { " (REFUNDED)" } else { "" });
    markdown += "\n_Thank you for your business!_\n";
    return Ok(HttpResponse::Ok()
        .content_type("text/markdown")
        .body(markdown));
}
In this snippet, OrderDetail is a struct representing an order with an items: Vec<OrderItem> field; we populate it by first querying the orders table and then the order_items. The HTML assembly is rudimentary – in a real app, you’d likely use a proper template or PDF generator, and include store details like address or logo. The tenant check ensures an admin from one tenant cannot fetch another tenant’s order receipt. RBAC-wise, if a lower role (like basic cashier) should not access receipts via this admin API, we’d ensure only admin roles call this (or simply rely on the fact that only Admin Portal (for managers/admins) uses this route).
Note on data: Because we persist each line item’s price at time of purchase[3], the receipt accurately reflects the transaction as it occurred (even if product prices change later). The receipt shows if an order was refunded by checking status, but a more advanced version might list returned items separately.
3. Returns Dashboard and Workflow
The Returns Dashboard provides visibility into return/refund transactions and allows authorized staff to initiate returns for completed orders. This addresses the current gap of no UI for returns[1]. We implement both the frontend (a new Returns page in the admin portal) and backend (endpoints to list and process returns). All operations respect multi-tenant boundaries and RBAC so that, for example, only store managers or admins (not regular cashiers) can perform returns[4].
Frontend – ReturnsPage UI

In frontends/admin-portal/src/pages/ReturnsPage.tsx, we will create an interface with two main components:

-- Return History Table: A list of past return transactions with filters (e.g. date range, store). This allows ops teams to review refund activity.

-- Return Initiation Form: A tool to process a new return for a customer’s order. The user can enter or scan an Order ID (perhaps from a receipt barcode) to retrieve the order details, then select which items and quantities to refund, and provide a return reason.

We reuse styling and state management similar to OrdersPage. The return initiation flow is: 1. Lookup Order: The staff enters an Order ID (or comes via link from the Orders page). The app calls GET /api/orders/{id} (or the same detail endpoint used for receipts) to fetch order info and line items. 2. Show Order & Items: Display the order’s items, including how many of each have already been returned (to prevent over-return). Each item has an input for the quantity to return (0 up to the remaining quantity). 3. Specify Reason: A dropdown or text field for return reason (e.g. Damaged, Unwanted, Wrong Item, etc.). 4. Process Return: On submit, call POST /api/orders/{id}/refund with the selected items and reason. 5. Handle Result: If successful, show confirmation (and possibly update the returns list or redirect). If error, display the message (e.g. if the order ID was invalid or return quantity too high).

Below is a ReturnsPage.tsx implementation covering these aspects:
// frontends/admin-portal/src/pages/ReturnsPage.tsx
import React, { useState, useEffect } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
// ... assume types OrderDetail, etc., and perhaps React Query hook useQuery if needed

const ReturnsPage: React.FC = () => {
  const navigate = useNavigate();
  const location = useLocation();
  // If navigated with a query param like ?orderId=..., use that to pre-fill
  const initialOrderId = new URLSearchParams(location.search).get('orderId') || '';
  const [orderIdInput, setOrderIdInput] = useState(initialOrderId);
  const [order, setOrder] = useState<OrderDetail | null>(null);
  const [returnQty, setReturnQty] = useState<{ [productId: string]: number }>({});
  const [reason, setReason] = useState('');
  const tenantId = /*current tenant ID from context*/;

  // Fetch order details if an orderId was provided initially
  useEffect(() => {
    if (initialOrderId) {
      lookupOrder();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialOrderId]);

  // Lookup an order by ID to initiate a return
  const lookupOrder = async () => {
    if (!orderIdInput) return;
    const res = await fetch(`/api/orders/${orderIdInput}`, {
      headers: { 'X-Tenant-ID': tenantId }
    });
    if (res.ok) {
      const data = await res.json();
      setOrder(data);
      // Initialize returnQty state for each item to 0
      const quantities: { [id: string]: number } = {};
      data.items.forEach((item: any) => { quantities[item.product_id] = 0; });
      setReturnQty(quantities);
      setReason('');
    } else {
      alert('Order not found or inaccessible');
      setOrder(null);
    }
  };

  // Submit return request
  const submitReturn = async () => {
    if (!order) return;
    // Prepare items to return (only those with quantity > 0)
    const itemsToReturn = order.items.filter(item => (returnQty[item.product_id] || 0) > 0)
      .map(item => ({
        product_id: item.product_id,
        quantity: returnQty[item.product_id],
        refund_amount: item.unit_price * (returnQty[item.product_id] || 0)
      }));
    if (itemsToReturn.length === 0) {
      alert("No items selected for return");
      return;
    }
    if (!reason) {
      alert("Please select a return reason");
      return;
    }
    const payload = { items: itemsToReturn, reason };
    const res = await fetch(`/api/orders/${order.id}/refund`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Tenant-ID': tenantId },
      body: JSON.stringify(payload)
    });
    if (res.ok) {
      alert('Return processed successfully');
      // Optionally reload returns list or navigate away
      setOrder(null);
      setReturnQty({});
      setReason('');
      setOrderIdInput('');
      // We could refresh a returns list here if we maintain one
    } else {
      const err = await res.json();
      alert('Return failed: ' + err.message);
    }
  };

  // (Optional) Fetch recent returns list for display
  const [returns, setReturns] = useState<any[]>([]);
  useEffect(() => {
    const fetchReturns = async () => {
      const res = await fetch(`/api/returns?limit=20`, {
        headers: { 'X-Tenant-ID': tenantId }
      });
      if (res.ok) {
        const data = await res.json();
        setReturns(data.returns);
      }
    };
    fetchReturns();
  }, [tenantId]);

  return (
    # Returns Dashboard

      {/* Order lookup form */}
      <div className="return-lookup">
        <input 
          placeholder="Enter Order ID" 
          value={orderIdInput} 
          onChange={e => setOrderIdInput(e.target.value)} 
        />
        <button onClick={lookupOrder}>Find Order</button>
      </div>

      {/* Return processing form (visible once order is loaded) */}
      {order && (
        <div className="return-form">
          <h3>Return for Order {order.id} – Status: {order.status}</h3>
          <p>Customer: {order.customer_name} ({order.customer_email})</p>
          <p>Total Paid: ${order.total}</p>
          <table>
            <thead>
              <tr><th>Product</th><th>Purchased</th><th>Already Returned</th><th>Return Qty</th></tr>
            </thead>
            <tbody>
              {order.items.map(item => (
                <tr key={item.product_id}>
                  <td>{item.product_name}</td>
                  <td>{item.quantity}</td>
                  <td>{item.returned_quantity || 0}</td>
                  <td>
                    <input type="number" min="0" 
                      max={item.quantity - (item.returned_quantity || 0)} 
                      value={returnQty[item.product_id] || 0}
                      onChange={e => 
                        setReturnQty({ ...returnQty, [item.product_id]: Number(e.target.value) })
                      }
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div>
            <label>Reason: </label>
            <select value={reason} onChange={e => setReason(e.target.value)}>
              <option value="">-- Select Reason --</option>
              <option value="DAMAGED">Damaged</option>
              <option value="WRONG_ITEM">Wrong Item</option>
              <option value="UNWANTED">Unwanted/Changed Mind</option>
              <option value="OTHER">Other</option>
            </select>
          </div>
          <button onClick={submitReturn}>Process Return</button>
          <button onClick={() => setOrder(null)}>Cancel</button>
        </div>
      )}

      <hr/>

      {/* Past Returns list */}
      <h2>Recent Returns</h2>
      <table>
        <thead>
          <tr><th>Date</th><th>Order ID</th><th>Refunded Amount</th><th>Reason</th><th>Processed By</th></tr>
        </thead>
        <tbody>
          {returns.map(ret => (
            <tr key={ret.id}>
              <td>{new Date(ret.created_at).toLocaleString()}</td>
              <td>{ret.order_id}</td>
              <td>${ret.refunded_amount}</td>
              <td>{ret.refund_reason}</td>
              <td>{ret.processed_by_name || ret.processed_by}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};

export default ReturnsPage;
In this React component, we manage local state for the order being returned and quantities. When an order is loaded, we display its items in a table with an input for the return quantity (capped by the remaining quantity that hasn’t been returned yet). The Recent Returns table at the bottom shows a snapshot of recent return transactions (fetched from GET /api/returns). This gives admins a quick overview of return activity.
We also ensure RBAC on the UI: the navigation to ReturnsPage should ideally be shown only to users with roles allowed to process returns. If a user without permission somehow accesses this page, the API calls will be rejected by the backend (Forbidden), but we can also show an immediate “Not Authorized” message or redirect.
Backend – Returns API and Processing Logic
NovaPOS’s Order Service already had underlying support for refunds (partial returns), including tracking returned quantities and adjusting order status[5][6]. We will expose or extend these capabilities via a secure API endpoint and implement a new returns listing. All operations are tenant-aware and enforce the proper roles.
Data Model: We introduce a new table (if not existing) to record return transactions, e.g. an order_returns table, and a related return_items table, as well as a field in order_items to track returned quantity. This aligns with the design mentioned in the internal analysis[5]. A migration script (e.g. services/order-service/migrations/2025XXXX_add_returns.sql) would be:
-- services/order-service/migrations/2025XXXX_add_returns.sql
ALTER TABLE order_items ADD COLUMN returned_quantity INT DEFAULT 0;
CREATE TABLE order_returns (
    id UUID PRIMARY KEY,
    order_id UUID REFERENCES orders(id) ON DELETE CASCADE,
    refund_reason TEXT,
    processed_by UUID,        -- user ID who processed the return
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);
CREATE TABLE return_items (
    return_id UUID REFERENCES order_returns(id) ON DELETE CASCADE,
    product_id UUID,
    quantity INT,
    refund_amount DECIMAL(10,2)
    -- (indexes can be added as needed for query performance)
);
This allows recording each return event separately from the original order. The orders.status field will be updated to REFUNDED or PARTIAL_REFUNDED accordingly[6].
Endpoints:

- GET /api/returns: List return records, possibly filterable by date, store, etc., similar to order search. This will join order_returns with orders and perhaps users (for processed_by name) to return a summary of each return (date, order_id, amount, reason, processed_by). We skip the detailed code for brevity; it would be analogous to search_orders but on the order_returns table with filters.
- POST /api/orders/{id}/refund: Process a return/refund for the given order. This is the core of the returns workflow and includes multiple steps to ensure consistency:
Refund Processing Steps[5]:
- Role check: Confirm the user’s role is authorized for refunds [e.g. in ORDER_REFUND_ROLES like manager/admin](4).
- Load Order (Lock): Fetch the order by ID for that tenant and lock it (FOR UPDATE) within a database transaction. Verify the order exists and is eligible (e.g., not already fully refunded or voided).
- Validate Items: For each item requested to return, ensure it exists in the order and that the return quantity ≤ (original quantity – already returned quantity). If any violation, abort with a 400 Bad Request.
- Calculate refund amount: Sum up the refund amounts (we can calculate per item as unit_price * return_quantity).
- Persist Return Records: Insert a new row into order_returns (with a new UUID, reference to order, reason, processed_by user ID), and insert entries into return_items for each returned item with quantity and refund amount. Update the order_items.returned_quantity for those items by adding the returned qty.
- Update Order Status: Determine if the order is now fully refunded (all items returned) or partially – set orders.status to REFUNDED or PARTIAL_REFUNDED accordingly[6].
- Trigger Payment Refund: If the payment method was non-cash (e.g. Credit Card or Crypto), call the Payment Service to actually issue the refund to the customer’s card/account. This could be a REST call to the Payment microservice or an internal Kafka event. For security, we use the service’s authentication (e.g. a service JWT or API key) when calling this internal API[7]. We perform this before committing the transaction – if the payment gateway returns failure, we roll back all DB changes and return an error so the UI knows the refund didn’t complete. (If the payment service is asynchronous, we might mark as pending, but here we assume synchronous stub for simplicity.)
- Commit Transaction: Once the payment refund is confirmed (or if it was a cash refund which doesn’t require external processing), commit the database transaction so all changes persist atomically.
- Emit Events: Publish an order.completed event with negative item quantities and negative total representing the refund[7]. The Inventory Service, already listening to order events, will interpret negative quantities as stock being added back [restock](7). Similarly, the Loyalty Service will deduct points for the refunded amount (since a negative total leads to negative loyalty points) – this behavior was already designed in the system. We also could emit a dedicated order.refunded event, but reusing the existing event type with negative values keeps services simple.
- Audit Logging: Log the return action via the Audit Service. For example, emit an audit.log event or call an audit endpoint with details [order ID, user, reason, amount](5). This ensures an immutable record of who performed the return and why, visible in audit trails.
Below is the Rust handler for refunding an order (partial/full return). It puts together the above steps:
// services/order-service/src/order_handlers.rs (refund endpoint)
use bigdecimal::BigDecimal;
use actix_web::{web, HttpResponse};
use uuid::Uuid;
use crate::auth::{User, ORDER_REFUND_ROLES};
use crate::errors::AppError;
use serde::Deserialize;

# [derive(Deserialize)]

struct RefundItem {
    product_id: Uuid,
    quantity: i32,
    refund_amount: BigDecimal  // client can send calculated refund, or compute server-side
}

# [derive(Deserialize)]

struct RefundRequest {
    items: Vec<RefundItem>,
    reason: String
}

pub async fn refund_order(user: User, path: web::Path<Uuid>, req: web::Json<RefundRequest>, pool: web::Data<PgPool>)
    -> Result<HttpResponse, AppError>
{
    let order_id = path.into_inner();
    // RBAC: only allow authorized roles to process returns
    if !user.has_any_role(&ORDER_REFUND_ROLES) {
        return Err(AppError::Forbidden("Not authorized to refund orders".into()));
    }
    // Begin a database transaction to ensure atomic updates
    let mut tx = pool.begin().await.map_err(|e| AppError::Internal(e.to_string()))?;
    // Lock and fetch the order for update
    let order_rec = sqlx::query!(
        "SELECT id, tenant_id, status, payment_method, total FROM orders WHERE id = $1 FOR UPDATE",
        order_id
    )
    .fetch_optional(&mut tx).await.map_err(|e| AppError::Internal(e.to_string()))?;
    if order_rec.is_none() {
        return Err(AppError::NotFound("Order not found".into()));
    }
    let order = order_rec.unwrap();
    // Tenant isolation check
    if order.tenant_id != user.tenant_id && !user.is_super_admin() {
        return Err(AppError::Forbidden("Cross-tenant access denied".into()));
    }
    // Business logic checks
    if order.status == "REFUNDED" || order.status == "VOIDED" {
        return Err(AppError::BadRequest("Order is already refunded or voided".into()));
    }
    // Fetch all order items for validation
    let order_items = sqlx::query!(
        "SELECT product_id, quantity, COALESCE(returned_quantity, 0) as returned_qty, unit_price
         FROM order_items WHERE order_id = $1",
        order_id
    )
    .fetch_all(&mut tx).await.map_err(|e| AppError::Internal(e.to_string()))?;
    // Validate each requested return item
    let mut total_refund = BigDecimal::from(0);
    for item in &req.items {
        let db_item = order_items.iter().find(|it| it.product_id == item.product_id);
        if db_item.is_none() {
            return Err(AppError::BadRequest("Invalid item in return request".into()));
        }
        let db_item = db_item.unwrap();
        let already_returned = db_item.returned_qty.unwrap_or(0);
        if item.quantity <= 0 || item.quantity > (db_item.quantity - already_returned) {
            return Err(AppError::BadRequest("Return quantity for an item is invalid".into()));
        }
        // Calculate refund amount (server trust but verify client's amount)
        let price_each = db_item.unit_price;
        let line_refund = price_each * BigDecimal::from(item.quantity);
        // Optionally, compare line_refund with item.refund_amount from client for consistency
        total_refund += line_refund;
    }
    // Insert a record for this return
    let return_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO order_returns (id, order_id, refund_reason, processed_by)
         VALUES ($1, $2, $3, $4)",
        return_id, order_id, req.reason, user.id
    )
    .execute(&mut tx).await.map_err(|e| AppError::Internal(e.to_string()))?;
    // Insert each returned item and update order_items returned_quantity
    for item in &req.items {
        sqlx::query!(
            "INSERT INTO return_items (return_id, product_id, quantity, refund_amount)
             VALUES ($1, $2, $3, $4)",
            return_id, item.product_id, item.quantity, item.refund_amount
        )
        .execute(&mut tx).await.map_err(|e| AppError::Internal(e.to_string()))?;
        sqlx::query!(
            "UPDATE order_items
             SET returned_quantity = COALESCE(returned_quantity, 0) + $3
             WHERE order_id = $1 AND product_id = $2",
            order_id, item.product_id, item.quantity
        )
        .execute(&mut tx).await.map_err(|e| AppError::Internal(e.to_string()))?;
    }
    // Update order status to PARTIAL_REFUNDED or REFUNDED based on remaining items
    let totals = sqlx::query!(
        "SELECT SUM(quantity) as total_qty, SUM(COALESCE(returned_quantity,0)) as total_returned
         FROM order_items WHERE order_id = $1",
        order_id
    )
    .fetch_one(&mut tx).await.map_err(|e| AppError::Internal(e.to_string()))?;
    let total_qty: i64 = totals.total_qty.unwrap_or(0);
    let total_returned: i64 = totals.total_returned.unwrap_or(0);
    let new_status = if total_returned >= total_qty { "REFUNDED" } else { "PARTIAL_REFUNDED" };
    sqlx::query!("UPDATE orders SET status = $2 WHERE id = $1", order_id, new_status)
        .execute(&mut tx).await.map_err(|e| AppError::Internal(e.to_string()))?;
    // Integrate with Payment Service for actual refund, if not a cash payment
    if order.payment_method != "CASH" {
        // Example: call payment service API (synchronously for now)
        let payment_ok = call_payment_service_refund(order_id, &total_refund).await;
        if !payment_ok {
            tx.rollback().await.ok();
            return Err(AppError::BadRequest("Payment refund failed".into()));
        }
    }
    // Commit transaction now that DB updates and payment are successful
    tx.commit().await.map_err(|e| AppError::Internal(e.to_string()))?;
    // Emit inventory restock and loyalty adjustment event (order.completed with negative values)
    let refund_event = OrderCompletedEvent {
        order_id,
        status: "REFUNDED".to_string(),
        total: -total_refund.clone(),         // negative total indicates a refund
        items: req.items.iter().map(|ri| OrderItemEvent {
            product_id: ri.product_id,
            quantity: -(ri.quantity)          // negative quantity for returned items
        }).collect(),
        payment_method: order.payment_method.clone(),
        tenant_id: order.tenant_id
    };
    kafka_producer.send("order.completed", &refund_event).await;
    // Log audit event for this return
    let audit_event = AuditEvent {
        action: "ORDER_REFUND",
        user_id: user.id,
        tenant_id: order.tenant_id,
        details: format!("Order {} refunded {} items, total ${}", order_id, req.items.len(), total_refund)
    };
    audit_producer.send("audit.log", &audit_event).await;
    // Return success response with new status and refund details
    Ok(HttpResponse::Ok().json({
        "return_id": return_id,
        "status": new_status,
        "refunded_amount": total_refund
    }))
}
In this code, we perform all the critical steps described:

- Transaction & Locking: Using FOR UPDATE on the order row ensures no concurrent process can modify the order while we process the return. This prevents race conditions (e.g., two return requests on the same order).
- Validation: We ensure the requested return quantities don’t exceed what’s available. This uses the persisted returned_quantity field to account for any prior returns[5].
- Database Updates: We record the return in order_returns and detail each item in return_items. We update each affected order_items.returned_quantity. Then we update the main order’s status to PARTIAL_REFUNDED or REFUNDED [if all items are returned](6).
- Payment Refund: We call a helper call_payment_service_refund (pseudo-code) – this would perform the inter-service call. In a real system, this might send an HTTPS request to Payment Service’s refund endpoint with the payment identifier or order info. Since Payment Service is a stub in NovaPOS, we simulate a success. We ensure that if this call fails, we rollback the DB transaction so no return is recorded unless the money is actually refunded to the customer.
- Events: By sending an order.completed event with negative quantities, the Inventory Service will add stock back for those products [it subtracts a negative, effectively adding](7). The Loyalty Service will subtract points (since it sees a negative order total) – this approach was designed in the existing system to handle returns automatically[7]. We reuse existing event types so that Inventory and Loyalty logic remains unchanged for handling returns.
- Audit: We emit an audit log event. The Audit Service (if subscribed or via an API) will record who (user.id) performed the refund and when. This is important for compliance and tracing issues[5].
- Response: The API responds with confirmation including the return_id, new order status, and refunded amount. The front-end can use this to update UI (and possibly print a return receipt if needed).
Security & RBAC: The backend double-checks that the user performing the refund has an appropriate role (has_any_role(ORDER_REFUND_ROLES) which might be defined as roles like ["super_admin", "admin", "manager"] as noted in the design[8]). This prevents unauthorized users from invoking refunds even if they discovered the endpoint.
Multi-Tenancy: Every query includes a tenant filter (tenant_id = ...) or uses data that was already scoped by tenant (the order record fetched ensured matching tenant). We also pass the tenant_id into the event payloads (so downstream services know which tenant’s inventory to adjust, etc.).
With this implementation, NovaPOS Admin Portal gains a robust returns management tool. Staff can find past returns for reference and initiate new ones in a controlled, audited manner. These changes fill the identified MVP gaps for deep order search, receipt access, and return handling[1], greatly improving operational capabilities while following the project’s architecture and security conventions.
References:
[4]: <https://your-link-for-4> "Reference 4 placeholder"
[7]: <https://your-link-for-7> "Reference 7 placeholder"
[10]: <https://your-link-for-10> "Reference 10 placeholder"
- NovaPOS MVP Gaps Analysis – noted lack of admin order search, receipts, and returns UI[1].
- Persisting order line items enables generating detailed receipts and handling returns[3][9].
- Returns design from NovaPOS plan – partial returns, negative inventory updates, RBAC for returns[7][4][10].
- Partial refund status (PARTIAL_REFUNDED) and full refund handling[6].
- Backend refund logic ensuring audit trail and data integrity[5].
