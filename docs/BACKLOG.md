# BACKLOG

Tracked improvement items. Each entry includes scope, motivation, implementation steps, and test strategy.

---

## ITEM-1: `SessionConfig::with_cwd()` builder

**Priority:** Low
**Scope:** `crates/coclai/src/runtime/client/profile.rs`

### Motivation

`SessionConfig` has no builder method to change `cwd` after construction. Currently, changing the working directory of an existing config requires:

```rust
// Workaround: must reconstruct from profile
let new_config = SessionConfig::from_profile(new_cwd, old_config.profile());
```

This is unnecessarily verbose for a common ergonomic operation.

### Implementation Plan

1. Add `with_cwd(self, cwd: impl Into<String>) -> Self` to `SessionConfig` in `profile.rs`.

   The implementation is trivial — one field mutation on the same builder pattern:

   ```rust
   /// Replace working directory.
   /// Allocation: one String. Complexity: O(cwd length).
   pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
       self.cwd = cwd.into();
       self
   }
   ```

2. This method does not need to be added to `RunProfile` (which has no `cwd` field) or to the `impl_profile_builder_methods!` macro (which is shared between `RunProfile` and `SessionConfig`).

3. No changes to `WorkflowConfig` are needed — its `cwd` field is part of the struct and can be replaced via `with_run_profile` or direct construction.

### Test Strategy

Add one unit test in `runtime/client/tests.rs`:

```rust
#[test]
fn session_config_with_cwd_replaces_cwd() {
    let config = SessionConfig::new("/original")
        .with_model("gpt-4o")
        .with_cwd("/replacement");
    assert_eq!(config.cwd, "/replacement");
    assert_eq!(config.model, Some("gpt-4o".to_owned()));
}
```

---

## ITEM-2: Derive `KNOWN` from `RPC_CONTRACT_DESCRIPTORS`

**Priority:** Low
**Scope:** `crates/coclai/src/runtime/rpc_contract.rs`

### Motivation

`methods::KNOWN` and `RPC_CONTRACT_DESCRIPTORS` are two arrays that must contain the same 15 method names in the same order. A test (`descriptor_catalog_matches_known_method_catalog`) guards the sync, but the dual-maintenance burden exists at all.

The ideal fix is to derive `KNOWN` from `RPC_CONTRACT_DESCRIPTORS` at compile time so there is only one authoritative source.

### Implementation Plan

**Option A: const array copy (zero runtime cost, no proc macro)**

Replace the current `KNOWN` literal with a `const fn` that builds it from descriptors:

```rust
const fn build_known() -> [&'static str; 15] {
    let mut out = [""; 15];
    let mut i = 0;
    while i < RPC_CONTRACT_DESCRIPTORS.len() {
        out[i] = RPC_CONTRACT_DESCRIPTORS[i].method;
        i += 1;
    }
    out
}

pub const KNOWN: [&'static str; 15] = build_known();
```

This requires:
- `RPC_CONTRACT_DESCRIPTORS` to be defined before `KNOWN` in the file (already the case).
- The `const fn` to appear between the descriptor array and the `KNOWN` constant.
- The `build_known` function to be `pub(super)` or private; it is not part of the public API.

**Option B: remove `KNOWN` entirely**

If `KNOWN` is only used in tests and internally in the validation lookup, it can be removed and callers replaced with `rpc_contract_descriptors().iter().map(|d| d.method)`.

Audit of current `KNOWN` usages:
- `rpc_contract.rs` tests use it in `known_method_catalog_is_stable` and `descriptor_catalog_matches_known_method_catalog`
- `appserver/mod.rs` does **not** re-export `KNOWN` — only individual method name constants are re-exported there

Option A is preferred because `KNOWN` is part of the public API surface (`rpc_contract::methods::KNOWN`) and removing it would be a breaking change.

### Implementation Steps (Option A)

1. In `rpc_contract.rs`, add `build_known` as a private `const fn` immediately before the `KNOWN` constant.
2. Replace the `KNOWN` literal with `KNOWN: [&'static str; 15] = build_known()`.
3. Run `cargo test` to confirm `known_method_catalog_is_stable` and `descriptor_catalog_matches_known_method_catalog` still pass.
4. The `descriptor_catalog_matches_known_method_catalog` test remains valid as a regression guard — its value is now "the derivation logic is correct" rather than "two arrays are in sync".

### Test Strategy

No new tests needed. The existing tests verify the output. After the change, confirm that `methods::KNOWN` is still accessible via `coclai::runtime::rpc_contract::methods::KNOWN` (the public path in `rpc_contract::methods` must remain intact).
