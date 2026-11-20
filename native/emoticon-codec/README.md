# Standalone animation decoder

`codec.cpp` is licensed GPL-3.0-or-later. It links libde265 1.1.3 (LGPL-3.0-or-later) and JBIG-KIT 2.2 (GPL-2.0-or-later). Full unmodified upstream sources, notices and manifests are in `source/`. The WeRC application and the helper bridge are not linked into this executable.

Rebuild from this directory in an x64 MSVC developer shell with CMake and Ninja:

```powershell
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 4
```

No network downloads or prebuilt library are required. The release uses a static MSVC runtime and scalar decoding, without AVX, system HEVC extensions or FFmpeg. Runtime components are found next to the application. The host pins the resulting executable digest.

Protocol v1 uses anonymous binary pipes. Five little-endian uint32 values: magic 0x45525657, version 1, width, height, frame count. Each frame contains length-prefixed JBIG alpha and HEVC picture blocks. Empty picture blocks are permitted because HEVC output is reordered. Dimensions are 1..1024, frames 1..120, cumulative pixels <=48,000,000 and input <=1MiB. Output repeats the header, then full RGBA frames in display order, then magic and decoded count. Transparent RGB is zeroed. Unsupported chroma/bit-depth/matrix, codec warnings, incomplete frames and extra input fail with nonzero exit. The host separately validates the wxgf envelope/checksum and retains original millisecond delays in the derived APNG. Output is released only after complete successful termination.

The source archive supplied alongside every binary includes this wrapper, build file, protocol documentation and all corresponding library source. The archive is the editable/relinkable source for this standalone component.
