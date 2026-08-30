import "@testing-library/jest-dom/vitest";

// Node 25 exposes an incomplete global localStorage when no persistence file was
// configured. That value can shadow jsdom's Storage implementation in workers.
if (
  typeof window !== "undefined" &&
  typeof window.localStorage?.clear !== "function"
) {
  const values = new Map<string, string>();
  const storage = Object.create(null) as Storage;
  Object.defineProperties(storage, {
    length: { configurable: true, get: () => values.size },
    clear: {
      configurable: true,
      value: () => {
        for (const key of values.keys()) delete (storage as unknown as Record<string, unknown>)[key];
        values.clear();
      },
    },
    getItem: { configurable: true, value: (key: string) => values.get(key) ?? null },
    key: { configurable: true, value: (index: number) => [...values.keys()][index] ?? null },
    removeItem: {
      configurable: true,
      value: (key: string) => {
        values.delete(key);
        delete (storage as unknown as Record<string, unknown>)[key];
      },
    },
    setItem: {
      configurable: true,
      value: (key: string, value: string) => {
        const stored = String(value);
        values.set(key, stored);
        Object.defineProperty(storage, key, {
          configurable: true,
          enumerable: true,
          get: () => values.get(key),
        });
      },
    },
  });
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: storage,
  });
}
