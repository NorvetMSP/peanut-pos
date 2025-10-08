# NovaPOS Persona Journey Maps

## Contents

- [Cashier](#cashier)
- [Shopper (End Customer)](#shopper-end-customer)
- [Retail IT Admin / System Configurator](#retail-it-admin--system-configurator)
- [Store Manager](#store-manager)
- [NovaPOS Internal Support Team](#novapos-internal-support-team)
- [Corporate HQ Admin](#corporate-hq-admin)
- [External Integration Partner (Valor, Coinbase, etc.)](#external-integration-partner-valor-coinbase-etc)
- [References](#references)

## Cashier

### Cashier — Stages

1. **Opening & Setup** — Clocking in and logging into NovaPOS, preparing the cash drawer and device (receipt paper, etc.).
2. **Active Sales & Checkout** — Serving customers by ringing up items and processing payments.
3. **Special Cases** — Handling loyalty lookups, online pickups, returns/exchanges, or crypto payments.
4. **Closing** — Balancing the register, logging out, syncing pending transactions.

### Cashier — Actions

- Logs into the POS at the start of shift; can use cached login if offline.
- Scans barcodes or searches items on the touchscreen to build carts (supports cached data offline).
- Applies discounts, loyalty rewards, and taxes; attaches customer profiles to apply offers.
- Processes payments across cash, cards, wallets, or crypto via a unified checkout.
- Issues printed or emailed receipts. Offline transactions are queued until sync.
- Handles **BOPIS** (Buy Online, Pickup In Store) orders via alert dashboard.
- Processes returns/exchanges and refunds through unified sales history.

### Cashier — Touchpoints

- **POS Register Interface (Edge POS)** — Offline-capable touchscreen interface.
- **Retail Hardware** — Barcode scanners, printers, cash drawers, and terminals.
- **Payment Devices** — Valor smart terminals and POS QR codes for crypto.
- **Customer-Facing Elements** — Displays, loyalty lookups, and rewards links.
- **Back-Office Portal** (limited use) — Occasionally used for info lookup.

### Cashier — Pain Points

- System downtime or slowness during high-traffic periods.
- Learning curve for crypto or new payment types.
- Handling exceptions and inventory discrepancies when offline.
- Hardware glitches (e.g., printer jams, scanner failure).
- Managing online order pickups while serving in-store customers.
- Loyalty lookup delays due to connectivity or workflow friction.

### Cashier — Opportunities for Improvement

- **Guided Workflows:** Simplify complex flows (returns, crypto) with prompts.
- **Offline Transparency:** Clear “queued/synced” visual feedback loop.
- **Integrated Loyalty:** One-click redemption or auto-recognition of customers.
- **Training Mode:** Sandbox and contextual help directly in POS.
- **BOPIS Tools:** Dedicated pickup dashboard, barcode confirmation, customer notifications.
- **Performance Optimization:** Maintain sub-second responsiveness at checkout.

---

## Shopper (End Customer)

### Shopper — Stages

1. **Discovery & Shopping** — Browses online or in-store, checks stock and reviews.
2. **Purchase & Checkout** — Completes purchases via POS or e-commerce checkout.
3. **Fulfillment & Pickup/Delivery** — Receives items in-store or via pickup/delivery.
4. **Post-Purchase & Support** — Receives receipts, manages returns, tracks loyalty.

### Shopper — Actions

- Shops across unified online/offline catalogs with real-time stock.
- Identifies self via login or phone number for loyalty recognition.
- Selects fulfillment (pickup or delivery) at checkout.
- Pays via preferred method — including crypto (USDC stablecoin).
- Receives receipt and notifications for fulfillment.
- Manages returns in-store for online or physical purchases.

### Shopper — Touchpoints

- **E-Commerce Website/App** — Unified catalog and real-time inventory.
- **Physical Store & POS** — Faster checkout, consistent pricing, and receipt sync.
- **Payment Interfaces** — Card terminals, wallet scans, or crypto QR.
- **Notifications & Communications** — Emails/SMS for orders and rewards.
- **Loyalty/CRM System** — Unified profile across channels.

### Shopper — Pain Points

- Inventory inaccuracy between online and in-store.
- Slow checkout or delayed pickups.
- Unfamiliar or unavailable payment methods.
- Disconnected loyalty experiences.
- Complicated or slow returns.
- Security or data privacy concerns.

### Shopper — Opportunities for Improvement

- **Omnichannel Fluidity:** “Reserve online, buy in-store” and cross-channel sync.
- **Personalized Checkout:** Contextual offers or greetings at POS display.
- **Enhanced BOPIS:** Real-time order status and customer check-in alerts.
- **Self-Service Options:** Mobile or kiosk checkout powered by NovaPOS.
- **Security Messaging:** Visible trust signals during payments.
- **Simplified Returns:** No-receipt loyalty-based return lookups.

---

## Retail IT Admin / System Configurator

### Retail IT Admin / System Configurator — Stages

1. **Setup & Deployment** — Hardware installation, POS software provisioning.
2. **Configuration & Integration** — Tax, payments, users, and systems setup.
3. **Testing & Training** — Validating end-to-end workflows pre-launch.
4. **Go-Live & Monitoring** — Ensuring uptime and addressing issues.
5. **Maintenance & Upgrades** — Updates, scaling, and compliance.

### Retail IT Admin / System Configurator — Actions

- Deploys devices and peripherals, validating connections.
- Configures network, VPNs, offline sync behavior.
- Sets tax rules, payment gateways, and integration keys.
- Links with ERP, accounting, or e-commerce APIs.
- Tests offline scenarios and user permissions.
- Trains staff and documents procedures.
- Monitors logs, applies updates, enforces MFA and security.

### Retail IT Admin / System Configurator — Touchpoints

- **Admin Portal** — Central configuration and dashboards.
- **Edge POS** — Local diagnostics and connection tests.
- **Peripheral Tools** — Terminal/printer management utilities.
- **Integration APIs** — Test and monitor connected systems.
- **Support Channels** — Vendor KB and escalation systems.
- **Monitoring Systems** — Device uptime and sync health.

### Retail IT Admin / System Configurator — Pain Points

- Manual setup for multi-store deployments.
- Integration complexities with legacy systems.
- Risk and reconciliation issues with offline sync.
- Coordinating updates across distributed hardware.
- High volume of non-system support requests.
- Security compliance pressure (PCI, GDPR).
- Scaling stress as transaction volume grows.

### Retail IT Admin / System Configurator — Opportunities for Improvement

- **Automated Deployment:** Remote installation and device management.
- **Unified Monitoring:** Store-level uptime, sync, and alerts dashboard.
- **Prebuilt Connectors:** Plug-and-play ERP/CRM integrations.
- **Auto-Updates:** Nightly upgrades and resilient failover design.
- **Security Tools:** Built-in password policy enforcement and alerts.
- **Learning Resources:** Ongoing admin training and best-practice guides.

---

## Store Manager

### Store Manager — Stages

1. **Opening & Prep** — Check systems, review reports, open store.
2. **Active Operations** — Oversee sales, inventory, and staff.
3. **Management Tasks** — Schedule, restock, and process inventory.
4. **Closeout** — Reconcile sales and finalize daily reports.

### Store Manager — Actions

- Monitors live sales dashboards and price accuracy.
- Manages staff access, resets passwords, and reviews performance.
- Tracks inventory and inter-store transfers.
- Authorizes overrides, returns, and discounts.
- Fulfills and monitors BOPIS orders.
- Reviews AI-driven insights (fraud or trend alerts).
- Runs end-of-day reconciliations and sync checks.

### Store Manager — Touchpoints

- **Admin Portal (Manager View)** — Store-specific analytics and reports.
- **POS (Supervisor Mode)** — Overrides and real-time assistance.
- **Mobile Tablet** — Portable inventory and dashboard access.
- **Reports/Exports** — Daily sales and inventory summaries.
- **Communication Tools** — Alerts, emails, or notifications.

### Store Manager — Pain Points

- Overload of unfiltered data or unclear AI alerts.
- Inventory mismatches requiring manual fixes.
- POS lag or outages during busy periods.
- Limited autonomy for local pricing or promos.
- BOPIS coordination under high load.
- Complex return workflows and fraud risk.
- Frequent retraining needs with updates.

### Store Manager — Opportunities for Improvement

- **Daily Brief Dashboard:** Concise KPIs and actionable alerts.
- **One-Click Inventory Adjustments:** Simplified count corrections.
- **Safe Local Promotions:** Controlled store-level discount flexibility.
- **Task-Oriented BOPIS Tools:** Smart order assignment and timing alerts.
- **Training Overlay Mode:** Interactive guidance in POS/portal.
- **Collaborative Alerts:** Two-way anomaly resolution with HQ.

---

## NovaPOS Internal Support Team

### NovaPOS Internal Support Team — Stages

1. **Monitoring & Alerting** — Track global performance and outages.
2. **Issue Intake** — Receive and prioritize client incidents.
3. **Diagnosis & Troubleshooting** — Investigate and reproduce issues.
4. **Resolution & Communication** — Fix or escalate problems.
5. **Feedback & Continuous Improvement** — Document learnings and update KB.

### NovaPOS Internal Support Team — Actions

- Monitor API health, sync latency, and error rates.
- Categorize tickets by severity and impact.
- Guide clients through diagnostics or run remote checks.
- Replicate complex bugs in sandbox environments.
- Coordinate with partners (e.g., Valor, Coinbase) for third-party issues.
- Communicate updates, workarounds, and post-mortems.
- Document resolutions for reuse and training.

### NovaPOS Internal Support Team — Touchpoints

- **Ticketing System** — Centralized incident tracking.
- **Super-Admin Console** — Tenant-level visibility and admin control.
- **Logs/Dashboards** — System-wide telemetry.
- **Knowledge Base** — Internal and public troubleshooting resources.
- **Communication Channels** — Chat, bridge calls, escalation queues.
- **Testing Environments** — Safe replication spaces.

### NovaPOS Internal Support Team — Pain Points

- Limited access to edge device diagnostics.
- Difficulty reproducing intermittent bugs.
- Managing client expectations under pressure.
- High volume of minor repetitive tickets.
- Cross-team coordination delays.
- Continuous learning demands with rapid updates.
- Complexity from multi-tenant support.

### NovaPOS Internal Support Team — Opportunities for Improvement

- **Remote Diagnostics:** Secure telemetry from edge devices.
- **AI Triage:** Automated ticket classification and KB lookup.
- **Expanded Self-Service:** Client portal for common issues.
- **Faster Escalation Path:** Streamlined engineering collaboration.
- **Status Dashboards:** Public service uptime and incident feeds.
- **Feedback Loops:** Turn frequent issues into roadmap fixes.

---

## Corporate HQ Admin

### Corporate HQ Admin — Stages

1. **Implementation & Setup** — Load master data and configure stores.
2. **Strategic Configuration** — Execute pricing, tax, and promo strategies.
3. **Oversight & Analytics** — Monitor sales and store performance.
4. **Coordination & Support** — Interface between corporate, stores, and IT.
5. **Continuous Improvement** — Expansion, integration, and scalability.

### Corporate HQ Admin — Actions

- Manage global catalogs, taxes, and pricing.
- Oversee chain-wide inventory and transfer requests.
- Analyze performance and trends through reports.
- Define loyalty rules and marketing segmentation.
- Handle escalations and central refunds.
- Onboard new stores or channels (marketplaces, pop-ups).

### Corporate HQ Admin — Touchpoints

- **Corporate Admin Portal** — Multi-store dashboard and configuration.
- **Analytics Tools** — PowerBI/Tableau integrations.
- **Integration Interfaces** — ERP, accounting, and CRM connectors.
- **Reports/Exports** — KPIs for executive briefing.
- **Vendor & Support Contacts** — Account management and licensing.

### Corporate HQ Admin — Pain Points

- Manual data aggregation and report customization.
- Data quality assurance across distributed stores.
- Risk of global misconfiguration.
- Sync delays causing visibility gaps.
- Role permission complexity.
- Report latency on large data volumes.
- Integration friction with external systems.

### Corporate HQ Admin — Opportunities for Improvement

- **Predictive Analytics:** AI-driven forecasting and natural-language BI.
- **Custom Report Builder:** Drag-and-drop visual reporting.
- **Data Validation Alerts:** Catch anomalies in near real-time.
- **Granular Role Controls:** Prebuilt access templates and impersonation mode.
- **Omnichannel Management:** Ship-from-store and channel expansion support.
- **Collaborative Notes:** Shared insights across HQ and stores.
- **Enterprise Scalability:** Dedicated success manager and tiered modules.

---

## External Integration Partner (Valor, Coinbase, etc.)

### External Integration Partner — Stages

1. **Onboarding & Agreement** — Define use case and technical alignment.
2. **Design & Development** — Build and validate API-level integration.
3. **Testing & Certification** — Joint QA and compliance verification.
4. **Deployment & Rollout** — Initial field rollout and observation.
5. **Maintenance & Evolution** — Version upgrades and performance monitoring.

### External Integration Partner — Actions

- Provide APIs, SDKs, and sandbox access.
- Collaborate on technical design documentation.
- Support joint sprints and code reviews.
- Conduct certification tests and fix interoperability issues.
- Offer training and documentation for client onboarding.
- Monitor live usage, metrics, and errors.
- Plan and deliver version updates or new features.

### External Integration Partner — Touchpoints

- **API/SDK Interfaces** — Core integration endpoints.
- **Integration Gateway** — Middleware layer and authentication.
- **Partner Dashboards** — Transaction logs and reconciliation tools.
- **Communication Channels** — Technical liaisons, issue trackers, meetings.
- **End-User Interaction Points** — Payment terminals, QR codes, checkout flows.

### External Integration Partner — Pain Points

- API contract mismatches or unclear documentation.
- Lengthy certification or compliance cycles.
- Misaligned product timelines.
- Difficult post-launch debugging and responsibility division.
- Evolving APIs causing maintenance burdens.
- Performance under peak transaction loads.

### External Integration Partner — Opportunities for Improvement

- **Versioned API Contracts:** Stable OpenAPI specs with mock simulators.
- **Shared Monitoring:** Joint telemetry and status dashboards.
- **Continuous Integration:** Auto-testing between systems on updates.
- **Co-Marketing & Success Stories:** Promote the joint value proposition.
- **Innovation Collaboration:** Jointly explore new payment and loyalty features.
- **Turnkey Setup:** “One-click” activation for retailers inside NovaPOS.

---

## References
