#include "net_compat.h" // first: winsock2.h must precede anything that pulls in windows.h
#include "video_receiver.h"
#include "xr_utils.h"

#include <cstring>
#include <vector>

namespace {

/// How long one recvfrom() may block, so stop() and frame timeouts are
/// noticed promptly.
constexpr int kRecvTimeoutMs = 20;

/// Large enough for the bursts an 80+ Mbps stream sends per frame.
constexpr int kSocketBufferBytes = 4 * 1024 * 1024;

/// Largest datagram the engine sends is 12 + 12 + 1200 bytes.
constexpr size_t kMaxPacket = 2048;

}  // namespace

bool VideoReceiver::start(uint16_t port, StatsReporter* stats, IdrCallback onIdrNeeded) {
    stop();
    if (!fvp_net::startup()) return false;

    const fvp_net::Socket s = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (s == fvp_net::kInvalidSocket) {
        LOGE("VideoReceiver: socket() failed");
        return false;
    }
    fvp_net::setIntOption(s, SOL_SOCKET, SO_RCVBUF, kSocketBufferBytes);
    fvp_net::setTimeoutOption(s, SO_RCVTIMEO, kRecvTimeoutMs);

    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    if (bind(s, reinterpret_cast<const sockaddr*>(&addr), sizeof(addr)) != 0) {
        LOGE("VideoReceiver: bind to UDP port %u failed", port);
        fvp_net::closeSocket(s);
        return false;
    }

    m_socket = s;
    m_onIdrNeeded = std::move(onIdrNeeded);
    {
        std::lock_guard<std::mutex> lock(m_assemblyMutex);
        m_assembler = FrameAssembler(stats);
        m_sessionActive = false;
    }
    {
        std::lock_guard<std::mutex> lock(m_queueMutex);
        m_frames.clear();
    }
    m_stop = false;
    m_thread = std::thread([this] { run(); });
    LOGI("VideoReceiver: listening on UDP port %u", port);
    return true;
}

void VideoReceiver::stop() {
    m_stop = true;
    if (m_thread.joinable()) m_thread.join();
    if (m_socket != fvp_net::kInvalidSocket) {
        fvp_net::closeSocket(m_socket);
        m_socket = fvp_net::kInvalidSocket;
    }
}

void VideoReceiver::beginSession(const std::string& serverIp) {
    in_addr server{};
    if (!fvp_net::parseIpv4(serverIp.c_str(), server)) {
        LOGE("VideoReceiver: bad server address %s", serverIp.c_str());
        return;
    }
    {
        std::lock_guard<std::mutex> lock(m_assemblyMutex);
        m_generation++;
        m_serverAddr = server;
        m_assembler.reset();
        m_sessionActive = true;
    }
    {
        std::lock_guard<std::mutex> lock(m_queueMutex);
        m_frames.clear();
    }
    m_lastPacket = Clock::now().time_since_epoch().count();
}

void VideoReceiver::endSession() {
    {
        std::lock_guard<std::mutex> lock(m_assemblyMutex);
        m_generation++;
        m_sessionActive = false;
    }
    std::lock_guard<std::mutex> lock(m_queueMutex);
    m_frames.clear();
}

bool VideoReceiver::popFrame(FrameAssembler::Frame& out) {
    std::lock_guard<std::mutex> lock(m_queueMutex);
    if (m_frames.empty()) return false;
    out = std::move(m_frames.front());
    m_frames.pop_front();
    return true;
}

void VideoReceiver::requireKeyframe() {
    FrameAssembler::Output out;
    {
        std::lock_guard<std::mutex> lock(m_assemblyMutex);
        if (!m_sessionActive) return;
        m_assembler.requireKeyframe(Clock::now(), out);
    }
    {
        std::lock_guard<std::mutex> lock(m_queueMutex);
        m_frames.clear();
    }
    if (out.requestIdr && m_onIdrNeeded) m_onIdrNeeded();
}

uint64_t VideoReceiver::framesDelivered() const {
    std::lock_guard<std::mutex> lock(m_assemblyMutex);
    return m_assembler.framesDelivered();
}

uint64_t VideoReceiver::framesLost() const {
    std::lock_guard<std::mutex> lock(m_assemblyMutex);
    return m_assembler.framesLost();
}

void VideoReceiver::run() {
    std::vector<uint8_t> buf(kMaxPacket);
    while (!m_stop) {
        sockaddr_in from{};
        socklen_t fromLen = sizeof(from);
        const int n = static_cast<int>(recvfrom(m_socket, reinterpret_cast<char*>(buf.data()),
                                                static_cast<int>(buf.size()), 0,
                                                reinterpret_cast<sockaddr*>(&from), &fromLen));
        const auto now = Clock::now();
        FrameAssembler::Output out;
        uint64_t generation = 0;
        {
            std::lock_guard<std::mutex> lock(m_assemblyMutex);
            if (!m_sessionActive) continue;
            generation = m_generation;
            if (n > 0 && from.sin_addr.s_addr == m_serverAddr.s_addr) {
                m_lastPacket = now.time_since_epoch().count();
                m_assembler.onPacket(buf.data(), static_cast<size_t>(n), now, out);
            }
            m_assembler.onTick(now, out);
        }

        if (!out.frames.empty()) {
            size_t dropped = 0;
            {
                std::lock_guard<std::mutex> lock(m_queueMutex);
                if (generation == m_generation) { // not superseded by a new session meanwhile
                    for (auto& frame : out.frames) m_frames.push_back(std::move(frame));
                    if (m_frames.size() > kMaxQueuedFrames) {
                        dropped = m_frames.size();
                        m_frames.clear();
                    }
                }
            }
            if (dropped > 0) {
                // Dropping any frame breaks the reference chain: drop them all
                // and restart from a keyframe.
                m_framesDropped += dropped;
                LOGW("VideoReceiver: decoder behind, dropped %zu queued frames", dropped);
                std::lock_guard<std::mutex> lock(m_assemblyMutex);
                m_assembler.requireKeyframe(now, out);
            }
        }
        if (out.requestIdr && m_onIdrNeeded) m_onIdrNeeded();
    }
}
