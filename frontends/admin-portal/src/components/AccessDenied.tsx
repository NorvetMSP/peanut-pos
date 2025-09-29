import React from "react";

interface AccessDeniedProps {
  title?: string;
  message?: string;
  children?: React.ReactNode;
}

const AccessDenied: React.FC<AccessDeniedProps> = ({
  title = "Access restricted",
  message = "You do not have permission to view this page.",
  children,
}) => {
  return (
    <div className="admin-section-modern" style={{ marginTop: "4rem" }}>
      <div className="admin-section-header">
        <h2>{title}</h2>
        <p>{message}</p>
      </div>
      <div className="admin-section-content" style={{ textAlign: "center" }}>
        {children ?? (
          <p>
            Please contact an administrator if you believe this is a mistake.
          </p>
        )}
      </div>
    </div>
  );
};

export default AccessDenied;
