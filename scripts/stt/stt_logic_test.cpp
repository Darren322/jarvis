// Tests for stt_logic. Plain checks, no framework; exits non-zero if any check fails.
#include "stt_logic.hpp"

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

using namespace stt;

static int failures = 0;

#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::fprintf(stderr, "FAIL %s:%d  %s\n", __FILE__, __LINE__, #cond); \
            ++failures;                                                          \
        }                                                                        \
    } while (0)

#define CHECK_EQ(actual, expected)                                                                                   \
    do {                                                                                                             \
        const std::string a_ = (actual), e_ = (expected);                                                            \
        if (a_ != e_) {                                                                                              \
            std::fprintf(stderr, "FAIL %s:%d  got \"%s\" want \"%s\"\n", __FILE__, __LINE__, a_.c_str(), e_.c_str()); \
            ++failures;                                                                                              \
        }                                                                                                            \
    } while (0)

static void put16(std::string& s, uint16_t v) {
    s += static_cast<char>(v & 0xFF);
    s += static_cast<char>(v >> 8);
}
static void put32(std::string& s, uint32_t v) {
    put16(s, static_cast<uint16_t>(v & 0xFFFF));
    put16(s, static_cast<uint16_t>(v >> 16));
}

static std::string fmt_chunk(uint16_t format, uint16_t channels, uint32_t rate, uint16_t bits) {
    std::string s = "fmt ";
    put32(s, 16);
    put16(s, format);
    put16(s, channels);
    put32(s, rate);
    put32(s, rate * channels * bits / 8);
    put16(s, static_cast<uint16_t>(channels * bits / 8));
    put16(s, bits);
    return s;
}

static std::string data_chunk(size_t samples, uint32_t declared) {
    std::string s = "data";
    put32(s, declared);
    for (size_t i = 0; i < samples; ++i) put16(s, static_cast<uint16_t>(i % 2 == 0 ? 0x4000 : 0xC000));
    return s;
}

static std::string wav(const std::string& chunks) {
    std::string s = "RIFF";
    put32(s, static_cast<uint32_t>(4 + chunks.size()));
    s += "WAVE";
    return s + chunks;
}

static std::string good_wav(size_t samples) {
    return wav(fmt_chunk(1, 1, 16000, 16) + data_chunk(samples, static_cast<uint32_t>(samples * 2)));
}

static void test_wav_accepts_the_expected_format() {
    const DecodedAudio audio = decode_wav(good_wav(16000));
    CHECK(audio.ok());
    CHECK(audio.samples.size() == 16000);
    CHECK(audio.samples[0] == 0.5f);   // 0x4000 / 32768
    CHECK(audio.samples[1] == -0.5f);  // 0xC000 as int16 = -16384
}

static void test_wav_finds_chunks_instead_of_assuming_a_44_byte_header() {
    std::string list = "LIST";
    put32(list, 5);  // odd size: one pad byte follows
    list += "abcde";
    list += '\0';
    const DecodedAudio audio = decode_wav(wav(list + fmt_chunk(1, 1, 16000, 16) + data_chunk(8000, 16000)));
    CHECK(audio.ok());
    CHECK(audio.samples.size() == 8000);
    const DecodedAudio trailing = decode_wav(wav(fmt_chunk(1, 1, 16000, 16) + data_chunk(8000, 16000) + list));
    CHECK(trailing.ok() && trailing.samples.size() == 8000);
}

static void test_wav_clamps_a_streamed_data_size_to_the_bytes_present() {
    CHECK(decode_wav(wav(fmt_chunk(1, 1, 16000, 16) + data_chunk(8000, 0xFFFFFFFFu))).samples.size() == 8000);
    CHECK(decode_wav(wav(fmt_chunk(1, 1, 16000, 16) + data_chunk(8000, 0x7FFFFFFFu))).samples.size() == 8000);
}

static void test_wav_rejects_a_wrong_format() {
    CHECK(!decode_wav("").ok());
    CHECK(!decode_wav("RIFF").ok());
    CHECK(!decode_wav(std::string(100, 'x')).ok());
    std::string not_wave = good_wav(8000);
    not_wave.replace(8, 4, "AVI ");
    CHECK(!decode_wav(not_wave).ok());
    CHECK_EQ(decode_wav(wav(fmt_chunk(1, 1, 8000, 16) + data_chunk(8000, 16000))).error, "WAV sample rate must be 16000 Hz");
    CHECK_EQ(decode_wav(wav(fmt_chunk(1, 2, 16000, 16) + data_chunk(8000, 16000))).error, "WAV must be mono");
    CHECK_EQ(decode_wav(wav(fmt_chunk(1, 1, 16000, 8) + data_chunk(8000, 16000))).error, "WAV must be 16-bit");
    CHECK_EQ(decode_wav(wav(fmt_chunk(3, 1, 16000, 16) + data_chunk(8000, 16000))).error, "WAV must be uncompressed PCM");
    CHECK_EQ(decode_wav(wav(data_chunk(8000, 16000) + fmt_chunk(1, 1, 16000, 16))).error,
             "WAV data chunk comes before the fmt chunk");
    CHECK_EQ(decode_wav(wav(fmt_chunk(1, 1, 16000, 16))).error, "WAV has no data chunk");
    CHECK_EQ(decode_wav(wav("")).error, "WAV has no fmt chunk");
}

static void test_wav_length_limits_at_their_boundaries() {
    CHECK(MIN_AUDIO_SAMPLES == 4800);
    CHECK(MAX_AUDIO_SAMPLES == 248000);
    CHECK(MAX_BODY_BYTES == 496000 + 65536);
    CHECK_EQ(decode_wav(good_wav(MIN_AUDIO_SAMPLES - 1)).error, "audio is shorter than 0.3 seconds");
    CHECK_EQ(decode_wav(good_wav(4640)).error, "audio is shorter than 0.3 seconds");  // 0.29 s
    CHECK(decode_wav(good_wav(4800)).ok());                                          // 0.30 s
    CHECK(decode_wav(good_wav(248000)).ok());                                         // 15.5 s
    CHECK_EQ(decode_wav(good_wav(MAX_AUDIO_SAMPLES + 1)).error, "audio is longer than 15.5 seconds");
    CHECK_EQ(decode_wav(good_wav(249600)).error, "audio is longer than 15.5 seconds");  // 15.6 s
    CHECK(!decode_wav(good_wav(0)).ok());
}

static void test_transcript_joins_segments() {
    CHECK_EQ(transcript_text({{0.0f, "Hello there."}}, 6.0), "Hello there.");
    CHECK_EQ(transcript_text({{0.0f, " Hello"}, {2.0f, " world"}}, 6.0), " Hello world");  // returned unchanged
    CHECK_EQ(transcript_text({{0.0f, "Hello"}, {2.0f, "world"}}, 6.0), "Hello world");     // no glued words
    CHECK_EQ(transcript_text({{0.0f, "Hello "}, {2.0f, "world"}}, 6.0), "Hello world");
    CHECK_EQ(transcript_text({}, 6.0), "");
}

static void test_transcript_drops_segments_that_start_at_or_after_the_audio_end() {
    CHECK_EQ(transcript_text({{0.0f, "one."}, {5.36f, "two."}, {14.88f, "you"}}, 15.0), "one. two. you");  // 14.88 < 15
    CHECK_EQ(transcript_text({{0.0f, "one."}, {15.0f, "you"}}, 15.0), "one.");  // starts exactly at the end
    CHECK_EQ(transcript_text({{0.0f, "one."}, {15.01f, "you"}}, 15.0), "one.");
    CHECK_EQ(transcript_text({{0.0f, "one."}, {14.99f, "you"}}, 15.0), "one. you");
    CHECK_EQ(transcript_text({{0.0f, "Her job"}, {6.0f, "you"}}, 6.0), "Her job");
}

static void test_transcript_turns_blank_output_into_empty_text() {
    for (const char* blank : {"", " ", " \n\t ", "...", " ... ", "\xE2\x80\xA6", ".", "[BLANK_AUDIO]", "[music]",
                              "(silence)", " [ Silence ] ", "[BLANK_AUDIO] ...", "(music) [applause]"}) {
        CHECK_EQ(transcript_text({{0.0f, blank}}, 6.0), "");
    }
    // The clip2-jieli Medium output: three "..." segments, the last two starting at or past the end of a 6 s clip.
    CHECK_EQ(transcript_text({{0.0f, "..."}, {6.0f, "..."}, {8.0f, "..."}}, 6.0), "");
    CHECK_EQ(transcript_text({{0.0f, "..."}, {2.0f, "..."}}, 6.0), "");
    // A dropped invented word leaves nothing.
    CHECK_EQ(transcript_text({{16.0f, "you"}}, 15.0), "");
}

static void test_transcript_keeps_real_text_around_markers() {
    CHECK_EQ(transcript_text({{0.0f, "[music] hello"}}, 6.0), "[music] hello");
    CHECK_EQ(transcript_text({{0.0f, "Wait... what"}}, 6.0), "Wait... what");
    CHECK_EQ(transcript_text({{0.0f, "(laughs) fine"}}, 6.0), "(laughs) fine");
    CHECK_EQ(transcript_text({{0.0f, "[unclosed"}}, 6.0), "[unclosed");
    CHECK_EQ(transcript_text({{0.0f, "Forget that"}}, 6.0), "Forget that");
}

static void test_json_escaping() {
    CHECK_EQ(json_escape("plain"), "plain");
    CHECK_EQ(json_escape("say \"hi\" \\ now"), "say \\\"hi\\\" \\\\ now");
    CHECK_EQ(json_escape("a\nb\r\tc"), "a\\nb\\r\\tc");
    CHECK_EQ(json_escape(std::string("\x01\x1f", 2)), "\\u0001\\u001f");
    CHECK_EQ(json_escape("caf\xC3\xA9 \xE2\x80\xA6 \xF0\x9F\x98\x80"), "caf\xC3\xA9 \xE2\x80\xA6 \xF0\x9F\x98\x80");  // UTF-8 kept
    CHECK_EQ(json_escape("a\xFF" "b"), "a\\ufffdb");                    // invalid byte
    CHECK_EQ(json_escape("a\xC3"), "a\\ufffd");                         // truncated sequence at the end
    CHECK_EQ(json_escape("\xC0\x80"), "\\ufffd\\ufffd");                // overlong NUL
    CHECK_EQ(json_escape("\xED\xA0\x80"), "\\ufffd\\ufffd\\ufffd");     // surrogate
    CHECK_EQ(text_json("it's \"ok\""), "{\"text\":\"it's \\\"ok\\\"\"}");
    CHECK_EQ(text_json(""), "{\"text\":\"\"}");
    CHECK_EQ(error_json("bad \"file\"", "invalid_request_error"),
             "{\"error\":{\"message\":\"bad \\\"file\\\"\",\"type\":\"invalid_request_error\"}}");
}

int main() {
    test_wav_accepts_the_expected_format();
    test_wav_finds_chunks_instead_of_assuming_a_44_byte_header();
    test_wav_clamps_a_streamed_data_size_to_the_bytes_present();
    test_wav_rejects_a_wrong_format();
    test_wav_length_limits_at_their_boundaries();
    test_transcript_joins_segments();
    test_transcript_drops_segments_that_start_at_or_after_the_audio_end();
    test_transcript_turns_blank_output_into_empty_text();
    test_transcript_keeps_real_text_around_markers();
    test_json_escaping();
    if (failures != 0) {
        std::fprintf(stderr, "%d check(s) failed\n", failures);
        return 1;
    }
    std::puts("all checks passed");
    return 0;
}
