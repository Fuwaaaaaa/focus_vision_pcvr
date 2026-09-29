#pragma once

#include <cstdint>

/**
 * The controllers present themselves to SteamVR as Oculus Touch (Quest 2)
 * controllers, as ALVR and Virtual Desktop do: the Focus Vision controllers
 * have the same buttons, and games ship bindings for Touch — for an unknown
 * controller type they ship none, leaving every user to bind by hand. The
 * input profile and render models are the ones SteamVR's own Oculus driver
 * installs ({oculus}/...). Pure data, tested in tests/test_touch_profile.cpp.
 */
namespace fvp_touch {

// ControllerState::button_flags, as the headset sends them.
inline constexpr uint32_t kPrimaryPressed = 0x01;    // A (right) / X (left)
inline constexpr uint32_t kSecondaryPressed = 0x02;  // B (right) / Y (left)
inline constexpr uint32_t kMenuPressed = 0x04;
inline constexpr uint32_t kSystemPressed = 0x08;
inline constexpr uint32_t kStickClicked = 0x10;
inline constexpr uint32_t kTriggerTouched = 0x20;
inline constexpr uint32_t kStickTouched = 0x40;
inline constexpr uint32_t kGripTouched = 0x80;

inline constexpr const char* kControllerType = "oculus_touch";
inline constexpr const char* kInputProfilePath = "{oculus}/input/touch_profile.json";
inline constexpr const char* kTrackingSystemName = "oculus";
inline constexpr const char* kManufacturer = "Oculus";

/// What SteamVR shows and stores for each controller.
struct Identity {
    const char* serial;                // also Prop_AttachedDeviceId_String
    const char* modelNumber;
    const char* renderModel;
    const char* registeredDeviceType;
    const char* iconBase;              // + "_off.png", "_ready.png", ...
};

inline Identity identity(bool left) {
    if (left) {
        return {"1WMHH000X00000_Controller_Left", "Miramar (Left Controller)",
                "oculus_quest2_controller_left", "oculus/1WMHH000X00000_Controller_Left",
                "{oculus}/icons/rifts_left_controller"};
    }
    return {"1WMHH000X00000_Controller_Right", "Miramar (Right Controller)",
            "oculus_quest2_controller_right", "oculus/1WMHH000X00000_Controller_Right",
            "{oculus}/icons/rifts_right_controller"};
}

/// Paths of the face buttons, which differ per hand: X / Y on the left,
/// A / B on the right. Everything else is the same on both.
struct FaceButtons {
    const char* primaryClick;
    const char* secondaryClick;
};

inline FaceButtons faceButtons(bool left) {
    if (left) return {"/input/x/click", "/input/y/click"};
    return {"/input/a/click", "/input/b/click"};
}

inline constexpr const char* kSystemClick = "/input/system/click";
inline constexpr const char* kTriggerValue = "/input/trigger/value";
inline constexpr const char* kTriggerTouch = "/input/trigger/touch";
inline constexpr const char* kGripValue = "/input/grip/value";
inline constexpr const char* kGripTouch = "/input/grip/touch";
inline constexpr const char* kStickX = "/input/joystick/x";
inline constexpr const char* kStickY = "/input/joystick/y";
inline constexpr const char* kStickClick = "/input/joystick/click";
inline constexpr const char* kStickTouch = "/input/joystick/touch";
inline constexpr const char* kHaptic = "/output/haptic";

/// The button states SteamVR gets from the headset's flags.
struct Buttons {
    bool primary = false;
    bool secondary = false;
    bool system = false;
    bool stickClick = false;
    bool stickTouch = false;
    bool triggerTouch = false;
    bool gripTouch = false;
};

inline Buttons buttons(uint32_t flags) {
    Buttons b;
    b.primary = (flags & kPrimaryPressed) != 0;
    b.secondary = (flags & kSecondaryPressed) != 0;
    // Touch has one button there: on the left it is the menu button, on
    // the right the Oculus button. Focus Vision's menu and VIVE buttons
    // map onto it.
    b.system = (flags & (kMenuPressed | kSystemPressed)) != 0;
    b.stickClick = (flags & kStickClicked) != 0;
    b.stickTouch = (flags & kStickTouched) != 0;
    b.triggerTouch = (flags & kTriggerTouched) != 0;
    b.gripTouch = (flags & kGripTouched) != 0;
    return b;
}

}  // namespace fvp_touch
