#pragma once

#include <openvr_driver.h>
#include <cstdint>
#include <mutex>

extern "C" {
#include "streaming_engine.h"
}

/**
 * Controller tracked device (left or right hand), presented to SteamVR as an
 * Oculus Touch controller (touch_profile.h).
 * Receives input state from the Rust streaming engine (which gets it from the real HMD).
 */
class CControllerDevice : public vr::ITrackedDeviceServerDriver
{
public:
    CControllerDevice(bool isLeft);
    ~CControllerDevice() = default;

    // ITrackedDeviceServerDriver
    vr::EVRInitError Activate(uint32_t unObjectId) override;
    void Deactivate() override;
    void EnterStandby() override {}
    void* GetComponent(const char* pchComponentNameAndVersion) override { return nullptr; }
    void DebugRequest(const char*, char* pchResponseBuffer, uint32_t unResponseBufferSize) override;
    vr::DriverPose_t GetPose() override;

    void RunFrame();

    const char* GetSerialNumber() const;
    bool IsLeft() const { return m_isLeft; }

    /// Called by server_driver when SteamVR triggers haptic output.
    void TriggerHaptic(float duration_s, float frequency, float amplitude);

    vr::VRInputComponentHandle_t GetHapticHandle() const { return m_hHaptic; }

private:
    void SetupProperties();
    void CreateInputComponents();
    /// Push every button/axis in `state` to SteamVR.
    void UpdateInputs(const ControllerState& state);

    bool m_isLeft;
    uint32_t m_objectId = vr::k_unTrackedDeviceIndexInvalid;
    vr::PropertyContainerHandle_t m_propertyContainer = vr::k_ulInvalidPropertyContainer;
    // RunFrame writes the pose, GetPose (SteamVR's threads) reads it.
    std::mutex m_poseMutex;
    vr::DriverPose_t m_pose{};
    /// Inputs currently hold values from the engine (vs. released).
    bool m_inputsLive = false;

    // Input component handles
    vr::VRInputComponentHandle_t m_hTrigger = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hGrip = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hJoystickX = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hJoystickY = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hPrimary = vr::k_ulInvalidInputComponentHandle;   // X (left) / A (right)
    vr::VRInputComponentHandle_t m_hSecondary = vr::k_ulInvalidInputComponentHandle; // Y (left) / B (right)
    vr::VRInputComponentHandle_t m_hSystem = vr::k_ulInvalidInputComponentHandle;    // menu / system
    vr::VRInputComponentHandle_t m_hThumbstickClick = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hTriggerTouch = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hThumbstickTouch = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hGripTouch = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hHaptic = vr::k_ulInvalidInputComponentHandle;
};
