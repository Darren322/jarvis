// Pure logic of the Jarvis speech-to-text server: WAV validation, transcript rules and JSON.
// No HailoRT and no HTTP here, so it builds and is tested on any machine.
#pragma once

#include <cstddef>
#include <string>
#include <string_view>
#include <vector>

namespace stt {

// Audio format the Hailo Whisper pipeline takes (see spec D2): 16 kHz, mono, signed 16-bit.
constexpr size_t SAMPLE_RATE_HZ = 16000;
constexpr size_t SAMPLES_PER_MS = SAMPLE_RATE_HZ / 1000;
constexpr size_t BYTES_PER_SAMPLE = 2;

// Audio length limits, in samples so the boundaries are exact.
// Under MIN_AUDIO_MS Jarvis says "didn't catch that" and never sends the clip (spec MIN_RECORDING).
constexpr size_t MIN_AUDIO_MS = 300;
// Jarvis stops recording at 15 s (spec MAX_RECORDING); the margin absorbs capture start-up slack.
constexpr size_t MAX_RECORDING_MS = 15000;
constexpr size_t MAX_AUDIO_MARGIN_MS = 500;
constexpr size_t MAX_AUDIO_MS = MAX_RECORDING_MS + MAX_AUDIO_MARGIN_MS;
constexpr size_t MIN_AUDIO_SAMPLES = MIN_AUDIO_MS * SAMPLES_PER_MS;  // 4,800
constexpr size_t MAX_AUDIO_SAMPLES = MAX_AUDIO_MS * SAMPLES_PER_MS;  // 248,000

// Request body limit: 15.5 s of PCM at 32,000 B/s = 496,000 B, plus headroom for the WAV
// header (44 B, more with extra chunks), the multipart boundaries and the small text fields.
constexpr size_t MAX_PCM_BYTES = MAX_AUDIO_SAMPLES * BYTES_PER_SAMPLE;
constexpr size_t BODY_HEADROOM_BYTES = 64 * 1024;
constexpr size_t MAX_BODY_BYTES = MAX_PCM_BYTES + BODY_HEADROOM_BYTES;  // 561,536

/// Result of validating and decoding an uploaded WAV file.
struct DecodedAudio {
    std::vector<float> samples;  // PCM float32 in [-1.0, 1.0), what HailoRT expects
    std::string error;           // empty when the audio is usable; otherwise the 4xx message
    bool ok() const { return error.empty(); }
};

/// Parses RIFF/WAVE (walking chunks to find "fmt " and "data"), requires PCM 16 kHz mono
/// 16-bit, and enforces MIN_AUDIO_SAMPLES and MAX_AUDIO_SAMPLES. A data size larger than the
/// bytes present (streamed WAVs) is clamped to the bytes present.
DecodedAudio decode_wav(std::string_view bytes);

/// One transcribed segment; mirrors the fields of HailoRT's Speech2Text::SegmentInfo that we use.
struct Segment {
    float start_sec;
    std::string text;
};

/// Applies the transcript rules: drops segments starting at or after `audio_seconds`, joins
/// the rest, and returns "" when only whitespace, ellipses or bracketed markers remain.
std::string transcript_text(const std::vector<Segment>& segments, double audio_seconds);

/// Escapes for a JSON string body. Invalid UTF-8 bytes become U+FFFD so the output is valid JSON.
std::string json_escape(std::string_view text);

/// {"text":"..."}, the success body Rig's transcription client decodes.
std::string text_json(std::string_view text);

/// {"error":{"message":"...","type":"..."}}, the OpenAI error envelope.
std::string error_json(std::string_view message, std::string_view type);

}  // namespace stt
