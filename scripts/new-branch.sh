#!/usr/bin/env bash
# Cut a work branch from the latest origin/develop.
#   scripts/new-branch.sh feat-cycles-board          -> STN-<next>/feat-cycles-board
#   scripts/new-branch.sh STN-020/fix-login-loop     -> as given
set -euo pipefail
arg=${1:?usage: scripts/new-branch.sh <type>-<description> | STN-NNN/<type>-<description>}
if [[ -n "$(git status --porcelain)" ]]; then
  echo "✖ Working tree is not clean. Commit or stash first." >&2
  exit 1
fi
git fetch --prune origin
if [[ "$arg" == STN-* ]]; then
  name=$arg
else
  last=$(git branch -a | grep -oE 'STN-[0-9]+' | sed 's/STN-//' | sort -n | tail -1)
  next=$(printf '%03d' $(( 10#${last:-0} + 1 )))
  name="STN-$next/$arg"
fi
"$(dirname "$0")/check-branch.sh" "$name"
git switch -c "$name" origin/develop
echo "✔ On $name (from origin/develop)"
