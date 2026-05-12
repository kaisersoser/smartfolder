# smartfolder Development Standards

## Mandatory Requirements

### 1. Comprehensive Code Documentation

Every code file, function, type, and module must be documented. This is **not optional**.

**Documentation Requirements:**
- ✅ Module-level doc comments (`//!`) explaining purpose and overview
- ✅ Every public function documented with purpose, logical flow, errors, and examples
- ✅ Every public struct/enum documented with field/variant descriptions
- ✅ Clear, concise language; explain *why* and *what*
- ✅ Logical flow diagrams for complex operations (numbered steps)
- ✅ Error conditions explicitly documented

**See CONTRIBUTING.md for detailed documentation standards.**

### 2. Code Quality

Before any commit:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --no-deps --lib  # Must succeed with no warnings
```

### 3. Safety Constraints

**Must preserve for v1:**
- No file content reading (metadata only)
- Preview all moves before execution
- Require explicit confirmation
- Never write outside root directory
- Never follow symlinks by default
- Maintain reliable undo/journal behavior

## Architecture Overview

**Three-Phase Workflow:**

1. **Scanner** → Collect file metadata (names, sizes, timestamps, types)
2. **Planner** → Apply rules to generate organization plan with conflict detection
3. **Executor** → Apply plan safely with transaction journaling

**Key Modules:**
- `scanner.rs`: Recursive directory traversal
- `planner.rs`: Rule matching and operation generation
- `apply.rs`: Safe execution with journals
- `recovery.rs`: Undo and rollback
- `rules.rs`: Built-in and custom rules
- `model.rs`: Data structures
- `paths.rs`: Path safety validation
- `storage.rs`: Persistent storage
- `error.rs`: Error types
- `cli/main.rs`: Command-line interface

## Documentation Examples

### Module Documentation

```rust
//! Directory scanning with metadata collection.
//!
//! Recursively traverses directories collecting file metadata (names, sizes,
//! timestamps, types) without reading file contents.
//!
//! # Features
//!
//! - Filtering by depth, hidden files, project folders
//! - Cancellable scans via `CancellationToken`
//! - Detailed warnings for unreadable entries
```

### Function Documentation

```rust
/// Generate a plan from a scan result and planning options.
///
/// # Logical Flow
///
/// 1. For each file in the scan:
///    - Apply rule matching to determine destination
///    - Check if destination path is safe
/// 2. Detect conflicts (exists, case-only, path too long)
/// 3. Mark conflicting operations as "not selected"
/// 4. Generate summary statistics
/// 5. Return complete plan
///
/// # Errors
///
/// Returns error for IO failures or invalid paths.
pub fn generate_plan(
    root: impl AsRef<Path>,
    scan: &ScanResult,
    options: &PlanOptions,
) -> Result<PlanRecord> {
```

### Type Documentation

```rust
/// Transaction record for auditing and undo.
///
/// Created when a plan is applied, tracks:
/// - Which operations were attempted
/// - Outcome of each operation (success, skip, failure)
/// - Errors encountered
/// - Overall transaction status
///
/// Journaled to disk for reliable undo support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionJournal {
    pub schema_version: u16,
    pub transaction_id: String,
    pub plan_id: String,
    pub root: PathBuf,
    pub status: TransactionStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub operations: Vec<TransactionOperation>,
}
```

## Code Review Checklist

- [ ] Module has doc comment with purpose
- [ ] All public functions documented
- [ ] All public types documented
- [ ] Documentation explains *why* and *what*
- [ ] Complex functions include step-by-step logical flow
- [ ] Error conditions documented
- [ ] `cargo doc --no-deps` succeeds
- [ ] Code passes `cargo clippy` and `cargo fmt`
- [ ] Tests pass: `cargo test --workspace`
- [ ] Safety constraints preserved

## Violations

Failing to document code or ignoring these standards will result in:
1. Pull request rejection
2. Request for resubmission with proper documentation
3. No merges without comprehensive documentation

**This is non-negotiable.**

---

*Last updated: May 12, 2026*
*Applies to all contributions, all commits, all code.*
