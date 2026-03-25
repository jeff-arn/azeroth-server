#!/bin/bash
# release.sh — Bump version, tag, and post release notes to Discord.
#
# Usage:
#   ./scripts/release.sh           Auto-detect bump type from commits since last tag
#   ./scripts/release.sh patch     Force a patch bump (+0.0.1)
#   ./scripts/release.sh minor     Force a minor bump (+0.1.0)
#   ./scripts/release.sh --dry-run Show what would happen without doing anything

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DISCORD_BOT_DIR="$PROJECT_DIR/discord-bot"

# Load secrets
if [ -f "$PROJECT_DIR/.env" ]; then
    set -a
    source "$PROJECT_DIR/.env"
    set +a
fi

cd "$PROJECT_DIR"

DRY_RUN=false
FORCE_BUMP=""

for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=true ;;
        patch|minor) FORCE_BUMP="$arg" ;;
        *) echo "Usage: $0 [patch|minor] [--dry-run]"; exit 1 ;;
    esac
done

# Get latest tag or default to v0.4.0
LATEST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "v0.4.0")
echo "Current version: $LATEST_TAG"

# Parse version components
VERSION="${LATEST_TAG#v}"
IFS='.' read -r MAJOR MINOR PATCH <<< "$VERSION"

# Determine bump type from commits since last tag
if [ -n "$FORCE_BUMP" ]; then
    BUMP_TYPE="$FORCE_BUMP"
else
    # Check commits since last tag (or all commits if no tag exists)
    if git rev-parse "$LATEST_TAG" >/dev/null 2>&1; then
        COMMITS=$(git log "$LATEST_TAG"..HEAD --pretty=format:"%s")
    else
        COMMITS=$(git log --pretty=format:"%s")
    fi

    if [ -z "$COMMITS" ]; then
        echo "No new commits since $LATEST_TAG. Nothing to release."
        exit 0
    fi

    BUMP_TYPE=""
    if echo "$COMMITS" | grep -qE '^feat(\(.*\))?!?:'; then
        BUMP_TYPE="minor"
    elif echo "$COMMITS" | grep -qE '^fix(\(.*\))?!?:'; then
        BUMP_TYPE="patch"
    fi

    if [ -z "$BUMP_TYPE" ]; then
        echo "No feat or fix commits since $LATEST_TAG. No version bump needed."
        echo ""
        echo "Commits since last tag:"
        if git rev-parse "$LATEST_TAG" >/dev/null 2>&1; then
            git log "$LATEST_TAG"..HEAD --pretty=format:"  %s"
        else
            git log --pretty=format:"  %s"
        fi
        echo ""
        echo "Use '$0 patch' or '$0 minor' to force a bump."
        exit 0
    fi
fi

# Calculate new version
case "$BUMP_TYPE" in
    patch) NEW_PATCH=$((PATCH + 1)); NEW_VERSION="$MAJOR.$MINOR.$NEW_PATCH" ;;
    minor) NEW_MINOR=$((MINOR + 1)); NEW_VERSION="$MAJOR.$NEW_MINOR.0" ;;
esac

NEW_TAG="v$NEW_VERSION"

# Build release notes from commits
if git rev-parse "$LATEST_TAG" >/dev/null 2>&1; then
    COMMIT_LOG=$(git log "$LATEST_TAG"..HEAD --pretty=format:"- %s")
else
    COMMIT_LOG=$(git log --pretty=format:"- %s")
fi

RELEASE_NOTES="**$NEW_TAG**

$COMMIT_LOG"

echo ""
echo "Bump type: $BUMP_TYPE"
echo "New version: $NEW_TAG"
echo ""
echo "Release notes:"
echo "$RELEASE_NOTES"
echo ""

if [ "$DRY_RUN" = true ]; then
    echo "[dry-run] Would tag $NEW_TAG and post to Discord."
    exit 0
fi

# Tag
git tag -a "$NEW_TAG" -m "Release $NEW_TAG"
echo "Tagged $NEW_TAG"

# Post to Discord
echo "Posting release notes to Discord..."
"$DISCORD_BOT_DIR/venv/bin/python3" "$DISCORD_BOT_DIR/post_patch_notes.py" "$RELEASE_NOTES"

echo ""
echo "Release $NEW_TAG complete."
echo "Don't forget to push: git push origin main --tags"
