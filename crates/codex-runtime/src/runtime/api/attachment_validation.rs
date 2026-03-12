use tokio::fs;

use super::{PromptAttachment, PromptRunError};

pub(super) async fn validate_prompt_attachments(
    cwd: &str,
    attachments: &[PromptAttachment],
) -> Result<(), PromptRunError> {
    for attachment in attachments {
        match attachment {
            PromptAttachment::AtPath { path, .. }
            | PromptAttachment::LocalImage { path }
            | PromptAttachment::Skill { path, .. } => {
                let resolved = super::resolve_attachment_path(cwd, path);
                if fs::metadata(&resolved).await.is_err() {
                    return Err(PromptRunError::AttachmentNotFound(
                        resolved.to_string_lossy().to_string(),
                    ));
                }
            }
            PromptAttachment::ImageUrl { .. } => {}
        }
    }
    Ok(())
}

pub(super) async fn hook_attachment_path_exists(cwd: &str, path: &str) -> bool {
    let resolved = super::resolve_attachment_path(cwd, path);
    fs::metadata(&resolved).await.is_ok()
}
