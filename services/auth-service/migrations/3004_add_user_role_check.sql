ALTER TABLE users
    ADD CONSTRAINT chk_users_role_allowed
    CHECK (role IN ('super_admin', 'admin', 'manager', 'cashier'));
