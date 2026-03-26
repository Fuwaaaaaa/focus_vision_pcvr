#pragma once

#include <openvr_driver.h>
#include <cstdint>

/**
 * Controller tracked device (left or right hand).
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

private:
    void SetupProperties();
    void CreateInputComponents();
    void UpdateInputFromControllerState();

    bool m_isLeft;
    uint32_t m_objectId = vr::k_unTrackedDeviceIndexInvalid;
    vr::PropertyContainerHandle_t m_propertyContainer = vr::k_ulInvalidPropertyContainer;
    vr::DriverPose_t m_pose{};

    // Input component handles
    vr::VRInputComponentHandle_t m_hTrigger = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hGrip = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hJoystickX = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hJoystickY = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hA = vr::k_ulInvalidInputComponentHandle; // A or X
    vr::VRInputComponentHandle_t m_hB = vr::k_ulInvalidInputComponentHandle; // B or Y
    vr::VRInputComponentHandle_t m_hMenu = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hSystem = vr::k_ulInvalidInputComponentHandle;
    vr::VRInputComponentHandle_t m_hThumbstickClick = vr::k_ulInvalidInputComponentHandle;
};
