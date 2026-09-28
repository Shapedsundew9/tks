#!/usr/bin/env bash
set -euo pipefail

# Configuration (keep in sync with configure-subtree.sh)
REMOTE_NAME="shared-dev"
REMOTE_URL="git@github.com:shapedsundew9/shared-dev.git"
PREFIX=".shared"
BRANCH="main"

# 1. Ensure inside a git repository and operate from root
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "Error: Not inside a Git repository." >&2
    exit 1
fi
cd "$(git rev-parse --show-toplevel)"

# 2. Check for uncommitted changes
if ! git diff-index --quiet HEAD -- 2>/dev/null; then
    echo "Error: Working directory has uncommitted changes. Commit or stash them first." >&2
    exit 1
fi

# 3. Add remote alias if not already registered
if git remote | grep -qx "$REMOTE_NAME"; then
    echo "Notice: Remote '$REMOTE_NAME' already exists."
else
    echo "Adding remote '$REMOTE_NAME' ($REMOTE_URL)..."
    git remote add -f "$REMOTE_NAME" "$REMOTE_URL"
fi

# Repo-local shortcuts: `git shared-pull` and `git shared-push`
SYNC_ENV="python3 $PREFIX/tools/scripts/sync_devcontainer_env.py --if-opted-in"
git config alias.shared-pull "!git subtree pull --prefix=$PREFIX $REMOTE_NAME $BRANCH --squash && $SYNC_ENV"
git config alias.shared-push "subtree push --prefix=$PREFIX $REMOTE_NAME $BRANCH"

# 4. Check if directory or tree already exists
if [ -d "$PREFIX" ] || git ls-tree -d HEAD "$PREFIX" 2>/dev/null | grep -q "$PREFIX"; then
    echo "Notice: Directory '$PREFIX' already exists. Subtree is already initialized."
    exit 0
fi

# 5. Ensure pull.rebase preserves merge commits
# If pull.rebase is set to true (boolean), standard git rebase will drop merge commits
# when pulling or using VS Code's "Sync", causing the subtree merge to be flattened into root.
# Setting it to 'merges' makes git pull use --rebase-merges to preserve subtree merges.
CURRENT_REBASE="$(git config pull.rebase 2>/dev/null || true)"
if [ "$CURRENT_REBASE" = "true" ]; then
    echo "Notice: Setting local repo 'pull.rebase' to 'merges' to preserve subtree merge commits."
    git config pull.rebase merges
fi

# 6. Ensure git-subtree is available (kept in sync with configure-subtree.sh; inlined for curl | bash)
BIN_DIR="$HOME/.local/bin"
if ! git subtree --help >/dev/null 2>&1; then
    for dir in /usr/lib/git-core /usr/libexec/git-core /usr/local/libexec/git-core /usr/share/doc/git/contrib/subtree; do
        if [ -x "$dir/git-subtree" ]; then
            mkdir -p "$BIN_DIR"
            ln -sf "$dir/git-subtree" "$BIN_DIR/git-subtree"
            echo "Notice: Linked git-subtree into $BIN_DIR."
            break
        fi
    done
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) export PATH="$BIN_DIR:$PATH" ;;
    esac
fi

if ! git subtree --help >/dev/null 2>&1; then
    echo "Error: 'git subtree' is not available. Please install git-subtree or ensure it is in your PATH." >&2
    exit 1
fi

# 7. Fetch latest commits from remote
echo "Fetching latest refs from '$REMOTE_NAME/$BRANCH'..."
git fetch "$REMOTE_NAME" "$BRANCH"

# 8. Add subtree using --squash
echo "Adding subtree into '$PREFIX'..."
git subtree add --prefix="$PREFIX" "$REMOTE_NAME" "$BRANCH" --squash

echo "Success: Subtree initialized at '$PREFIX'."
