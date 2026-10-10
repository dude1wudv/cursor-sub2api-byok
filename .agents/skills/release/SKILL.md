---
name: release
description: Validate and publish this fork's Windows x64 portable prerelease assets.
---

# Cursor Sub2API BYOK portable release

This fork publishes to `dude1wudv/cursor-sub2api-byok`. Preserve the fixed upstream attribution and MIT license; do not publish to upstream. The user's explicit release instruction authorizes commit, push, tag and prerelease creation for this repository. Never replace an existing release or tag without separate authorization.

1. Read repository instructions, inspect root and submodule status, preserve unrelated work. GitHub network Git uses `HTTP_PROXY=http://127.0.0.1:7890`, `HTTPS_PROXY=http://127.0.0.1:7890`, `GIT_TERMINAL_PROMPT=0`; stop if proxy fails.
2. Keep desktop package.json, Cargo.toml, tauri.conf.json and Cargo.lock versions aligned. A prerelease tag equals `v<version>`. Leave the independent server crate version unchanged.
3. Run necessary workspace tests, clippy with warnings denied, frontend check, and Windows Tauri `--no-bundle` build. `scripts/build-portable.ps1` assembles EXE, ZIP, LICENSE, THIRD-PARTY-NOTICES.txt and SHA256SUMS.txt. Release-level sums must also include the ZIP. Do not publish updater manifests, installer bundles or signing keys.
4. Record code/build, isolated recovery and real Cursor/model/tool verification separately in docs/VALIDATION.md. Fixtures never establish a real Cursor subagent success. Do not spend real model usage outside the user's authorized model/count/input scope.
5. Inspect staged changes for secrets and unintended files; commit and push only the target project. Publish a new public GitHub prerelease (`--prerelease`), verify its assets and hashes. No upstream updater workflow is used.
6. Recheck running Cursor/controller processes before local replacement. Never kill the user's processes or overwrite a running EXE. Preserve the existing desktop shortcut target and local data; verify installed EXE hash.
7. Commit and push the root submodule pointer only after the submodule commit is pushed, preserving unrelated root changes.
