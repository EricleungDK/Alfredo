/// <reference types="vite/client" />

declare global {
  var __ALFREDO_LOCALHOST_BRIDGE__:
    | {
        readonly endpoint: string;
        readonly token: string;
      }
    | undefined;
}

export {};
