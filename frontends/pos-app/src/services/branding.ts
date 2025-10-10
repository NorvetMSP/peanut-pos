export type BrandingConfig = {
  brandName?: string;
  brandHeaderLines?: string[];
};

export function parseBrandingFromEnv(env: Record<string, unknown>): BrandingConfig {
  const nameRaw = env.VITE_BRAND_NAME;
  const linesRaw = env.VITE_BRAND_HEADER_LINES;
  const brandName = typeof nameRaw === 'string' && nameRaw.trim().length > 0 ? nameRaw.trim() : undefined;
  const brandHeaderLines = typeof linesRaw === 'string' && linesRaw.trim().length > 0
    ? linesRaw.split('|').map((s: string) => s.trim()).filter((s: string) => s.length > 0)
    : undefined;
  return { brandName, brandHeaderLines };
}

export function getEnvBranding(): BrandingConfig {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const env: any = (import.meta as any).env ?? {};
  return parseBrandingFromEnv(env);
}

export async function resolveBranding(tenantId?: string | null, token?: string | null): Promise<BrandingConfig> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const env: any = (import.meta as any).env ?? {};
  const fallback = parseBrandingFromEnv(env);
  if (!tenantId) return fallback;

  const base = (import.meta as any).env?.VITE_AUTH_SERVICE_URL ?? 'http://localhost:8085';
  const headers: Record<string, string> = { 'X-Tenant-ID': String(tenantId) };
  if (token) headers.Authorization = `Bearer ${token}`;

  // Try a dedicated branding endpoint first
  try {
    const url = `${String(base).replace(/\/$/, '')}/tenants/${encodeURIComponent(String(tenantId))}/branding`;
    const resp = await fetch(url, { headers });
    if (resp.ok) {
      const data = await resp.json();
      const fromApi: BrandingConfig = {
        brandName: typeof data?.brand_name === 'string' && data.brand_name.trim().length > 0 ? data.brand_name.trim() : undefined,
        brandHeaderLines: Array.isArray(data?.header_lines)
          ? (data.header_lines as unknown[]).map(v => String(v)).map(s => s.trim()).filter(s => s.length > 0)
          : undefined,
      };
      return { ...fallback, ...fromApi };
    }
    if (resp.status !== 404) {
      // For other errors, just fall through to next attempt/fallback
    }
  } catch {
    // ignore network errors; use fallback or next attempt
  }

  // Try tenant details and infer fields if present
  try {
    const url = `${String(base).replace(/\/$/, '')}/tenants/${encodeURIComponent(String(tenantId))}`;
    const resp = await fetch(url, { headers });
    if (resp.ok) {
      const data = await resp.json();
      const name = typeof data?.brand_name === 'string' ? data.brand_name
        : typeof data?.display_name === 'string' ? data.display_name
        : typeof data?.name === 'string' ? data.name
        : undefined;
      const header = Array.isArray(data?.brand_header_lines)
        ? (data.brand_header_lines as unknown[]).map(v => String(v)).map(s => s.trim()).filter(s => s.length > 0)
        : undefined;
      return { ...fallback, brandName: name ?? fallback.brandName, brandHeaderLines: header ?? fallback.brandHeaderLines };
    }
  } catch {
    // ignore
  }

  return fallback;
}
