#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

git -C "$REPO_ROOT" config core.hooksPath "$REPO_ROOT/.githooks"

echo "Git hooks installed: $REPO_ROOT/.githooks"
