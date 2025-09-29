import { useAuth } from "../AuthContext";

export const useRoleList = (): readonly string[] => {
  const { roles } = useAuth();
  return roles;
};

export const useHasAnyRole = (allowed: readonly string[]): boolean => {
  const { hasAnyRole: hasRoleAccess } = useAuth();
  return hasRoleAccess(allowed);
};
