#include <gtest/gtest.h>

#include "audio_sequence.h"

TEST(AudioSequence, InOrderPacketsMissNothing) {
    AudioSequence s;
    EXPECT_EQ(s.onPacket(100), 0);
    EXPECT_EQ(s.onPacket(101), 0);
    EXPECT_EQ(s.onPacket(102), 0);
}

TEST(AudioSequence, AGapIsCountedAndCapped) {
    AudioSequence s;
    s.onPacket(10);
    EXPECT_EQ(s.onPacket(13), 2) << "11 and 12 are missing";
    EXPECT_EQ(s.onPacket(14), 0);
    EXPECT_EQ(s.onPacket(114), AudioSequence::kMaxConcealed) << "a stall conceals at most 50 ms";
}

TEST(AudioSequence, LateAndRepeatedPacketsAreDropped) {
    AudioSequence s;
    s.onPacket(10);
    s.onPacket(13);
    EXPECT_EQ(s.onPacket(12), -1) << "concealed already";
    EXPECT_EQ(s.onPacket(13), -1) << "repeated";
    EXPECT_EQ(s.onPacket(14), 0) << "the late ones don't move the numbering";
}

TEST(AudioSequence, NumberingWrapsAround) {
    AudioSequence s;
    s.onPacket(65534);
    EXPECT_EQ(s.onPacket(65535), 0);
    EXPECT_EQ(s.onPacket(1), 1) << "0 is missing";
}

TEST(AudioSequence, ASenderStartingOverIsFollowed) {
    // A new session numbers from 0 again; without this every packet would
    // read as late until the numbering caught up.
    AudioSequence s;
    s.onPacket(30000);
    EXPECT_EQ(s.onPacket(0), 0);
    EXPECT_EQ(s.onPacket(1), 0);

    s.reset();
    EXPECT_EQ(s.onPacket(500), 0) << "after reset the first packet starts the numbering";
}
