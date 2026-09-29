// The controllers as SteamVR sees them: Oculus Touch (Quest 2), so games'
// Touch bindings apply. REGRESSION: the driver pointed SteamVR at
// {focus_vision_pcvr}/input/controller_profile.json, which does not exist,
// and set no controller type — SteamVR had no bindings for the controllers.

#include <gtest/gtest.h>
#include "touch_profile.h"

#include <string>

using namespace fvp_touch;

TEST(TouchProfile, UsesSteamVrsOwnTouchProfile) {
    EXPECT_STREQ(kControllerType, "oculus_touch");
    EXPECT_STREQ(kInputProfilePath, "{oculus}/input/touch_profile.json");
}

TEST(TouchProfile, EachHandHasItsOwnIdentity) {
    const Identity left = identity(true);
    const Identity right = identity(false);
    EXPECT_NE(std::string(left.serial), right.serial) << "SteamVR tells devices apart by serial";
    EXPECT_NE(std::string(left.serial).find("_Left"), std::string::npos);
    EXPECT_NE(std::string(right.serial).find("_Right"), std::string::npos);
    EXPECT_EQ(std::string(left.registeredDeviceType), std::string("oculus/") + left.serial);
    EXPECT_EQ(std::string(right.registeredDeviceType), std::string("oculus/") + right.serial);
    EXPECT_STREQ(left.renderModel, "oculus_quest2_controller_left");
    EXPECT_STREQ(right.renderModel, "oculus_quest2_controller_right");
}

TEST(TouchProfile, FaceButtonsAreXyOnTheLeftAndAbOnTheRight) {
    EXPECT_STREQ(faceButtons(true).primaryClick, "/input/x/click");
    EXPECT_STREQ(faceButtons(true).secondaryClick, "/input/y/click");
    EXPECT_STREQ(faceButtons(false).primaryClick, "/input/a/click");
    EXPECT_STREQ(faceButtons(false).secondaryClick, "/input/b/click");
}

TEST(TouchProfile, EachFlagReachesItsButton) {
    EXPECT_TRUE(buttons(kPrimaryPressed).primary);
    EXPECT_TRUE(buttons(kSecondaryPressed).secondary);
    EXPECT_TRUE(buttons(kStickClicked).stickClick);
    EXPECT_TRUE(buttons(kStickTouched).stickTouch);
    EXPECT_TRUE(buttons(kTriggerTouched).triggerTouch);
    EXPECT_TRUE(buttons(kGripTouched).gripTouch);

    const Buttons none = buttons(0);
    EXPECT_FALSE(none.primary || none.secondary || none.system || none.stickClick ||
                 none.stickTouch || none.triggerTouch || none.gripTouch);

    const Buttons onlyPrimary = buttons(kPrimaryPressed);
    EXPECT_FALSE(onlyPrimary.secondary || onlyPrimary.system) << "one flag, one button";
}

TEST(TouchProfile, MenuAndSystemBothPressTouchsSystemButton) {
    // Touch has one button in that place: menu on the left, Oculus on the
    // right.
    EXPECT_TRUE(buttons(kMenuPressed).system);
    EXPECT_TRUE(buttons(kSystemPressed).system);
    EXPECT_TRUE(buttons(kMenuPressed | kSystemPressed).system);
}
