export const ROLE_SUPER_ADMIN = "super_admin" as const;
export const ROLE_ADMIN = "admin" as const;
export const ROLE_MANAGER = "manager" as const;
export const ROLE_CASHIER = "cashier" as const;

export const ROLE_PRIORITY = [
  ROLE_SUPER_ADMIN,
  ROLE_ADMIN,
  ROLE_MANAGER,
  ROLE_CASHIER,
] as const;

export type Role = (typeof ROLE_PRIORITY)[number];
export type RoleList = readonly Role[];

export const MANAGER_ROLES = [
  ROLE_SUPER_ADMIN,
  ROLE_ADMIN,
  ROLE_MANAGER,
] as const;
export const ADMIN_ROLES = [ROLE_SUPER_ADMIN, ROLE_ADMIN] as const;
export const SUPER_ADMIN_ROLES = [ROLE_SUPER_ADMIN] as const;

export const ensureRoleOrder = (values: Iterable<string>): string[] => {
  const seen = new Set<string>();
  for (const value of values) {
    if (typeof value === "string" && value.trim().length > 0) {
      seen.add(value.trim());
    }
  }
  return Array.from(seen).sort((a, b) => {
    const aIndex = ROLE_PRIORITY.indexOf(a as Role);
    const bIndex = ROLE_PRIORITY.indexOf(b as Role);
    const safeA = aIndex === -1 ? Number.MAX_SAFE_INTEGER : aIndex;
    const safeB = bIndex === -1 ? Number.MAX_SAFE_INTEGER : bIndex;
    return safeA - safeB || a.localeCompare(b);
  });
};

export const roleLabel = (value: string): string =>
  value
    .split("_")
    .map((part) => (part ? part.charAt(0).toUpperCase() + part.slice(1) : part))
    .join(" ");
