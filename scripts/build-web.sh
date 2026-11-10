#!/usr/bin/env bash
# Build Plane's three React Router apps as static SPAs into web/apps/<app>/build/client.
# The Saturn binary serves them: web at /, admin at /god-mode, space at /spaces.
set -euo pipefail
cd "$(dirname "$0")/../web"
pnpm install --frozen-lockfile
export VITE_API_BASE_URL="" VITE_WEB_BASE_URL="" VITE_ADMIN_BASE_URL="" VITE_SPACE_BASE_URL="" VITE_LIVE_BASE_URL=""
export VITE_ADMIN_BASE_PATH=/god-mode VITE_SPACE_BASE_PATH=/spaces VITE_LIVE_BASE_PATH=/live
pnpm turbo run build --filter=web --filter=admin --filter=space
