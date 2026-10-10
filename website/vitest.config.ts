/// <reference types="vitest/config" />
import { getViteConfig } from "astro/config";
import astroConfig from "./astro.config.mjs";

// Runs the unit tests through Astro's Vite config, so `.astro` components
// render as they do in the build. Vitest sets `import.meta.env.BASE_URL` to
// `/` itself, so the base path from the Astro config is passed on here.
export default getViteConfig({
  test: {
    include: ["tests/unit/**/*.test.ts"],
    env: { BASE_URL: astroConfig.base ?? "/" },
  },
});
