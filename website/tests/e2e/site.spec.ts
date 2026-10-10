import { expect, test, type Page } from "@playwright/test";

const base = "/firebrick/";

// Visits every page reachable from the home page and returns their paths.
// Fails on the first internal link that doesn't resolve.
async function crawl(page: Page): Promise<string[]> {
  const visited = new Set<string>();
  const queue = [base];
  while (queue.length > 0) {
    const path = queue.shift() as string;
    if (visited.has(path)) continue;
    visited.add(path);
    const response = await page.goto(path);
    expect(response?.status(), `status of ${path}`).toBe(200);
    if (!response?.headers()["content-type"]?.includes("text/html")) continue;
    queue.push(...(await internalLinks(page)));
  }
  return [...visited];
}

// Returns the paths of the links on the page that stay on the site.
async function internalLinks(page: Page): Promise<string[]> {
  const hrefs = await page
    .locator("a[href]")
    .evaluateAll((links) => links.map((a) => (a as HTMLAnchorElement).href));
  const origin = new URL(page.url()).origin;
  return hrefs
    .map((href) => new URL(href))
    .filter((url) => url.origin === origin)
    .map((url) => url.pathname);
}

test("every internal link resolves under the base path", async ({ page }) => {
  const paths = await crawl(page);

  expect(paths).toContain(`${base}docs/`);
});

test("no page scrolls horizontally on a 375px wide screen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  for (const path of await crawl(page)) {
    await page.goto(path);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    );
    expect(overflow, `horizontal overflow of ${path}`).toBeLessThanOrEqual(0);
  }
});
