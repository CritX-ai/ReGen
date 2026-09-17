#!/bin/sh
# Run from your site's root, beside regen.toml and wrangler.jsonc.
# Prerequisites: rustup, a native linker, Node.js 22+, and npm.
# Once locally: sh cloudflare-workers.sh setup
# Commit package.json and package-lock.json before using install in CI.
# CI: install -> build -> dry-run; deploy only in an authorized publishing job.
# Supply CLOUDFLARE_ACCOUNT_ID and a scoped CLOUDFLARE_API_TOKEN through CI
# settings; locally, use Wrangler login instead. Never commit credentials.
# Set site.base_url to the public URL before build; TOML does not expand env vars.
set -eu

case "${1:-}" in
  setup)
    npm install --save-dev --save-exact wrangler@4.129.0
    ;;
  install)
    rustup toolchain install 1.98.1 --profile minimal
    cargo +1.98.1 install regen-ssg --version 1.1.1 --locked
    npm ci
    ;;
  build)
    regen build --site . --profile release
    ;;
  preview)
    # Build first with site.base_url = "http://127.0.0.1:8787/" if desired.
    npx --no-install wrangler dev --local --ip 127.0.0.1 --port 8787
    ;;
  dry-run)
    # Local upload preparation only: no remote routing, DNS or HTTPS check.
    npx --no-install wrangler deploy --dry-run --outdir .wrangler/dry-run
    ;;
  deploy)
    # Replaces this Worker's deployed site. Check account/name before invoking.
    npx --no-install wrangler deploy
    ;;
  *)
    printf '%s\n' 'Usage: sh cloudflare-workers.sh {setup|install|build|preview|dry-run|deploy}' >&2
    exit 2
    ;;
esac
