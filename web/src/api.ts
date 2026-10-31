export async function api<T>(path: string, options?: RequestInit): Promise<T> {
  const response = await fetch(path, { ...options, credentials: 'same-origin', cache: 'no-store', headers: { 'custom-cf': 'bypass-cache', ...(options?.body ? { 'Content-Type': 'application/json' } : {}), ...options?.headers } });
  if (!response.ok) { let message = `요청에 실패했습니다 (${response.status})`; try { message = (await response.json()).error || message; } catch { /* generic message */ } throw new Error(message); }
  if (response.status === 204) return undefined as T;
  return response.json();
}
