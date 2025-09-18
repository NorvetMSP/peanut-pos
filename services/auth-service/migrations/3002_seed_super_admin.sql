INSERT INTO tenants (id, name)
VALUES ('00000000-0000-0000-0000-000000000001', 'NovaPOS HQ')
ON CONFLICT (id) DO NOTHING;

INSERT INTO users (id, tenant_id, name, email, role, password_hash)
VALUES (
  '00000000-0000-0000-0000-000000000101',
  '00000000-0000-0000-0000-000000000001',
  'Super Admin',
  'admin@novapos.local',
  'super_admin',
  'admin123'
)
ON CONFLICT (tenant_id, email) DO NOTHING;
