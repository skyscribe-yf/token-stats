# Kimi2 Quota Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-point the existing `kimi_ex` quota fetcher to read credentials from `~/.kimi-code-user2/credentials/kimi-code.json`.

**Architecture:** Change the default fallback path in `get_kimi_credentials_path_ex()` (backend/src/quota/kimi.rs). No API or frontend changes.

**Tech Stack:** Rust (Axum backend), existing test harness with `temp_env`.

---

### Task 1: Update default EX credentials path

**Files:**
- Modify: `backend/src/quota/kimi.rs:24-35`

- [ ] **Step 1: Change the default path in `get_kimi_credentials_path_ex()`**

Replace:
```rust
pub fn get_kimi_credentials_path_ex() -> PathBuf {
    if let Ok(path) = std::env::var("KIMI_CREDENTIALS_PATH_EX") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".kimi")
        .join("credentials")
        .join("kimi-code-ex.json")
}
```

With:
```rust
pub fn get_kimi_credentials_path_ex() -> PathBuf {
    if let Ok(path) = std::env::var("KIMI_CREDENTIALS_PATH_EX") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".kimi-code-user2")
        .join("credentials")
        .join("kimi-code.json")
}
```

- [ ] **Step 2: Commit the path change**

```bash
git add backend/src/quota/kimi.rs
git commit -m "feat: point kimi_ex quota to ~/.kimi-code-user2 credentials"
```

---

### Task 2: Add unit test for the new default path

**Files:**
- Modify: `backend/src/quota/kimi.rs` (append to `mod tests`)

- [ ] **Step 1: Write a test asserting the new default path**

Add inside `#[cfg(test)] mod tests` near the other env-var tests:

```rust
#[test]
fn test_get_kimi_credentials_path_ex_default() {
    temp_env::with_var("KIMI_CREDENTIALS_PATH_EX", None::<&str>, || {
        temp_env::with_var("HOME", Some("/tmp/fakehome"), || {
            let path = get_kimi_credentials_path_ex();
            assert_eq!(
                path,
                std::path::PathBuf::from("/tmp/fakehome/.kimi-code-user2/credentials/kimi-code.json")
            );
        });
    });
}
```

- [ ] **Step 2: Run the new test to verify it passes**

```bash
cd backend && cargo test test_get_kimi_credentials_path_ex_default -- --nocapture
```

Expected: `test result: ok. 1 passed; 0 failed`

- [ ] **Step 3: Run the full `kimi` module test suite**

```bash
cd backend && cargo test quota::kimi -- --nocapture
```

Expected: All existing tests still pass.

- [ ] **Step 4: Commit the test**

```bash
git add backend/src/quota/kimi.rs
git commit -m "test: assert kimi_ex credentials default path"
```

---

### Task 3: Build and sanity-check

**Files:**
- Modify: none (verification only)

- [ ] **Step 1: Build the backend in release mode**

```bash
cd backend && cargo build --release
```

Expected: Clean build, zero errors.

- [ ] **Step 2: Start the backend and hit the quota endpoint**

```bash
cd backend && ./target/release/token-stats-backend &
sleep 2
curl -s http://localhost:3000/api/quota | jq '.kimi_ex'
```

Expected: JSON response with `available: true` (assuming `~/.kimi-code-user2/credentials/kimi-code.json` exists and is valid) or `available: false` with an error if the file is missing / token expired.

- [ ] **Step 3: Stop the test server**

```bash
kill %1
```

---

### Self-Review Checklist

- [ ] Spec coverage: The spec requires changing the default EX path to `~/.kimi-code-user2/credentials/kimi-code.json` — Task 1 covers this.
- [ ] No placeholders: Every step has exact code and commands.
- [ ] Type consistency: Uses existing `PathBuf`, `temp_env`, and `std::env` patterns from the same file.
