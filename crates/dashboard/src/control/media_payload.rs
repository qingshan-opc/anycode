//! Media attachment payloads and capability-driven delivery routing.
//!
//! The **model registry is the sole authority** on modality support: no
//! runtime substring heuristics (they used to live here, in
//! `catalog_service.rs`, and in a dashboard-ui mirror — see roadmap P1.1).
//! Delivery precedence is strict: native multimodal → degraded modality
//! (video→frames, audio→transcript) → OCR text as the explicit last resort.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionImagePayload {
    pub mime_type: String,
    pub data_base64: String,
}

const MAX_IMAGES: usize = 3;
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Video attachments: sampled frames reuse the image limits; the source clip
/// itself is bounded separately.
pub const MAX_VIDEO_FRAMES: usize = 16;
pub const MAX_VIDEO_BYTES: u64 = 200 * 1024 * 1024;

pub fn validate_vision_payloads(images: &[VisionImagePayload]) -> Result<()> {
    if images.len() > MAX_IMAGES {
        bail!("at most {MAX_IMAGES} vision images per message");
    }
    use base64::Engine;
    for (i, img) in images.iter().enumerate() {
        if img.mime_type.trim().is_empty() {
            bail!("vision image {i}: mime_type is required");
        }
        if img.data_base64.trim().is_empty() {
            bail!("vision image {i}: data_base64 is required");
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(img.data_base64.trim())
            .map_err(|e| anyhow::anyhow!("vision image {i}: invalid base64: {e}"))?;
        if bytes.len() > MAX_IMAGE_BYTES {
            bail!(
                "vision image {i}: exceeds {} MB limit",
                MAX_IMAGE_BYTES / (1024 * 1024)
            );
        }
    }
    Ok(())
}

/// Convert API payloads to core [`VisionImage`] values for embedded chat metadata.
pub fn to_core_images(images: &[VisionImagePayload]) -> Vec<anycode_core::VisionImage> {
    images
        .iter()
        .map(|img| anycode_core::VisionImage::new(img.mime_type.clone(), img.data_base64.clone()))
        .collect()
}

/// Media modality of a chat attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaModality {
    Image,
    Video,
    Audio,
}

/// A media attachment routed through the chat pipeline.
#[derive(Debug, Clone)]
pub struct MediaPayload {
    pub modality: MediaModality,
    /// Image bytes (image modality) or sampled video frames (video modality).
    pub images: Vec<VisionImagePayload>,
    /// Text extracted ahead of routing (audio transcript, subtitle track), if any.
    pub text: Option<String>,
    /// Source media file for native provider upload (when supported).
    pub source_path: Option<PathBuf>,
}

/// How attached media should be delivered to the chat brain.
#[derive(Debug, Clone)]
pub enum MediaDelivery {
    /// Pass media through as native multimodal content (model advertises the modality).
    Native,
    /// Video → sampled frames as vision images (model has Vision but not Video).
    FrameImages,
    /// Audio/video → transcript text appended to the user prompt.
    TranscriptText,
    /// Images → OCR text appended to the user prompt (strict last resort).
    OcrText(String),
    /// No delivery path — caller must surface the guidance message.
    Unsupported,
}

/// Whether the active chat model advertises `cap` in the registry.
/// The registry is the only source of truth — no substring fallback.
pub fn active_chat_has(cap: anycode_llm::capability_catalog::ModelCapability) -> Result<bool> {
    use anycode_llm::ResolvedModelRegistry;
    let (_, cfg) = crate::config_patch::read_config_value(None)?;
    let registry = ResolvedModelRegistry::from_config(&cfg);
    Ok(registry
        .active_item(anycode_llm::capability_catalog::ModelCapability::Chat)
        .is_some_and(|item| item.capabilities.contains(&cap)))
}

/// Whether the active chat model advertises vision / multimodal input.
pub fn active_chat_supports_vision() -> Result<bool> {
    active_chat_has(anycode_llm::capability_catalog::ModelCapability::Vision)
}

/// Whether Apple OCR (or equivalent local helper) can extract text from images.
/// Chat may be text-only; OCR is a delegated capability slot.
pub fn ocr_fallback_available() -> bool {
    use anycode_llm::media::apple_media::{self, NO_EXTRA_PATHS};
    if !apple_media::apple_media_available() {
        return false;
    }
    apple_media::query_capabilities(NO_EXTRA_PATHS)
        .map(|c| c.ocr)
        .unwrap_or(false)
}

/// Accept images when chat has vision **or** OCR fallback can serve the brain.
pub fn can_accept_images_for_chat() -> Result<bool> {
    Ok(active_chat_supports_vision()? || ocr_fallback_available())
}

/// Capability-driven delivery resolution. Precedence is strict:
/// 1. **Native** — the chat model advertises the attachment's modality.
/// 2. **Degraded modality** — video→`FrameImages` (needs Vision), audio/video
///    with a transcript→`TranscriptText`.
/// 3. **OcrText** — images only, when Apple OCR is available (explicit last
///    resort per product wish #3: 非必要不调用本机 OCR).
/// 4. **Unsupported** — hard reject with guidance.
pub fn resolve_media_delivery(payload: &MediaPayload) -> Result<MediaDelivery> {
    use anycode_llm::capability_catalog::ModelCapability as Cap;
    match payload.modality {
        MediaModality::Image => {
            if payload.images.is_empty() || active_chat_supports_vision()? {
                return Ok(MediaDelivery::Native);
            }
            if !ocr_fallback_available() {
                return Ok(MediaDelivery::Unsupported);
            }
            let text = ocr_images_to_text(&payload.images)?;
            if text.trim().is_empty() {
                return Ok(MediaDelivery::Unsupported);
            }
            Ok(MediaDelivery::OcrText(text))
        }
        MediaModality::Video => {
            if active_chat_has(Cap::Video)? {
                return Ok(MediaDelivery::Native);
            }
            if !payload.images.is_empty() && active_chat_supports_vision()? {
                return Ok(MediaDelivery::FrameImages);
            }
            if payload.text.as_ref().is_some_and(|t| !t.trim().is_empty()) {
                return Ok(MediaDelivery::TranscriptText);
            }
            Ok(MediaDelivery::Unsupported)
        }
        MediaModality::Audio => {
            if active_chat_has(Cap::AudioInput)? {
                return Ok(MediaDelivery::Native);
            }
            if payload.text.as_ref().is_some_and(|t| !t.trim().is_empty()) {
                return Ok(MediaDelivery::TranscriptText);
            }
            Ok(MediaDelivery::Unsupported)
        }
    }
}

/// User-facing guidance when [`MediaDelivery::Unsupported`] is resolved.
pub fn unsupported_delivery_guidance(modality: MediaModality) -> String {
    match modality {
        MediaModality::Image => {
            "Active chat model does not support vision, and OCR is unavailable. \
             Enable Apple OCR (Desktop) or switch chat to a vision-capable model."
                .into()
        }
        MediaModality::Video => {
            "Active chat model cannot understand video (no native video, no vision \
             for frames, no transcript). Switch chat to a video- or vision-capable model."
                .into()
        }
        MediaModality::Audio => {
            "Active chat model cannot understand audio and no transcript is available. \
             Switch chat to an audio-capable model or attach a transcript."
                .into()
        }
    }
}

/// Legacy image-only wrapper kept for the existing dispatch funnel: Native or
/// OCR text; hard-bails with guidance when neither path exists.
#[derive(Debug, Clone)]
pub enum VisionDelivery {
    /// Pass images through as multimodal content (chat supports vision).
    Native,
    /// OCR text to append to the user prompt; do not send raw images to chat.
    OcrText(String),
}

/// Resolve image delivery for the active chat model.
/// Text-only brains (e.g. DeepSeek Flash) get OCR text instead of a hard reject.
pub fn resolve_vision_delivery(images: &[VisionImagePayload]) -> Result<VisionDelivery> {
    if images.is_empty() {
        return Ok(VisionDelivery::Native);
    }
    validate_vision_payloads(images)?;
    let payload = MediaPayload {
        modality: MediaModality::Image,
        images: images.to_vec(),
        text: None,
        source_path: None,
    };
    match resolve_media_delivery(&payload)? {
        MediaDelivery::Native => Ok(VisionDelivery::Native),
        MediaDelivery::OcrText(text) => Ok(VisionDelivery::OcrText(text)),
        _ => bail!(unsupported_delivery_guidance(MediaModality::Image)),
    }
}

/// Run local OCR on each image and return a prompt appendix for the chat brain.
pub fn ocr_images_to_text(images: &[VisionImagePayload]) -> Result<String> {
    use anycode_llm::media::apple_media::{self, NO_EXTRA_PATHS};
    use base64::Engine;
    validate_vision_payloads(images)?;
    let mut parts = Vec::with_capacity(images.len());
    for (i, img) in images.iter().enumerate() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(img.data_base64.trim())
            .map_err(|e| anyhow::anyhow!("vision image {i}: invalid base64: {e}"))?;
        let text = apple_media::ocr_image_bytes(NO_EXTRA_PATHS, &img.mime_type, &bytes, None)
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "OCR failed or returned empty text for image {} (mime={})",
                    i + 1,
                    img.mime_type
                )
            })?;
        parts.push(format!("[image {}]\n{}", i + 1, text.trim()));
    }
    Ok(parts.join("\n\n"))
}

pub fn append_ocr_to_prompt(prompt: &str, ocr_text: &str) -> String {
    let mut out = prompt.to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(
        "--- OCR from attached images (chat model is text-only; OCR capability used) ---\n",
    );
    out.push_str(ocr_text.trim());
    out
}

/// Append a media transcript (audio STT / video subtitles) to the user prompt.
pub fn append_transcript_to_prompt(prompt: &str, label: &str, transcript: &str) -> String {
    let mut out = prompt.to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&format!("--- Transcript from attached {label} ---\n"));
    out.push_str(transcript.trim());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn rejects_oversized_payload() {
        let huge = "a".repeat(MAX_IMAGE_BYTES + 1);
        use base64::engine::general_purpose::STANDARD;
        let encoded = STANDARD.encode(huge.as_bytes());
        let err = validate_vision_payloads(&[VisionImagePayload {
            mime_type: "image/png".into(),
            data_base64: encoded,
        }])
        .unwrap_err();
        assert!(err.to_string().contains("limit"));
    }

    #[test]
    fn to_core_images_maps_payloads() {
        let core = to_core_images(&[VisionImagePayload {
            mime_type: "image/jpeg".into(),
            data_base64: "abc123".into(),
        }]);
        assert_eq!(core.len(), 1);
        assert_eq!(core[0].mime_type, "image/jpeg");
        assert_eq!(core[0].data_base64, "abc123");
    }

    #[test]
    fn append_ocr_to_prompt_labels_capability_path() {
        let out = append_ocr_to_prompt("请看图", "[image 1]\n你好");
        assert!(out.contains("请看图"));
        assert!(out.contains("OCR from attached images"));
        assert!(out.contains("你好"));
    }

    #[test]
    fn append_transcript_labels_source() {
        let out = append_transcript_to_prompt("总结这段视频", "video", "大家好…");
        assert!(out.contains("Transcript from attached video"));
        assert!(out.contains("大家好"));
    }

    #[test]
    fn unsupported_guidance_covers_all_modalities() {
        for m in [
            MediaModality::Image,
            MediaModality::Video,
            MediaModality::Audio,
        ] {
            assert!(!unsupported_delivery_guidance(m).is_empty());
        }
    }
}
