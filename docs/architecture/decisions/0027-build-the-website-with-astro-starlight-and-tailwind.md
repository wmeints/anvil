# 27. Build the website with Astro, Starlight and Tailwind

## Status

Accepted

## Context

All of Firebrick's user documentation sits in `README.md`. Firebrick needs a
product website with a landing page (#77) and browsable documentation (#76). The
site must be static, so it can be served from GitHub Pages, and it must make no
third-party requests. The landing page and the docs share one visual design.

The site's dependencies come from npm. Packages on npm can run install scripts
on the machine that installs them, which is a supply-chain risk on developer
machines and in CI.

## Decision

`website/` is an [Astro](https://astro.build/) project with static output.

- [Starlight](https://starlight.astro.build/) renders the docs from Markdown
  under `src/content/docs/docs/`, so they're served under `/docs/` and `/` is
  free for the landing page. It provides the sidebar, search, table of contents,
  code blocks and edit links that the docs design needs.
- [Tailwind CSS](https://tailwindcss.com/) styles the landing page and custom
  components. `@astrojs/starlight-tailwind` connects it to Starlight's theme, so
  the design tokens that #77 adds to `src/styles/global.css` apply to both.
- `starlight-links-validator` fails the build on a broken link in the docs.
- `astro.config.mjs` sets `site` and `base` to the repository's GitHub Pages URL
  from the start, so links that ignore the base path fail the tests before the
  site is deployed.
- [pnpm](https://pnpm.io/) installs the dependencies. pnpm doesn't run the
  install scripts of dependencies unless they're approved by name;
  `website/pnpm-workspace.yaml` sets `strictDepBuilds: true`, so an install
  fails on an unapproved script, and its `allowBuilds` map is empty. `mise.toml`
  pins Node (the active LTS line) and pnpm, so local builds and CI use the same
  versions.

## Consequences

- The site builds to plain files in `website/dist` that any static host can
  serve.
- A dependency that starts needing an install script breaks the install until
  someone approves it in `allowBuilds`, which a review must justify.
- pnpm 11 replaced `onlyBuiltDependencies` with `allowBuilds`; older guides and
  issues that name the old setting mean the same thing.
- Starlight's own components and styles are customized through `customCss` and
  component overrides, so upgrades of Starlight can still change the look and
  need a visual check.
- The repository now has a Node toolchain next to the Rust one.
