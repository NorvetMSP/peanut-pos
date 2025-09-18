import { useCallback, useEffect, useMemo, useState } from 'react';
import { useAuth } from '../AuthContext';

export type Product = {
  id: string;
  name: string;
  price: number;
  description: string;
  active: boolean;
  sku?: string | null;
  image_url?: string | null;
  category?: string | null;
};

type UseProductsResult = {
  products: Product[];
  categories: string[];
  isLoading: boolean;
  isOfflineResult: boolean;
  error: string | null;
  reload: () => Promise<void>;
};

const cacheKeyForTenant = (tenantId: string) => `productCache:${tenantId}`;

const normalizeProducts = (input: unknown): Product[] => {
  if (!Array.isArray(input)) return [];
  const normalized: Product[] = [];
  for (const item of input) {
    if (!item || typeof item !== 'object') continue;
    const record = item as Record<string, unknown>;
    const id = record.id;
    const name = record.name;
    const price = record.price;
    if ((typeof id !== 'string' && typeof id !== 'number') || typeof name !== 'string') continue;
    const numericPrice = Number(price);
    if (!Number.isFinite(numericPrice)) continue;
    const description = typeof record.description === 'string' ? record.description : '';
    const active = record.active !== false;
    const sku = record.sku === null || record.sku === undefined ? null : String(record.sku);
    const imageUrl = record.image_url === null || record.image_url === undefined ? null : String(record.image_url);
    const category = record.category === null || record.category === undefined ? null : String(record.category);
    normalized.push({
      id: typeof id === 'string' ? id : String(id),
      name,
      price: numericPrice,
      description,
      active,
      sku,
      image_url: imageUrl,
      category,
    });
  }
  return normalized;
};

const loadFromCache = (tenantId: string): Product[] | null => {
  try {
    const cachedRaw = window.localStorage.getItem(cacheKeyForTenant(tenantId));
    if (!cachedRaw) return null;
    const parsed = JSON.parse(cachedRaw);
    return normalizeProducts(parsed);
  } catch {
    return null;
  }
};

export const useProducts = (): UseProductsResult => {
  const { currentUser, token } = useAuth();
  const tenantId = useMemo(() => {
    const raw = currentUser?.tenant_id;
    if (raw === undefined || raw === null) return null;
    return String(raw);
  }, [currentUser?.tenant_id]);

  const PRODUCT_SERVICE_URL = useMemo(() => {
    const base = import.meta.env.VITE_PRODUCT_SERVICE_URL ?? 'http://localhost:8081';
    return base.replace(/\/$/, '');
  }, []);

  const [products, setProducts] = useState<Product[]>([]);
  const [categories, setCategories] = useState<string[]>(['All']);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [isOfflineResult, setIsOfflineResult] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!tenantId) {
      setProducts([]);
      setCategories(['All']);
    }
  }, [tenantId]);

  const loadProducts = useCallback(async () => {
    if (!tenantId) return;

    setIsLoading(true);
    setError(null);
    setIsOfflineResult(false);

    const headers: HeadersInit = { 'X-Tenant-ID': tenantId };
    if (token) headers.Authorization = `Bearer ${token}`;

    try {
      const response = await fetch(`${PRODUCT_SERVICE_URL}/products`, { headers });
      if (!response.ok) {
        throw new Error(`Failed to fetch products (${response.status})`);
      }
      const raw = await response.json();
      const normalized = normalizeProducts(raw);
      window.localStorage.setItem(cacheKeyForTenant(tenantId), JSON.stringify(normalized));
      const activeProducts = normalized.filter(product => product.active);
      setProducts(activeProducts);
      const derivedCategories = new Set<string>(['All']);
      for (const product of activeProducts) {
        if (product.category && product.category.trim().length > 0) {
          derivedCategories.add(product.category.trim());
        } else if (product.description) {
          const match = product.description.match(/Category:\s*([\w\s]+)/i);
          if (match && match[1]) {
            derivedCategories.add(match[1].trim());
          }
        }
      }
      setCategories(Array.from(derivedCategories));
    } catch (err) {
      console.warn('Unable to load products', err);
      const fallback = loadFromCache(tenantId);
      if (fallback) {
        const activeFallback = fallback.filter(product => product.active);
        setProducts(activeFallback);
        setIsOfflineResult(true);
      } else {
        setProducts([]);
      }
      setError('Product catalog unavailable offline.');
    } finally {
      setIsLoading(false);
    }
  }, [PRODUCT_SERVICE_URL, tenantId, token]);

  useEffect(() => {
    loadProducts().catch(err => console.error('Error loading products', err));
  }, [loadProducts]);

  const reload = useCallback(async () => {
    await loadProducts();
  }, [loadProducts]);

  return {
    products,
    categories,
    isLoading,
    isOfflineResult,
    error,
    reload,
  };
};
