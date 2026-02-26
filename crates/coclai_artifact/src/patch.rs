use sha2::{Digest, Sha256};

use super::{DocPatch, DomainError, PatchConflict, ValidatedPatch};

pub(crate) fn map_patch_conflict(conflict: PatchConflict) -> DomainError {
    match conflict {
        PatchConflict::RevisionMismatch { expected, actual } => {
            DomainError::Conflict { expected, actual }
        }
        other => DomainError::Validation(other.to_string()),
    }
}

pub fn compute_revision(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Validate structural patch invariants before any mutation.
/// `end_line` is exclusive.
/// Allocation: clones only the validated edit list. Complexity: O(e), e = edit count.
pub fn validate_doc_patch(text: &str, patch: &DocPatch) -> Result<ValidatedPatch, PatchConflict> {
    let current_revision = compute_revision(text);
    if current_revision != patch.expected_revision {
        return Err(PatchConflict::RevisionMismatch {
            expected: patch.expected_revision.clone(),
            actual: current_revision,
        });
    }

    let line_count = line_count(text);
    let mut prev_start = 0usize;
    let mut prev_end = 0usize;

    for (index, edit) in patch.edits.iter().enumerate() {
        if edit.start_line == 0
            || edit.start_line > edit.end_line
            || edit.end_line > line_count.saturating_add(1)
        {
            return Err(PatchConflict::InvalidRange {
                index,
                start_line: edit.start_line,
                end_line: edit.end_line,
                line_count,
            });
        }

        if index > 0 {
            if edit.start_line < prev_start {
                return Err(PatchConflict::NotSorted {
                    index,
                    prev_start,
                    start: edit.start_line,
                });
            }
            if edit.start_line < prev_end {
                return Err(PatchConflict::Overlap {
                    index,
                    prev_end,
                    start: edit.start_line,
                });
            }
        }

        prev_start = edit.start_line;
        prev_end = edit.end_line;
    }

    Ok(ValidatedPatch {
        edits: patch.edits.clone(),
    })
}

/// Apply validated edits in reverse order to avoid index drift.
/// Allocation: line buffer + replacement line buffers. Complexity: O(L + R + e).
pub fn apply_doc_patch(text: &str, validated_patch: &ValidatedPatch) -> String {
    let mut lines = split_lines(text);

    for edit in validated_patch.edits.iter().rev() {
        let start_idx = edit.start_line.saturating_sub(1);
        let end_idx = edit.end_line.saturating_sub(1);
        let replacement = split_lines(&edit.replacement);
        lines.splice(start_idx..end_idx, replacement);
    }

    lines.concat()
}

fn line_count(text: &str) -> usize {
    split_lines(text).len()
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').map(ToOwned::to_owned).collect()
}
