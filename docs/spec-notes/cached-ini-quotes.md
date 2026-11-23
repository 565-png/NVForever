# Cached INI file quotes

[observed: own authored native_description_28 public-script probe, 2026-11-12] The cached read of a quoted file template returned `Air %.0f`, preserving the percent placeholder without its surrounding file quotes. The `read` row completed; this is an API observation, not native-client shutdown/stability acceptance.

[observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + ShowOffNVSE 184 with the 10 official base masters, clean-room black-box script probe `core_21_cached_ini_quotes_stripping` (`GetINIString_Cached`/`ReloadIniCache`, reproducible via `#3678` / PR `#3679`, `tools/capture/nvse/`); no DLL or source consulted] Reading file values loaded into the cache removes one matching outer double-quote pair (`"hello"` -> `hello`, `""` -> `""` length `0`), leaves unmatched (`"unmatched` -> `"unmatched`) and internal (`a"b"c` -> `a"b"c`) quotes intact, and leaves a missing-key caller default (`"defQ"` -> `"defQ"`) unstripped (`Q=[hello]`, `U=["unmatched]`, `I=[a"b"c]`, `E=[](0)`, `M=["defQ"]`).

[hypothesis] Values already created/set in the cache and `HasINISetting` warmup retain their existing semantics; those unprobed boundaries remain NVForever regression expectations checked by `small_libs.rs::cached_ini_file_quotes_are_removed_without_changing_default_or_cached_values`.

[observed: change scope] This engine patch contains no Titans mod-source/template rewrite. The separate Titans `%z` source workaround is excluded; no full Titans compatibility claim follows from quote removal.
