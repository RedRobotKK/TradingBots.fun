// build.rs — bakes the current git commit hash into the binary at compile time.
// Accessed in code via: env!("GIT_COMMIT_HASH")
use std::process::Command;

fn main() {
    // Try `git rev-parse --short HEAD` first; fall back to "unknown" if git
    // is unavailable (CI without a git checkout, or shallow clone).
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_COMMIT_HASH={}", hash);

    // Re-run whenever HEAD moves (new commit, branch switch, etc.)
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
}
