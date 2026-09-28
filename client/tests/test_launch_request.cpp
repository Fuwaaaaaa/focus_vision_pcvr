// The launch request: PC address + PIN handed to the client at start (the
// companion's `am start --es fvp_server ... --es fvp_pin ...`).

#include <gtest/gtest.h>

#include "launch_request.h"

using fvp_launch::LaunchRequest;
using fvp_launch::parseLaunchRequest;

TEST(LaunchRequest, ServerAndPin) {
    LaunchRequest r;
    ASSERT_TRUE(parseLaunchRequest("server=192.168.1.10\npin=012345\n", r));
    EXPECT_EQ(r.server.ip, "192.168.1.10");
    EXPECT_EQ(r.server.port, fvp_session::DEFAULT_CONTROL_PORT);
    EXPECT_EQ(r.pin, 12345u) << "leading zeros belong to the PIN";
    EXPECT_EQ(r.udpBasePort, fvp_client_protocol::DEFAULT_UDP_BASE_PORT);
}

TEST(LaunchRequest, PortsAndWindowsLineEndings) {
    LaunchRequest r;
    ASSERT_TRUE(parseLaunchRequest("server=10.0.0.5:19944\r\npin=999999\r\nudp_port=19945\r\n", r));
    EXPECT_EQ(r.server.ip, "10.0.0.5");
    EXPECT_EQ(r.server.port, 19944);
    EXPECT_EQ(r.pin, 999999u);
    EXPECT_EQ(r.udpBasePort, 19945);
}

TEST(LaunchRequest, UnknownKeysAreIgnored) {
    LaunchRequest r;
    EXPECT_TRUE(parseLaunchRequest("version=2\nserver=10.0.0.5\npin=000000", r));
    EXPECT_EQ(r.pin, 0u);
}

TEST(LaunchRequest, MissingOrMalformedValuesRejectTheRequest) {
    LaunchRequest r;
    EXPECT_FALSE(parseLaunchRequest("", r));
    EXPECT_FALSE(parseLaunchRequest("server=10.0.0.5\n", r)) << "no PIN";
    EXPECT_FALSE(parseLaunchRequest("pin=123456\n", r)) << "no server";
    EXPECT_FALSE(parseLaunchRequest("server=pc.local\npin=123456", r)) << "IPv4 only";
    EXPECT_FALSE(parseLaunchRequest("server=10.0.0.5\npin=12345", r)) << "five digits";
    EXPECT_FALSE(parseLaunchRequest("server=10.0.0.5\npin=1234567", r));
    EXPECT_FALSE(parseLaunchRequest("server=10.0.0.5\npin=12a456", r));
    EXPECT_FALSE(parseLaunchRequest("server=10.0.0.5\npin=123456\nudp_port=65533", r))
        << "base+3 would overflow";
    EXPECT_FALSE(parseLaunchRequest("server=10.0.0.5\npin=123456\nudp_port=0", r));
}
