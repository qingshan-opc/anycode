//! Model capability taxonomy (chat vs media modalities).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What a configured model profile is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelCapability {
    Chat,
    /// Multimodal chat input (vision); may share transport with chat.
    Vision,
    /// Native video understanding input (frames/clips ride the chat transport).
    Video,
    /// Native audio understanding input (speech/audio rides the chat transport).
    AudioInput,
    Embedding,
    Stt,
    Tts,
    ImageGen,
    VideoGen,
    /// Reserved for rerank APIs.
    Rerank,
}

impl Serialize for ModelCapability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ModelCapability {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).ok_or_else(|| {
            serde::de::Error::unknown_variant(
                &s,
                &[
                    "chat",
                    "vision",
                    "video_input",
                    "audio_input",
                    "embedding",
                    "stt",
                    "tts",
                    "image",
                    "image_gen",
                    "video",
                    "video_gen",
                    "rerank",
                ],
            )
        })
    }
}

impl ModelCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Vision => "vision",
            // Distinct from VideoGen's legacy "video" label: this is input
            // understanding, not generation.
            Self::Video => "video_input",
            Self::AudioInput => "audio_input",
            Self::Embedding => "embedding",
            Self::Stt => "stt",
            Self::Tts => "tts",
            Self::ImageGen => "image",
            Self::VideoGen => "video",
            Self::Rerank => "rerank",
        }
    }

    pub fn all() -> &'static [ModelCapability] {
        &[
            Self::Chat,
            Self::Vision,
            Self::Video,
            Self::AudioInput,
            Self::Embedding,
            Self::Stt,
            Self::Tts,
            Self::ImageGen,
            Self::VideoGen,
        ]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "chat" => Some(Self::Chat),
            "vision" | "multimodal" => Some(Self::Vision),
            "video_input" | "video_understanding" | "video-input" => Some(Self::Video),
            "audio_input" | "audio_understanding" | "audio" => Some(Self::AudioInput),
            "embedding" | "embed" => Some(Self::Embedding),
            "stt" | "speech_to_text" | "transcription" => Some(Self::Stt),
            "tts" | "text_to_speech" => Some(Self::Tts),
            "speech" => Some(Self::Stt),
            "image" | "image_gen" | "imagegen" => Some(Self::ImageGen),
            "video" | "video_gen" | "videogen" => Some(Self::VideoGen),
            "rerank" => Some(Self::Rerank),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_capability_aliases() {
        assert_eq!(ModelCapability::parse("stt"), Some(ModelCapability::Stt));
        assert_eq!(
            ModelCapability::parse("image_gen"),
            Some(ModelCapability::ImageGen)
        );
        assert_eq!(
            ModelCapability::parse("image"),
            Some(ModelCapability::ImageGen)
        );
    }

    #[test]
    fn video_and_audio_input_roundtrip() {
        // Input modalities serialize distinctly from generation's legacy labels.
        assert_eq!(ModelCapability::Video.as_str(), "video_input");
        assert_eq!(ModelCapability::AudioInput.as_str(), "audio_input");
        assert_eq!(
            ModelCapability::parse("video_input"),
            Some(ModelCapability::Video)
        );
        assert_eq!(
            ModelCapability::parse("audio"),
            Some(ModelCapability::AudioInput)
        );
        // Legacy "video" label still means generation.
        assert_eq!(
            ModelCapability::parse("video"),
            Some(ModelCapability::VideoGen)
        );
        assert!(ModelCapability::all().contains(&ModelCapability::Video));
        assert!(ModelCapability::all().contains(&ModelCapability::AudioInput));
    }

    #[test]
    fn deserializes_legacy_image_capability() {
        let cap: ModelCapability = serde_json::from_value(serde_json::json!("image")).unwrap();
        assert_eq!(cap, ModelCapability::ImageGen);
        let cap: ModelCapability = serde_json::from_value(serde_json::json!("video")).unwrap();
        assert_eq!(cap, ModelCapability::VideoGen);
    }

    #[test]
    fn serializes_image_as_legacy_label() {
        assert_eq!(
            serde_json::to_string(&ModelCapability::ImageGen).unwrap(),
            "\"image\""
        );
    }
}
