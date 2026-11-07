// jarvis-stt: speech-to-text HTTP server for the Hailo-10H (HailoRT Speech2Text + cpp-httplib).
//
//   GET  /health, /v1/health         200 {"status":"ok"} once the model is loaded
//   POST /v1/audio/transcriptions    multipart, required "file" (WAV 16 kHz mono 16-bit) -> {"text":"..."}
//
// Privacy (spec D7): audio stays in memory and is never written to disk. Neither audio nor
// transcript text is ever logged; a log line has only sizes, audio length, latency and status.
#include "stt_logic.hpp"

#include <csignal>
#include <unistd.h>

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "hailo/genai/speech2text/speech2text.hpp"
#include "httplib.h"

namespace {

using Clock = std::chrono::steady_clock;

// Service defaults; each is a setting (argument or environment variable), not compiled in.
constexpr const char* DEFAULT_HOST = "0.0.0.0";  // LAN only, like Gemma on 8080
constexpr int DEFAULT_PORT = 8082;

// Two or more threads so /health answers while one request waits for or runs a transcription.
constexpr size_t HTTP_THREADS = 4;
// Longest wait for the next bytes of a request. Stops a POST that announces a body and never
// sends it (whisper-server hung forever on that). A normal upload takes milliseconds on the LAN.
constexpr auto REQUEST_READ_TIMEOUT = std::chrono::seconds(10);
constexpr auto RESPONSE_WRITE_TIMEOUT = std::chrono::seconds(10);
constexpr auto KEEP_ALIVE_TIMEOUT = std::chrono::seconds(5);
// Passed to HailoRT. Medium needs 11.4 s for 15 s of audio; the client waits 20 s (spec).
constexpr auto GENERATE_TIMEOUT = std::chrono::seconds(30);

constexpr const char* INVALID_REQUEST = "invalid_request_error";
constexpr const char* SERVER_ERROR = "server_error";
constexpr const char* JSON = "application/json";

struct Settings {
    std::string model_path;
    std::string host = DEFAULT_HOST;
    int port = DEFAULT_PORT;
};

void print_usage(const char* program) {
    std::fprintf(stderr,
                 "usage: %s [--model HEF] [--host ADDR] [--port N]\n"
                 "  --model  Whisper HEF path        (env JARVIS_STT_HEF, required)\n"
                 "  --host   address to bind         (env JARVIS_STT_HOST, default %s)\n"
                 "  --port   TCP port                (env JARVIS_STT_PORT, default %d)\n",
                 program, DEFAULT_HOST, DEFAULT_PORT);
}

// Arguments win over environment variables. Returns false (after printing why) on bad settings.
bool parse_settings(int argc, char** argv, Settings& settings) {
    if (const char* v = std::getenv("JARVIS_STT_HEF")) settings.model_path = v;
    if (const char* v = std::getenv("JARVIS_STT_HOST")) settings.host = v;
    std::string port_text;
    if (const char* v = std::getenv("JARVIS_STT_PORT")) port_text = v;

    for (int i = 1; i < argc; ++i) {
        const std::string arg = argv[i];
        std::string* target = arg == "--model" ? &settings.model_path
                              : arg == "--host" ? &settings.host
                              : arg == "--port" ? &port_text
                                                : nullptr;
        if (target == nullptr || i + 1 >= argc) {
            print_usage(argv[0]);
            return false;
        }
        *target = argv[++i];
    }

    if (!port_text.empty()) {
        char* end = nullptr;
        const long port = std::strtol(port_text.c_str(), &end, 10);
        if (*end != '\0' || port < 1 || port > 65535) {
            std::fprintf(stderr, "invalid port \"%s\"\n", port_text.c_str());
            return false;
        }
        settings.port = static_cast<int>(port);
    }
    if (settings.model_path.empty()) {
        std::fprintf(stderr, "no model: pass --model or set JARVIS_STT_HEF\n");
        print_usage(argv[0]);
        return false;
    }
    return true;
}

// Owns the device and the loaded model. One transcription at a time: the mutex serialises callers.
class Transcriber {
public:
    static std::unique_ptr<Transcriber> load(const std::string& hef_path) {
        auto vdevice = hailort::VDevice::create_shared();
        if (!vdevice) {
            std::fprintf(stderr, "cannot open the Hailo device (status %d)\n", static_cast<int>(vdevice.status()));
            return nullptr;
        }
        auto model = hailort::genai::Speech2Text::create(vdevice.value(), hailort::genai::Speech2TextParams(hef_path));
        if (!model) {
            std::fprintf(stderr, "cannot load the model (status %d)\n", static_cast<int>(model.status()));
            return nullptr;
        }
        auto params = model->create_generator_params();
        if (!params) {
            std::fprintf(stderr, "cannot create generator parameters (status %d)\n", static_cast<int>(params.status()));
            return nullptr;
        }
        hailort::genai::Speech2TextGeneratorParams generator = params.value();
        generator.set_task(hailort::genai::Speech2TextTask::TRANSCRIBE);
        generator.set_language("en");
        return std::unique_ptr<Transcriber>(new Transcriber(vdevice.value(), std::move(model.value()), generator));
    }

    /// Returns the segments, or the HailoRT status on failure.
    hailort::Expected<std::vector<hailort::genai::Speech2Text::SegmentInfo>> transcribe(std::vector<float>& samples) {
        std::lock_guard<std::mutex> lock(mutex_);
        return model_.generate_all_segments(hailort::MemoryView(samples.data(), samples.size() * sizeof(float)),
                                            params_, GENERATE_TIMEOUT);
    }

private:
    Transcriber(std::shared_ptr<hailort::VDevice> vdevice, hailort::genai::Speech2Text&& model,
                const hailort::genai::Speech2TextGeneratorParams& params)
        : vdevice_(std::move(vdevice)), model_(std::move(model)), params_(params) {}

    // Declaration order matters: the model is destroyed before the device it runs on.
    std::shared_ptr<hailort::VDevice> vdevice_;
    hailort::genai::Speech2Text model_;
    hailort::genai::Speech2TextGeneratorParams params_;
    std::mutex mutex_;
};

struct Reply {
    int status;
    std::string body;
    double audio_seconds = -1.0;  // -1 until the audio has been decoded
};

Reply error_reply(int status, const std::string& message, double audio_seconds = -1.0) {
    return {status, stt::error_json(message, status >= 500 ? SERVER_ERROR : INVALID_REQUEST), audio_seconds};
}

size_t content_length(const httplib::Request& req) {
    return static_cast<size_t>(std::strtoull(req.get_header_value("Content-Length").c_str(), nullptr, 10));
}

void log_line(const char* what, int status, size_t request_bytes, double audio_seconds, long long latency_ms) {
    if (audio_seconds >= 0) {
        std::fprintf(stderr, "%s status=%d request_bytes=%zu audio_s=%.2f latency_ms=%lld\n", what, status,
                     request_bytes, audio_seconds, latency_ms);
    } else {
        std::fprintf(stderr, "%s status=%d request_bytes=%zu latency_ms=%lld\n", what, status, request_bytes,
                     latency_ms);
    }
}

Reply transcribe_request(const httplib::Request& req, Transcriber& transcriber) {
    // Rig sends language only when set; only English is supported. model, prompt and temperature are ignored.
    if (req.form.has_field("language") && req.form.get_field("language") != "en") {
        return error_reply(400, "only language \"en\" is supported");
    }
    if (!req.form.has_file("file")) {
        return error_reply(400, "missing multipart file part \"file\"");
    }

    stt::DecodedAudio audio = stt::decode_wav(req.form.get_file("file").content);
    if (!audio.ok()) return error_reply(400, audio.error);
    const double audio_seconds = static_cast<double>(audio.samples.size()) / stt::SAMPLE_RATE_HZ;

    auto segments = transcriber.transcribe(audio.samples);
    if (!segments) {
        const std::string message = "speech-to-text failed (hailo status " + std::to_string(static_cast<int>(segments.status())) + ")";
        return error_reply(500, message, audio_seconds);
    }

    std::vector<stt::Segment> parts;
    for (const auto& s : segments.value()) parts.push_back({s.start_sec, s.text});
    return {200, stt::text_json(stt::transcript_text(parts, audio_seconds)), audio_seconds};
}

void install_handlers(httplib::Server& svr, Transcriber& transcriber) {
    svr.new_task_queue = [] { return new httplib::ThreadPool(HTTP_THREADS); };
    svr.set_payload_max_length(stt::MAX_BODY_BYTES);
    svr.set_read_timeout(REQUEST_READ_TIMEOUT);
    svr.set_write_timeout(RESPONSE_WRITE_TIMEOUT);
    svr.set_keep_alive_timeout(KEEP_ALIVE_TIMEOUT);

    // The model is loaded before listening starts, so answering at all means ready. Jarvis probes
    // /health at the server root (as it does for llama-server); /v1/health sits next to the API.
    const auto health = [](const httplib::Request&, httplib::Response& res) {
        res.set_content("{\"status\":\"ok\"}", JSON);
    };
    svr.Get("/health", health);
    svr.Get("/v1/health", health);

    svr.Post("/v1/audio/transcriptions", [&transcriber](const httplib::Request& req, httplib::Response& res) {
        const auto start = Clock::now();
        const Reply reply = transcribe_request(req, transcriber);
        res.status = reply.status;
        res.set_content(reply.body, JSON);
        const auto latency = std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - start);
        log_line("transcription", reply.status, content_length(req), reply.audio_seconds, latency.count());
    });

    // Errors raised by httplib itself (413, malformed request, 404, ...) get the same envelope.
    // Replies built by our handlers already carry a body and pass through untouched.
    svr.set_error_handler([](const httplib::Request& req, httplib::Response& res) {
        if (!res.body.empty()) return httplib::Server::HandlerResponse::Unhandled;
        res.set_content(stt::error_json(httplib::status_message(res.status), res.status >= 500 ? SERVER_ERROR : INVALID_REQUEST), JSON);
        log_line("rejected", res.status, content_length(req), -1.0, 0);
        return httplib::Server::HandlerResponse::Handled;
    });
    svr.set_exception_handler([](const httplib::Request&, httplib::Response& res, std::exception_ptr) {
        res.status = 500;
        res.set_content(stt::error_json("internal error", SERVER_ERROR), JSON);
    });
}

}  // namespace

int main(int argc, char** argv) {
    // Block the shutdown signals before any thread exists so every thread inherits the mask and
    // main alone receives them through sigwait.
    sigset_t shutdown_signals;
    sigemptyset(&shutdown_signals);
    sigaddset(&shutdown_signals, SIGINT);
    sigaddset(&shutdown_signals, SIGTERM);
    pthread_sigmask(SIG_BLOCK, &shutdown_signals, nullptr);
    std::signal(SIGPIPE, SIG_IGN);

    Settings settings;
    if (!parse_settings(argc, argv, settings)) return 2;

    const auto load_start = Clock::now();
    std::unique_ptr<Transcriber> transcriber = Transcriber::load(settings.model_path);
    if (!transcriber) return 1;
    const auto load_ms = std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - load_start).count();
    std::fprintf(stderr, "model loaded in %lld ms\n", static_cast<long long>(load_ms));

    httplib::Server server;
    install_handlers(server, *transcriber);
    if (!server.bind_to_port(settings.host, settings.port)) {
        std::fprintf(stderr, "cannot bind %s:%d\n", settings.host.c_str(), settings.port);
        return 1;
    }
    std::thread listener([&server] {
        server.listen_after_bind();
        kill(getpid(), SIGTERM);  // listening ended on its own; wake main so the process exits
    });
    server.wait_until_ready();
    std::fprintf(stderr, "listening on %s:%d\n", settings.host.c_str(), settings.port);

    int signal_number = 0;
    sigwait(&shutdown_signals, &signal_number);
    std::fprintf(stderr, "signal %d: shutting down\n", signal_number);
    server.stop();  // lets a running transcription finish, then returns
    listener.join();
    transcriber.reset();  // releases the device
    std::fprintf(stderr, "stopped\n");
    return 0;
}
