CREATE TABLE products (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    name TEXT NOT NULL,
    price NUMERIC(10,2) NOT NULL,
    description TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_products_tenant ON products(tenant_id);
