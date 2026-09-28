#!/usr/bin/env bash
# Prepare a clone of a repository that already contains the .shared subtree.
set -euo pipefail

# Keep in sync with setup.sh.
REMOTE_NAME="shared-dev"
REMOTE_URL="git@github.com:shapedsundew9/shared-dev.git"
PREFIX=".shared"
BRANCH="main"

cd "$(git rev-parse --show-toplevel)"

# 1. Make `git subtree` available when git is built from source without contrib/subtree.
BIN_DIR="$HOME/.local/bin"
if ! git subtree --help >/dev/null 2>&1; then
    for dir in /usr/lib/git-core /usr/libexec/git-core /usr/local/libexec/git-core /usr/share/doc/git/contrib/subtree; do
        if [ -x "$dir/git-subtree" ]; then
            mkdir -p "$BIN_DIR"
            ln -sf "$dir/git-subtree" "$BIN_DIR/git-subtree"
            echo "Linked git-subtree into $BIN_DIR."
            break
        fi
    done
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) export PATH="$BIN_DIR:$PATH" ;;
    esac
fi

if ! git subtree --help >/dev/null 2>&1; then
    echo "Error: 'git subtree' is not available. Install git-subtree (e.g. the 'git' apt package)." >&2
    exit 1
fi

# 2. Remotes are not cloned, so re-add the subtree remote.
if ! git remote | grep -qx "$REMOTE_NAME"; then
    git remote add "$REMOTE_NAME" "$REMOTE_URL"
    echo "Added remote '$REMOTE_NAME' ($REMOTE_URL)."
fi

# Repo-local shortcuts: `git shared-pull` and `git shared-push`
SYNC_ENV="python3 $PREFIX/tools/scripts/sync_devcontainer_env.py --if-opted-in"
git config alias.shared-pull "!git subtree pull --prefix=$PREFIX $REMOTE_NAME $BRANCH --squash && $SYNC_ENV"
git config alias.shared-push "subtree push --prefix=$PREFIX $REMOTE_NAME $BRANCH"

# 3. subtree pull/push need the remote history; SSH may be unavailable during container creation.
if ! git fetch --quiet "$REMOTE_NAME"; then
    echo "Warning: could not fetch '$REMOTE_NAME'. Run 'git fetch $REMOTE_NAME' before subtree pull/push." >&2
fi

# 4. Catch compose changes that arrived without `git shared-pull` (takes effect on the next rebuild).
$SYNC_ENV
