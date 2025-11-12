#!/bin/bash
set -e

# Script to create a new release
# Usage: ./scripts/release.sh 1.0.0

if [ $# -eq 0 ]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 1.0.0"
    exit 1
fi

VERSION=$1
echo "Creating release for version ${VERSION}"

# Validate version format (basic check)
if ! [[ $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: Version must be in format X.Y.Z (e.g., 1.0.0)"
    exit 1
fi

# Check if we're on main/master branch
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "main" && "$BRANCH" != "master" ]]; then
    echo "Warning: You are not on main/master branch (current: $BRANCH)"
    read -p "Continue? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Check if working directory is clean
if [[ -n $(git status -s) ]]; then
    echo "Error: Working directory is not clean. Please commit or stash your changes."
    git status -s
    exit 1
fi

# Update version in Cargo.toml
echo "Updating Cargo.toml..."
sed -i.bak "s/^version = .*/version = \"${VERSION}\"/" Cargo.toml
rm Cargo.toml.bak

# Update Cargo.lock
echo "Updating Cargo.lock..."
cargo check

# Show changes
echo ""
echo "Version updated to ${VERSION}:"
grep "^version" Cargo.toml

# Commit changes
echo ""
read -p "Commit and tag version ${VERSION}? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    git add Cargo.toml Cargo.lock
    git commit -m "chore: bump version to ${VERSION}"
    git tag "v${VERSION}"

    echo ""
    echo "✓ Version bumped and tagged as v${VERSION}"
    echo ""
    echo "To push the release, run:"
    echo "  git push && git push origin v${VERSION}"
    echo ""
    echo "Or use the GitHub workflow:"
    echo "  gh workflow run update-version.yml -f version=${VERSION}"
else
    echo "Aborted. Rolling back changes..."
    git checkout Cargo.toml Cargo.lock
fi
