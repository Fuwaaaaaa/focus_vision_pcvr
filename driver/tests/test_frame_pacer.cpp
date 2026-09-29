// Frame pacing in PostPresent: SteamVR times vsync itself, and the driver
// waits out each frame's slot at the refresh rate (as ALVR does). It
// replaced a thread that sent SteamVR vsync events of its own.

#include <gtest/gtest.h>
#include "frame_pacer.h"

using namespace std::chrono_literals;
using Clock = FramePacer::Clock;

TEST(FramePacer, PeriodFollowsTheRefreshRate) {
    EXPECT_NEAR(std::chrono::duration<double>(FramePacer(90.0).period()).count(), 1.0 / 90, 1e-9);
    EXPECT_NEAR(std::chrono::duration<double>(FramePacer(120.0).period()).count(), 1.0 / 120, 1e-9);
    EXPECT_NEAR(std::chrono::duration<double>(FramePacer(0.0).period()).count(), 1.0 / 90, 1e-9)
        << "no refresh rate: 90 Hz rather than no pacing";
}

TEST(FramePacer, FramesThatFinishEarlyWaitForTheirSlot) {
    FramePacer pacer(100.0);  // 10 ms slots
    const Clock::time_point t0 = Clock::now();
    EXPECT_EQ(pacer.deadline(t0), t0 + 10ms);
    // The next frame finished 3 ms into its slot: it waits until the slot
    // ends, on the grid rather than 10 ms from when it finished.
    EXPECT_EQ(pacer.deadline(t0 + 13ms), t0 + 20ms);
    EXPECT_EQ(pacer.deadline(t0 + 20ms), t0 + 30ms);
}

TEST(FramePacer, ALateFrameWaitsForNothingAndTheGridRestarts) {
    FramePacer pacer(100.0);
    const Clock::time_point t0 = Clock::now();
    pacer.deadline(t0);  // slot ends at 10 ms
    // A 45 ms stall: no wait, and the following frames are not let through
    // back to back to catch up on the missed slots.
    EXPECT_EQ(pacer.deadline(t0 + 55ms), t0 + 55ms);
    EXPECT_EQ(pacer.deadline(t0 + 56ms), t0 + 65ms);
}

TEST(FramePacer, ChangingTheRateStartsAFreshGrid) {
    FramePacer pacer(100.0);
    const Clock::time_point t0 = Clock::now();
    pacer.deadline(t0);
    pacer.setRefreshRate(50.0);
    EXPECT_EQ(pacer.deadline(t0 + 1ms), t0 + 21ms);
}
