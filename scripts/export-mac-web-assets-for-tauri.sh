#!/bin/bash
# Exports the Mac app's generated web pages (board, notes, 3D overlay, orb) and
# system prompts into tauri/generated/, so the Tauri app (Windows and Linux)
# uses exactly what the Mac app builds. The Mac sources stay the single source
# of truth: run this on a Mac after changing VisualExplanationPanel.swift,
# Model3DOverlay.swift, VisualToolkit/, a prompt in CompanionManager.swift or
# tauri/prompt-substitutions.json, then commit tauri/generated/.
#
# How: the two page-building Swift files are compiled together with the prompt
# literals (pulled out of CompanionManager.swift) and a tiny main.swift that
# writes everything out. Access is widened from `private static` to `static`
# only in temporary copies; the Mac sources are never modified.
set -euo pipefail

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
mac_sources_directory="$repository_root/macos/leanring-buddy"
build_directory="$(mktemp -d)"
trap 'rm -rf "$build_directory"' EXIT

for page_source_file in VisualExplanationPanel.swift Model3DOverlay.swift; do
    sed -E 's/private static (let|var|func)/static \1/' \
        "$mac_sources_directory/$page_source_file" > "$build_directory/$page_source_file"
done

# Every `private static let <name>SystemPrompt = """ ... """` (and the one-line
# agentContinuationPrompt) becomes a member of `enum ExportedPrompts`.
# Compiling them as Swift keeps Swift's exact multiline-literal rules and the
# one interpolation (the icon names in the visual prompt).
awk '
    BEGIN { print "enum ExportedPrompts {" }
    /^    private static let [a-zA-Z0-9]*(SystemPrompt|ContinuationPrompt) = / {
        sub(/private static/, "static")
        print
        if ($0 ~ /"""$/) { inside_multiline_prompt = 1 }
        next
    }
    inside_multiline_prompt {
        print
        if ($0 ~ /^    """$/) { inside_multiline_prompt = 0 }
    }
    END { print "}" }
' "$mac_sources_directory/CompanionManager.swift" > "$build_directory/ExportedPrompts.swift"

exported_prompt_count=$(grep -c '^    static let ' "$build_directory/ExportedPrompts.swift" || true)
if [ "$exported_prompt_count" -ne 6 ]; then
    echo "error: expected 6 prompts in CompanionManager.swift, found $exported_prompt_count" >&2
    exit 1
fi

mkdir -p "$build_directory/bin"
# -default-isolation MainActor matches the Xcode build setting
# SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor.
swiftc -swift-version 5 -default-isolation MainActor -O \
    "$build_directory/VisualExplanationPanel.swift" \
    "$build_directory/Model3DOverlay.swift" \
    "$build_directory/ExportedPrompts.swift" \
    "$repository_root/scripts/export-mac-web-assets/main.swift" \
    -o "$build_directory/bin/export-mac-web-assets"

# An unbundled executable's Bundle.main finds resources next to the binary,
# so the Swift code's bundledResourceData(named:) works unchanged.
cp "$mac_sources_directory/VisualToolkit/"* "$build_directory/bin/"

"$build_directory/bin/export-mac-web-assets" "$repository_root"
echo "exported to $repository_root/tauri/generated"
