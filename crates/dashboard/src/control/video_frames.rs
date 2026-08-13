//! Video understanding pipeline (roadmap P1.2).
//!
//! Upload-time: extract frames (AVFoundation helper on macOS, ffmpeg on PATH
//! as the cross-platform safety net) + pull the audio track through the
//! configured STT stack. Everything is cached in a manifest next to the clip.
//! Send-time: [`resolve_video_attachment`] maps the manifest through
//! capability-driven delivery (native → frames → transcript → reject).
//!
//! No bundled ffmpeg: macOS uses the `anycode-apple-media` Swift helper
//! (AVAssetImageGenerator / AVAssetExportSession); ffmpeg/ffprobe on PATH are
//! only a fallback.

use super::media_payload::{
    append_transcript_to_prompt, resolve_media_delivery, MediaDelivery, MediaModality,
    MediaPayload, VisionImagePayload, MAX_VIDEO_BYTES, MAX_VIDEO_FRAMES,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Longer clips are rejected up front — frame sampling + STT stop being
/// useful beyond this, and token/latency costs explode.
pub const MAX_VIDEO_DURATION_SECS: f64 = 600.0;
const FRAME_DIMENSION: u32 = 768;
/// Roughly one frame per N seconds of footage.
const SECONDS_PER_FRAME: f64 = 2.0;

fn video_attach_root() -> PathBuf {
    crate::cancel_ipc::dashboard_state_dir().join("video-attach")
}

fn video_dir(video_ref: &str) -> Result<PathBuf> {
    if video_ref.is_empty()
        || video_ref.len() > 64
        || !video_ref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("invalid video_ref");
    }
    Ok(video_attach_root().join(video_ref))
}

/// Uniform sampling cadence: ~1 frame per 2s, clamped to [1, MAX_VIDEO_FRAMES].
pub fn frame_count_for_duration(duration_secs: f64) -> usize {
    if !duration_secs.is_finite() || duration_secs <= 0.0 {
        return 1;
    }
    (duration_secs / SECONDS_PER_FRAME)
        .ceil()
        .clamp(1.0, MAX_VIDEO_FRAMES as f64) as usize
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoManifest {
    pub duration_secs: f64,
    pub frame_count: usize,
    pub has_audio: bool,
    pub transcript: Option<String>,
    pub extractor: String,
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

fn load_manifest(video_ref: &str) -> Result<VideoManifest> {
    let dir = video_dir(video_ref)?;
    let text = std::fs::read_to_string(manifest_path(&dir))
        .with_context(|| format!("video {video_ref} not prepared (missing manifest)"))?;
    serde_json::from_str(&text).context("parse video manifest")
}

fn frames_dir(dir: &Path) -> PathBuf {
    dir.join("frames")
}

/// ffmpeg/ffprobe on PATH (never bundled) — the cross-platform fallback.
fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ffprobe_duration(video_path: &Path) -> Result<f64> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(video_path)
        .output()
        .context("run ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .context("parse ffprobe duration")
}

/// ffmpeg fallback: uniform fps sampling + short-side-768 JPEG + 16k mono wav.
fn extract_with_ffmpeg(
    video_path: &Path,
    work: &Path,
) -> Result<(f64, Vec<PathBuf>, Option<PathBuf>)> {
    let duration = ffprobe_duration(video_path)?;
    let frames = frames_dir(work);
    std::fs::create_dir_all(&frames)?;
    let frame_glob = frames.join("frame-%03d.jpg");
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(video_path)
        .args([
            "-vf",
            &format!(
                "fps=1/{SECONDS_PER_FRAME},scale=768:768:force_original_aspect_ratio=increase"
            ),
            "-frames:v",
            &MAX_VIDEO_FRAMES.to_string(),
            "-q:v",
            "5",
        ])
        .arg(&frame_glob)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("run ffmpeg for frames")?;
    if !status.success() {
        bail!("ffmpeg frame extraction failed");
    }
    let mut frame_paths: Vec<PathBuf> = std::fs::read_dir(&frames)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jpg"))
        .collect();
    frame_paths.sort();
    if frame_paths.is_empty() {
        bail!("ffmpeg produced no frames");
    }

    // `-map a?` makes the audio leg optional (no audio track → no wav, no error).
    let wav = work.join("audio.wav");
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(video_path)
        .args(["-map", "a?", "-vn", "-ac", "1", "-ar", "16000"])
        .arg(&wav)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("run ffmpeg for audio")?;
    let audio = if status.success()
        && wav.exists()
        && wav.metadata().map(|m| m.len() > 44).unwrap_or(false)
    {
        Some(wav)
    } else {
        None
    };
    Ok((duration, frame_paths, audio))
}

/// Extract frames + audio: AVFoundation helper first, ffmpeg fallback.
fn extract_frames_and_audio(
    video_path: &Path,
    work: &Path,
) -> Result<(f64, Vec<PathBuf>, Option<PathBuf>, &'static str)> {
    let frames = frames_dir(work);
    match anycode_llm::media::apple_media::extract_video_frames(
        anycode_llm::media::apple_media::NO_EXTRA_PATHS,
        video_path,
        &frames,
        MAX_VIDEO_FRAMES as u32,
        FRAME_DIMENSION,
    ) {
        Ok(hit) if !hit.frames.is_empty() => {
            return Ok((
                hit.duration_secs,
                hit.frames,
                hit.audio_path,
                "avfoundation",
            ));
        }
        Ok(_) => tracing::warn!("apple media video_frames returned zero frames; trying ffmpeg"),
        Err(e) => tracing::debug!("apple media video_frames unavailable: {e}; trying ffmpeg"),
    }
    if !ffmpeg_available() {
        bail!(
            "video frame extraction unavailable: Apple media helper missing and no ffmpeg on PATH"
        );
    }
    let (duration, frames, audio) = extract_with_ffmpeg(video_path, work)?;
    Ok((duration, frames, audio, "ffmpeg"))
}

/// Transcribe an audio file through the configured STT capability slot
/// (apple-speech / builtin whisper / whisper.cpp server — whatever the user
/// enabled). No STT configured → `Ok(None)`, frames alone still deliver.
async fn transcribe_audio_file(audio: &Path) -> Result<Option<String>> {
    use anycode_llm::media::{MediaClientRegistry, SttClient};
    let (_, cfg) = crate::config_patch::read_config_value(None)?;
    let reg = MediaClientRegistry::from_config(&cfg);
    let Some(stt) = reg.stt.as_ref() else {
        return Ok(None);
    };
    let bytes = std::fs::read(audio).context("read audio track")?;
    let filename = audio
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("audio.wav")
        .to_string();
    let client = SttClient::new(stt.profile.clone());
    let result = client
        .transcribe(&bytes, &filename)
        .await
        .map_err(|e| anyhow::anyhow!("STT failed: {e}"))?;
    let text = result.text.trim().to_string();
    Ok((!text.is_empty()).then_some(text))
}

/// Upload-time preparation: validate, extract, transcribe, persist manifest.
/// Returns the manifest for the API response.
pub async fn prepare_video(video_ref: &str, source: &Path) -> Result<VideoManifest> {
    let size = source.metadata().map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        bail!("empty video file");
    }
    if size > MAX_VIDEO_BYTES {
        bail!("video exceeds {} MB limit", MAX_VIDEO_BYTES / (1024 * 1024));
    }
    let dir = video_dir(video_ref)?;
    std::fs::create_dir_all(&dir)?;

    let (duration, frame_paths, audio_path, extractor) = tokio::task::spawn_blocking({
        let source = source.to_path_buf();
        let dir = dir.clone();
        move || extract_frames_and_audio(&source, &dir)
    })
    .await
    .context("join frame extraction")??;
    if duration > MAX_VIDEO_DURATION_SECS {
        let _ = std::fs::remove_dir_all(&dir);
        bail!(
            "video too long ({duration:.0}s > {}s limit)",
            MAX_VIDEO_DURATION_SECS as u64
        );
    }

    let transcript = match audio_path.as_ref() {
        Some(audio) => match transcribe_audio_file(audio).await {
            Ok(t) => t,
            Err(e) => {
                // STT failure degrades to frames-only; never sinks delivery.
                tracing::warn!("video transcript failed, continuing frames-only: {e:#}");
                None
            }
        },
        None => None,
    };

    let manifest = VideoManifest {
        duration_secs: duration,
        frame_count: frame_paths.len(),
        has_audio: audio_path.is_some(),
        transcript,
        extractor: extractor.to_string(),
    };
    std::fs::write(
        manifest_path(&dir),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

/// Register an uploaded clip under a fresh video_ref; returns the ref.
pub fn new_video_ref() -> String {
    format!("vid_{}", uuid::Uuid::new_v4().simple())
}

pub fn video_source_path(video_ref: &str, ext: &str) -> Result<PathBuf> {
    let dir = video_dir(video_ref)?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("source.{ext}")))
}

/// What the send path needs: frames to attach + prompt appendix.
pub struct VideoAttachmentOutcome {
    pub frames: Vec<VisionImagePayload>,
    pub appendix: Option<String>,
}

/// Send-time routing: manifest → capability-driven delivery → frames/appendix.
/// Errors carry user-facing guidance (unsupported delivery, missing upload).
pub fn resolve_video_attachment(video_ref: &str) -> Result<VideoAttachmentOutcome, String> {
    let manifest = load_manifest(video_ref).map_err(|e| format!("video attachment: {e:#}"))?;
    let dir = video_dir(video_ref).map_err(|e| e.to_string())?;
    let frames = read_frame_payloads(&frames_dir(&dir), manifest.frame_count)?;

    let payload = MediaPayload {
        modality: MediaModality::Video,
        images: frames.clone(),
        text: manifest.transcript.clone(),
        source_path: None,
    };
    let delivery = resolve_media_delivery(&payload).map_err(|e| e.to_string())?;
    let appendix = manifest
        .transcript
        .as_deref()
        .map(|t| append_transcript_to_prompt("", "video", t));
    match delivery {
        // Native (chat advertises video) and FrameImages (vision-only brain)
        // both ride the chat transport's image channel with the sampled
        // frames; provider-native file upload (e.g. Gemini Files API) is a
        // deliberate follow-up — see plan P1.2 notes.
        MediaDelivery::Native | MediaDelivery::FrameImages => {
            if frames.is_empty() {
                return Err("video frames missing on disk; re-attach the video".into());
            }
            Ok(VideoAttachmentOutcome { frames, appendix })
        }
        MediaDelivery::TranscriptText => match appendix {
            Some(ap) => Ok(VideoAttachmentOutcome {
                frames: Vec::new(),
                appendix: Some(ap),
            }),
            None => Err("video has no transcript and the chat model cannot see frames".into()),
        },
        _ => Err(super::media_payload::unsupported_delivery_guidance(
            MediaModality::Video,
        )),
    }
}

fn read_frame_payloads(dir: &Path, expected: usize) -> Result<Vec<VisionImagePayload>, String> {
    use base64::Engine;
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("read frames dir: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jpg"))
        .collect();
    paths.sort();
    paths.truncate(expected);
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let bytes = std::fs::read(&p).map_err(|e| format!("read frame {}: {e}", p.display()))?;
        out.push(VisionImagePayload {
            mime_type: "image/jpeg".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_count_scales_with_duration_and_caps() {
        assert_eq!(frame_count_for_duration(0.0), 1);
        assert_eq!(frame_count_for_duration(1.5), 1);
        assert_eq!(frame_count_for_duration(20.0), 10);
        assert_eq!(frame_count_for_duration(31.0), 16);
        assert_eq!(frame_count_for_duration(600.0), MAX_VIDEO_FRAMES);
        assert_eq!(frame_count_for_duration(f64::NAN), 1);
    }

    #[test]
    fn video_ref_rejects_traversal() {
        assert!(video_dir("../etc").is_err());
        assert!(video_dir("a/b").is_err());
        assert!(video_dir("").is_err());
        assert!(video_dir("vid_abc123-def").is_ok());
    }

    #[test]
    fn manifest_roundtrip() {
        let manifest = VideoManifest {
            duration_secs: 12.5,
            frame_count: 7,
            has_audio: true,
            transcript: Some("hello".into()),
            extractor: "ffmpeg".into(),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: VideoManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.frame_count, 7);
        assert_eq!(back.transcript.as_deref(), Some("hello"));
    }

    /// End-to-end extraction: AVFoundation helper on macOS, ffmpeg fallback
    /// elsewhere. Skips when neither extractor is available (CI without media
    /// tooling).
    #[test]
    fn extractor_produces_frames_and_audio_for_a_real_clip() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg not on PATH (cannot synthesize test clip)");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let clip = tmp.path().join("clip.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=3:size=640x360:rate=15",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&clip)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        if !status.success() {
            eprintln!("skip: ffmpeg cannot synthesize a test clip here");
            return;
        }
        let work = tmp.path().join("work");
        let (duration, frames, audio, extractor) =
            extract_frames_and_audio(&clip, &work).expect("extraction should succeed");
        assert!((duration - 3.0).abs() < 1.0, "duration {duration}");
        assert!(!frames.is_empty(), "frames expected via {extractor}");
        assert!(audio.is_some(), "audio track expected via {extractor}");
        let payloads = read_frame_payloads(&frames_dir(&work), frames.len()).unwrap();
        assert_eq!(payloads.len(), frames.len());
        assert!(payloads.iter().all(|p| p.mime_type == "image/jpeg"));
    }
}

/// Best-effort cleanup of a prepared/uploaded video bundle.
pub fn discard_video(video_ref: &str) {
    if let Ok(dir) = video_dir(video_ref) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
