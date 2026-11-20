# WeRC local changes to mp4parse 0.17.0

Upstream: https://github.com/mozilla/mp4parse-rust (MPL-2.0). Original copyright headers and license retained.

Add hvc1/hev1, the HEVC codec identity and bounded hvcC record parsing. All sample tables, byte ranges and original media bytes are preserved. This is container parsing, not a decoder. Tests and registry-only benchmark dependencies are excluded from the build manifest. The source modifications are included here for redistribution under MPL-2.0.
