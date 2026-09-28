#pragma once
// Socket portability for the client's networking code. The app runs on
// Android (POSIX sockets); the host tests also build the same code with MSVC
// (Winsock), so the C++ client can be run against the real engine on a PC
// (client/tests/test_session_e2e.cpp).

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#else
#include <arpa/inet.h>
#include <cerrno>
#include <fcntl.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

#include <cstdint>

namespace fvp_net {

#ifdef _WIN32
using Socket = SOCKET;
inline const Socket kInvalidSocket = INVALID_SOCKET;
#else
using Socket = int;
inline constexpr Socket kInvalidSocket = -1;
#endif

/// Winsock must be started once per process; POSIX needs nothing.
inline bool startup() {
#ifdef _WIN32
    static const bool ok = [] {
        WSADATA data;
        return WSAStartup(MAKEWORD(2, 2), &data) == 0;
    }();
    return ok;
#else
    return true;
#endif
}

inline void closeSocket(Socket s) {
#ifdef _WIN32
    closesocket(s);
#else
    ::close(s);
#endif
}

inline bool setNonBlocking(Socket s, bool on) {
#ifdef _WIN32
    u_long mode = on ? 1 : 0;
    return ioctlsocket(s, FIONBIO, &mode) == 0;
#else
    int flags = fcntl(s, F_GETFL, 0);
    if (flags < 0) return false;
    flags = on ? (flags | O_NONBLOCK) : (flags & ~O_NONBLOCK);
    return fcntl(s, F_SETFL, flags) == 0;
#endif
}

inline bool setIntOption(Socket s, int level, int name, int value) {
    return setsockopt(s, level, name, reinterpret_cast<const char*>(&value), sizeof(value)) == 0;
}

/// Bound a blocking recv / recvfrom (SO_RCVTIMEO) or send (SO_SNDTIMEO).
inline bool setTimeoutOption(Socket s, int name, int ms) {
#ifdef _WIN32
    DWORD t = static_cast<DWORD>(ms);
    return setsockopt(s, SOL_SOCKET, name, reinterpret_cast<const char*>(&t), sizeof(t)) == 0;
#else
    timeval tv{};
    tv.tv_sec = ms / 1000;
    tv.tv_usec = (ms % 1000) * 1000;
    return setsockopt(s, SOL_SOCKET, name, &tv, sizeof(tv)) == 0;
#endif
}

/// Parse a dotted-quad IPv4 address into `out` (network byte order).
inline bool parseIpv4(const char* ip, in_addr& out) {
    return inet_pton(AF_INET, ip, &out) == 1;
}

/// TCP connect to `ip:port`, giving up after `timeoutMs` (a wrong address
/// would otherwise block for the OS SYN timeout, 20 s or more). Returns a
/// connected, blocking socket, or kInvalidSocket.
inline Socket connectTcp(const char* ip, uint16_t port, int timeoutMs) {
    if (!startup()) return kInvalidSocket;
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    if (!parseIpv4(ip, addr.sin_addr)) return kInvalidSocket;

    Socket s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (s == kInvalidSocket) return kInvalidSocket;
    setNonBlocking(s, true);

    if (::connect(s, reinterpret_cast<const sockaddr*>(&addr), sizeof(addr)) != 0) {
#ifdef _WIN32
        const bool inProgress = WSAGetLastError() == WSAEWOULDBLOCK;
#else
        const bool inProgress = errno == EINPROGRESS;
#endif
        if (!inProgress) {
            closeSocket(s);
            return kInvalidSocket;
        }
#ifdef _WIN32
        fd_set writable, failed;
        FD_ZERO(&writable);
        FD_ZERO(&failed);
        FD_SET(s, &writable);
        FD_SET(s, &failed);
        timeval tv{};
        tv.tv_sec = timeoutMs / 1000;
        tv.tv_usec = (timeoutMs % 1000) * 1000;
        const int ready = select(0, nullptr, &writable, &failed, &tv);
        if (ready <= 0 || FD_ISSET(s, &failed)) {
            closeSocket(s);
            return kInvalidSocket;
        }
#else
        pollfd pfd{};
        pfd.fd = s;
        pfd.events = POLLOUT;
        if (poll(&pfd, 1, timeoutMs) <= 0) {
            closeSocket(s);
            return kInvalidSocket;
        }
#endif
        int soError = 0;
        socklen_t len = sizeof(soError);
        getsockopt(s, SOL_SOCKET, SO_ERROR, reinterpret_cast<char*>(&soError), &len);
        if (soError != 0) {
            closeSocket(s);
            return kInvalidSocket;
        }
    }
    setNonBlocking(s, false);
    return s;
}

}  // namespace fvp_net
