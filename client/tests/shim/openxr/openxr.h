#pragma once
// Host-test shim for <openxr/openxr.h>: just what xr_utils.h references, so
// hardware-independent client sources that include it (fec_decoder.cpp) build
// with a desktop toolchain. Only on the include path of client/tests.
#include <cstdint>

typedef int32_t XrResult;
#define XR_FAILED(result) ((result) < 0)
