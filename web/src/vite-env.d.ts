/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_MATCHMAKER_URL: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
