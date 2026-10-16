---
name: create-release
description: Create a version tag and a GitHub Release for macos-kvm-gui with polished notes. Use when the user asks to release, cut a version, tag a release or publish a new version.
---

# Releasing macos-kvm-gui

Pushing a `v*` tag triggers `.github/workflows/release.yml`, which tests, builds the binary with the tag as its version, packages it and creates the GitHub Release. This skill covers the tag and the release notes.

## Before you start

```bash
gh auth status            # needs repo scope
git status                # must be clean
git fetch origin --tags
git checkout main
git pull --ff-only
make test                 # gofmt, vet and race tests must pass locally
```

## Versioning

Semantic versioning with a `v` prefix: `v0.MINOR.PATCH` until the first stable release.

- `feat:` commits since the last tag → bump MINOR (`v0.1.3` → `v0.2.0`).
- Only `fix:`, `perf:`, `refactor:`, `ci:`, `docs:` or `chore:` → bump PATCH (`v0.1.3` → `v0.1.4`).
- Release titles also use the `v` prefix.

```bash
LAST=$(git describe --tags --abbrev=0)
git log "$LAST"..HEAD --oneline
```

## Process

### 1. Review and categorize the changes

```bash
git log "$LAST"..HEAD --oneline
git log "$LAST"..HEAD --stat
```

Sections, in this order; omit empty ones:

| Section | Prefixes |
|---|---|
| **✨ New Features** | `feat:` |
| **🔧 Improvements** | `fix:`, `perf:`, `refactor:`, `ci:` |
| **🧹 Cleanup** | `chore:`, `cleanup:` |
| **📚 Documentation** | `docs:`, `doc:` |

### 2. Tag and push

```bash
NEXT=v0.X.Y
git tag -a "$NEXT" -m "Release $NEXT"
git push origin "$NEXT"
```

### 3. Watch the workflow

```bash
gh run list --workflow release.yml --limit 3
gh run watch "$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
gh release view "$NEXT"
```

The workflow attaches:

| Artifact | Notes |
|---|---|
| `macos-kvm-gui-vX.Y.Z-linux-amd64.tar.gz` | binary, README, LICENSE, THIRD_PARTY_NOTICES.md |
| `macos-kvm-gui_X.Y.Z_amd64.deb` | `/usr/bin/macos-kvm-gui`, menu entry, scalable icon; depends on qemu-system-x86, qemu-utils, pkexec |
| `*.sha256` | one per artifact |

### 4. Replace the generated notes

The workflow publishes GitHub's auto-generated notes. Replace them with the categorized ones. Always keep the home lab notice and say what was validated on real hardware.

~~~bash
cat > /tmp/release-notes-$NEXT.md <<'EOF'
> **For home lab use.** macOS is Apple's software, and its license only allows it on Apple-branded hardware.

## ✨ New Features

- ...

## 🔧 Improvements

- ...

## 📚 Documentation

- ...

## Install

```sh
sudo apt install ./macos-kvm-gui_X.Y.Z_amd64.deb   # menu entry "macOS KVM"
# or
tar -xzf macos-kvm-gui-vX.Y.Z-linux-amd64.tar.gz && ./macos-kvm-gui-vX.Y.Z-linux-amd64/macos-kvm-gui
```

Requires Linux x86_64 with glibc 2.38+ (Ubuntu 24.04+, Debian 13+) and KVM.

**Validated:** <what was booted end to end, e.g. macOS Sonoma 14.8.9 on Ubuntu 24.04 / QEMU 8.2.2>

**Full Changelog**: https://github.com/jniltinho/macos-kvm-gui/compare/vPREV...vNEXT
EOF
gh release edit "$NEXT" --title "$NEXT" --notes-file /tmp/release-notes-$NEXT.md
~~~

### 5. Verify

```bash
gh release view "$NEXT" --json name,assets --jq '.name, (.assets[].name)'
gh release download "$NEXT" -D /tmp/check-$NEXT && (cd /tmp/check-$NEXT && sha256sum -c *.sha256)
```

Check that:
- the title is `$NEXT`;
- four assets are attached: the `.tar.gz`, the `.deb` and one `.sha256` for each;
- the checksums pass and the binary opens (`./macos-kvm-gui` shows "macOS KVM vX.Y.Z" in the title bar);
- the Full Changelog link covers the right range.

## Building a release locally

`make package VERSION=v0.X.Y` produces the same files in `dist/` without publishing anything.
