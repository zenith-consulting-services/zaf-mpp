#!/usr/bin/env bash
# Shallow-clones joniles/mpxj at a pinned commit into tests/data/mpxj/.
# Integration tests read MPP14 sample files from mpxj's junit/data
# directory and compare against values ported from mpxj's own JUnit
# assertions. The clone is gitignored; run this script before `cargo test`
# to enable the integration tests. Without it, they skip with a message.

set -euo pipefail

# Pinned to a specific commit so test fixtures and expected values stay in
# sync with the JUnit assertions this crate's tests were ported from.
MPXJ_COMMIT="d77ad47ee51775f81c368cb0a3eff85fce129afd"
REPO_URL="https://github.com/joniles/mpxj.git"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="$SCRIPT_DIR/../tests/data/mpxj"

if [ -d "$DEST/.git" ]; then
  echo "mpxj checkout already present at $DEST"
  exit 0
fi

rm -rf "$DEST"
mkdir -p "$DEST"

git init -q "$DEST"
git -C "$DEST" remote add origin "$REPO_URL"
git -C "$DEST" fetch -q --depth 1 origin "$MPXJ_COMMIT"
git -C "$DEST" checkout -q FETCH_HEAD

echo "mpxj checked out at $MPXJ_COMMIT in $DEST"
