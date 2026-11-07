#include "stt_logic.hpp"

#include <cctype>
#include <cstdint>
#include <cstdio>
#include <cstring>

namespace stt {
namespace {

constexpr size_t RIFF_HEADER_BYTES = 12;
constexpr size_t CHUNK_HEADER_BYTES = 8;
constexpr size_t FMT_MIN_BYTES = 16;
constexpr uint16_t WAVE_FORMAT_PCM = 1;
constexpr uint16_t EXPECTED_CHANNELS = 1;
constexpr uint16_t EXPECTED_BITS = 16;

uint16_t le16(const unsigned char* p) { return static_cast<uint16_t>(p[0] | (p[1] << 8)); }
uint32_t le32(const unsigned char* p) {
    return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8) |
           (static_cast<uint32_t>(p[2]) << 16) | (static_cast<uint32_t>(p[3]) << 24);
}

DecodedAudio fail(const char* message) {
    DecodedAudio out;
    out.error = message;
    return out;
}

bool is_space(char c) { return std::isspace(static_cast<unsigned char>(c)) != 0; }

// Length of the valid UTF-8 sequence starting at s[i], or 0 if it is invalid.
size_t utf8_sequence_length(std::string_view s, size_t i) {
    const auto byte = [&](size_t k) { return static_cast<unsigned char>(s[i + k]); };
    const unsigned char lead = byte(0);
    size_t length = 0;
    uint32_t code = 0;
    uint32_t smallest = 0;  // rejects overlong encodings
    if (lead < 0x80) {
        return 1;
    } else if (lead >= 0xC2 && lead <= 0xDF) {
        length = 2, code = lead & 0x1F, smallest = 0x80;
    } else if ((lead & 0xF0) == 0xE0) {
        length = 3, code = lead & 0x0F, smallest = 0x800;
    } else if (lead >= 0xF0 && lead <= 0xF4) {
        length = 4, code = lead & 0x07, smallest = 0x10000;
    } else {
        return 0;
    }
    if (i + length > s.size()) return 0;
    for (size_t k = 1; k < length; ++k) {
        if ((byte(k) & 0xC0) != 0x80) return 0;
        code = (code << 6) | (byte(k) & 0x3F);
    }
    if (code < smallest || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) return 0;
    return length;
}

// True when `text` holds nothing but whitespace, dots, "…", and [bracketed] or (parenthesised)
// markers such as [BLANK_AUDIO], [music] or (silence).
bool is_silence_marker(std::string_view text) {
    constexpr std::string_view ELLIPSIS = "\xE2\x80\xA6";
    size_t i = 0;
    while (i < text.size()) {
        const char c = text[i];
        if (is_space(c) || c == '.') {
            ++i;
        } else if (text.compare(i, ELLIPSIS.size(), ELLIPSIS) == 0) {
            i += ELLIPSIS.size();
        } else if (c == '[' || c == '(') {
            const size_t close = text.find(c == '[' ? ']' : ')', i + 1);
            if (close == std::string_view::npos) return false;
            i = close + 1;
        } else {
            return false;
        }
    }
    return true;
}

}  // namespace

DecodedAudio decode_wav(std::string_view bytes) {
    const auto* data = reinterpret_cast<const unsigned char*>(bytes.data());
    if (bytes.size() < RIFF_HEADER_BYTES || std::memcmp(data, "RIFF", 4) != 0 ||
        std::memcmp(data + 8, "WAVE", 4) != 0) {
        return fail("file is not a RIFF/WAVE file");
    }

    bool have_fmt = false;
    size_t pos = RIFF_HEADER_BYTES;
    while (pos + CHUNK_HEADER_BYTES <= bytes.size()) {
        const unsigned char* chunk = data + pos;
        const size_t body = pos + CHUNK_HEADER_BYTES;
        const size_t declared = le32(chunk + 4);
        const size_t available = bytes.size() - body;

        if (std::memcmp(chunk, "fmt ", 4) == 0) {
            if (declared < FMT_MIN_BYTES || available < FMT_MIN_BYTES) return fail("WAV fmt chunk is too short");
            const uint16_t format = le16(data + body);
            const uint16_t channels = le16(data + body + 2);
            const uint32_t rate = le32(data + body + 4);
            const uint16_t bits = le16(data + body + 14);
            if (format != WAVE_FORMAT_PCM) return fail("WAV must be uncompressed PCM");
            if (rate != SAMPLE_RATE_HZ) return fail("WAV sample rate must be 16000 Hz");
            if (channels != EXPECTED_CHANNELS) return fail("WAV must be mono");
            if (bits != EXPECTED_BITS) return fail("WAV must be 16-bit");
            have_fmt = true;
        } else if (std::memcmp(chunk, "data", 4) == 0) {
            if (!have_fmt) return fail("WAV data chunk comes before the fmt chunk");
            const size_t pcm_bytes = declared < available ? declared : available;
            const size_t sample_count = pcm_bytes / BYTES_PER_SAMPLE;
            if (sample_count < MIN_AUDIO_SAMPLES) return fail("audio is shorter than 0.3 seconds");
            if (sample_count > MAX_AUDIO_SAMPLES) return fail("audio is longer than 15.5 seconds");

            DecodedAudio out;
            out.samples.resize(sample_count);
            for (size_t i = 0; i < sample_count; ++i) {
                const auto sample = static_cast<int16_t>(le16(data + body + BYTES_PER_SAMPLE * i));
                out.samples[i] = static_cast<float>(sample) / 32768.0f;
            }
            return out;
        }
        pos = body + declared + (declared & 1);  // chunks are padded to an even size
    }
    return fail(have_fmt ? "WAV has no data chunk" : "WAV has no fmt chunk");
}

std::string transcript_text(const std::vector<Segment>& segments, double audio_seconds) {
    std::string joined;
    for (const Segment& segment : segments) {
        if (segment.start_sec >= audio_seconds) continue;  // invented past the end of the audio
        if (!joined.empty() && !segment.text.empty() && !is_space(joined.back()) && !is_space(segment.text.front())) {
            joined += ' ';
        }
        joined += segment.text;
    }
    return is_silence_marker(joined) ? std::string() : joined;
}

std::string json_escape(std::string_view text) {
    std::string out;
    out.reserve(text.size());
    for (size_t i = 0; i < text.size();) {
        const size_t length = utf8_sequence_length(text, i);
        if (length == 0) {
            out += "\\ufffd";
            ++i;
            continue;
        }
        const char c = text[i];
        if (length > 1) {
            out.append(text.substr(i, length));
        } else if (c == '"') {
            out += "\\\"";
        } else if (c == '\\') {
            out += "\\\\";
        } else if (c == '\n') {
            out += "\\n";
        } else if (c == '\r') {
            out += "\\r";
        } else if (c == '\t') {
            out += "\\t";
        } else if (static_cast<unsigned char>(c) < 0x20 || c == 0x7F) {
            char escape[8];
            std::snprintf(escape, sizeof escape, "\\u%04x", static_cast<unsigned>(static_cast<unsigned char>(c)));
            out += escape;
        } else {
            out += c;
        }
        i += length;
    }
    return out;
}

std::string text_json(std::string_view text) { return "{\"text\":\"" + json_escape(text) + "\"}"; }

std::string error_json(std::string_view message, std::string_view type) {
    return "{\"error\":{\"message\":\"" + json_escape(message) + "\",\"type\":\"" + json_escape(type) + "\"}}";
}

}  // namespace stt
