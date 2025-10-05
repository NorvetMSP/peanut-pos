-- Add SKU and tax_code to products; backfill nullable then enforce partial uniqueness
ALTER TABLE products
  ADD COLUMN IF NOT EXISTS sku TEXT NULL,
  ADD COLUMN IF NOT EXISTS tax_code TEXT NULL;

-- Default tax_code to 'STD' where missing
UPDATE products SET tax_code = 'STD' WHERE tax_code IS NULL;

-- Index for fast lookup by sku within tenant; unique when sku is present
CREATE UNIQUE INDEX IF NOT EXISTS idx_products_tenant_sku_unique
  ON products(tenant_id, sku)
  WHERE sku IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_products_sku
  ON products(sku)
  WHERE sku IS NOT NULL;
