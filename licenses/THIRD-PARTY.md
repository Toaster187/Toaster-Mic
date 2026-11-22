# Drittanbieter-Komponenten

Toaster Mic 0.5 ist in Rust geschrieben und bindet folgende wesentliche Bibliotheken ein. Die vollständige, versionsgenaue Liste ergibt sich aus `Cargo.lock` (`cargo tree --format "{p} {l}"`).

| Komponente | Verwendung | Lizenz |
| --- | --- | --- |
| Slint | Oberfläche Windows und Android | Slint Royalty-free License 2.0 (Hinweis „Made with Slint“ im Info-Bereich beider Apps) |
| Skia (über Slint, Android) | Rendering der Android-Oberfläche | BSD-3-Clause |
| RustCrypto: `aes-gcm`, `p256`, `hmac`, `sha2` | Verschlüsselung, Schlüsselaustausch | MIT oder Apache-2.0 |
| `nnnoiseless` (RNNoise-Port) | Studio-Rauschunterdrückung | BSD-3-Clause |
| `realfft` / `rustfft` | GCC-PHAT im Studio-Modus | MIT oder Apache-2.0 |
| `windows`, `windows-core` | WASAPI, Audiositzungen, Registry | MIT oder Apache-2.0 |
| `tray-icon`, `muda` | Symbol im Infobereich | MIT oder Apache-2.0 |
| `ndk`, `android-activity`, `jni` | AAudio, NativeActivity, JNI | MIT oder Apache-2.0 |
| `serde`, `serde_json` | Einstellungen | MIT oder Apache-2.0 |
| Android NDK libc++ (statisch) | C++-Laufzeit für Skia | Apache-2.0 mit LLVM-Ausnahme |

Die Unicode-Daten (ICU4X, über Slint) stehen unter Unicode-3.0. Weitere kleine Abhängigkeiten nutzen MIT, Apache-2.0, Zlib, BSL-1.0 oder BSD-Lizenzen.
