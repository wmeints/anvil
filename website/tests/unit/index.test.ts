import { experimental_AstroContainer as AstroContainer } from "astro/container";
import { expect, test } from "vitest";
import Index from "../../src/pages/index.astro";

test("the home page links to the docs under the base path", async () => {
  const container = await AstroContainer.create();

  const html = await container.renderToString(Index);

  expect(html).toContain('href="/firebrick/docs/"');
});
