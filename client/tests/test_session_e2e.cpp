// End-to-end: the Android client's networking code (StreamSession,
// TcpControlClient over MbedTLS, VideoReceiver, FrameAssembler) against the
// real engine — the focus-vision-headless binary of the simulator build,
// which runs the production TCP + TLS 1.3 (rustls) + PIN + RTP + FEC + UDP
// path with synthetic video.
//
// Until this test, the C++ client had never talked to the engine: the
// engine's own E2E tests use a Rust mock client.
//
// Skipped unless FVP_HEADLESS_BIN names the binary:
//   cargo build --release -p streaming-engine --features simulator --bins
//   FVP_HEADLESS_BIN=target/release/focus-vision-headless(.exe)

#include "net_compat.h" // first: winsock2.h must precede windows.h

#include <gtest/gtest.h>

#include "stream_session.h"
#include "video_receiver.h"

#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <filesystem>
#include <fstream>
#include <functional>
#include <mutex>
#include <optional>
#include <random>
#include <string>
#include <thread>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#else
#include <csignal>
#include <sys/types.h>
#include <sys/wait.h>
#endif

namespace fs = std::filesystem;
using namespace std::chrono_literals;
using Clock = std::chrono::steady_clock;
using fvp_session::SessionState;

namespace {

/// The engine as a child process. Its log (stdout + stderr) is read on a
/// thread for the pairing PINs it prints ("Pairing PIN: 123456").
class EngineProcess {
public:
    ~EngineProcess() { terminate(); }

    bool start(const std::string& exe, const std::vector<std::string>& args) {
#ifdef _WIN32
        SECURITY_ATTRIBUTES sa{sizeof(sa), nullptr, TRUE};
        HANDLE readEnd = nullptr;
        HANDLE writeEnd = nullptr;
        if (!CreatePipe(&readEnd, &writeEnd, &sa, 0)) return false;
        SetHandleInformation(readEnd, HANDLE_FLAG_INHERIT, 0);
        STARTUPINFOA si{};
        si.cb = sizeof(si);
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdOutput = writeEnd;
        si.hStdError = writeEnd;
        si.hStdInput = nullptr;
        std::string cmd = "\"" + exe + "\"";
        for (const auto& a : args) cmd += " \"" + a + "\"";
        PROCESS_INFORMATION pi{};
        const BOOL ok = CreateProcessA(nullptr, cmd.data(), nullptr, nullptr, TRUE,
                                       CREATE_NO_WINDOW, nullptr, nullptr, &si, &pi);
        CloseHandle(writeEnd);
        if (!ok) {
            CloseHandle(readEnd);
            return false;
        }
        CloseHandle(pi.hThread);
        m_process = pi.hProcess;
        m_readEnd = readEnd;
        m_reader = std::thread([this] {
            char buf[4096];
            DWORD n = 0;
            while (ReadFile(m_readEnd, buf, sizeof(buf), &n, nullptr) && n > 0) feed(buf, n);
        });
#else
        int fds[2];
        if (pipe(fds) != 0) return false;
        const pid_t pid = fork();
        if (pid < 0) return false;
        if (pid == 0) {
            dup2(fds[1], 1);
            dup2(fds[1], 2);
            close(fds[0]);
            close(fds[1]);
            std::vector<char*> argv;
            argv.push_back(const_cast<char*>(exe.c_str()));
            for (const auto& a : args) argv.push_back(const_cast<char*>(a.c_str()));
            argv.push_back(nullptr);
            execv(exe.c_str(), argv.data());
            _exit(127);
        }
        close(fds[1]);
        m_pid = pid;
        m_readFd = fds[0];
        m_reader = std::thread([this] {
            char buf[4096];
            ssize_t n = 0;
            while ((n = read(m_readFd, buf, sizeof(buf))) > 0) feed(buf, static_cast<size_t>(n));
        });
#endif
        return true;
    }

    void terminate() {
#ifdef _WIN32
        if (m_process) {
            TerminateProcess(m_process, 1);
            WaitForSingleObject(m_process, 5000);
            CloseHandle(m_process);
            m_process = nullptr;
        }
        if (m_reader.joinable()) m_reader.join();
        if (m_readEnd) {
            CloseHandle(m_readEnd);
            m_readEnd = nullptr;
        }
#else
        if (m_pid > 0) {
            kill(m_pid, SIGTERM);
            int status = 0;
            waitpid(m_pid, &status, 0);
            m_pid = -1;
        }
        if (m_reader.joinable()) m_reader.join();
        if (m_readFd >= 0) {
            close(m_readFd);
            m_readFd = -1;
        }
#endif
    }

    /// The `index`-th PIN the engine printed (0 = its first listen).
    std::optional<uint32_t> pin(size_t index, std::chrono::milliseconds timeout) {
        std::unique_lock<std::mutex> lock(m_mutex);
        if (!m_changed.wait_for(lock, timeout, [&] { return m_pins.size() > index; })) {
            return std::nullopt;
        }
        return m_pins[index];
    }

    size_t pinCount() {
        std::lock_guard<std::mutex> lock(m_mutex);
        return m_pins.size();
    }

    std::string log() {
        std::lock_guard<std::mutex> lock(m_mutex);
        std::string out;
        for (const auto& l : m_lines) out += "  | " + l + "\n";
        return out;
    }

private:
    void feed(const char* data, size_t n) {
        std::lock_guard<std::mutex> lock(m_mutex);
        for (size_t i = 0; i < n; i++) {
            if (data[i] == '\n') {
                line(m_partial);
                m_partial.clear();
            } else if (data[i] != '\r') {
                m_partial += data[i];
            }
        }
        m_changed.notify_all();
    }

    void line(const std::string& l) {
        m_lines.push_back(l);
        if (m_lines.size() > 200) m_lines.pop_front();
        for (const char* marker : {"Pairing PIN: ", "New PIN: "}) {
            const auto at = l.find(marker);
            if (at == std::string::npos) continue;
            const std::string digits = l.substr(at + std::strlen(marker), 6);
            if (digits.size() == 6 && digits.find_first_not_of("0123456789") == std::string::npos) {
                const uint32_t pin = static_cast<uint32_t>(std::stoul(digits));
                // The first listen prints the same PIN twice; keep changes only.
                if (m_pins.empty() || m_pins.back() != pin) m_pins.push_back(pin);
            }
        }
    }

    std::mutex m_mutex;
    std::condition_variable m_changed;
    std::vector<uint32_t> m_pins;
    std::deque<std::string> m_lines;
    std::string m_partial;
    std::thread m_reader;
#ifdef _WIN32
    HANDLE m_process = nullptr;
    HANDLE m_readEnd = nullptr;
#else
    pid_t m_pid = -1;
    int m_readFd = -1;
#endif
};

bool portFree(int type, uint16_t port) {
    const fvp_net::Socket s = socket(AF_INET, type, 0);
    if (s == fvp_net::kInvalidSocket) return false;
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    const bool ok = bind(s, reinterpret_cast<const sockaddr*>(&addr), sizeof(addr)) == 0;
    fvp_net::closeSocket(s);
    return ok;
}

/// A base port B with TCP B (control) and UDP B+1..B+4 free: the engine's
/// udp_port is B+1, so video goes to B+2, tracking to B+3, audio to B+4.
uint16_t pickPorts() {
    std::mt19937 rng(std::random_device{}());
    for (int attempt = 0; attempt < 100; attempt++) {
        const uint16_t base = static_cast<uint16_t>(20000 + (rng() % 3000) * 10);
        bool free = portFree(SOCK_STREAM, base);
        for (uint16_t i = 1; free && i <= 4; i++) free = portFree(SOCK_DGRAM, base + i);
        if (free) return base;
    }
    return 0;
}

bool waitUntil(const std::function<bool()>& done, std::chrono::milliseconds timeout) {
    const auto deadline = Clock::now() + timeout;
    while (Clock::now() < deadline) {
        if (done()) return true;
        std::this_thread::sleep_for(10ms);
    }
    return done();
}

class SessionE2E : public ::testing::Test {
protected:
    void SetUp() override {
        const char* bin = std::getenv("FVP_HEADLESS_BIN");
        if (bin == nullptr || *bin == '\0') {
            GTEST_SKIP() << "FVP_HEADLESS_BIN not set (focus-vision-headless from the simulator build)";
        }
        ASSERT_TRUE(fvp_net::startup());
        base = pickPorts();
        ASSERT_NE(base, 0) << "no free port block";

        dir = fs::temp_directory_path() /
              ("fvp_client_e2e_" + std::to_string(base) + "_" +
               std::to_string(std::random_device{}()));
        fs::create_directories(dir);
        fingerprintPath = (dir / "server_fingerprint.hex").string();
        const fs::path config = dir / "engine.toml";
        {
            std::ofstream out(config);
            out << "[network]\n"
                << "tcp_port = " << base << "\n"
                << "udp_port = " << base + 1 << "\n"
                << "[video]\n"
                << "framerate = 60\n"
                << "[audio]\n"
                << "enabled = false\n";
        }
        ASSERT_TRUE(engine.start(bin, {"--config", config.string(), "--duration", "90"}))
            << "could not start " << bin;
        const auto first = engine.pin(0, 15s);
        ASSERT_TRUE(first.has_value()) << "the engine printed no PIN\n" << engine.log();
        pin = *first;

        ASSERT_TRUE(receiver.start(videoPort(), &session.stats(), [this] { session.requestIdr(); }));
        session.setStreamStartHook([this](const TcpControlClient::StreamConfig&, const std::string& ip) {
            receiver.beginSession(ip);
        });
        session.setStreamEndHook([this] { receiver.endSession(); });
    }

    void TearDown() override {
        session.stop();
        receiver.stop();
        if (HasFailure()) {
            std::fprintf(stderr, "engine log (last lines):\n%s", engine.log().c_str());
        }
        engine.terminate();
        std::error_code ec;
        fs::remove_all(dir, ec);
    }

    uint16_t videoPort() const { return static_cast<uint16_t>(base + 2); }

    SessionSettings settings(uint32_t usePin) const {
        SessionSettings s;
        s.serverIp = "127.0.0.1";
        s.controlPort = base;
        s.pin = usePin;
        s.fingerprintStorePath = fingerprintPath;
        return s;
    }

    /// Frames delivered during `span`, in arrival order.
    std::vector<FrameAssembler::Frame> collectFrames(std::chrono::milliseconds span) {
        std::vector<FrameAssembler::Frame> frames;
        const auto end = Clock::now() + span;
        while (Clock::now() < end) {
            FrameAssembler::Frame f;
            while (receiver.popFrame(f)) frames.push_back(std::move(f));
            std::this_thread::sleep_for(5ms);
        }
        return frames;
    }

    std::vector<ServerEvent> drainEvents() {
        std::vector<ServerEvent> events;
        ServerEvent e;
        while (session.pollEvent(e)) events.push_back(e);
        return events;
    }

    static bool has(const std::vector<ServerEvent>& events, ServerEvent::Type type) {
        for (const auto& e : events) {
            if (e.type == type) return true;
        }
        return false;
    }

    uint16_t base = 0;
    fs::path dir;
    std::string fingerprintPath;
    uint32_t pin = 0;
    EngineProcess engine;
    StreamSession session;
    VideoReceiver receiver;
};

bool startsWithStartCode(const std::vector<uint8_t>& d) {
    return (d.size() >= 4 && d[0] == 0 && d[1] == 0 && d[2] == 0 && d[3] == 1) ||
           (d.size() >= 3 && d[0] == 0 && d[1] == 0 && d[2] == 1);
}

}  // namespace

TEST_F(SessionE2E, PairsAndStreamsVideoFromTheRealEngine) {
    session.start(settings(pin));
    ASSERT_TRUE(waitUntil([&] { return session.isStreaming(); }, 15s))
        << "never reached Streaming (state " << static_cast<int>(session.state()) << ")";

    const auto config = session.streamConfig();
    EXPECT_EQ(config.framerate, 60u);
    EXPECT_GT(config.width, 0u);
    EXPECT_GT(config.encodedWidth, 0u);
    EXPECT_EQ(config.layout, fvp_client_protocol::STEREO_SIDE_BY_SIDE)
        << "the engine sends both eyes (v5)";
    EXPECT_TRUE(config.framePose) << "the engine sends each frame's render pose (v6)";

    const auto frames = collectFrames(3s);
    // 60 fps for 3 s; up to a GOP (1 s) goes by before the first keyframe.
    ASSERT_GE(frames.size(), 60u);
    EXPECT_TRUE(frames.front().keyframe) << "decoding must start at a keyframe";
    bool sawSliced = false;
    bool sawBulk = false;
    for (size_t i = 0; i < frames.size(); i++) {
        // v6: the render pose first. The headless engine renders as if the
        // head turns left 0.01 rad a frame.
        fvp_client_protocol::FramePose pose;
        const size_t prefix =
            fvp_client_protocol::parseFramePose(frames[i].data.data(), frames[i].data.size(), pose);
        ASSERT_EQ(prefix, fvp_client_protocol::FRAME_POSE_LEN) << "frame " << frames[i].frameIndex;
        EXPECT_TRUE(pose.known);
        EXPECT_NEAR(pose.orientation[1], std::sin(frames[i].frameIndex * 0.005f), 1e-5f)
            << "frame " << frames[i].frameIndex;
        const std::vector<uint8_t> nal(frames[i].data.begin() + prefix, frames[i].data.end());
        EXPECT_TRUE(startsWithStartCode(nal)) << "frame " << frames[i].frameIndex;
        if (i > 0) {
            EXPECT_GT(frames[i].frameIndex, frames[i - 1].frameIndex) << "out of order";
        }
        // Synthetic IDRs are ~24 KB (sliced FEC above 16 KB), P-frames ~4 KB (bulk).
        (frames[i].data.size() > 16 * 1024 ? sawSliced : sawBulk) = true;
    }
    EXPECT_TRUE(sawSliced) << "no sliced (IDR-sized) frame came through";
    EXPECT_TRUE(sawBulk) << "no bulk (P-frame-sized) frame came through";
    // Loopback UDP loses nothing. A busy CI runner pausing the receiver
    // cost 4, 12 and 26 frames of ~180 while the assembler timed a frame
    // from its first packet (fixed: from its last). A tenth still catches a
    // receive path that drops frames for real.
    EXPECT_LE(receiver.framesLost(), frames.size() / 10);

    const auto events = drainEvents();
    EXPECT_TRUE(has(events, ServerEvent::Type::Streaming));
    EXPECT_TRUE(has(events, ServerEvent::Type::HeartbeatAck))
        << "the engine answers every HEARTBEAT; none came back";

    // TOFU: the server is pinned once pairing succeeded.
    std::ifstream in(fingerprintPath);
    std::string pinned;
    std::getline(in, pinned);
    EXPECT_EQ(pinned.size(), 64u);
}

TEST_F(SessionE2E, SendsTheHeadsetsViewToTheEngine) {
    // VIEW_CONFIG: the engine logs what it got (and hands it to the driver,
    // which sets SteamVR's projection). Set before the session, as the app
    // does from its first frame: it goes out once the session starts.
    constexpr float kDeg = 3.14159265358979f / 180.0f;
    const fvp_client_protocol::EyeFov left{-52 * kDeg, 45 * kDeg, 41 * kDeg, -49 * kDeg};
    const fvp_client_protocol::EyeFov right{-45 * kDeg, 52 * kDeg, 41 * kDeg, -49 * kDeg};
    const auto view = fvp_client_protocol::buildViewConfigPayload(left, right, 0.064f);
    session.setViewConfig(view);
    session.start(settings(pin));
    ASSERT_TRUE(waitUntil([&] { return session.isStreaming(); }, 15s));

    auto engineGot = [&](const char* text) {
        return waitUntil([&] { return engine.log().find(text) != std::string::npos; }, 5s);
    };
    EXPECT_TRUE(engineGot("VIEW_CONFIG: left eye -52.0/45.0/41.0/-49.0"))
        << engine.log();
    EXPECT_TRUE(engineGot("right eye -45.0/52.0/41.0/-49.0"))
        << engine.log();
    EXPECT_TRUE(engineGot("IPD 64.0 mm")) << engine.log();

    // The same view again sends nothing new; a changed IPD goes out.
    session.setViewConfig(view);
    session.setViewConfig(fvp_client_protocol::buildViewConfigPayload(left, right, 0.066f));
    EXPECT_TRUE(engineGot("IPD 66.0 mm")) << engine.log();
}

TEST_F(SessionE2E, WrongPinIsRejectedAndNotRetried) {
    session.start(settings((pin + 1) % 1000000));
    ASSERT_TRUE(waitUntil([&] {
        for (const auto& e : drainEvents()) {
            if (e.type == ServerEvent::Type::PinRejected) return true;
        }
        return false;
    }, 15s)) << "no PinRejected (state " << static_cast<int>(session.state()) << ")";

    // Retrying would only walk into the engine's lockout.
    std::this_thread::sleep_for(2s);
    EXPECT_EQ(session.state(), SessionState::Disconnected);
    EXPECT_EQ(session.streamCount(), 0u);
    EXPECT_FALSE(fs::exists(fingerprintPath)) << "a server is pinned only after pairing succeeds";

    // The right PIN still pairs: one wrong attempt is far from the lockout.
    session.start(settings(pin));
    EXPECT_TRUE(waitUntil([&] { return session.isStreaming(); }, 15s));
}

TEST_F(SessionE2E, ReconnectsWithinTheEngineHoldAndStreamsAgain) {
    session.start(settings(pin));
    ASSERT_TRUE(waitUntil([&] { return session.isStreaming(); }, 15s));
    ASSERT_FALSE(collectFrames(1500ms).empty());

    // A Wi-Fi drop: the connection goes away without DISCONNECT. The engine
    // holds the session for 5 s and takes the same PIN again.
    session.dropConnectionForTest();
    ASSERT_TRUE(waitUntil([&] { return !session.isStreaming(); }, 5s));
    ASSERT_TRUE(waitUntil([&] { return session.isStreaming() && session.streamCount() == 2; }, 10s))
        << "did not stream again (state " << static_cast<int>(session.state()) << ")";

    const auto frames = collectFrames(3s);
    EXPECT_GE(frames.size(), 30u);
    if (!frames.empty()) EXPECT_TRUE(frames.front().keyframe);
    EXPECT_EQ(engine.pinCount(), 1u) << "the hold keeps the session's PIN";
}

TEST_F(SessionE2E, RefusesAServerThatDoesNotMatchThePinnedCertificate) {
    {
        std::ofstream out(fingerprintPath);
        out << std::string(64, '0') << "\n"; // some other PC was paired before
    }
    session.start(settings(pin));
    std::this_thread::sleep_for(3s);
    EXPECT_EQ(session.streamCount(), 0u);
    const auto state = session.state();
    EXPECT_TRUE(state == SessionState::Connecting || state == SessionState::Reconnecting)
        << "state " << static_cast<int>(state);
    EXPECT_FALSE(has(drainEvents(), ServerEvent::Type::PinRejected))
        << "the PIN must never be sent to an unpinned server";
}
