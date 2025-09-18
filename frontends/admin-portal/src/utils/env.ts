export type EnvRecord = Record<string, string | undefined>;

const envCache: EnvRecord = (import.meta.env ?? {}) as EnvRecord;

export const getEnvString = (key: string): string | undefined => envCache[key];

export const resolveServiceUrl = (key: string, fallback: string): string => {
  const raw = getEnvString(key);
  const base = typeof raw === 'string' && raw.trim().length > 0 ? raw : fallback;
  return base.replace(/\/$/, '');
};
