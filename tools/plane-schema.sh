#!/usr/bin/env bash
# Regenerate migrations/0001_schema.sql and 0002_defaults.sql from a Plane checkout.
# Needs PostgreSQL and uv. Saturn itself never runs Python; this is a maintainer tool.
#   tools/plane-schema.sh ../plane
set -euo pipefail
plane=${1:?usage: tools/plane-schema.sh <path-to-plane-checkout>}
here=$(cd "$(dirname "$0")/.." && pwd)
db=saturn_schema_$$
createdb "$db"
trap 'dropdb --if-exists "$db"' EXIT
export SECRET_KEY=x DATABASE_URL=postgres://localhost/$db REDIS_URL=redis://localhost:6379/
uv_version=$(grep -oE 'required-version = "==[0-9.]+"' "$plane/apps/api/pyproject.toml" | grep -oE '[0-9.]+[0-9]')
(cd "$plane/apps/api" && uvx "uv@$uv_version" run python manage.py migrate -v 0)
last=$(psql -d "$db" -Atc "select name from django_migrations where app='db' order by id desc limit 1")
{
  echo "-- Plane PostgreSQL schema (Django migrations up to db $last), dumped with pg_dump --schema-only."
  echo "-- Regenerate with tools/plane-schema.sh when Plane changes its models."
  pg_dump --schema-only --no-owner --no-privileges --no-comments "$db" | grep -v '^SET \|^SELECT pg_catalog\|^--\|^\\' | cat -s
} > "$here/migrations/0001_schema.sql"
(cd "$plane/apps/api" && uvx "uv@$uv_version" run python "$here/tools/django_defaults.py") | grep -v debug_toolbar > "$here/migrations/0002_defaults.sql.new"
echo "wrote migrations/0001_schema.sql; review migrations/0002_defaults.sql.new and merge it into 0002_defaults.sql by hand"
