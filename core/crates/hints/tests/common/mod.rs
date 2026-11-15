//! Screens more than one test reads.

/// A coding-agent answer: two-space indent, hard-wrapped by the agent at
/// the pane width (real newlines with the indent repeated).
pub const SHEET: &[&str] = &[
    "  Right, first up, the big one you'd paste into the prompt:",
    "",
    r#"  ! find . -type f -name "*.swift" -not -path "./vendor/*" -not -path "./.dev/*" -exec grep -l"#,
    r#"  "TerminalHint" {} + | xargs -I{} sh -c 'echo "== {}"; wc -l "{}"' | sort -t: -k2 -n | tee"#,
    "  /tmp/hints-report.txt",
    "",
    "  The docs live at",
    "  https://docs.example.invalid/azulejo/hints?section=terminal&lang=en-GB#visible-items, and you",
    "  can find the mirror if you squint hard enough.",
    "",
    "  Plain domain, no scheme: mirror.example.org",
    "",
    r#"  $ cargo build --release --manifest-path shared/Cargo.toml --features "hints,mirror" && ls"#,
    "  -lah target/release",
    "",
    "  The flaky box sits at 192.0.2.47, and its noisy neighbour is at 198.51.100.12 on port 8443",
    "  (198.51.100.12:8443).",
    "",
    "  Session UUID: 3f9c2a7e-8b41-4d6f-a0c3-5e17b9d2f864",
    "",
    "  That bug got squashed in commit 7c3e9a1f4b2d8e05a6f1c9b3d7e2a4f80b5c6d19 (short form",
    "  7c3e9a1).",
    "",
    "  Check out feature/terminal-hints-v2 before you rebase onto release/2026.11, or just stay on",
    "  main if you're feeling brave.",
];
