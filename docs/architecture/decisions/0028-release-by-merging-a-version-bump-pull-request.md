# 28. Release by merging a version bump pull request

## Status

Accepted

## Context

A release needs the workspace version in `Cargo.toml` and `Cargo.lock`, a
matching `vX.Y.Z` tag and the release workflow of
[ADR 0001](0001-release-through-github-releases-and-ghcr.md). Doing these by
hand allows a tag on a commit that never passed CI, a tag that doesn't match the
version, or a release with only GitHub's generated list of pull requests as its
notes. GitHub Actions has two limits that shape the automation:

- A pull request opened with the workflow's `GITHUB_TOKEN` doesn't start the CI
  workflow, so a bump PR opened by a workflow would never get its checks without
  a personal access token or a GitHub App.
- A tag pushed with the `GITHUB_TOKEN` doesn't start workflows on the tag push,
  so a workflow that tags can't rely on `release.yaml`'s tag trigger.

## Decision

- The `create-release` skill picks the next version from the conventional
  commits since the last tag: a breaking change or a `feat` commit bumps the
  minor version, anything else the patch version, as long as firebrick is at
  0.x. It opens a PR with the user's `gh` credentials that bumps the version and
  adds the release notes to `CHANGELOG.md`, waits for the checks and merges it.
- `.github/workflows/tag-release.yaml` runs on pushes to main that change
  `Cargo.toml`. When no tag exists for the workspace version, it tags the commit
  and calls `release.yaml` as a reusable workflow with the tag.
- `release.yaml` takes the tag from its input when it's called and from the ref
  when a tag is pushed, and puts the tag's section of `CHANGELOG.md` above the
  generated notes.

## Consequences

- Every release tag points at a merge commit whose PR passed CI, and its version
  matches `Cargo.toml` by construction.
- Releasing needs no secrets beyond the `GITHUB_TOKEN`.
- The release notes are reviewed in the bump PR and kept in the repository.
- A failed release leaves the tag in place, so running `tag-release.yaml` again
  skips it; the failed jobs have to be rerun instead.
- The commit message rules decide the proposed bump, so a `feat` commit for
  developer tooling bumps the minor version unless the user overrides it.
