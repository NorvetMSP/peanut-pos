import React from "react";
import { useHasAnyRole } from "../hooks/useRoleAccess";
import AccessDenied from "./AccessDenied";

interface RoleGuardOptions {
  title?: string;
  message?: string;
  fallbackContent?: React.ReactNode;
}

export function withRoleGuard<P extends Record<string, unknown>>(
  Component: React.ComponentType<P>,
  allowedRoles: readonly string[],
  options?: RoleGuardOptions,
): React.FC<P> {
  const Guarded: React.FC<P> = (props: P) => {
    const hasAccess = useHasAnyRole(allowedRoles);

    if (!hasAccess) {
      return (
        <AccessDenied title={options?.title} message={options?.message}>
          {options?.fallbackContent}
        </AccessDenied>
      );
    }

    return <Component {...(props as P)} />;
  };

  Guarded.displayName = `WithRoleGuard(${Component.displayName ?? Component.name ?? "Component"})`;
  return Guarded;
}
