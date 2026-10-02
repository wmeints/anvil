# 1. Use Ubuntu's docker.io package in the base image

## Status

Accepted

## Context

The `anvil-base` image gives coding agents a Docker engine. We can install
Docker from Ubuntu's archive (`docker.io`) or from Docker's own apt repository
(Docker CE).

## Decision

We install Ubuntu's `docker.io` package.

## Consequences

- The image needs no extra apt repository or signing key, and the package
  builds for every architecture Ubuntu supports, including `arm64`.
- Ubuntu ships security fixes for the package with the rest of the archive.
- Docker's version lags behind Docker CE, and plugins such as Compose and
  Buildx aren't included. Derived images can add them when they need them.
