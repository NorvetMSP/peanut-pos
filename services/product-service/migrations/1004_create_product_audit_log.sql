CREATE TABLE product_audit_log (
	id UUID PRIMARY KEY,
	product_id UUID NOT NULL,
	tenant_id UUID NOT NULL,
	actor_id UUID NULL,
	actor_name TEXT NULL,
	actor_email TEXT NULL,
	action TEXT NOT NULL,
	changes JSONB NOT NULL,
	created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_product_audit_log_tenant ON product_audit_log (tenant_id);
CREATE INDEX idx_product_audit_log_product ON product_audit_log (product_id);
