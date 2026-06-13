# Contributing

Thanks for improving ctx.

This repository is the public source home for the ctx Agentic Development Environment (ADE).

## What belongs here

- ADE daemon, desktop, and web workbench source
- Source-build documentation for local development
- Public docs and README media from `ctxrs/ctx`
- Issue templates and repository metadata

Deployment-specific services and operations should stay optional for local source builds.

## Working style

- Prefer small, reviewable changes.
- Keep public build paths local-first and source-buildable.
- Do not add maintainer-only infrastructure, signing flows, credential tooling, remote-runner pools, or unpublished artifact systems as required public build steps.
- Do not include generated build outputs, local worktrees, personal paths, secrets, or non-public operational docs.
