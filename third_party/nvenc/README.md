# NVENC API header

`nvEncodeAPI.h` — NVIDIA Video Codec SDK 12.2 encoder API header, unmodified,
from [FFmpeg/nv-codec-headers](https://github.com/FFmpeg/nv-codec-headers)
tag `n12.2.72.0` (`include/ffnvcodec/nvEncodeAPI.h`).

License: MIT, © NVIDIA Corporation (the notice is at the top of the header).

The driver compiles against this header only. The NVENC runtime
(`nvEncodeAPI64.dll`) ships with the NVIDIA display driver and is loaded at
run time; `NvencEncoder` refuses to start if the driver reports an NVENC API
version older than 12.2.

To update: replace the file with the header from a newer tag and adjust
`driver/src/nvenc_config.h` if the API changed.
