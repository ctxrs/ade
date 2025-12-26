import { defineConfig } from "vitest/config";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  resolve: {
    alias: {
      "@context/types": path.resolve(__dirname, "../../packages/context-types/src/index.ts"),
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    setupFiles: ["./vitest.setup.ts"],
    server: {
      deps: {
        inline: [/^@context\//],
      },
    },
  },
});
